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
use serde::Serialize;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
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
    activate_invitation, admin_audit_events, admin_audit_events_csv, admin_create_restore,
    admin_history_metadata, admin_jobs, admin_list_devices, admin_list_invitations,
    admin_list_projects, admin_list_users, admin_login, admin_logout, admin_overview, admin_reauth,
    admin_refresh, admin_resend_invitation, admin_restore_jobs, admin_revoke_invitation,
    admin_sync_attempts, admin_trends, bootstrap, cancel_restore, change_password,
    create_invitation, create_project, create_restore, create_snapshot, current_user,
    disable_account, disable_project, enable_account, get_payload, get_project, get_restore,
    head_payload, history, list_devices, list_projects, list_snapshots, login, logout,
    object_history, pull, purge_account, purge_project, push, put_payload, refresh,
    register_device, rename_project, reopen_restore_project, revoke_device, update_device,
};

const DEFAULT_UPLOAD_TEMP_MAX_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) struct TempUploadBudget {
    used_bytes: AtomicU64,
    max_bytes: u64,
}

impl Default for TempUploadBudget {
    fn default() -> Self {
        let max_bytes = std::env::var("TASKTIPS_UPLOAD_TEMP_MAX_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value >= 1024 * 1024)
            .unwrap_or(DEFAULT_UPLOAD_TEMP_MAX_BYTES);
        Self {
            used_bytes: AtomicU64::new(0),
            max_bytes,
        }
    }
}

impl TempUploadBudget {
    pub(crate) fn reserve(self: &Arc<Self>, bytes: u64) -> Option<TempUploadReservation> {
        let mut current = self.used_bytes.load(Ordering::Acquire);
        loop {
            let next = current.checked_add(bytes)?;
            if next > self.max_bytes {
                return None;
            }
            match self.used_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(TempUploadReservation {
                        budget: Arc::clone(self),
                        bytes,
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }
}

pub(crate) struct TempUploadReservation {
    budget: Arc<TempUploadBudget>,
    bytes: u64,
}

impl Drop for TempUploadReservation {
    fn drop(&mut self) {
        self.budget
            .used_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[derive(Clone, Default)]
pub struct AppState {
    readiness: Readiness,
    pub(crate) database: Option<Persistence>,
    pub(crate) auth: Option<AuthService>,
    pub(crate) object_store: Option<ObjectStore>,
    pub(crate) cursor: Option<CursorSigner>,
    pub(crate) metrics: Arc<ApiMetrics>,
    pub(crate) admin_origin: Option<Arc<str>>,
    pub(crate) upload_budget: Arc<TempUploadBudget>,
}

#[derive(Default)]
pub struct ApiMetrics {
    pub sync_pull_total: AtomicU64,
    pub sync_push_total: AtomicU64,
    pub sync_conflict_total: AtomicU64,
    pub payload_upload_bytes_total: AtomicU64,
    pub payload_download_bytes_total: AtomicU64,
    pub restore_job_total: AtomicU64,
}

impl ApiMetrics {
    fn openmetrics(&self) -> String {
        format!(
            "# TYPE sync_pull_total counter\nsync_pull_total {}\n# TYPE sync_push_total counter\nsync_push_total {}\n# TYPE sync_conflict_total counter\nsync_conflict_total {}\n# TYPE payload_upload_bytes_total counter\npayload_upload_bytes_total {}\n# TYPE payload_download_bytes_total counter\npayload_download_bytes_total {}\n# TYPE restore_job_total counter\nrestore_job_total {}\n# EOF\n",
            self.sync_pull_total.load(Ordering::Relaxed),
            self.sync_push_total.load(Ordering::Relaxed),
            self.sync_conflict_total.load(Ordering::Relaxed),
            self.payload_upload_bytes_total.load(Ordering::Relaxed),
            self.payload_download_bytes_total.load(Ordering::Relaxed),
            self.restore_job_total.load(Ordering::Relaxed),
        )
    }
}

impl AppState {
    #[must_use]
    pub fn new(
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
            metrics: Arc::new(ApiMetrics::default()),
            admin_origin: None,
            upload_budget: Arc::new(TempUploadBudget::default()),
        }
    }

    #[must_use]
    pub fn with_sync(mut self, object_store: ObjectStore, cursor: CursorSigner) -> Self {
        self.object_store = Some(object_store);
        self.cursor = Some(cursor);
        self
    }

    #[must_use]
    pub fn with_admin_origin(mut self, origin: impl Into<Arc<str>>) -> Self {
        self.admin_origin = Some(origin.into());
        self
    }

    #[must_use]
    pub fn unavailable(readiness: Readiness) -> Self {
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
        .route("/api/v1/openapi.yaml", get(openapi))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/admin/auth/login", post(admin_login))
        .route("/api/v1/admin/auth/refresh", post(admin_refresh))
        .route("/api/v1/admin/auth/logout", post(admin_logout))
        .route("/api/v1/admin/auth/re-auth", post(admin_reauth))
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
        .route("/api/v1/projects/{projectId}/purge", post(purge_project))
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
        .route(
            "/api/v1/projects/{projectId}/restores/{restoreId}/cancel",
            post(cancel_restore),
        )
        .route("/api/v1/devices", get(list_devices))
        .route("/api/v1/devices/register", post(register_device))
        .route("/api/v1/devices/{deviceId}", patch(update_device))
        .route("/api/v1/devices/{deviceId}/revoke", post(revoke_device))
        .route("/api/v1/admin/users", get(admin_list_users))
        .route(
            "/api/v1/admin/invitations",
            get(admin_list_invitations).post(create_invitation),
        )
        .route(
            "/api/v1/admin/invitations/{invitationId}/revoke",
            post(admin_revoke_invitation),
        )
        .route(
            "/api/v1/admin/invitations/{invitationId}/resend",
            post(admin_resend_invitation),
        )
        .route(
            "/api/v1/admin/users/{userId}/disable",
            post(disable_account),
        )
        .route("/api/v1/admin/users/{userId}/enable", post(enable_account))
        .route("/api/v1/admin/users/{userId}/purge", post(purge_account))
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
        .route(
            "/api/v1/admin/projects/{projectId}/restore-reopen",
            post(reopen_restore_project),
        )
        .route("/api/v1/admin/restores", get(admin_restore_jobs))
        .route("/api/v1/admin/jobs", get(admin_jobs))
        .route("/api/v1/admin/sync-attempts", get(admin_sync_attempts))
        .route("/api/v1/admin/metrics/trends", get(admin_trends))
        .route("/api/v1/admin/audit-events", get(admin_audit_events))
        .route(
            "/api/v1/admin/audit-events.csv",
            get(admin_audit_events_csv),
        )
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

async fn metrics(State(state): State<AppState>) -> Response {
    let mut response = (StatusCode::OK, state.metrics.openmetrics()).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/openmetrics-text; version=1.0.0; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::{AppState, Readiness, ReadinessSnapshot, build_application_router, build_router};

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
                    .uri("/api/v1/openapi.yaml")
                    .body(Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("router should respond");
        assert_eq!(response.status(), StatusCode::OK);

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

    #[tokio::test]
    async fn metrics_exposes_operational_counters() {
        use axum::{
            body::{Body, to_bytes},
            http::{Request, StatusCode},
        };
        use std::sync::atomic::Ordering;
        use tower::ServiceExt;

        let state = AppState::unavailable(Readiness::unavailable());
        state.metrics.sync_pull_total.store(2, Ordering::Relaxed);
        let response = build_application_router(state)
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("sync_pull_total 2"));
        assert!(text.ends_with("# EOF\n"));
    }
}
