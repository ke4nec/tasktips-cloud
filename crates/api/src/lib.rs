use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use prometheus_client::{encoding::text::encode, registry::Registry};
use serde::Serialize;
use tower_http::trace::TraceLayer;

#[derive(Clone, Copy, Debug, Default)]
pub struct Readiness {
    pub database: bool,
    pub object_store: bool,
}

impl Readiness {
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

pub fn build_router(readiness: Readiness) -> Router {
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness_check))
        .route("/metrics", get(metrics))
        .with_state(readiness)
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
    let status = if readiness.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(HealthResponse {
            status: if readiness.is_ready() {
                "ready"
            } else {
                "notReady"
            },
            database: Some(readiness.database),
            object_store: Some(readiness.object_store),
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
    use super::Readiness;

    #[test]
    fn readiness_requires_every_dependency() {
        assert!(!Readiness::default().is_ready());
        assert!(
            Readiness {
                database: true,
                object_store: true,
            }
            .is_ready()
        );
    }
}
