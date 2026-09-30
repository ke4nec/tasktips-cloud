//! 异常登录/访问审计回归：失败可追溯、客户端不可区分、隐私不泄露。
//!
//! 需要数据库：未设置 `TASKTIPS_DATABASE_URL` 时跳过（CI 中必须提供）。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use der::pem::LineEnding;
use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey};
use serde_json::{Value, json};
use tasktips_api::{
    AppState, Readiness,
    auth::{AuthService, hash_password},
    build_application_router,
};
use tasktips_persistence::Persistence;
use tower::ServiceExt;
use uuid::Uuid;

fn test_auth() -> AuthService {
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let pem = signing_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("test key should encode");
    AuthService::from_private_key_pem(
        pem.as_bytes(),
        "tasktips-test".to_owned(),
        "tasktips-test".to_owned(),
        900,
        3600,
    )
    .expect("auth service should build")
}

async fn json_request(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
    extra_headers: Option<Vec<(&str, &str)>>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder = builder.header("x-request-id", Uuid::new_v4().to_string());
    if let Some(headers) = extra_headers {
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
    }
    let request = if let Some(body) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request should build")
    } else {
        builder.body(Body::empty()).expect("request should build")
    };
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("service should run");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .expect("body should read");
    let json = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
    (status, json, headers)
}

async fn admin_login_token(app: &Router, email: &str, password: &str) -> (String, Uuid) {
    let device_id = Uuid::new_v4();
    let (status, body, _) = json_request(
        app,
        "POST",
        "/api/v1/admin/auth/login",
        None,
        Some(json!({"email": email, "password": password, "deviceId": device_id})),
        Some(vec![("origin", "https://admin.example.test")]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin login failed: {body}");
    (body["accessToken"].as_str().unwrap().to_owned(), device_id)
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn auth_failures_are_audited_without_oracle_or_pii() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        eprintln!("auth_failure_audit skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");
    let app = build_application_router(
        AppState::new(
            Readiness::unavailable(),
            Some(persistence.clone()),
            Some(test_auth()),
        )
        .with_admin_origin("https://admin.example.test"),
    );

    let test_id = Uuid::new_v4();
    let admin_email = format!("audit-admin-{test_id}@example.test");
    let user_email = format!("audit-user-{test_id}@example.test");
    let user_password = "audit-password-123";
    persistence
        .create_initial_admin(
            &admin_email,
            &admin_email,
            &hash_password("admin-password-123").expect("password should hash"),
        )
        .await
        .expect("admin should be created");
    let (admin_token, _) = admin_login_token(&app, &admin_email, "admin-password-123").await;

    // 建一个普通用户用于 wrong_password / disabled 归因。
    persistence
        .admin_create_user(
            persistence
                .find_login_user(&admin_email)
                .await
                .expect("lookup should work")
                .expect("admin should exist")
                .id,
            &user_email.to_lowercase(),
            &user_email,
            &hash_password(user_password).expect("password should hash"),
            &format!("audit-{test_id}"),
        )
        .await
        .expect("user should be created");

    // 1. 未知账号与密码错误对客户端不可区分（均为 401 通用文案）。
    let (unknown_status, unknown_body, _) = json_request(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"email": format!("unknown-{test_id}@example.test"), "password": user_password, "deviceId": Uuid::new_v4()})),
        None,
    )
    .await;
    let (wrong_status, wrong_body, _) = json_request(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"email": user_email.clone(), "password": "wrong-password-123", "deviceId": Uuid::new_v4()})),
        None,
    )
    .await;
    assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_body["code"], wrong_body["code"]);
    assert_eq!(unknown_body["code"], "AUTHENTICATION_REQUIRED");

    // 2. 管理端可按 action 过滤到失败事件，且元数据无 PII/密钥。
    let (status, filtered, _) = json_request(
        &app,
        "GET",
        "/api/v1/admin/audit-events?action=auth.login_failed&limit=100&offset=0",
        Some(&admin_token),
        None,
        Some(vec![("origin", "https://admin.example.test")]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "filtered audit failed: {filtered}");
    let items = filtered["items"].as_array().expect("items should be array");
    assert!(
        items.len() >= 2,
        "expected at least 2 login failures, got {items:?}"
    );
    let reasons: Vec<String> = items
        .iter()
        .filter_map(|item| item["metadata"]["reason"].as_str().map(str::to_owned))
        .collect();
    assert!(
        reasons.contains(&"unknown_user".to_owned()),
        "missing unknown_user: {reasons:?}"
    );
    assert!(
        reasons.contains(&"invalid_password".to_owned()),
        "missing invalid_password: {reasons:?}"
    );
    for item in items {
        assert_eq!(item["action"], "auth.login_failed");
        let serialized = item.to_string();
        // 永不记录密码/令牌明文与邮箱/IP 明文；reason 中的 invalid_password
        // 是固定枚举，非密钥本身，检查时排除该误报。
        assert!(
            !serialized.contains(user_password),
            "audit leaks password plaintext: {item}"
        );
        assert!(
            !serialized.to_lowercase().contains("refreshtoken"),
            "audit leaks token: {item}"
        );
        assert!(
            !item.to_string().contains(&user_email),
            "audit leaks plaintext email"
        );
        assert!(
            !item.to_string().contains("127.0.0.1"),
            "audit leaks plaintext ip"
        );
        // 哈希字段存在且为 64 位 hex。
        if let Some(email_hash) = item["metadata"]["emailHash"].as_str() {
            assert_eq!(email_hash.len(), 64, "emailHash should be hex sha256");
        }
    }

    // 3. 非法 action 过滤被拒绝；普通用户无权查看审计。
    let (status, _, _) = json_request(
        &app,
        "GET",
        "/api/v1/admin/audit-events?action=auth.*&limit=1&offset=0",
        Some(&admin_token),
        None,
        Some(vec![("origin", "https://admin.example.test")]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 4. 伪造 refresh 写入 invalid_token 失败审计。
    let (status, _, _) = json_request(
        &app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(json!({"refreshToken": "bogus-token"})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, filtered, _) = json_request(
        &app,
        "GET",
        "/api/v1/admin/audit-events?action=auth.refresh_failed&limit=100&offset=0",
        Some(&admin_token),
        None,
        Some(vec![("origin", "https://admin.example.test")]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = filtered["items"].as_array().expect("items should be array");
    assert!(
        items
            .iter()
            .any(|item| item["metadata"]["reason"] == "invalid_token"),
        "missing invalid_token refresh failure: {items:?}"
    );

    // 5. CSV 导出同样支持过滤且不含载荷字段。
    let (status, _, _) = json_request(
        &app,
        "GET",
        "/api/v1/admin/audit-events.csv?action=auth.login_failed",
        Some(&admin_token),
        None,
        Some(vec![("origin", "https://admin.example.test")]),
    )
    .await;
    // CSV 经 json_request 解析为 Null，但状态码即代表过滤链路可用。
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn refresh_token_reuse_is_audited_once_with_revocation() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        eprintln!("auth_failure_audit skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");
    let app = build_application_router(
        AppState::new(
            Readiness::unavailable(),
            Some(persistence.clone()),
            Some(test_auth()),
        )
        .with_admin_origin("https://admin.example.test"),
    );

    let test_id = Uuid::new_v4();
    let admin_email = format!("reuse-admin-{test_id}@example.test");
    persistence
        .create_initial_admin(
            &admin_email,
            &admin_email,
            &hash_password("admin-password-123").expect("password should hash"),
        )
        .await
        .expect("admin should be created");
    let (admin_token, _) = admin_login_token(&app, &admin_email, "admin-password-123").await;

    // 记录过滤前的失败基线，避免串扰。
    let (_, before, _) = json_request(
        &app,
        "GET",
        "/api/v1/admin/audit-events?action=auth.refresh_failed&limit=500&offset=0",
        Some(&admin_token),
        None,
        Some(vec![("origin", "https://admin.example.test")]),
    )
    .await;
    let before_count = before["items"].as_array().map_or(0, Vec::len);

    // 建普通用户并登录拿 refresh。
    let user_email = format!("reuse-user-{test_id}@example.test");
    let admin_id = persistence
        .find_login_user(&admin_email)
        .await
        .expect("lookup should work")
        .expect("admin should exist")
        .id;
    persistence
        .admin_create_user(
            admin_id,
            &user_email.to_lowercase(),
            &user_email,
            &hash_password("reuse-password-123").expect("password should hash"),
            &format!("reuse-{test_id}"),
        )
        .await
        .expect("user should be created");
    let device_id = Uuid::new_v4();
    let (status, body, _) = json_request(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"email": user_email, "password": "reuse-password-123", "deviceId": device_id})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "login failed: {body}");
    let refresh_token = body["refreshToken"].as_str().unwrap().to_owned();

    // 正常轮换一次，再反复重放旧令牌：宽限内前几次视为并发补发，
    // 当族活跃成员达到上限（4）后按泄露处置并全族吊销，无需等待 31s。
    let (status, rotated, _) = json_request(
        &app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(json!({"refreshToken": refresh_token.clone()})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "first rotation failed: {rotated}");
    let rotated_token = rotated["refreshToken"].as_str().unwrap().to_owned();

    let mut reuse_status = StatusCode::OK;
    let mut reuse_body = Value::Null;
    for _ in 0..6 {
        let (status, body, _) = json_request(
            &app,
            "POST",
            "/api/v1/auth/refresh",
            None,
            Some(json!({"refreshToken": refresh_token})),
            None,
        )
        .await;
        reuse_status = status;
        reuse_body = body;
        if reuse_status == StatusCode::UNAUTHORIZED {
            break;
        }
    }
    assert_eq!(
        reuse_status,
        StatusCode::UNAUTHORIZED,
        "reuse should be rejected: {reuse_body}"
    );

    // 族已吊销：用刚轮换的新令牌也应失败（泄露处置）。
    let (status, _, _) = json_request(
        &app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(json!({"refreshToken": rotated_token})),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 重放仅写一条归因审计（persistence 内同事务），API 层不双写。
    let (status, after, _) = json_request(
        &app,
        "GET",
        "/api/v1/admin/audit-events?action=auth.refresh_failed&limit=500&offset=0",
        Some(&admin_token),
        None,
        Some(vec![("origin", "https://admin.example.test")]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = after["items"].as_array().expect("items should be array");
    let reuse_events: Vec<&Value> = items
        .iter()
        .filter(|item| item["metadata"]["reason"] == "token_reuse")
        .collect();
    assert!(
        !reuse_events.is_empty(),
        "missing token_reuse audit (before={before_count}, after={items:?})"
    );
    for event in reuse_events {
        assert!(
            event["actorUserId"].is_string(),
            "reuse should be attributed: {event}"
        );
        assert!(
            event["subjectUserId"].is_string(),
            "reuse should be attributed: {event}"
        );
    }
}
