use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use prometheus_client::{encoding::text::encode, registry::Registry};
use serde::Serialize;
use tasktips_object_store::ObjectStore;
use tasktips_persistence::Persistence;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

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
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness_check))
        .route("/metrics", get(metrics))
        .route("/openapi.yaml", get(openapi))
        .fallback(not_found)
        .with_state(readiness)
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

async fn readiness_check(State(readiness): State<Readiness>) -> impl IntoResponse {
    let snapshot = readiness.check().await;
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
