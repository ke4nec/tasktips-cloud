pub mod auth;
pub mod cursor;
mod routes;

use axum::{
    BoxError, Json, Router,
    error_handling::HandleErrorLayer,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use prometheus_client::{encoding::text::encode, registry::Registry};
use serde::Serialize;
use std::time::Duration;
use tasktips_object_store::ObjectStore;
use tasktips_persistence::Persistence;
use tower::{ServiceBuilder, timeout::TimeoutLayer};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

use auth::AuthService;
use cursor::CursorSigner;
use routes::{
    activate_invitation, admin_audit_events, admin_create_restore, admin_history_metadata,
    admin_list_devices, admin_list_projects, admin_list_users, admin_overview, admin_restore_jobs,
    admin_sync_attempts, bootstrap, change_password, create_invitation, create_project,
    create_restore, create_snapshot, current_user, disable_account, disable_project,
    enable_account, get_payload, get_project, get_restore, head_payload, history, list_devices,
    list_projects, list_snapshots, login, logout, object_history, pull, push, put_payload, refresh,
    register_device, rename_project, revoke_device, update_device,
};

#[derive(Clone, Default)]
pub struct AppState {
    readiness: Readiness,
    pub(crate) database: Option<Persistence>,
    pub(crate) auth: Option<AuthService>,
    pub(crate) object_store: Option<ObjectStore>,
    pub(crate) cursor: Option<CursorSigner>,
}

impl AppState {
    #[must_use]
    pub const fn new(
        readiness: Readiness,
        database: Option<Persistence>,
        auth: Option<AuthService>,
    ) -> Self {
        Self {
            readiness,
            database,
            auth,
            object_store: None,
            cursor: None,
        }
    }

    #[must_use]
    pub fn with_sync(mut self, object_store: ObjectStore, cursor: CursorSigner) -> Self {
        self.object_store = Some(object_store);
        self.cursor = Some(cursor);
        self
    }

    #[must_use]
    pub const fn unavailable(readiness: Readiness) -> Self {
        Self::new(readiness, None, None)
    }
}

#[derive(Clone, Default)]
pub struct Readiness {
    database: Option<Persistence>,
    object_store: Option<ObjectStore>,
}

impl Readiness {
    #[must_use]
    pub const fn new(database: Option<Persistence>, object_store: Option<ObjectStore>) -> Self {
        Self {
            database,
            object_store,
        }
    }

    #[must_use]
    pub const fn unavailable() -> Self {
        Self::new(None, None)
    }

    async fn check(&self) -> ReadinessSnapshot {
        let database = match &self.database {
            Some(database) => database.is_ready().await.unwrap_or(false),
            None => false,
        };
        let object_store = match &self.object_store {
            Some(object_store) => object_store.is_ready().await,
            None => false,
        };
        ReadinessSnapshot {
            database,
            object_store,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReadinessSnapshot {
    pub database: bool,
    pub object_store: bool,
}

impl ReadinessSnapshot {
    #[must_use]
    pub const fn is_ready(self) -> bool {
        self.database && self.object_store
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
    database: Option<bool>,
    object_store: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponse {
    code: &'static str,
    message: &'static str,
    retryable: bool,
    request_id: String,
}

pub fn build_router(readiness: Readiness) -> Router {
    build_application_router(AppState::unavailable(readiness))
}

#[allow(clippy::too_many_lines)]
pub fn build_application_router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness_check))
        .route("/metrics", get(metrics))
        .route("/openapi.yaml", get(openapi))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route(
            "/api/v1/auth/invitations/activate",
            post(activate_invitation),
        )
        .route("/api/v1/me", get(current_user))
        .route("/api/v1/me/password", patch(change_password))
        .route("/api/v1/projects", get(list_projects).post(create_project))
        .route(
            "/api/v1/projects/{projectId}",
            get(get_project).patch(rename_project),
        )
        .route(
            "/api/v1/projects/{projectId}/disable",
            post(disable_project),
        )
        .route(
            "/api/v1/projects/{projectId}/payloads/{contentHash}",
            get(get_payload).head(head_payload).put(put_payload),
        )
        .route(
            "/api/v1/projects/{projectId}/sync/bootstrap",
            post(bootstrap),
        )
        .route("/api/v1/projects/{projectId}/sync/pull", post(pull))
        .route("/api/v1/projects/{projectId}/sync/push", post(push))
        .route("/api/v1/projects/{projectId}/history", get(history))
        .route(
            "/api/v1/projects/{projectId}/objects/{kind}/{objectId}/history",
            get(object_history),
        )
        .route(
            "/api/v1/projects/{projectId}/snapshots",
            get(list_snapshots).post(create_snapshot),
        )
        .route(
            "/api/v1/projects/{projectId}/restores",
            post(create_restore),
        )
        .route(
            "/api/v1/projects/{projectId}/restores/{restoreId}",
            get(get_restore),
        )
        .route("/api/v1/devices", get(list_devices))
        .route("/api/v1/devices/register", post(register_device))
        .route("/api/v1/devices/{deviceId}", patch(update_device))
        .route("/api/v1/devices/{deviceId}/revoke", post(revoke_device))
        .route("/api/v1/admin/users", get(admin_list_users))
        .route("/api/v1/admin/invitations", post(create_invitation))
        .route(
            "/api/v1/admin/users/{userId}/disable",
            post(disable_account),
        )
        .route("/api/v1/admin/users/{userId}/enable", post(enable_account))
        .route(
            "/api/v1/admin/users/{userId}/projects",
            get(admin_list_projects),
        )
        .route(
            "/api/v1/admin/users/{userId}/devices",
            get(admin_list_devices),
        )
        .route("/api/v1/admin/overview", get(admin_overview))
        .route(
            "/api/v1/admin/projects/{projectId}/history-metadata",
            get(admin_history_metadata),
        )
        .route(
            "/api/v1/admin/projects/{projectId}/restores",
            post(admin_create_restore),
        )
        .route("/api/v1/admin/restores", get(admin_restore_jobs))
        .route("/api/v1/admin/sync-attempts", get(admin_sync_attempts))
        .route("/api/v1/admin/audit-events", get(admin_audit_events))
        .fallback(not_found)
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(
                    |headers: HeaderMap, _: BoxError| async move {
                        let request_id = headers
                            .get("x-request-id")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("unknown")
                            .to_owned();
                        (
                            StatusCode::REQUEST_TIMEOUT,
                            Json(ErrorResponse {
                                code: "REQUEST_TIMEOUT",
                                message: "请求处理超时",
                                retryable: true,
                                request_id,
                            }),
                        )
                    },
                ))
                .layer(TimeoutLayer::new(Duration::from_secs(10))),
        )
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(TraceLayer::new_for_http())
}

async fn liveness() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "live",
        database: None,
        object_store: None,
    })
}

async fn readiness_check(State(state): State<AppState>) -> impl IntoResponse {
    let snapshot = state.readiness.check().await;
    let status = if snapshot.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(HealthResponse {
            status: if snapshot.is_ready() {
                "ready"
            } else {
                "notReady"
            },
            database: Some(snapshot.database),
            object_store: Some(snapshot.object_store),
        }),
    )
}

async fn openapi() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/yaml; charset=utf-8")],
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/openapi.yaml"
        )),
    )
}

async fn not_found(headers: HeaderMap) -> impl IntoResponse {
    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            code: "NOT_FOUND",
            message: "资源不存在",
            retryable: false,
            request_id,
        }),
    )
}

async fn metrics() -> Response {
    let registry = Registry::default();
    let mut body = String::new();
    let status = if encode(&mut body, &registry).is_ok() {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    let mut response = (status, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/openmetrics-text; version=1.0.0; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::{Readiness, ReadinessSnapshot, build_router};

    #[test]
    fn readiness_requires_every_dependency() {
        assert!(!ReadinessSnapshot::default().is_ready());
        assert!(
            ReadinessSnapshot {
                database: true,
                object_store: true,
            }
            .is_ready()
        );
    }

    #[tokio::test]
    async fn router_exposes_openapi_and_request_id_errors() {
        use axum::{
            body::Body,
            http::{Request, StatusCode, header},
        };
        use tower::ServiceExt;

        let app = build_router(Readiness::unavailable());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/missing")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(response.headers().contains_key("x-request-id"));
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");

        let response = build_router(Readiness::unavailable())
            .oneshot(
                Request::builder()
                    .uri("/openapi.yaml")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("router should respond");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/yaml; charset=utf-8"
        );

        let response = build_router(Readiness::unavailable())
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("router should respond");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
