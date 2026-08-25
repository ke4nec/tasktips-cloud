use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use der::pem::LineEnding;
use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tasktips_api::{
    AppState, Readiness,
    auth::{AuthService, hash_password},
    build_application_router,
    cursor::CursorSigner,
};
use tasktips_object_store::{ObjectStore, RustFsConfig};
use tasktips_persistence::{
    NewInvitation, NewRefreshToken, Persistence, invitation_expiry, refresh_expiry,
};
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

const TODO_ID: &str = "01J5MZ6K6AC2F4Y17D8Q1T8PXP";

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn stage_c_two_device_sync_is_cas_idempotent_and_isolated() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        return;
    };
    let Ok(endpoint) = std::env::var("RUSTFS_ENDPOINT") else {
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");
    let object_store = ObjectStore::with_credentials(
        RustFsConfig::new(
            endpoint,
            std::env::var("RUSTFS_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            std::env::var("RUSTFS_BUCKET").expect("test bucket is required"),
        ),
        std::env::var("RUSTFS_ACCESS_KEY").expect("test access key is required"),
        std::env::var("RUSTFS_SECRET_KEY").expect("test secret key is required"),
    );
    assert!(object_store.is_ready().await);

    let auth = test_auth();
    let test_id = Uuid::new_v4();
    let admin_email = format!("stage-c-admin-{test_id}@example.test");
    let admin = persistence
        .create_initial_admin(
            &admin_email,
            &admin_email,
            &hash_password("admin-password-123").expect("password should hash"),
        )
        .await
        .expect("admin should be created");
    let user_one = activate_user(&persistence, admin.id, test_id, 1).await;
    let user_two = activate_user(&persistence, admin.id, test_id, 2).await;
    let device_two_id = Uuid::new_v4();
    persistence
        .create_login_session(
            user_one.user_id,
            device_two_id,
            &refresh_record(),
            "stage-c-device-two",
        )
        .await
        .expect("second device should login");
    let project = persistence
        .create_project(user_one.user_id, "user", "Stage C")
        .await
        .expect("project should be created");
    let device_one = DeviceToken {
        access_token: auth
            .issue_access_token(user_one.user_id, "user", user_one.device_id)
            .expect("access token should issue")
            .token,
        device_id: user_one.device_id,
    };
    let device_two = DeviceToken {
        access_token: auth
            .issue_access_token(user_one.user_id, "user", device_two_id)
            .expect("access token should issue")
            .token,
        device_id: device_two_id,
    };
    let outsider = DeviceToken {
        access_token: auth
            .issue_access_token(user_two.user_id, "user", user_two.device_id)
            .expect("access token should issue")
            .token,
        device_id: user_two.device_id,
    };
    let app = build_application_router(
        AppState::new(
            Readiness::unavailable(),
            Some(persistence.clone()),
            Some(auth),
        )
        .with_sync(
            object_store,
            CursorSigner::new([9_u8; 32]).expect("cursor signer should build"),
        ),
    );

    let bootstrap_one = bootstrap(&app, project.id, &device_one).await;
    let bootstrap_two = bootstrap(&app, project.id, &device_two).await;
    let device_two_cursor = bootstrap_two["cursor"]
        .as_str()
        .expect("final bootstrap page should contain cursor")
        .to_owned();
    assert_eq!(bootstrap_one["items"], json!([]));

    let first_payload = b"---\nschemaVersion: 1\n---\nfirst";
    let first_hash = upload_payload(&app, project.id, &device_one, first_payload).await;
    assert_payload_access(
        &app,
        project.id,
        &device_two,
        &outsider,
        &first_hash,
        first_payload,
    )
    .await;
    assert_hash_mismatch(&app, project.id, &device_one).await;

    let first_push = push_object(
        &app,
        project.id,
        &device_one,
        "initial-request",
        1,
        None,
        1,
        &first_hash,
    )
    .await;
    assert_eq!(first_push["results"][0]["status"], "applied");
    let repeated = push_object(
        &app,
        project.id,
        &device_one,
        "initial-request",
        1,
        None,
        1,
        &first_hash,
    )
    .await;
    assert_eq!(repeated, first_push);
    assert_idempotency_conflict(&app, project.id, &device_one, &first_hash).await;
    assert_generation_mismatch(&app, project.id, &device_one, &first_hash).await;

    let device_one_hash = upload_payload(&app, project.id, &device_one, b"device-one").await;
    let device_two_hash = upload_payload(&app, project.id, &device_two, b"device-two").await;
    let (left, right) = tokio::join!(
        push_object(
            &app,
            project.id,
            &device_one,
            "concurrent-one",
            1,
            Some(1),
            2,
            &device_one_hash,
        ),
        push_object(
            &app,
            project.id,
            &device_two,
            "concurrent-two",
            1,
            Some(1),
            2,
            &device_two_hash,
        )
    );
    let statuses = [
        left["results"][0]["status"].as_str(),
        right["results"][0]["status"].as_str(),
    ];
    assert!(statuses.contains(&Some("applied")));
    assert!(statuses.contains(&Some("conflict")));

    let tombstone = push_tombstone(&app, project.id, &device_one, 2, 3).await;
    assert_eq!(tombstone["results"][0]["status"], "applied");
    let pulled = pull(&app, project.id, &device_two, &device_two_cursor).await;
    let changes = pulled["changes"]
        .as_array()
        .expect("changes should be an array");
    assert_eq!(
        changes.len(),
        3,
        "idempotent retry must not duplicate revisions"
    );
    assert_eq!(changes.last().unwrap()["type"], "tombstone");

    let (status, error) = json_request(
        &app,
        "POST",
        &format!("/api/v1/projects/{}/sync/pull", project.id),
        &device_two.access_token,
        json!({"cursor": format!("{}x", pulled["nextCursor"].as_str().unwrap())}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], "CURSOR_INVALID");
}

async fn assert_payload_access(
    app: &Router,
    project_id: Uuid,
    owner: &DeviceToken,
    outsider: &DeviceToken,
    hash: &str,
    expected: &[u8],
) {
    let path = format!("/api/v1/projects/{project_id}/payloads/{hash}");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", owner.access_token),
                )
                .header(header::RANGE, "bytes=0-3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        to_bytes(response.into_body(), 16).await.unwrap(),
        &expected[..4]
    );

    let (status, error) = json_request(app, "GET", &path, &outsider.access_token, json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], "PROJECT_NOT_FOUND");
}

async fn assert_hash_mismatch(app: &Router, project_id: Uuid, device: &DeviceToken) {
    let wrong_hash = "0".repeat(64);
    let response = raw_request(
        app,
        "PUT",
        &format!("/api/v1/projects/{project_id}/payloads/{wrong_hash}"),
        &device.access_token,
        "text/markdown",
        b"different",
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

async fn assert_idempotency_conflict(
    app: &Router,
    project_id: Uuid,
    device: &DeviceToken,
    hash: &str,
) {
    let (status, error) = json_request(
        app,
        "POST",
        &format!("/api/v1/projects/{project_id}/sync/push"),
        &device.access_token,
        object_push_body(device, "initial-request", 1, Some(1), 2, hash),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "IDEMPOTENCY_CONFLICT");
}

async fn assert_generation_mismatch(
    app: &Router,
    project_id: Uuid,
    device: &DeviceToken,
    hash: &str,
) {
    let (status, error) = json_request(
        app,
        "POST",
        &format!("/api/v1/projects/{project_id}/sync/push"),
        &device.access_token,
        object_push_body(device, "wrong-generation", 9, Some(1), 2, hash),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "GENERATION_MISMATCH");
}

async fn bootstrap(app: &Router, project_id: Uuid, device: &DeviceToken) -> Value {
    let (status, body) = json_request(
        app,
        "POST",
        &format!("/api/v1/projects/{project_id}/sync/bootstrap"),
        &device.access_token,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "bootstrap failed: {body}");
    body
}

async fn pull(app: &Router, project_id: Uuid, device: &DeviceToken, cursor: &str) -> Value {
    let (status, body) = json_request(
        app,
        "POST",
        &format!("/api/v1/projects/{project_id}/sync/pull"),
        &device.access_token,
        json!({"cursor": cursor}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "pull failed: {body}");
    body
}

async fn upload_payload(
    app: &Router,
    project_id: Uuid,
    device: &DeviceToken,
    bytes: &[u8],
) -> String {
    let hash = hex::encode(Sha256::digest(bytes));
    let response = raw_request(
        app,
        "PUT",
        &format!("/api/v1/projects/{project_id}/payloads/{hash}"),
        &device.access_token,
        "text/markdown; charset=utf-8",
        bytes,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    hash
}

#[allow(clippy::too_many_arguments)]
async fn push_object(
    app: &Router,
    project_id: Uuid,
    device: &DeviceToken,
    request_id: &str,
    generation: i64,
    base_revision: Option<i64>,
    revision: i64,
    hash: &str,
) -> Value {
    let (status, body) = json_request(
        app,
        "POST",
        &format!("/api/v1/projects/{project_id}/sync/push"),
        &device.access_token,
        object_push_body(
            device,
            request_id,
            generation,
            base_revision,
            revision,
            hash,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "push {request_id} failed: {body}");
    body
}

async fn push_tombstone(
    app: &Router,
    project_id: Uuid,
    device: &DeviceToken,
    base_revision: i64,
    revision: i64,
) -> Value {
    let (status, body) = json_request(
        app,
        "POST",
        &format!("/api/v1/projects/{project_id}/sync/push"),
        &device.access_token,
        json!({
            "requestId": "delete-request",
            "generation": 1,
            "objects": [],
            "tombstones": [{
                "kind": "todo",
                "id": TODO_ID,
                "revision": revision,
                "baseRevision": base_revision,
                "deletedAt": OffsetDateTime::now_utc(),
                "deviceId": device.device_id
            }]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "tombstone failed: {body}");
    body
}

fn object_push_body(
    device: &DeviceToken,
    request_id: &str,
    generation: i64,
    base_revision: Option<i64>,
    revision: i64,
    hash: &str,
) -> Value {
    json!({
        "requestId": request_id,
        "generation": generation,
        "objects": [{
            "kind": "todo",
            "id": TODO_ID,
            "schemaVersion": 1,
            "revision": revision,
            "baseRevision": base_revision,
            "contentHash": hash,
            "updatedAt": OffsetDateTime::from_unix_timestamp(1_704_067_200).unwrap(),
            "deviceId": device.device_id
        }],
        "tombstones": []
    })
}

async fn raw_request(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    media_type: &str,
    bytes: &[u8],
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, media_type)
                .header(header::CONTENT_LENGTH, bytes.len())
                .body(Body::from(bytes.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn json_request(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

struct ActivatedUser {
    user_id: Uuid,
    device_id: Uuid,
}

struct DeviceToken {
    access_token: String,
    device_id: Uuid,
}

async fn activate_user(
    persistence: &Persistence,
    admin_id: Uuid,
    test_id: Uuid,
    index: u8,
) -> ActivatedUser {
    let token_hash = Sha256::digest(format!("stage-c-{test_id}-{index}")).to_vec();
    persistence
        .create_invitation(
            admin_id,
            &NewInvitation {
                email_normalized: format!("stage-c-user-{index}-{test_id}@example.test"),
                email_display: format!("stage-c-user-{index}-{test_id}@example.test"),
                token_hash: token_hash.clone(),
                expires_at: invitation_expiry(),
            },
            "stage-c-invite",
        )
        .await
        .expect("invitation should be created");
    let device_id = Uuid::new_v4();
    let session = persistence
        .activate_invitation(
            &token_hash,
            &hash_password("user-password-123").expect("password should hash"),
            device_id,
            &refresh_record(),
            "stage-c-activate",
        )
        .await
        .expect("invitation should activate");
    ActivatedUser {
        user_id: session.user.id,
        device_id,
    }
}

fn refresh_record() -> NewRefreshToken {
    NewRefreshToken {
        token_hash: Uuid::new_v4().as_bytes().to_vec(),
        family_id: Uuid::new_v4(),
        expires_at: refresh_expiry(3600),
    }
}

fn test_auth() -> AuthService {
    let signing_key = SigningKey::from_bytes(&[3_u8; 32]);
    let private_key = signing_key
        .to_pkcs8_pem(LineEnding::default())
        .expect("test key should encode");
    AuthService::from_private_key_pem(
        private_key.as_bytes(),
        "stage-c-tests".to_owned(),
        "tasktips-tests".to_owned(),
        3600,
        3600,
    )
    .expect("test auth should build")
}
