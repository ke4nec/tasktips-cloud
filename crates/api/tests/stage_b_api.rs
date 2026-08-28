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

#[tokio::test]
async fn stage_b_http_workflow_enforces_isolation_and_rotation() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "TASKTIPS_DATABASE_URL is required for stage_b_api in CI"
        );
        eprintln!("stage_b_api skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");
    let auth = test_auth();
    let app = build_application_router(
        AppState::new(
            Readiness::unavailable(),
            Some(persistence.clone()),
            Some(auth),
        )
        .with_admin_origin("https://admin.example.test"),
    );

    let test_id = Uuid::new_v4();
    let admin_email = format!("api-admin-{test_id}@example.test");
    let admin_record = persistence
        .create_initial_admin(
            &admin_email,
            &admin_email,
            &hash_password("admin-password-123").expect("password should hash"),
        )
        .await
        .expect("admin should be created");
    let admin = admin_login(
        &app,
        &admin_email,
        "admin-password-123",
        Uuid::new_v4(),
        admin_record.id,
    )
    .await;
    assert_admin_refresh_cookie(&app, &admin_email).await;
    let user_one = invite_and_activate(&app, &admin.access_token, test_id, 1).await;
    let user_two = invite_and_activate(&app, &admin.access_token, test_id, 2).await;

    assert_project_http_isolation(&app, &user_one, &user_two).await;
    assert_device_http_isolation(&app, &user_one, &user_two).await;
    assert_admin_http_boundary(&app, &admin, &user_one, &user_two).await;
    assert_refresh_http_reuse(&app, &user_one).await;
    assert_logout_http_revokes_device_tokens(&app, &user_two).await;
    assert_device_http_revocation(&app, &user_two).await;
    assert_account_purge_requires_reauth(&app, &admin, &user_one).await;
    assert_account_disable(&app, &persistence, &admin, &user_one).await;
}

async fn assert_project_http_isolation(app: &Router, user_one: &Tokens, user_two: &Tokens) {
    let (status, project_one) = json_request(
        app,
        "POST",
        "/api/v1/projects",
        Some(&user_one.access_token),
        Some(json!({"name": "User one"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, project_two) = json_request(
        app,
        "POST",
        "/api/v1/projects",
        Some(&user_two.access_token),
        Some(json!({"name": "User two"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(project_one["id"], project_two["id"]);

    let (status, error) = json_request(
        app,
        "GET",
        &format!("/api/v1/projects/{}", project_two["id"].as_str().unwrap()),
        Some(&user_one.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], "PROJECT_NOT_FOUND");

    for method in ["PATCH", "POST"] {
        let path = if method == "PATCH" {
            format!("/api/v1/projects/{}", project_two["id"].as_str().unwrap())
        } else {
            format!(
                "/api/v1/projects/{}/disable",
                project_two["id"].as_str().unwrap()
            )
        };
        let body = (method == "PATCH").then(|| json!({"name": "Unauthorized"}));
        let (status, error) =
            json_request(app, method, &path, Some(&user_one.access_token), body).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error["code"], "PROJECT_NOT_FOUND");
    }

    let (status, projects) = json_request(
        app,
        "GET",
        "/api/v1/projects",
        Some(&user_one.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects["items"].as_array().unwrap().len(), 1);
    assert_eq!(projects["items"][0]["id"], project_one["id"]);
}

async fn assert_device_http_isolation(app: &Router, user_one: &Tokens, user_two: &Tokens) {
    let (status, registered) = json_request(
        app,
        "POST",
        "/api/v1/devices/register",
        Some(&user_one.access_token),
        Some(json!({
            "deviceId": user_one.device_id,
            "displayName": "User one device",
            "platform": "linux",
            "appVersion": "1.0.0"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(registered["id"], user_one.device_id.to_string());

    let (status, devices) = json_request(
        app,
        "GET",
        "/api/v1/devices",
        Some(&user_one.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(devices["items"].as_array().unwrap().len(), 1);
    assert_eq!(devices["items"][0]["id"], user_one.device_id.to_string());

    let (status, error) = json_request(
        app,
        "PATCH",
        &format!("/api/v1/devices/{}", user_two.device_id),
        Some(&user_one.access_token),
        Some(json!({"displayName": "Unauthorized"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], "NOT_FOUND");

    let (status, error) = json_request(
        app,
        "POST",
        &format!("/api/v1/devices/{}/revoke", user_two.device_id),
        Some(&user_one.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], "NOT_FOUND");
}

#[allow(clippy::too_many_lines)]
async fn assert_admin_http_boundary(
    app: &Router,
    admin: &Tokens,
    user_one: &Tokens,
    user_two: &Tokens,
) {
    let (status, _) = json_request(
        app,
        "GET",
        "/api/v1/admin/users",
        Some(&user_one.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = json_request(
        app,
        "GET",
        "/api/v1/projects",
        Some(&admin.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    for (method, path, body) in [
        (
            "POST",
            "/api/v1/admin/invitations".to_owned(),
            Some(json!({"email": "unauthorized@example.test"})),
        ),
        (
            "GET",
            format!("/api/v1/admin/users/{}/projects", user_two.user_id),
            None,
        ),
        (
            "GET",
            format!("/api/v1/admin/users/{}/devices", user_two.user_id),
            None,
        ),
        ("GET", "/api/v1/admin/projects".to_owned(), None),
        ("GET", "/api/v1/admin/devices".to_owned(), None),
    ] {
        let (status, error) =
            json_request(app, method, &path, Some(&user_one.access_token), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(error["code"], "AUTHORIZATION_DENIED");
    }

    let (status, users) = json_request(
        app,
        "GET",
        "/api/v1/admin/users",
        Some(&admin.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let serialized = users.to_string();
    assert!(!serialized.contains("passwordHash"));
    assert!(!serialized.contains("payload"));
    assert!(!serialized.contains("objectKey"));
    assert!(!serialized.contains("downloadUrl"));

    for resource in ["projects", "devices"] {
        let (status, metadata) = json_request(
            app,
            "GET",
            &format!("/api/v1/admin/users/{}/{resource}", user_one.user_id),
            Some(&admin.access_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(metadata["items"].as_array().unwrap().len(), 1);
        let serialized = metadata.to_string();
        assert!(!serialized.contains("payload"));
        assert!(!serialized.contains("objectKey"));
        assert!(!serialized.contains("downloadUrl"));
        assert!(!serialized.contains("credential"));
    }

    for resource in ["projects", "devices"] {
        let (status, first_page) = json_request(
            app,
            "GET",
            &format!("/api/v1/admin/{resource}?limit=1&offset=0"),
            Some(&admin.access_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first_page["items"].as_array().unwrap().len(), 1);
        assert_eq!(first_page["hasMore"], true);
        assert_eq!(first_page["nextOffset"], 1);
        let first_id = first_page["items"][0]["id"].clone();
        let serialized = first_page.to_string();
        assert!(!serialized.contains("payload"));
        assert!(!serialized.contains("contentHash"));
        assert!(!serialized.contains("objectKey"));
        assert!(!serialized.contains("downloadUrl"));
        assert!(!serialized.contains("credential"));

        let (status, second_page) = json_request(
            app,
            "GET",
            &format!("/api/v1/admin/{resource}?limit=1&offset=1"),
            Some(&admin.access_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second_page["items"].as_array().unwrap().len(), 1);
        assert_ne!(second_page["items"][0]["id"], first_id);
    }

    let (status, overview) = json_request(
        app,
        "GET",
        "/api/v1/admin/overview",
        Some(&admin.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "overview response: {overview}");
    assert!(overview.get("payloadBytes").is_some());
    assert!(!overview.to_string().contains("objectKey"));
    for path in [
        "/api/v1/admin/sync-attempts?limit=1&offset=0",
        "/api/v1/admin/audit-events?limit=1&offset=0",
        "/api/v1/admin/restores?limit=1&offset=0",
        "/api/v1/admin/jobs?limit=1&offset=0",
    ] {
        let (status, metadata) =
            json_request(app, "GET", path, Some(&admin.access_token), None).await;
        assert_eq!(status, StatusCode::OK);
        let serialized = metadata.to_string();
        assert!(!serialized.contains("contentHash"));
        assert!(!serialized.contains("objectKey"));
        assert!(!serialized.contains("downloadUrl"));
    }
    let (status, trends) = json_request(
        app,
        "GET",
        "/api/v1/admin/metrics/trends?days=30",
        Some(&admin.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trends["days"], 30);
    assert!(!trends.to_string().contains("contentHash"));
}

async fn assert_refresh_http_reuse(app: &Router, user_one: &Tokens) {
    let (status, rotated) = json_request(
        app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(json!({"refreshToken": user_one.refresh_token})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rotated_refresh = rotated["refreshToken"].as_str().unwrap().to_owned();
    let (status, _) = json_request(
        app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(json!({"refreshToken": user_one.refresh_token})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = json_request(
        app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(json!({"refreshToken": rotated_refresh})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

async fn assert_device_http_revocation(app: &Router, user_two: &Tokens) {
    let (status, _) = json_request(
        app,
        "POST",
        &format!("/api/v1/devices/{}/revoke", user_two.device_id),
        Some(&user_two.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, revoked) =
        json_request(app, "GET", "/api/v1/me", Some(&user_two.access_token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(revoked["code"], "DEVICE_REVOKED");
}

async fn assert_logout_http_revokes_device_tokens(app: &Router, user_two: &Tokens) {
    let (status, body) = json_request(
        app,
        "POST",
        "/api/v1/auth/logout",
        Some(&user_two.access_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "logout response: {body}");
    let (status, error) = json_request(
        app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(json!({"refreshToken": user_two.refresh_token})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error["code"], "AUTHENTICATION_REQUIRED");
}

async fn assert_account_disable(
    app: &Router,
    persistence: &Persistence,
    admin: &Tokens,
    user_one: &Tokens,
) {
    let pending_email = format!("pending-{}@example.test", Uuid::new_v4());
    let (status, _) = json_request(
        app,
        "POST",
        "/api/v1/admin/invitations",
        Some(&admin.access_token),
        Some(json!({"email": pending_email})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pending_user_id = persistence
        .find_login_user(&pending_email)
        .await
        .expect("pending user lookup should succeed")
        .expect("pending user should exist")
        .id;
    let (status, error) = json_request(
        app,
        "POST",
        &format!("/api/v1/admin/users/{pending_user_id}/enable"),
        Some(&admin.access_token),
        Some(json!({"reason": "pending account must be activated by invitation"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "INVALID_ACCOUNT_STATUS_TRANSITION");

    let (status, _) = json_request(
        app,
        "POST",
        &format!("/api/v1/admin/users/{}/disable", user_one.user_id),
        Some(&admin.access_token),
        Some(json!({"reason": "stage B isolation test"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, disabled) =
        json_request(app, "GET", "/api/v1/me", Some(&user_one.access_token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(disabled["code"], "ACCOUNT_DISABLED");
}

async fn assert_account_purge_requires_reauth(app: &Router, admin: &Tokens, user: &Tokens) {
    let (status, reauth) = json_request(
        app,
        "POST",
        "/api/v1/admin/auth/re-auth",
        Some(&admin.access_token),
        Some(json!({"password": "admin-password-123"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let nonce = reauth["nonce"]
        .as_str()
        .expect("re-auth nonce should exist");
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/admin/users/{}/purge", user.user_id))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", admin.access_token),
        )
        .header(header::ORIGIN, "https://admin.example.test")
        .header("sec-fetch-site", "same-origin")
        .header("x-reauth-nonce", nonce)
        .body(Body::from(
            json!({"confirmed": false, "reason": "stage B purge export"}).to_string(),
        ))
        .expect("account purge request should build");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("account purge should respond");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("account purge body should collect");
    let body: Value = serde_json::from_slice(&bytes).expect("account purge should return JSON");
    assert_eq!(body["confirmationRequired"], true);
    assert!(body["exportId"].as_str().is_some());
    assert!(body.get("objectKey").is_none());
}

struct Tokens {
    access_token: String,
    refresh_token: String,
    device_id: Uuid,
    user_id: Uuid,
}

async fn admin_login(
    app: &Router,
    email: &str,
    password: &str,
    device_id: Uuid,
    user_id: Uuid,
) -> Tokens {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://admin.example.test")
        .header("sec-fetch-site", "same-origin")
        .body(Body::from(
            json!({"email": email, "password": password, "deviceId": device_id}).to_string(),
        ))
        .expect("admin login request should build");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("admin login should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("admin login body should collect");
    let body: Value = serde_json::from_slice(&bytes).expect("admin login should return JSON");
    let access_token = body["accessToken"].as_str().unwrap().to_owned();
    Tokens {
        access_token,
        refresh_token: String::new(),
        device_id,
        user_id,
    }
}

async fn invite_and_activate(
    app: &Router,
    admin_access_token: &str,
    test_id: Uuid,
    ordinal: u8,
) -> Tokens {
    let email = format!("api-user-{ordinal}-{test_id}@example.test");
    let (status, invitation) = json_request(
        app,
        "POST",
        "/api/v1/admin/invitations",
        Some(admin_access_token),
        Some(json!({"email": email})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let device_id = Uuid::new_v4();
    let (status, body) = json_request(
        app,
        "POST",
        "/api/v1/auth/invitations/activate",
        None,
        Some(json!({
            "invitationToken": invitation["invitationToken"],
            "password": format!("user-password-{ordinal}-123"),
            "deviceId": device_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "activation response: {body}");
    tokens_from_body(app, &body, device_id).await
}

async fn tokens_from_body(app: &Router, body: &Value, device_id: Uuid) -> Tokens {
    let access_token = body["accessToken"].as_str().unwrap().to_owned();
    let (status, current_user) =
        json_request(app, "GET", "/api/v1/me", Some(&access_token), None).await;
    assert_eq!(status, StatusCode::OK);
    Tokens {
        access_token,
        refresh_token: body["refreshToken"].as_str().unwrap_or_default().to_owned(),
        device_id,
        user_id: current_user["id"].as_str().unwrap().parse().unwrap(),
    }
}

async fn assert_admin_refresh_cookie(app: &Router, email: &str) {
    let device_id = Uuid::new_v4();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://admin.example.test")
        .header("sec-fetch-site", "same-origin")
        .body(Body::from(
            json!({
                "email": email,
                "password": "admin-password-123",
                "deviceId": device_id
            })
            .to_string(),
        ))
        .expect("admin login request should build");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("admin login should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("admin login should set refresh cookie");
    assert!(set_cookie.starts_with("tasktips_admin_refresh="));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("Secure"));
    assert!(set_cookie.contains("SameSite=Strict"));
    let cookie = set_cookie
        .split(';')
        .next()
        .expect("cookie should have a value");

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/admin/auth/refresh")
        .header(header::ORIGIN, "https://admin.example.test")
        .header("sec-fetch-site", "same-origin")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("cookie refresh request should build");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("cookie refresh should respond");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key(header::SET_COOKIE));
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("cookie refresh body should collect");
    let body: Value = serde_json::from_slice(&bytes).expect("cookie refresh should return JSON");
    assert!(body.get("refreshToken").is_none());
}

async fn json_request(
    app: &Router,
    method: &str,
    path: &str,
    access_token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if path.starts_with("/api/v1/admin/") {
        request = request
            .header(header::ORIGIN, "https://admin.example.test")
            .header("sec-fetch-site", "same-origin");
    }
    if let Some(access_token) = access_token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {access_token}"));
    }
    let body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).expect("request should build"))
        .await
        .expect("router should respond");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("response body should collect");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response should be JSON")
    };
    (status, body)
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
