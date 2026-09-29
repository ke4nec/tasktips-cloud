//! 浏览器会话接口（设计文档 §8.2）：Cookie 刷新、Origin 校验、no-store 与幂等登出。
//!
//! 需要数据库：未设置 `TASKTIPS_DATABASE_URL` 时跳过（CI 中必须提供）。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
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

const WEB_ORIGIN: &str = "https://app.example.test";
const ADMIN_ORIGIN: &str = "https://admin.example.test";
const WEB_PASSWORD: &str = "web-password-123";

struct WebApp {
    app: Router,
    web_email: String,
    admin_email: String,
    /// 未使用的第二个邀请：激活语义测试专用（对应 `invitation_token`）。
    invitation_email: String,
    invitation_token: String,
}

/// 未设置数据库时返回 `None`（CI 中必须提供）。
async fn web_app() -> Option<WebApp> {
    let database_url = std::env::var("TASKTIPS_DATABASE_URL").ok()?;
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
        .with_admin_origin(ADMIN_ORIGIN)
        .with_web_origin(WEB_ORIGIN),
    );

    let test_id = Uuid::new_v4();
    let admin_email = format!("web-admin-{test_id}@example.test");
    persistence
        .create_initial_admin(
            &admin_email,
            &admin_email,
            &hash_password("admin-password-123").expect("password should hash"),
        )
        .await
        .expect("admin should be created");
    let admin_token = admin_login_token(&app, &admin_email).await;
    let (web_email, invitation_token) = create_invitation(&app, &admin_token, &test_id, 1).await;
    // 登录/登出测试需要已激活账号：通过 web 激活接口完成（顺带端到端预热一次）。
    let activated_cookie = activate_cookie(&app, &invitation_token).await;
    assert!(!activated_cookie.is_empty());
    // 再留一份未使用邀请给激活语义测试。
    let (invitation_email, second_invitation) =
        create_invitation(&app, &admin_token, &test_id, 2).await;
    Some(WebApp {
        app,
        web_email,
        admin_email,
        invitation_email,
        invitation_token: second_invitation,
    })
}

#[tokio::test]
async fn web_activation_sets_cookie_and_token_stays_user_scoped() {
    let Some(web) = web_app().await else {
        assert!(std::env::var_os("CI").is_none());
        eprintln!("web_auth skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let invitation_token = &web.invitation_token;

    // 激活：Cookie 下发，响应体不含 refreshToken。
    let (status, body, headers) = web_request(
        &web.app,
        "POST",
        "/api/v1/web/auth/invitations/activate",
        None,
        Some(json!({
            "invitationToken": invitation_token,
            "password": WEB_PASSWORD,
            "deviceId": Uuid::new_v4()
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "activation response: {body}");
    assert!(body["accessToken"].as_str().is_some());
    assert!(body["expiresIn"].as_u64().is_some_and(|value| value >= 1));
    assert!(body.get("refreshToken").is_none());
    assert_no_store(&headers);
    web_refresh_cookie(&headers);

    // 激活即获得可用 access token，且仅限普通用户能力。
    let token = body["accessToken"].as_str().unwrap().to_owned();
    let (status, me) = bearer_request(&web.app, "GET", "/api/v1/me", &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["email"].as_str(), Some(web.invitation_email.as_str()));
    assert_eq!(me["role"].as_str(), Some("user"));
    let (status, _) = bearer_request(&web.app, "GET", "/api/v1/admin/users", &token, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn web_refresh_rotates_cookie_and_reuse_revokes_family() {
    let Some(web) = web_app().await else {
        assert!(std::env::var_os("CI").is_none());
        eprintln!("web_auth skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let invitation_token = &web.invitation_token;
    let first_cookie = activate_cookie(&web.app, invitation_token).await;

    // 首次刷新成功并轮换出新 Cookie，新 Cookie 立即可用。
    let (status, body, headers) = refresh(&web.app, &first_cookie).await;
    assert_eq!(status, StatusCode::OK, "refresh body: {body}");
    assert!(body["accessToken"].as_str().is_some());
    let rotated_cookie = web_refresh_cookie(&headers);
    let (status, _, headers) = refresh(&web.app, &rotated_cookie).await;
    assert_eq!(status, StatusCode::OK);
    let active_cookie = web_refresh_cookie(&headers);

    // 轮换宽限窗口内重放旧 Cookie：视为并发轮换，补发族内新令牌而非吊销。
    let (status, _, _) = refresh(&web.app, &first_cookie).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "replay inside the rotation grace window must succeed"
    );

    // 把旧 Cookie 对应令牌的轮换时间拨回窗口之外，重放按泄露处理：
    // 401 且全族吊销（活跃成员一并失效）。
    let retired_token = rotated_cookie
        .split_once('=')
        .expect("cookie should be a name=value pair")
        .1;
    let retired_hash = tasktips_api::auth::opaque_token_hash(retired_token).clone();
    let database_url = std::env::var("TASKTIPS_DATABASE_URL").unwrap();
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("test pool should connect");
    sqlx::query(
        "UPDATE refresh_tokens SET used_at = CURRENT_TIMESTAMP - INTERVAL '31 seconds' \
         WHERE token_hash = $1",
    )
    .bind(&retired_hash)
    .execute(&pool)
    .await
    .expect("grace window backdate should apply");
    let (status, _, _) = refresh(&web.app, &rotated_cookie).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = refresh(&web.app, &active_cookie).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn web_login_enforces_password_role_and_origin() {
    let Some(web) = web_app().await else {
        assert!(std::env::var_os("CI").is_none());
        eprintln!("web_auth skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };

    // 正确密码换 Cookie；错误密码与管理员账号都拒绝。
    let (status, _, headers) = login(&web.app, &web.web_email, WEB_PASSWORD, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_no_store(&headers);
    web_refresh_cookie(&headers);
    let (status, _, _) = login(&web.app, &web.web_email, "wrong-password-123", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let admin_email = &web.admin_email;
    let (status, _, _) = login(&web.app, admin_email, "admin-password-123", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Origin 与 Fetch Metadata：错误/缺失 Origin、跨站 sec-fetch-site 一律 403。
    let (status, _, headers) = login(
        &web.app,
        &web.web_email,
        WEB_PASSWORD,
        Some("https://evil.example.test"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_no_store(&headers);
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/web/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"email": web.web_email, "password": WEB_PASSWORD, "deviceId": Uuid::new_v4()})
                .to_string(),
        ))
        .expect("originless login request should build");
    let (status, _, _) = send(&web.app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/web/auth/refresh")
        .header(header::ORIGIN, WEB_ORIGIN)
        .header("sec-fetch-site", "cross-site")
        .body(Body::empty())
        .expect("cross-site refresh request should build");
    let (status, _, _) = send(&web.app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 非预期请求格式：未知字段 400（deny_unknown_fields），非 POST 方法 405。
    let (status, _, _) = web_request(
        &web.app,
        "POST",
        "/api/v1/web/auth/login",
        None,
        Some(json!({
            "email": web.web_email,
            "password": WEB_PASSWORD,
            "deviceId": Uuid::new_v4(),
            "refreshToken": "extraneous"
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = web_request(
        &web.app,
        "PUT",
        "/api/v1/web/auth/refresh",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn web_logout_revokes_cookie_device_and_stays_idempotent() {
    let Some(web) = web_app().await else {
        assert!(std::env::var_os("CI").is_none());
        eprintln!("web_auth skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };

    // 登出：按 Cookie 撤销设备会话并清理 Cookie；该 Cookie 之后刷新被拒绝。
    let first_session_cookie = login_cookie(&web.app, &web.web_email).await;
    let (status, _, headers) = web_request(
        &web.app,
        "POST",
        "/api/v1/web/auth/logout",
        Some(&first_session_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_no_store(&headers);
    let clear_cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("logout should clear the refresh cookie");
    assert!(clear_cookie.starts_with("tasktips_web_refresh=;"));
    assert!(clear_cookie.contains("Max-Age=0"));
    let (status, _, _) = refresh(&web.app, &first_session_cookie).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 重复登出（无 Cookie）依旧 204。
    let (status, _, _) = web_request(
        &web.app,
        "POST",
        "/api/v1/web/auth/logout",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 登出只撤销对应设备：另一设备的会话 Cookie 仍可刷新。
    let other_device_cookie = login_cookie(&web.app, &web.web_email).await;
    let (status, _, _) = web_request(
        &web.app,
        "POST",
        "/api/v1/web/auth/logout",
        Some(&other_device_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let third_cookie = login_cookie(&web.app, &web.web_email).await;
    let (status, _, _) = refresh(&web.app, &third_cookie).await;
    assert_eq!(status, StatusCode::OK);
}

async fn activate_cookie(app: &Router, invitation_token: &str) -> String {
    let (status, _, headers) = web_request(
        app,
        "POST",
        "/api/v1/web/auth/invitations/activate",
        None,
        Some(json!({
            "invitationToken": invitation_token,
            "password": WEB_PASSWORD,
            "deviceId": Uuid::new_v4()
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    web_refresh_cookie(&headers)
}

async fn login_cookie(app: &Router, email: &str) -> String {
    let (status, _, headers) = login(app, email, WEB_PASSWORD, None).await;
    assert_eq!(status, StatusCode::OK);
    web_refresh_cookie(&headers)
}

async fn login(
    app: &Router,
    email: &str,
    password: &str,
    origin_override: Option<&str>,
) -> (StatusCode, Value, HeaderMap) {
    web_request(
        app,
        "POST",
        "/api/v1/web/auth/login",
        None,
        Some(json!({"email": email, "password": password, "deviceId": Uuid::new_v4()})),
        origin_override,
    )
    .await
}

async fn refresh(app: &Router, cookie: &str) -> (StatusCode, Value, HeaderMap) {
    web_request(
        app,
        "POST",
        "/api/v1/web/auth/refresh",
        Some(cookie),
        None,
        None,
    )
    .await
}

/// 发送带 Web Origin 的请求；`origin_override` 用于跨站断言。
async fn web_request(
    app: &Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
    origin_override: Option<&str>,
) -> (StatusCode, Value, HeaderMap) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::ORIGIN, origin_override.unwrap_or(WEB_ORIGIN))
        .header("sec-fetch-site", "same-origin");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let payload = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    send(
        app,
        request
            .body(payload)
            .expect("web auth request should build"),
    )
    .await
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Value, HeaderMap) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("router should respond");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("body should collect");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body, headers)
}

async fn bearer_request(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let payload = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let (status, body, _) = send(
        app,
        request.body(payload).expect("bearer request should build"),
    )
    .await;
    (status, body)
}

fn assert_no_store(headers: &HeaderMap) {
    assert_eq!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
}

/// 校验 Cookie 属性并返回 `name=value` 对。
fn web_refresh_cookie(headers: &HeaderMap) -> String {
    let set_cookie = headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("web auth response should set the refresh cookie");
    assert!(set_cookie.starts_with("tasktips_web_refresh="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("Secure"));
    assert!(set_cookie.contains("SameSite=Strict"));
    assert!(set_cookie.contains("Path=/api/v1/web/auth"));
    assert!(!set_cookie.to_ascii_lowercase().contains("domain="));
    set_cookie
        .split(';')
        .next()
        .expect("cookie should have a value")
        .to_owned()
}

async fn admin_login_token(app: &Router, email: &str) -> String {
    let (status, body, _) = send(
        app,
        Request::builder()
            .method("POST")
            .uri("/api/v1/admin/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ADMIN_ORIGIN)
            .header("sec-fetch-site", "same-origin")
            .body(Body::from(
                json!({"email": email, "password": "admin-password-123", "deviceId": Uuid::new_v4()})
                    .to_string(),
            ))
            .expect("admin login request should build"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["accessToken"].as_str().unwrap().to_owned()
}

async fn create_invitation(
    app: &Router,
    admin_token: &str,
    test_id: &Uuid,
    ordinal: u8,
) -> (String, String) {
    let email = format!("web-user-{ordinal}-{test_id}@example.test");
    let (status, body, _) = send(
        app,
        Request::builder()
            .method("POST")
            .uri("/api/v1/admin/invitations")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
            .header(header::ORIGIN, ADMIN_ORIGIN)
            .header("sec-fetch-site", "same-origin")
            .body(Body::from(json!({"email": email}).to_string()))
            .expect("invitation request should build"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token = body["invitationToken"]
        .as_str()
        .expect("invitation token should exist")
        .to_owned();
    (email, token)
}

fn test_auth() -> AuthService {
    let signing_key = SigningKey::from_bytes(&[42; 32]);
    let document = signing_key.to_pkcs8_der().expect("test key should encode");
    let private_key = pem::encode(&pem::Pem::new("PRIVATE KEY", document.as_bytes()));
    AuthService::from_private_key_pem(
        private_key.as_bytes(),
        "tasktips-test".to_owned(),
        "tasktips-test-client".to_owned(),
        900,
        3_600,
    )
    .expect("test auth service should initialize")
}
