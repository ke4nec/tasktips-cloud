//! 登录暴力破解防护回归测试：per-IP 预算防"换邮箱绕过"（密码喷洒），
//! 以及管理端项目搜索的 LIKE 通配符字面量化。
//!
//! 需要数据库：未设置 `TASKTIPS_DATABASE_URL` 时跳过（CI 中必须提供）。

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
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

/// TEST-NET-3 地址：与真实流量隔离，测试间互不污染共享限流桶。
const SPRAY_CLIENT_IP: &str = "203.0.113.7";

#[tokio::test]
async fn login_password_spraying_from_one_ip_hits_rate_limit() {
    let Some(app) = login_app().await else {
        assert!(std::env::var_os("CI").is_none());
        eprintln!("auth_rate_limit skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };

    // 预置共享限流桶到阈值边缘（119/120）：真实 Argon2 校验在 debug 构建下
    // 约每秒一次，直接打满 120 次会超过 1 分钟窗口导致计数被重置，因此
    // 从数据库桶侧预置，只需两次真实登录即可确定性验证。
    let database_url = std::env::var("TASKTIPS_DATABASE_URL").unwrap();
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    let bucket = format!("login-ip:{SPRAY_CLIENT_IP}");
    for _ in 0..119 {
        assert!(
            persistence
                .consume_distributed_rate_limit(&bucket, 120, time::Duration::minutes(1))
                .await
                .expect("rate limit bucket should consume"),
            "preset consumes must stay under the limit"
        );
    }

    // 同一来源地址、每次换邮箱：第 120 次仍在预算内（401 密码错误），
    // 第 121 次即使换新邮箱也必须被 per-IP 预算拦截。
    let first = spray_login(&app, "spray-first@example.test").await;
    assert_eq!(
        first,
        StatusCode::UNAUTHORIZED,
        "first attempt passes the budget"
    );
    let second = spray_login(&app, "spray-rotated@example.test").await;
    assert_eq!(
        second,
        StatusCode::TOO_MANY_REQUESTS,
        "rotating the email must not dodge the per-IP budget"
    );

    // 其他来源地址不受该桶影响。
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", "203.0.113.8")
        .body(Body::from(
            json!({
                "email": "other-source@example.test",
                "password": "whatever-password",
                "deviceId": Uuid::new_v4()
            })
            .to_string(),
        ))
        .expect("control request should build");
    let response = app.oneshot(request).await.expect("router should respond");
    assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

async fn spray_login(app: &Router, email: &str) -> StatusCode {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", SPRAY_CLIENT_IP)
        .body(Body::from(
            json!({
                "email": email,
                "password": "spray-password-123",
                "deviceId": Uuid::new_v4()
            })
            .to_string(),
        ))
        .expect("login request should build");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("router should respond");
    response.status()
}

#[tokio::test]
async fn admin_project_search_matches_percent_literally() {
    let Some((app, admin_email)) = search_app().await else {
        assert!(std::env::var_os("CI").is_none());
        eprintln!("auth_rate_limit skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };

    // 造一个名字含字面 % 的项目：admin 建用户 → 用户登录 → 建项目。
    let admin_token = admin_login(&app, &admin_email).await;
    let user_email = format!("search-user-{}@example.test", Uuid::new_v4());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/users")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://admin.example.test")
        .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
        .body(Body::from(
            json!({"email": user_email, "password": "user-password-123"}).to_string(),
        ))
        .expect("create user request should build");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::CREATED);

    let user_token = user_login(&app, &user_email).await;
    let project_request = Request::builder()
        .method("POST")
        .uri("/api/v1/projects")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {user_token}"))
        .body(Body::from(
            json!({"name": format!("discount-50%-{}", &user_email[..8])}).to_string(),
        ))
        .expect("project request should build");
    let response = app
        .clone()
        .oneshot(project_request)
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::CREATED);

    // 搜索 "%" 只能命中名字含字面 % 的项目，而不是全部项目。
    let request = Request::builder()
        .uri("/api/v1/admin/projects?search=%25")
        .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
        .body(Body::empty())
        .expect("search request should build");
    let response = app.oneshot(request).await.expect("router should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("search body should read");
    let items = serde_json::from_slice::<Value>(&body)
        .expect("search body should be json")
        .get("items")
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = items
        .as_array()
        .expect("items should be an array")
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert!(!names.is_empty(), "literal-percent project should match");
    assert!(
        names.iter().all(|name| name.contains('%')),
        "all matches must contain a literal percent, got {names:?}"
    );
}

async fn login_app() -> Option<Router> {
    let database_url = std::env::var("TASKTIPS_DATABASE_URL").ok()?;
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");
    Some(build_application_router(AppState::new(
        Readiness::unavailable(),
        Some(persistence),
        Some(test_auth()),
    )))
}

async fn search_app() -> Option<(Router, String)> {
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
        .with_admin_origin("https://admin.example.test"),
    );
    let admin_email = format!("search-admin-{}@example.test", Uuid::new_v4());
    persistence
        .create_initial_admin(
            &admin_email,
            &admin_email,
            &hash_password("admin-password-123").expect("password should hash"),
        )
        .await
        .expect("admin should be created");
    Some((app, admin_email))
}

async fn admin_login(app: &Router, email: &str) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://admin.example.test")
        .body(Body::from(
            json!({
                "email": email,
                "password": "admin-password-123",
                "deviceId": Uuid::new_v4()
            })
            .to_string(),
        ))
        .expect("admin login request should build");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("admin login body should read");
    serde_json::from_slice::<Value>(&body).expect("admin login body should be json")["accessToken"]
        .as_str()
        .expect("accessToken should exist")
        .to_owned()
}

async fn user_login(app: &Router, email: &str) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", "203.0.113.9")
        .body(Body::from(
            json!({
                "email": email,
                "password": "user-password-123",
                "deviceId": Uuid::new_v4()
            })
            .to_string(),
        ))
        .expect("user login request should build");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("router should respond");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "user login should succeed"
    );
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("user login body should read");
    serde_json::from_slice::<Value>(&body).expect("user login body should be json")["accessToken"]
        .as_str()
        .expect("accessToken should exist")
        .to_owned()
}

fn test_auth() -> AuthService {
    let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
    let document = signing_key.to_pkcs8_der().expect("test key should encode");
    let private_key = pem::encode(&pem::Pem::new("PRIVATE KEY", document.as_bytes()));
    AuthService::from_private_key_pem(
        private_key.as_bytes(),
        "issuer".to_owned(),
        "audience".to_owned(),
        900,
        3600,
    )
    .expect("auth service should build")
}
