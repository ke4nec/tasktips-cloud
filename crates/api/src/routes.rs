use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{
        FromRequest, FromRequestParts, Path, Query, Request, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::Instant;
use std::{net::IpAddr, path::PathBuf};
use tasktips_application::{
    AccountStatus, ObjectKind, normalize_email, valid_password, validate_account_purge_phase,
    validate_restore_target,
};
use tasktips_object_store::ObjectStoreError;
use tasktips_persistence::{
    DeviceProfile, NewInvitation, NewRefreshToken, NewSyncRevision, Persistence, PersistenceError,
    StoredPayload, SyncRecord, UserRecord, invitation_expiry, refresh_expiry,
};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    AppState,
    auth::{
        AccessClaims, AuthError, AuthOperation, AuthService, hash_password, opaque_token_hash,
        verify_password,
    },
    cursor::{CursorClaims, CursorKind},
};

const ADMIN_REFRESH_COOKIE: &str = "tasktips_admin_refresh";
const MAX_OPTIONAL_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponse {
    code: &'static str,
    message: &'static str,
    retryable: bool,
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    body: ErrorResponse,
}

impl ApiError {
    fn code(&self) -> &'static str {
        self.body.code
    }

    fn new(
        status: StatusCode,
        code: &'static str,
        message: &'static str,
        retryable: bool,
        request_id: String,
    ) -> Self {
        Self {
            status,
            body: ErrorResponse {
                code,
                message,
                retryable,
                request_id,
                details: None,
            },
        }
    }

    fn with_details(mut self, details: serde_json::Value) -> Self {
        self.body.details = Some(details);
        self
    }

    fn invalid(message: &'static str, request_id: String) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
            message,
            false,
            request_id,
        )
    }

    fn authentication(request_id: String) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "AUTHENTICATION_REQUIRED",
            "认证信息无效或已过期",
            false,
            request_id,
        )
    }

    fn internal(request_id: String) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            "服务暂时无法完成请求",
            true,
            request_id,
        )
    }

    fn unavailable(request_id: String) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "STORAGE_UNAVAILABLE",
            "服务依赖尚未就绪",
            true,
            request_id,
        )
    }

    fn authorization(request_id: String) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "AUTHORIZATION_DENIED",
            "当前账号无权执行此操作",
            false,
            request_id,
        )
    }

    fn rate_limited(request_id: String) -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "RATE_LIMITED",
            "请求过于频繁，请稍后重试",
            true,
            request_id,
        )
    }

    fn cursor_invalid(request_id: String) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "CURSOR_INVALID",
            "同步 cursor 无效",
            false,
            request_id,
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = request_id(request.headers());
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|_rejection: JsonRejection| {
                ApiError::invalid("请求 JSON 格式或字段无效", request_id)
            })
    }
}

pub struct ApiLimitedJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiLimitedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let request_id = request_id(request.headers());
        let bytes = to_bytes(request.into_body(), MAX_OPTIONAL_JSON_BYTES)
            .await
            .map_err(|_| ApiError::invalid("请求 JSON 超过 2 MiB", request_id.clone()))?;
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(|_| ApiError::invalid("请求 JSON 格式或字段无效", request_id))
    }
}

pub struct ApiPath<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiPath<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = request_id(&parts.headers);
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|_rejection: PathRejection| ApiError::invalid("路径参数格式无效", request_id))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginRequest {
    email: String,
    password: String,
    device_id: Uuid,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminLoginRequest {
    email: String,
    password: String,
    device_id: Uuid,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefreshRequest {
    refresh_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminReauthRequest {
    password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvitationActivationRequest {
    invitation_token: String,
    password: String,
    device_id: Uuid,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminTokenResponse {
    access_token: String,
    expires_in: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReauthResponse {
    nonce: String,
    expires_in: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentUserResponse {
    id: Uuid,
    email: String,
    role: String,
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasswordChangeRequest {
    current_password: String,
    new_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectRequest {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterDeviceRequest {
    device_id: Uuid,
    display_name: String,
    platform: String,
    app_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateDeviceRequest {
    display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInvitationRequest {
    email: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvitationResponse {
    id: Uuid,
    email: String,
    invitation_token: String,
    expires_at: time::OffsetDateTime,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountStatusRequest {
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PurgeProjectRequest {
    password: String,
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountPurgeRequest {
    export_id: Option<Uuid>,
    confirmed: bool,
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapRequest {
    page_token: Option<String>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PullRequest {
    cursor: String,
    limit: Option<i64>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PushRequest {
    request_id: String,
    generation: i64,
    objects: Vec<PushObject>,
    tombstones: Vec<PushTombstone>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PushObject {
    kind: ObjectKind,
    id: String,
    schema_version: i32,
    revision: i64,
    base_revision: Option<i64>,
    content_hash: String,
    updated_at: time::OffsetDateTime,
    device_id: Uuid,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PushTombstone {
    kind: ObjectKind,
    id: String,
    revision: i64,
    base_revision: Option<i64>,
    deleted_at: time::OffsetDateTime,
    device_id: Uuid,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryQuery {
    after_sequence: Option<i64>,
    limit: Option<i64>,
    kind: Option<String>,
    object_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreRequest {
    snapshot_id: Option<Uuid>,
    target_change_sequence: Option<i64>,
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminRestoreRequest {
    snapshot_id: Option<Uuid>,
    target_change_sequence: Option<i64>,
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReasonRequest {
    reason: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminPageQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminProjectQuery {
    search: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminTrendQuery {
    days: Option<i32>,
}

pub async fn head_payload(
    State(state): State<AppState>,
    ApiPath((project_id, content_hash)): ApiPath<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    validate_content_hash(&content_hash, &request_id)?;
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    database
        .validate_sync_project(claims.sub, &claims.role, project_id)
        .await
        .map_err(|error| map_project_error(error, request_id.clone()))?;
    let object_store = sync_object_store(&state, &request_id)?;
    let payload = object_store
        .head_payload(claims.sub, project_id, &content_hash)
        .await
        .map_err(|error| map_object_store(&error, request_id.clone()))?
        .ok_or_else(|| payload_not_found(request_id.clone()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, payload.size)
        .header(header::CONTENT_TYPE, payload.media_type)
        .body(Body::empty())
        .map_err(|_| ApiError::internal(request_id))
}

#[allow(clippy::too_many_lines)]
pub async fn put_payload(
    State(state): State<AppState>,
    ApiPath((project_id, content_hash)): ApiPath<(Uuid, String)>,
    request: Request,
) -> Result<Response, ApiError> {
    let request_id = request_id(request.headers());
    validate_content_hash(&content_hash, &request_id)?;
    let content_length = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|length| *length <= MAX_PAYLOAD_BYTES)
        .ok_or_else(|| ApiError::invalid("Content-Length 缺失或超过 10 MiB", request_id.clone()))?;
    let media_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty() && value.len() <= 255)
        .map(str::to_owned)
        .ok_or_else(|| ApiError::invalid("Content-Type 缺失或无效", request_id.clone()))?;
    let (database, claims) = authenticate(&state, request.headers(), &request_id).await?;
    let auth = state
        .auth
        .as_ref()
        .ok_or_else(|| ApiError::unavailable(request_id.clone()))?;
    if !auth.allow_auth_request(
        AuthOperation::PayloadUpload,
        &format!("{}:{project_id}", claims.sub),
    ) {
        return Err(ApiError::rate_limited(request_id));
    }
    enforce_shared_rate_limit(
        database,
        &format!("payload_upload:{}:{project_id}", claims.sub),
        &request_id,
    )
    .await?;
    database
        .validate_sync_project(claims.sub, &claims.role, project_id)
        .await
        .map_err(|error| map_project_error(error, request_id.clone()))?;
    let object_store = sync_object_store(&state, &request_id)?;

    let _temp_reservation = state
        .upload_budget
        .reserve(u64::try_from(content_length).unwrap_or(u64::MAX))
        .ok_or_else(|| ApiError::unavailable(request_id.clone()))?;
    let temporary_dir =
        std::env::var_os("TASKTIPS_UPLOAD_TEMP_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    tokio::fs::create_dir_all(&temporary_dir)
        .await
        .map_err(|_| ApiError::unavailable(request_id.clone()))?;
    let temporary_path = temporary_dir.join(format!(
        "tasktips-payload-{}-{}.tmp",
        claims.sub,
        Uuid::new_v4()
    ));
    let mut temporary_options = tokio::fs::OpenOptions::new();
    temporary_options.write(true).create_new(true);
    #[cfg(unix)]
    temporary_options.mode(0o600);
    let mut temporary_file = temporary_options
        .open(&temporary_path)
        .await
        .map_err(|_| ApiError::unavailable(request_id.clone()))?;
    let mut stream = request.into_body().into_data_stream();
    let mut hasher = Sha256::new();
    let mut received = 0_usize;
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| ApiError::invalid("payload 请求体读取失败", request_id.clone()));
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tokio::fs::remove_file(&temporary_path).await;
                return Err(error);
            }
        };
        let next_size = received.saturating_add(chunk.len());
        if next_size > content_length || next_size > MAX_PAYLOAD_BYTES {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(ApiError::invalid(
                "payload 大小与 Content-Length 不一致",
                request_id,
            ));
        }
        hasher.update(&chunk);
        if temporary_file.write_all(&chunk).await.is_err() {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(ApiError::unavailable(request_id));
        }
        received = next_size;
    }
    if temporary_file.flush().await.is_err() {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(ApiError::unavailable(request_id));
    }
    drop(temporary_file);
    if received != content_length {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(ApiError::invalid(
            "payload 大小与 Content-Length 不一致",
            request_id,
        ));
    }
    if hex::encode(hasher.finalize()) != content_hash {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "CONTENT_HASH_MISMATCH",
            "payload 的 SHA-256 与路径不一致",
            false,
            request_id,
        ));
    }
    let payload_lock = match database
        .acquire_payload_lock(project_id, &content_hash)
        .await
    {
        Ok(lock) => lock,
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(map_project_error(error, request_id));
        }
    };
    let payload = match object_store
        .put_payload_file(
            claims.sub,
            project_id,
            &content_hash,
            &media_type,
            &temporary_path,
            content_length as u64,
        )
        .await
    {
        Ok(payload) => payload,
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            payload_lock.rollback().await;
            return Err(map_object_store(&error, request_id));
        }
    };
    let _ = tokio::fs::remove_file(&temporary_path).await;
    state.metrics.payload_upload_bytes_total.fetch_add(
        u64::try_from(content_length).unwrap_or(u64::MAX),
        std::sync::atomic::Ordering::Relaxed,
    );
    payload_lock
        .register_payload(
            claims.sub,
            &claims.role,
            project_id,
            &StoredPayload {
                content_hash: payload.content_hash,
                bucket: payload.bucket,
                object_key: payload.key,
                size: i64::try_from(payload.size)
                    .map_err(|_| ApiError::internal(request_id.clone()))?,
                media_type: payload.media_type,
            },
        )
        .await
        .map_err(|error| map_project_error(error, request_id.clone()))?;
    Response::builder()
        .status(StatusCode::CREATED)
        .body(Body::empty())
        .map_err(|_| ApiError::internal(request_id))
}

pub async fn get_payload(
    State(state): State<AppState>,
    ApiPath((project_id, content_hash)): ApiPath<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    validate_content_hash(&content_hash, &request_id)?;
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    database
        .validate_sync_project(claims.sub, &claims.role, project_id)
        .await
        .map_err(|error| map_project_error(error, request_id.clone()))?;
    let download = sync_object_store(&state, &request_id)?
        .get_payload(claims.sub, project_id, &content_hash, range.as_deref())
        .await
        .map_err(|error| map_object_store(&error, request_id.clone()))?;
    if let Some(length) = download.content_length {
        state.metrics.payload_download_bytes_total.fetch_add(
            u64::try_from(length).unwrap_or_default(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    let mut response = Response::builder()
        .status(if download.content_range.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, download.media_type)
        .header(header::ACCEPT_RANGES, "bytes");
    if let Some(length) = download.content_length {
        response = response.header(header::CONTENT_LENGTH, length);
    }
    if let Some(content_range) = download.content_range {
        response = response.header(header::CONTENT_RANGE, content_range);
    }
    response
        .body(Body::from_stream(ReaderStream::new(
            download.body.into_async_read(),
        )))
        .map_err(|_| ApiError::internal(request_id))
}

#[allow(clippy::too_many_lines)]
pub async fn bootstrap(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<BootstrapRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let limit = valid_limit(request.limit, 200, 1000, &request_id)?;
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let signer = state
        .cursor
        .as_ref()
        .ok_or_else(|| ApiError::unavailable(request_id.clone()))?;
    let page_claims = request
        .page_token
        .as_deref()
        .map(|cursor| {
            verify_cursor(
                signer,
                cursor,
                CursorKind::Bootstrap,
                project_id,
                claims.sub,
                &request_id,
            )
        })
        .transpose()?;
    let page = if let Some(page_claims) = page_claims.as_ref() {
        let manifest_id = page_claims
            .manifest_id
            .ok_or_else(|| ApiError::cursor_invalid(request_id.clone()))?;
        database
            .bootstrap_manifest_page(
                claims.sub,
                &claims.role,
                claims.device_id,
                project_id,
                manifest_id,
                page_claims.offset,
                limit,
            )
            .await
            .map_err(|error| match error {
                PersistenceError::NotFound => ApiError::cursor_invalid(request_id.clone()),
                other => map_project_error(other, request_id.clone()),
            })?
    } else {
        let manifest_id = Uuid::new_v4();
        let manifest = database
            .prepare_bootstrap_manifest(claims.sub, &claims.role, project_id, manifest_id)
            .await
            .map_err(|error| map_project_error(error, request_id.clone()))?;
        let object_store = state
            .object_store
            .as_ref()
            .ok_or_else(|| ApiError::unavailable(request_id.clone()))?;
        let stored = object_store
            .put_bootstrap_manifest(
                claims.sub,
                project_id,
                manifest.id,
                &manifest.hash,
                Bytes::from(manifest.bytes.clone()),
            )
            .await
            .map_err(|error| map_object_store(&error, request_id.clone()))?;
        database
            .record_bootstrap_manifest(
                claims.sub,
                &claims.role,
                project_id,
                &manifest,
                &stored.bucket,
                &stored.key,
            )
            .await
            .map_err(|error| map_project_error(error, request_id.clone()))?;
        database
            .bootstrap_manifest_page(
                claims.sub,
                &claims.role,
                claims.device_id,
                project_id,
                manifest.id,
                0,
                limit,
            )
            .await
            .map_err(|error| map_project_error(error, request_id.clone()))?
    };
    let next_offset = page_claims.as_ref().map_or(0, |claims| claims.offset)
        + i64::try_from(page.records.len()).map_err(|_| ApiError::internal(request_id.clone()))?;
    let next_page_token = page
        .has_more
        .then(|| {
            signer.sign(cursor_claims(
                CursorKind::Bootstrap,
                project_id,
                claims.sub,
                page.generation,
                page.snapshot_sequence,
                next_offset,
                Some(page.manifest_id),
            ))
        })
        .transpose()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    let cursor = (!page.has_more)
        .then(|| {
            signer.sign(cursor_claims(
                CursorKind::Pull,
                project_id,
                claims.sub,
                page.generation,
                page.snapshot_sequence,
                0,
                None,
            ))
        })
        .transpose()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    Ok(Json(serde_json::json!({
        "generation": page.generation,
        "items": page.records.iter().map(sync_record_json).collect::<Vec<_>>(),
        "hasMore": page.has_more,
        "nextPageToken": next_page_token,
        "cursor": cursor
    })))
}

pub async fn pull(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<PullRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let limit = valid_limit(request.limit, 200, 1000, &request_id)?;
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let signer = state
        .cursor
        .as_ref()
        .ok_or_else(|| ApiError::unavailable(request_id.clone()))?;
    let cursor = verify_cursor(
        signer,
        &request.cursor,
        CursorKind::Pull,
        project_id,
        claims.sub,
        &request_id,
    )?;
    let page = database
        .pull_page(
            claims.sub,
            &claims.role,
            claims.device_id,
            project_id,
            cursor.generation,
            cursor.change_sequence,
            limit,
        )
        .await
        .map_err(|error| match error {
            PersistenceError::NotFound => ApiError::cursor_invalid(request_id.clone()),
            other => map_project_error(other, request_id.clone()),
        })?;
    let next_cursor = signer
        .sign(cursor_claims(
            CursorKind::Pull,
            project_id,
            claims.sub,
            page.generation,
            page.next_sequence,
            0,
            None,
        ))
        .map_err(|_| ApiError::internal(request_id))?;
    state
        .metrics
        .sync_pull_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(Json(serde_json::json!({
        "generation": page.generation,
        "changes": page.records.iter().map(sync_record_json).collect::<Vec<_>>(),
        "nextCursor": next_cursor,
        "hasMore": page.has_more
    })))
}

#[allow(clippy::too_many_lines)]
pub async fn push(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiLimitedJson(request): ApiLimitedJson<PushRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    if request.generation <= 0
        || request.request_id.trim().is_empty()
        || request.request_id.len() > 128
        || request
            .objects
            .len()
            .saturating_add(request.tombstones.len())
            > 100
    {
        return Err(ApiError::invalid("push 请求字段或批量大小无效", request_id));
    }
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let started = Instant::now();
    let mut revisions = Vec::with_capacity(request.objects.len() + request.tombstones.len());
    for object in &request.objects {
        validate_push_item(
            object.kind,
            &object.id,
            object.schema_version,
            object.revision,
            object.base_revision,
            &object.content_hash,
            object.device_id,
            claims.device_id,
            &request_id,
        )?;
        revisions.push(NewSyncRevision {
            kind: object.kind,
            id: object.id.clone(),
            schema_version: Some(object.schema_version),
            base_revision: object.base_revision,
            content_hash: Some(object.content_hash.clone()),
            changed_at: object.updated_at,
            tombstone: false,
        });
    }
    for tombstone in &request.tombstones {
        validate_revision_fields(
            tombstone.kind,
            &tombstone.id,
            tombstone.revision,
            tombstone.base_revision,
            tombstone.device_id,
            claims.device_id,
            &request_id,
        )?;
        revisions.push(NewSyncRevision {
            kind: tombstone.kind,
            id: tombstone.id.clone(),
            schema_version: None,
            base_revision: tombstone.base_revision,
            content_hash: None,
            changed_at: tombstone.deleted_at,
            tombstone: true,
        });
    }
    let canonical_request = serde_json::to_vec(&request)
        .map_err(|_| ApiError::invalid("push 请求无法规范化", request_id.clone()))?;
    let request_hash = hex::encode(Sha256::digest(canonical_request));
    let push_result = database
        .push_with_outcome(
            claims.sub,
            &claims.role,
            claims.device_id,
            project_id,
            request.generation,
            &request.request_id,
            &request_hash,
            &revisions,
        )
        .await;
    match push_result {
        Ok(outcome) => {
            let response = outcome.response;
            state
                .metrics
                .sync_push_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let item_count = response
                .get("results")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            let status = if response
                .get("results")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item["status"] == "conflict"))
            {
                state
                    .metrics
                    .sync_conflict_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                "conflict"
            } else {
                "succeeded"
            };
            if !outcome.replayed {
                let _ = database
                    .record_sync_attempt(
                        claims.sub,
                        &claims.role,
                        &tasktips_persistence::SyncAttemptRecord {
                            id: Uuid::new_v4(),
                            owner_user_id: claims.sub,
                            project_id,
                            device_id: Some(claims.device_id),
                            operation: "push".to_owned(),
                            status: status.to_owned(),
                            error_code: None,
                            item_count: i32::try_from(item_count).unwrap_or(i32::MAX),
                            latency_ms: i32::try_from(started.elapsed().as_millis()).ok(),
                            created_at: time::OffsetDateTime::now_utc(),
                        },
                    )
                    .await;
            }
            Ok(Json(response))
        }
        Err(error) => {
            let replayed = matches!(&error, PersistenceError::IdempotencyReplay { .. });
            state
                .metrics
                .sync_push_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let api_error = map_project_error(error, request_id);
            if !replayed {
                let _ = database
                    .record_sync_attempt(
                        claims.sub,
                        &claims.role,
                        &tasktips_persistence::SyncAttemptRecord {
                            id: Uuid::new_v4(),
                            owner_user_id: claims.sub,
                            project_id,
                            device_id: Some(claims.device_id),
                            operation: "push".to_owned(),
                            status: "failed".to_owned(),
                            error_code: Some(api_error.code().to_owned()),
                            item_count: 0,
                            latency_ms: i32::try_from(started.elapsed().as_millis()).ok(),
                            created_at: time::OffsetDateTime::now_utc(),
                        },
                    )
                    .await;
            }
            Err(api_error)
        }
    }
}

pub async fn history(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    history_response(&state, project_id, headers, query, None, None).await
}

pub async fn object_history(
    State(state): State<AppState>,
    ApiPath((project_id, kind, object_id)): ApiPath<(Uuid, String, String)>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let kind = parse_object_kind(&kind)
        .ok_or_else(|| ApiError::invalid("对象类型无效", request_id(&headers)))?;
    history_response(
        &state,
        project_id,
        headers,
        query,
        Some(kind),
        Some(object_id),
    )
    .await
}

async fn history_response(
    state: &AppState,
    project_id: Uuid,
    headers: HeaderMap,
    query: HistoryQuery,
    kind: Option<ObjectKind>,
    object_id: Option<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    if query.after_sequence.unwrap_or(0) < 0 {
        return Err(ApiError::invalid("afterSequence 无效", request_id));
    }
    if query.object_id.is_some() && kind.is_none() {
        return Err(ApiError::invalid("history 查询条件无效", request_id));
    }
    let query_kind = match query.kind.as_deref() {
        Some(value) => Some(
            parse_object_kind(value)
                .ok_or_else(|| ApiError::invalid("对象类型无效", request_id.clone()))?,
        ),
        None => None,
    };
    let limit = valid_limit(query.limit, 200, 1000, &request_id)?;
    let (database, claims) = authenticate(state, &headers, &request_id).await?;
    let (records, has_more) = database
        .list_history(
            claims.sub,
            &claims.role,
            project_id,
            kind.or(query_kind),
            object_id.as_deref().or(query.object_id.as_deref()),
            query.after_sequence.unwrap_or(0),
            limit,
        )
        .await
        .map_err(|error| map_project_error(error, request_id.clone()))?;
    let next_sequence = records
        .last()
        .filter(|_| has_more)
        .map(|record| record.change_sequence);
    Ok(Json(json!({
        "items": records.iter().map(sync_record_json).collect::<Vec<_>>(),
        "hasMore": has_more,
        "nextSequence": next_sequence
    })))
}

pub async fn create_snapshot(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<tasktips_persistence::SnapshotRecord>), ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let snapshot_id = Uuid::new_v4();
    let (generation, sequence, manifest, bytes) = database
        .snapshot_manifest(claims.sub, &claims.role, project_id, snapshot_id)
        .await
        .map_err(|error| map_project_error(error, request_id.clone()))?;
    let hash = hex::encode(Sha256::digest(&bytes));
    let object_store = sync_object_store(&state, &request_id)?;
    let stored = object_store
        .put_manifest(
            claims.sub,
            project_id,
            snapshot_id,
            &hash,
            bytes::Bytes::from(bytes),
        )
        .await
        .map_err(|error| map_object_store(&error, request_id.clone()))?;
    let snapshot = database
        .record_snapshot(
            claims.sub,
            &claims.role,
            project_id,
            snapshot_id,
            generation,
            sequence,
            &hash,
            &stored.bucket,
            &stored.key,
            manifest,
        )
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok((StatusCode::CREATED, Json(snapshot)))
}

pub async fn list_snapshots(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let snapshots = database
        .list_snapshots(claims.sub, &claims.role, project_id)
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok(Json(json!({"items": snapshots})))
}

pub async fn create_restore(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<RestoreRequest>,
) -> Result<(StatusCode, Json<tasktips_persistence::RestoreJobRecord>), ApiError> {
    let request_id = request_id(&headers);
    let reason = valid_text(&request.reason, 512, "恢复原因无效", &request_id)?;
    if validate_restore_target(request.snapshot_id, request.target_change_sequence).is_err() {
        return Err(ApiError::invalid("恢复目标无效", request_id));
    }
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let job = database
        .enqueue_restore(
            claims.sub,
            &claims.role,
            project_id,
            request.snapshot_id,
            request.target_change_sequence,
            reason,
            &request_id,
        )
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    state
        .metrics
        .restore_job_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok((StatusCode::ACCEPTED, Json(job)))
}

pub async fn get_restore(
    State(state): State<AppState>,
    ApiPath((project_id, restore_id)): ApiPath<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<tasktips_persistence::RestoreJobRecord>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let job = database
        .get_restore_job(claims.sub, &claims.role, project_id, restore_id)
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok(Json(job))
}

pub async fn cancel_restore(
    State(state): State<AppState>,
    ApiPath((project_id, restore_id)): ApiPath<(Uuid, Uuid)>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ReasonRequest>,
) -> Result<Json<tasktips_persistence::RestoreJobRecord>, ApiError> {
    let request_id = request_id(&headers);
    let reason = valid_text(&request.reason, 512, "取消原因无效", &request_id)?;
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let job = database
        .cancel_restore(
            claims.sub,
            &claims.role,
            project_id,
            restore_id,
            reason,
            &request_id,
        )
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok(Json(job))
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    let email = normalize_email(&request.email)
        .ok_or_else(|| ApiError::invalid("邮箱格式无效", request_id.clone()))?;
    let (database, auth) = services(&state, &request_id)?;
    if !auth.allow_auth_request(AuthOperation::Login, &auth_rate_limit_key(&headers, &email)) {
        return Err(ApiError::rate_limited(request_id));
    }
    enforce_shared_rate_limit(
        database,
        &format!("login:{}", auth_rate_limit_key(&headers, &email)),
        &request_id,
    )
    .await?;
    let user = database
        .find_login_user(&email)
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    let Some(user) = user else {
        run_password_task(request.password, |password| {
            hash_password(&password).map(|_| true)
        })
        .await
        .map_err(|_| ApiError::authentication(request_id.clone()))?;
        return Err(ApiError::authentication(request_id));
    };
    let password_hash = user.password_hash.clone();
    let valid = run_password_task(request.password, move |password| {
        Ok(verify_password(&password, &password_hash))
    })
    .await
    .map_err(|_| ApiError::internal(request_id.clone()))?;
    if !valid {
        return Err(ApiError::authentication(request_id));
    }
    if user.role != "user" {
        return Err(ApiError::authentication(request_id));
    }
    if user.status != "active" {
        return Err(account_disabled(request_id));
    }

    let opaque = auth
        .issue_refresh_token()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    let refresh = NewRefreshToken {
        token_hash: opaque.hash,
        family_id: Uuid::new_v4(),
        expires_at: refresh_expiry(auth.refresh_ttl_seconds()),
    };
    let session = database
        .create_login_session(user.id, request.device_id, &refresh, &request_id)
        .await
        .map_err(|error| match error {
            PersistenceError::NotFound => ApiError::authentication(request_id.clone()),
            other => map_persistence(other, request_id.clone()),
        })?;
    user_token_response(
        auth,
        &session.user,
        session.device_id,
        opaque.raw,
        &request_id,
    )
}

pub async fn admin_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<AdminLoginRequest>,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let email = normalize_email(&request.email)
        .ok_or_else(|| ApiError::invalid("邮箱格式无效", request_id.clone()))?;
    let (database, auth) = services(&state, &request_id)?;
    if !auth.allow_auth_request(
        AuthOperation::AdminLogin,
        &auth_rate_limit_key(&headers, &email),
    ) {
        return Err(ApiError::rate_limited(request_id));
    }
    enforce_shared_rate_limit(
        database,
        &format!("admin_login:{}", auth_rate_limit_key(&headers, &email)),
        &request_id,
    )
    .await?;
    let user = database
        .find_login_user(&email)
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    let Some(user) = user else {
        run_password_task(request.password, |password| {
            hash_password(&password).map(|_| true)
        })
        .await
        .map_err(|_| ApiError::authentication(request_id.clone()))?;
        return Err(ApiError::authentication(request_id));
    };
    let password_hash = user.password_hash.clone();
    let valid = run_password_task(request.password, move |password| {
        Ok(verify_password(&password, &password_hash))
    })
    .await
    .map_err(|_| ApiError::internal(request_id.clone()))?;
    if !valid || user.role != "system_admin" {
        return Err(ApiError::authentication(request_id));
    }
    if user.status != "active" {
        return Err(account_disabled(request_id));
    }
    let opaque = auth
        .issue_refresh_token()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    let refresh = NewRefreshToken {
        token_hash: opaque.hash,
        family_id: Uuid::new_v4(),
        expires_at: refresh_expiry(auth.refresh_ttl_seconds()),
    };
    let session = database
        .create_login_session(user.id, request.device_id, &refresh, &request_id)
        .await
        .map_err(|error| match error {
            PersistenceError::NotFound => ApiError::authentication(request_id.clone()),
            other => map_persistence(other, request_id.clone()),
        })?;
    admin_token_response(
        auth,
        &session.user,
        session.device_id,
        &opaque.raw,
        &request_id,
    )
}

pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<RefreshRequest>,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    let (database, auth) = services(&state, &request_id)?;
    if !auth.allow_auth_request(
        AuthOperation::Refresh,
        &auth_rate_limit_key(&headers, "refresh"),
    ) {
        return Err(ApiError::rate_limited(request_id));
    }
    enforce_shared_rate_limit(
        database,
        &format!("refresh:{}", auth_rate_limit_key(&headers, "refresh")),
        &request_id,
    )
    .await?;
    let refresh_token = request.refresh_token;
    if refresh_token.is_empty() {
        return Err(ApiError::authentication(request_id));
    }
    let opaque = auth
        .issue_refresh_token()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    let replacement = NewRefreshToken {
        token_hash: opaque.hash,
        family_id: Uuid::new_v4(),
        expires_at: refresh_expiry(auth.refresh_ttl_seconds()),
    };
    let session = database
        .rotate_refresh_token_for_role(
            &opaque_token_hash(&refresh_token),
            &replacement,
            "user",
            &request_id,
        )
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    user_token_response(
        auth,
        &session.user,
        session.device_id,
        opaque.raw,
        &request_id,
    )
}

pub async fn admin_refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let (database, auth) = services(&state, &request_id)?;
    if !auth.allow_auth_request(
        AuthOperation::AdminRefresh,
        &auth_rate_limit_key(&headers, "admin-refresh"),
    ) {
        return Err(ApiError::rate_limited(request_id));
    }
    enforce_shared_rate_limit(
        database,
        &format!(
            "admin_refresh:{}",
            auth_rate_limit_key(&headers, "admin-refresh")
        ),
        &request_id,
    )
    .await?;
    let refresh_token = refresh_token_from_cookie(&headers)
        .ok_or_else(|| ApiError::authentication(request_id.clone()))?;
    let opaque = auth
        .issue_refresh_token()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    let replacement = NewRefreshToken {
        token_hash: opaque.hash,
        family_id: Uuid::new_v4(),
        expires_at: refresh_expiry(auth.refresh_ttl_seconds()),
    };
    let session = database
        .rotate_refresh_token_for_role(
            &opaque_token_hash(&refresh_token),
            &replacement,
            "system_admin",
            &request_id,
        )
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    admin_token_response(
        auth,
        &session.user,
        session.device_id,
        &opaque.raw,
        &request_id,
    )
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    database
        .logout_device(claims.sub, &claims.role, claims.device_id, &request_id)
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn admin_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    if refresh_token_from_cookie(&headers).is_none() {
        return Err(ApiError::authentication(request_id));
    }
    database
        .logout_device(claims.sub, &claims.role, claims.device_id, &request_id)
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    database
        .revoke_reauth_nonces(claims.sub, claims.device_id)
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        admin_refresh_cookie_header("", 0, &request_id)?,
    );
    Ok(response)
}

pub async fn admin_reauth(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<AdminReauthRequest>,
) -> Result<Json<ReauthResponse>, ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let (database, auth) = services(&state, &request_id)?;
    let (_, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    if !auth.allow_auth_request(
        AuthOperation::AdminReauth,
        &auth_rate_limit_key(&headers, "admin-reauth"),
    ) {
        return Err(ApiError::rate_limited(request_id));
    }
    enforce_shared_rate_limit(
        database,
        &format!(
            "admin_reauth:{}",
            auth_rate_limit_key(&headers, "admin-reauth")
        ),
        &request_id,
    )
    .await?;
    let user = database
        .current_user(claims.sub, &claims.role)
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    let password_hash = user.password_hash.clone();
    let valid = run_password_task(request.password, move |password| {
        Ok(verify_password(&password, &password_hash))
    })
    .await
    .map_err(|_| ApiError::internal(request_id.clone()))?;
    if !valid {
        return Err(ApiError::authentication(request_id));
    }
    let (nonce, expires_in) = auth
        .issue_reauth_nonce()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    database
        .create_reauth_nonce(
            claims.sub,
            claims.device_id,
            &nonce.hash,
            time::OffsetDateTime::now_utc() + time::Duration::seconds(expires_in),
        )
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    Ok(Json(ReauthResponse {
        nonce: nonce.raw,
        expires_in,
    }))
}

pub async fn activate_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<InvitationActivationRequest>,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    let (database, auth) = services(&state, &request_id)?;
    if !auth.allow_auth_request(
        AuthOperation::InvitationActivation,
        &auth_rate_limit_key(&headers, "invitation"),
    ) {
        return Err(ApiError::rate_limited(request_id));
    }
    enforce_shared_rate_limit(
        database,
        &format!(
            "invitation_activation:{}",
            auth_rate_limit_key(&headers, "invitation")
        ),
        &request_id,
    )
    .await?;
    if !valid_password(&request.password) {
        return Err(ApiError::invalid("密码至少需要 12 个字符", request_id));
    }
    let password_hash = run_password_task(request.password, |password| hash_password(&password))
        .await
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    let opaque = auth
        .issue_refresh_token()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    let refresh = NewRefreshToken {
        token_hash: opaque.hash,
        family_id: Uuid::new_v4(),
        expires_at: refresh_expiry(auth.refresh_ttl_seconds()),
    };
    let session = database
        .activate_invitation(
            &opaque_token_hash(&request.invitation_token),
            &password_hash,
            request.device_id,
            &refresh,
            &request_id,
        )
        .await
        .map_err(|error| match error {
            PersistenceError::InvalidInvitation
            | PersistenceError::NotFound
            | PersistenceError::DeviceRevoked => {
                ApiError::invalid("邀请无效或已过期", request_id.clone())
            }
            other => map_persistence(other, request_id.clone()),
        })?;
    user_token_response(
        auth,
        &session.user,
        session.device_id,
        opaque.raw,
        &request_id,
    )
}

pub async fn current_user(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<CurrentUserResponse>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let user = database
        .current_user(claims.sub, &claims.role)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(CurrentUserResponse {
        id: user.id,
        email: user.email,
        role: user.role,
        status: user.status,
    }))
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<PasswordChangeRequest>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id(&headers);
    if !valid_password(&request.new_password) {
        return Err(ApiError::invalid("新密码至少需要 12 个字符", request_id));
    }
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let user = database
        .current_user(claims.sub, &claims.role)
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    let current_valid = run_password_task(request.current_password, move |password| {
        Ok(verify_password(&password, &user.password_hash))
    })
    .await
    .map_err(|_| ApiError::internal(request_id.clone()))?;
    if !current_valid {
        return Err(ApiError::authentication(request_id));
    }
    let password_hash =
        run_password_task(request.new_password, |password| hash_password(&password))
            .await
            .map_err(|_| ApiError::internal(request_id.clone()))?;
    database
        .change_password(claims.sub, &claims.role, &password_hash)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let projects = database
        .list_projects(claims.sub, &claims.role)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(serde_json::json!({"items": projects})))
}

pub async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ProjectRequest>,
) -> Result<(StatusCode, Json<tasktips_persistence::ProjectRecord>), ApiError> {
    let request_id = request_id(&headers);
    let name = valid_text(&request.name, 128, "项目名称无效", &request_id)?;
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let project = database
        .create_project(claims.sub, &claims.role, name)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok((StatusCode::CREATED, Json(project)))
}

pub async fn get_project(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
) -> Result<Json<tasktips_persistence::ProjectRecord>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let project = database
        .get_project(claims.sub, &claims.role, project_id)
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok(Json(project))
}

pub async fn rename_project(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ProjectRequest>,
) -> Result<Json<tasktips_persistence::ProjectRecord>, ApiError> {
    let request_id = request_id(&headers);
    let name = valid_text(&request.name, 128, "项目名称无效", &request_id)?;
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let project = database
        .rename_project(claims.sub, &claims.role, project_id, name)
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok(Json(project))
}

pub async fn disable_project(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
) -> Result<Json<tasktips_persistence::ProjectRecord>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let project = database
        .disable_project(claims.sub, &claims.role, project_id)
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok(Json(project))
}

pub async fn purge_project(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<PurgeProjectRequest>,
) -> Result<(StatusCode, Json<tasktips_persistence::JobRecord>), ApiError> {
    let request_id = request_id(&headers);
    let reason = valid_text(&request.reason, 512, "清除原因无效", &request_id)?;
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let auth = state
        .auth
        .as_ref()
        .ok_or_else(|| ApiError::unavailable(request_id.clone()))?;
    if !auth.allow_auth_request(
        AuthOperation::Purge,
        &format!("{}:{}", claims.sub, auth_rate_limit_key(&headers, "purge")),
    ) {
        return Err(ApiError::rate_limited(request_id));
    }
    enforce_shared_rate_limit(database, &format!("purge:{}", claims.sub), &request_id).await?;
    let user = database
        .current_user(claims.sub, &claims.role)
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    let password_hash = user.password_hash;
    let valid = run_password_task(request.password, move |password| {
        Ok(verify_password(&password, &password_hash))
    })
    .await
    .map_err(|_| ApiError::internal(request_id.clone()))?;
    if !valid {
        return Err(ApiError::authentication(request_id));
    }
    let job = database
        .enqueue_project_purge(claims.sub, &claims.role, project_id, reason, &request_id)
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

pub async fn list_devices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let devices = database
        .list_devices(claims.sub, &claims.role)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(serde_json::json!({"items": devices})))
}

pub async fn register_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<RegisterDeviceRequest>,
) -> Result<(StatusCode, Json<tasktips_persistence::DeviceRecord>), ApiError> {
    let request_id = request_id(&headers);
    let display_name = valid_text(&request.display_name, 128, "设备名称无效", &request_id)?;
    let platform = valid_text(&request.platform, 64, "平台名称无效", &request_id)?;
    let app_version = valid_text(&request.app_version, 64, "应用版本无效", &request_id)?;
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    if claims.device_id != request.device_id {
        return Err(ApiError::authorization(request_id));
    }
    let device = database
        .register_device(
            claims.sub,
            &claims.role,
            request.device_id,
            &DeviceProfile {
                display_name: display_name.to_owned(),
                platform: platform.to_owned(),
                app_version: app_version.to_owned(),
            },
        )
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok((StatusCode::CREATED, Json(device)))
}

pub async fn update_device(
    State(state): State<AppState>,
    ApiPath(device_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<UpdateDeviceRequest>,
) -> Result<Json<tasktips_persistence::DeviceRecord>, ApiError> {
    let request_id = request_id(&headers);
    let display_name = valid_text(&request.display_name, 128, "设备名称无效", &request_id)?;
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let device = database
        .update_device(claims.sub, &claims.role, device_id, display_name)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(device))
}

pub async fn revoke_device(
    State(state): State<AppState>,
    ApiPath(device_id): ApiPath<Uuid>,
    headers: HeaderMap,
) -> Result<Json<tasktips_persistence::DeviceRecord>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate(&state, &headers, &request_id).await?;
    let device = database
        .revoke_device(claims.sub, &claims.role, device_id, &request_id)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(device))
}

pub async fn create_invitation(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<CreateInvitationRequest>,
) -> Result<(StatusCode, Json<CreateInvitationResponse>), ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let email = normalize_email(&request.email)
        .ok_or_else(|| ApiError::invalid("邮箱格式无效", request_id.clone()))?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let auth = state
        .auth
        .as_ref()
        .ok_or_else(|| ApiError::unavailable(request_id.clone()))?;
    let token = auth
        .issue_invitation_token()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    let record = database
        .create_invitation(
            claims.sub,
            &NewInvitation {
                email_normalized: email,
                email_display: request.email.trim().to_owned(),
                token_hash: token.hash,
                expires_at: invitation_expiry(),
            },
            &request_id,
        )
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok((
        StatusCode::CREATED,
        Json(CreateInvitationResponse {
            id: record.id,
            email: record.email,
            invitation_token: token.raw,
            expires_at: record.expires_at,
        }),
    ))
}

pub async fn admin_list_invitations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminPageQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (limit, offset) = admin_page_bounds(query.limit, query.offset, &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let (invitations, has_more) = database
        .admin_list_invitations_page(claims.sub, limit, offset)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    let next_offset = has_more.then_some(offset.saturating_add(limit));
    Ok(Json(
        json!({"items": invitations, "hasMore": has_more, "nextOffset": next_offset, "limit": limit, "offset": offset}),
    ))
}

pub async fn admin_revoke_invitation(
    State(state): State<AppState>,
    ApiPath(invitation_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ReasonRequest>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let reason = valid_text(&request.reason, 500, "操作原因无效", &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    database
        .admin_revoke_invitation(claims.sub, invitation_id, reason, &request_id)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn admin_resend_invitation(
    State(state): State<AppState>,
    ApiPath(invitation_id): ApiPath<Uuid>,
    headers: HeaderMap,
) -> Result<Json<CreateInvitationResponse>, ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let auth = state
        .auth
        .as_ref()
        .ok_or_else(|| ApiError::unavailable(request_id.clone()))?;
    let token = auth
        .issue_invitation_token()
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    let record = database
        .admin_resend_invitation(
            claims.sub,
            invitation_id,
            &token.hash,
            invitation_expiry(),
            &request_id,
        )
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(CreateInvitationResponse {
        id: record.id,
        email: record.email,
        invitation_token: token.raw,
        expires_at: record.expires_at,
    }))
}

pub async fn admin_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminPageQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (limit, offset) = admin_page_bounds(query.limit, query.offset, &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let (users, has_more) = database
        .admin_list_users_page(claims.sub, limit, offset)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    let next_offset = has_more.then_some(offset.saturating_add(limit));
    Ok(Json(
        json!({"items": users, "hasMore": has_more, "nextOffset": next_offset, "limit": limit, "offset": offset}),
    ))
}

pub async fn admin_list_all_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminProjectQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (limit, offset) = admin_page_bounds(query.limit, query.offset, &request_id)?;
    let search = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if search.is_some_and(|value| value.chars().count() > 120) {
        return Err(ApiError::invalid("项目搜索条件无效", request_id));
    }
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let (projects, has_more) = database
        .admin_list_all_projects_page(claims.sub, limit, offset, search)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    let next_offset = has_more.then_some(offset.saturating_add(limit));
    Ok(Json(
        json!({"items": projects, "hasMore": has_more, "nextOffset": next_offset, "limit": limit, "offset": offset}),
    ))
}

pub async fn admin_list_all_devices(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminPageQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (limit, offset) = admin_page_bounds(query.limit, query.offset, &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let (devices, has_more) = database
        .admin_list_all_devices_page(claims.sub, limit, offset)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    let next_offset = has_more.then_some(offset.saturating_add(limit));
    Ok(Json(
        json!({"items": devices, "hasMore": has_more, "nextOffset": next_offset, "limit": limit, "offset": offset}),
    ))
}

pub async fn admin_list_projects(
    State(state): State<AppState>,
    ApiPath(user_id): ApiPath<Uuid>,
    headers: HeaderMap,
    Query(query): Query<AdminPageQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (limit, offset) = admin_page_bounds(query.limit, query.offset, &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let (projects, has_more) = database
        .admin_list_projects_page(claims.sub, user_id, limit, offset)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    let next_offset = has_more.then_some(offset.saturating_add(limit));
    Ok(Json(
        json!({"items": projects, "hasMore": has_more, "nextOffset": next_offset, "limit": limit, "offset": offset}),
    ))
}

pub async fn admin_list_devices(
    State(state): State<AppState>,
    ApiPath(user_id): ApiPath<Uuid>,
    headers: HeaderMap,
    Query(query): Query<AdminPageQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (limit, offset) = admin_page_bounds(query.limit, query.offset, &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let (devices, has_more) = database
        .admin_list_devices_page(claims.sub, user_id, limit, offset)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    let next_offset = has_more.then_some(offset.saturating_add(limit));
    Ok(Json(
        json!({"items": devices, "hasMore": has_more, "nextOffset": next_offset, "limit": limit, "offset": offset}),
    ))
}

pub async fn admin_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<tasktips_persistence::AdminOverview>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let overview = database
        .admin_overview(claims.sub)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(overview))
}

pub async fn admin_history_metadata(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let limit = valid_limit(query.limit, 200, 1000, &request_id)?;
    let after_sequence = query.after_sequence.unwrap_or(0);
    if after_sequence < 0 {
        return Err(ApiError::invalid("afterSequence 无效", request_id));
    }
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let (items, has_more) = database
        .admin_history_metadata(claims.sub, project_id, after_sequence, limit)
        .await
        .map_err(|error| map_project_error(error, request_id.clone()))?;
    let next_sequence = items
        .last()
        .filter(|_| has_more)
        .map(|item| item.change_sequence);
    Ok(Json(
        json!({"items": items, "hasMore": has_more, "nextSequence": next_sequence}),
    ))
}

pub async fn admin_create_restore(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<AdminRestoreRequest>,
) -> Result<(StatusCode, Json<tasktips_persistence::RestoreJobRecord>), ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let reason = valid_text(&request.reason, 512, "恢复原因无效", &request_id)?;
    if validate_restore_target(request.snapshot_id, request.target_change_sequence).is_err() {
        return Err(ApiError::invalid("恢复目标无效", request_id));
    }
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let job = database
        .admin_enqueue_restore(
            claims.sub,
            project_id,
            request.snapshot_id,
            request.target_change_sequence,
            reason,
            &request_id,
        )
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    state
        .metrics
        .restore_job_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok((StatusCode::ACCEPTED, Json(job)))
}

pub async fn reopen_restore_project(
    State(state): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ReasonRequest>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let reason = valid_text(&request.reason, 500, "操作原因无效", &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    database
        .reopen_failed_restore(claims.sub, project_id, reason, &request_id)
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn admin_operations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminPageQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (limit, offset) = admin_page_bounds(query.limit, query.offset, &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let (operations, has_more) = database
        .admin_list_operations_page(claims.sub, limit, offset)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    let next_offset = has_more.then_some(offset.saturating_add(limit));
    Ok(Json(
        json!({"items": operations, "hasMore": has_more, "nextOffset": next_offset, "limit": limit, "offset": offset}),
    ))
}

pub async fn admin_audit_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminPageQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (limit, offset) = admin_page_bounds(query.limit, query.offset, &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let (events, has_more) = database
        .admin_list_audit_events_page(claims.sub, limit, offset)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    let next_offset = has_more.then_some(offset.saturating_add(limit));
    Ok(Json(
        json!({"items": events, "hasMore": has_more, "nextOffset": next_offset, "limit": limit, "offset": offset}),
    ))
}

pub async fn admin_trends(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminTrendQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let days = query.days.unwrap_or(30);
    if !(1..=90).contains(&days) {
        return Err(ApiError::invalid("趋势天数无效", request_id));
    }
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let points = database
        .admin_sync_trends(claims.sub, days)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(json!({"items": points, "days": days})))
}

pub async fn admin_audit_events_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let events = database
        .admin_list_audit_events(claims.sub)
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    let mut csv =
        String::from("id,action,actorUserId,subjectUserId,projectId,requestId,createdAt\n");
    for event in events {
        use std::fmt::Write as _;
        writeln!(
            csv,
            "{},{},{},{},{},{},{}",
            event.id,
            csv_field(&event.action),
            csv_optional_uuid(event.actor_user_id),
            csv_optional_uuid(event.subject_user_id),
            csv_optional_uuid(event.project_id),
            csv_field(event.request_id.as_deref().unwrap_or_default()),
            csv_field(&event.created_at.to_string()),
        )
        .map_err(|_| ApiError::internal(request_id.clone()))?;
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"audit-events.csv\"",
        )
        .body(Body::from(csv))
        .map_err(|_| ApiError::internal(request_id))
}

fn csv_optional_uuid(value: Option<Uuid>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn csv_field(value: &str) -> String {
    if value
        .bytes()
        .any(|byte| matches!(byte, b',' | b'"' | b'\r' | b'\n'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

pub async fn disable_account(
    State(state): State<AppState>,
    ApiPath(user_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<AccountStatusRequest>,
) -> Result<StatusCode, ApiError> {
    set_account_status(state, headers, user_id, request, AccountStatus::Disabled).await
}

pub async fn enable_account(
    State(state): State<AppState>,
    ApiPath(user_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<AccountStatusRequest>,
) -> Result<StatusCode, ApiError> {
    set_account_status(state, headers, user_id, request, AccountStatus::Active).await
}

pub async fn purge_account(
    State(state): State<AppState>,
    ApiPath(user_id): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<AccountPurgeRequest>,
) -> Result<(StatusCode, Json<tasktips_persistence::AccountPurgeResponse>), ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    if validate_account_purge_phase(request.export_id, request.confirmed).is_err() {
        return Err(ApiError::invalid("账号清除确认参数无效", request_id));
    }
    let reason = valid_text(&request.reason, 512, "清除原因无效", &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let auth = state
        .auth
        .as_ref()
        .ok_or_else(|| ApiError::unavailable(request_id.clone()))?;
    if !auth.allow_auth_request(
        AuthOperation::Purge,
        &format!(
            "{}:{}",
            claims.sub,
            auth_rate_limit_key(&headers, "account-purge")
        ),
    ) {
        return Err(ApiError::rate_limited(request_id));
    }
    enforce_shared_rate_limit(
        database,
        &format!("account_purge:{}", claims.sub),
        &request_id,
    )
    .await?;
    let nonce = headers
        .get("x-reauth-nonce")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::authentication(request_id.clone()))?;
    let consumed = database
        .consume_reauth_nonce(claims.sub, claims.device_id, &opaque_token_hash(nonce))
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    if !consumed {
        return Err(ApiError::authentication(request_id));
    }
    let response = if let Some(export_id) = request.export_id {
        database
            .confirm_account_purge(claims.sub, user_id, export_id, reason, &request_id)
            .await
    } else {
        database
            .request_account_purge_export(claims.sub, user_id, reason, &request_id)
            .await
    }
    .map_err(|error| map_persistence(error, request_id))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn set_account_status(
    state: AppState,
    headers: HeaderMap,
    user_id: Uuid,
    request: AccountStatusRequest,
    status: AccountStatus,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id(&headers);
    validate_admin_origin(&state, &headers, &request_id)?;
    let reason = valid_text(&request.reason, 500, "操作原因无效", &request_id)?;
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    if claims.sub == user_id && status != AccountStatus::Active {
        return Err(ApiError::invalid("管理员不能禁用当前账号", request_id));
    }
    database
        .admin_set_account_status(claims.sub, user_id, status, reason, &request_id)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn authenticate<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
    request_id: &str,
) -> Result<(&'a Persistence, AccessClaims), ApiError> {
    let (database, claims) = authenticate_access(state, headers, request_id).await?;
    if claims.role != "user" {
        return Err(ApiError::authorization(request_id.to_owned()));
    }
    Ok((database, claims))
}

async fn authenticate_access<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
    request_id: &str,
) -> Result<(&'a Persistence, AccessClaims), ApiError> {
    let (database, auth) = services(state, request_id)?;
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::authentication(request_id.to_owned()))?;
    let token = authorization
        .split_once(' ')
        .filter(|(scheme, token)| scheme.eq_ignore_ascii_case("bearer") && !token.is_empty())
        .map(|(_, token)| token)
        .ok_or_else(|| ApiError::authentication(request_id.to_owned()))?;
    let claims = auth
        .validate_access_token(token)
        .map_err(|_| ApiError::authentication(request_id.to_owned()))?;
    database
        .validate_access(claims.sub, claims.device_id, &claims.role)
        .await
        .map_err(|error| map_persistence(error, request_id.to_owned()))?;
    Ok((database, claims))
}

async fn authenticate_admin<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
    request_id: &str,
) -> Result<(&'a Persistence, AccessClaims), ApiError> {
    let (database, claims) = authenticate_access(state, headers, request_id).await?;
    if claims.role != "system_admin" {
        return Err(ApiError::authorization(request_id.to_owned()));
    }
    Ok((database, claims))
}

fn services<'a>(
    state: &'a AppState,
    request_id: &str,
) -> Result<(&'a Persistence, &'a AuthService), ApiError> {
    match (&state.database, &state.auth) {
        (Some(database), Some(auth)) => Ok((database, auth)),
        _ => Err(ApiError::unavailable(request_id.to_owned())),
    }
}

fn user_token_response(
    auth: &AuthService,
    user: &UserRecord,
    device_id: Uuid,
    refresh_token: String,
    request_id: &str,
) -> Result<Response, ApiError> {
    let access = auth
        .issue_access_token(user.id, &user.role, device_id)
        .map_err(|_| ApiError::internal(request_id.to_owned()))?;
    Ok(Json(TokenResponse {
        access_token: access.token,
        refresh_token,
        expires_in: access.expires_in,
    })
    .into_response())
}

fn admin_token_response(
    auth: &AuthService,
    user: &UserRecord,
    device_id: Uuid,
    refresh_token: &str,
    request_id: &str,
) -> Result<Response, ApiError> {
    let access = auth
        .issue_access_token(user.id, &user.role, device_id)
        .map_err(|_| ApiError::internal(request_id.to_owned()))?;
    let mut response = Json(AdminTokenResponse {
        access_token: access.token,
        expires_in: access.expires_in,
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        admin_refresh_cookie_header(refresh_token, auth.refresh_ttl_seconds(), request_id)?,
    );
    Ok(response)
}

fn refresh_token_from_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value.split(';').find_map(|part| {
                let (name, token) = part.trim().split_once('=')?;
                (name == ADMIN_REFRESH_COOKIE && !token.is_empty()).then(|| token.to_owned())
            })
        })
}

fn auth_rate_limit_key(headers: &HeaderMap, identity: &str) -> String {
    let client = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse::<IpAddr>().ok())
        .map_or_else(
            || "unknown-client".to_owned(),
            |address| address.to_string(),
        );
    format!("{client}:{}", hex::encode(opaque_token_hash(identity)))
}

async fn enforce_shared_rate_limit(
    database: &Persistence,
    bucket_key: &str,
    request_id: &str,
) -> Result<(), ApiError> {
    let allowed = database
        .consume_distributed_rate_limit(bucket_key, 120, time::Duration::minutes(1))
        .await
        .map_err(|error| map_persistence(error, request_id.to_owned()))?;
    if allowed {
        Ok(())
    } else {
        Err(ApiError::rate_limited(request_id.to_owned()))
    }
}

fn admin_refresh_cookie_header(
    token: &str,
    max_age_seconds: i64,
    request_id: &str,
) -> Result<HeaderValue, ApiError> {
    let value = format!(
        "{ADMIN_REFRESH_COOKIE}={token}; Max-Age={max_age_seconds}; Path=/api/v1/admin/auth; HttpOnly; Secure; SameSite=Strict"
    );
    HeaderValue::from_str(&value).map_err(|_| ApiError::internal(request_id.to_owned()))
}

fn validate_admin_origin(
    state: &AppState,
    headers: &HeaderMap,
    request_id: &str,
) -> Result<(), ApiError> {
    let Some(expected) = state.admin_origin.as_deref() else {
        return Err(ApiError::internal(request_id.to_owned()));
    };
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if origin != Some(expected)
        || headers
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("cross-site"))
    {
        return Err(ApiError::authorization(request_id.to_owned()));
    }
    Ok(())
}

async fn run_password_task<T, F>(password: String, operation: F) -> Result<T, AuthError>
where
    T: Send + 'static,
    F: FnOnce(String) -> Result<T, AuthError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || operation(password))
        .await
        .map_err(|_| AuthError::Password)?
}

fn sync_object_store<'a>(
    state: &'a AppState,
    request_id: &str,
) -> Result<&'a tasktips_object_store::ObjectStore, ApiError> {
    state
        .object_store
        .as_ref()
        .ok_or_else(|| ApiError::unavailable(request_id.to_owned()))
}

fn map_object_store(error: &ObjectStoreError, request_id: String) -> ApiError {
    match error {
        ObjectStoreError::NotFound => payload_not_found(request_id),
        ObjectStoreError::HashMismatch => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "CONTENT_HASH_MISMATCH",
            "payload 的 SHA-256 校验失败",
            false,
            request_id,
        ),
        ObjectStoreError::InvalidRange => ApiError::new(
            StatusCode::RANGE_NOT_SATISFIABLE,
            "INVALID_REQUEST",
            "Range 请求无效",
            false,
            request_id,
        ),
        ObjectStoreError::Backend => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "STORAGE_UNAVAILABLE",
            "对象存储暂时不可用",
            true,
            request_id,
        ),
    }
}

fn payload_not_found(request_id: String) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "PAYLOAD_NOT_FOUND",
        "payload 不存在",
        false,
        request_id,
    )
}

fn validate_content_hash(content_hash: &str, request_id: &str) -> Result<(), ApiError> {
    if content_hash.len() != 64
        || !content_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ApiError::invalid(
            "contentHash 必须是小写 SHA-256 十六进制字符串",
            request_id.to_owned(),
        ));
    }
    Ok(())
}

fn valid_limit(
    value: Option<i64>,
    default: i64,
    maximum: i64,
    request_id: &str,
) -> Result<i64, ApiError> {
    let value = value.unwrap_or(default);
    if !(1..=maximum).contains(&value) {
        return Err(ApiError::invalid("分页 limit 无效", request_id.to_owned()));
    }
    Ok(value)
}

fn admin_page_bounds(
    limit: Option<i64>,
    offset: Option<i64>,
    request_id: &str,
) -> Result<(i64, i64), ApiError> {
    let limit = valid_limit(limit, 100, 500, request_id)?;
    let offset = offset.unwrap_or(0);
    if !(0..=1_000_000).contains(&offset) {
        return Err(ApiError::invalid("分页 offset 无效", request_id.to_owned()));
    }
    Ok((limit, offset))
}

fn cursor_claims(
    kind: CursorKind,
    project_id: Uuid,
    owner_user_id: Uuid,
    generation: i64,
    change_sequence: i64,
    offset: i64,
    manifest_id: Option<Uuid>,
) -> CursorClaims {
    CursorClaims {
        schema_version: 1,
        kind,
        project_id,
        owner_user_id,
        generation,
        change_sequence,
        offset,
        manifest_id,
        issued_at: 0,
    }
}

fn verify_cursor(
    signer: &crate::cursor::CursorSigner,
    cursor: &str,
    kind: CursorKind,
    project_id: Uuid,
    owner_user_id: Uuid,
    request_id: &str,
) -> Result<CursorClaims, ApiError> {
    let claims = signer
        .verify(cursor)
        .map_err(|_| ApiError::cursor_invalid(request_id.to_owned()))?;
    if claims.kind != kind
        || claims.project_id != project_id
        || claims.owner_user_id != owner_user_id
        || claims.generation <= 0
        || claims.change_sequence < 0
        || claims.offset < 0
        || (kind == CursorKind::Pull && claims.manifest_id.is_some())
    {
        return Err(ApiError::cursor_invalid(request_id.to_owned()));
    }
    Ok(claims)
}

fn sync_record_json(record: &SyncRecord) -> serde_json::Value {
    if record.tombstone {
        serde_json::json!({
            "type": "tombstone",
            "kind": record.kind,
            "id": record.id,
            "revision": record.revision,
            "baseRevision": record.base_revision,
            "deletedAt": record.changed_at,
            "deviceId": record.device_id,
            "changeSequence": record.change_sequence
        })
    } else {
        serde_json::json!({
            "type": "object",
            "kind": record.kind,
            "id": record.id,
            "schemaVersion": record.schema_version,
            "revision": record.revision,
            "baseRevision": record.base_revision,
            "contentHash": record.content_hash,
            "updatedAt": record.changed_at,
            "deviceId": record.device_id,
            "changeSequence": record.change_sequence
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_push_item(
    kind: ObjectKind,
    id: &str,
    schema_version: i32,
    revision: i64,
    base_revision: Option<i64>,
    content_hash: &str,
    device_id: Uuid,
    authenticated_device_id: Uuid,
    request_id: &str,
) -> Result<(), ApiError> {
    if schema_version <= 0 {
        return Err(ApiError::invalid(
            "schemaVersion 无效",
            request_id.to_owned(),
        ));
    }
    validate_content_hash(content_hash, request_id)?;
    validate_revision_fields(
        kind,
        id,
        revision,
        base_revision,
        device_id,
        authenticated_device_id,
        request_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_revision_fields(
    kind: ObjectKind,
    id: &str,
    revision: i64,
    base_revision: Option<i64>,
    device_id: Uuid,
    authenticated_device_id: Uuid,
    request_id: &str,
) -> Result<(), ApiError> {
    let expected_revision = base_revision.unwrap_or(0).checked_add(1);
    if revision <= 0
        || base_revision.is_some_and(|base| base <= 0)
        || expected_revision != Some(revision)
        || device_id != authenticated_device_id
        || !valid_object_id(kind, id)
    {
        return Err(ApiError::invalid(
            "同步对象 revision、deviceId 或 id 无效",
            request_id.to_owned(),
        ));
    }
    Ok(())
}

fn valid_object_id(kind: ObjectKind, id: &str) -> bool {
    match kind {
        ObjectKind::Todo => {
            id.len() == 26
                && id.bytes().all(|byte| {
                    matches!(
                        byte,
                        b'0'..=b'9'
                            | b'A'..=b'H'
                            | b'J'..=b'K'
                            | b'M'..=b'N'
                            | b'P'..=b'T'
                            | b'V'..=b'Z'
                    )
                })
        }
        ObjectKind::Classification => id == "classification",
        ObjectKind::Index => id == "index",
        ObjectKind::Image => {
            !id.is_empty()
                && id.len() <= 255
                && id != "."
                && id != ".."
                && !id
                    .bytes()
                    .any(|byte| byte == b'/' || byte == b'\\' || byte == 0)
        }
    }
}

fn parse_object_kind(value: &str) -> Option<ObjectKind> {
    match value {
        "todo" => Some(ObjectKind::Todo),
        "classification" => Some(ObjectKind::Classification),
        "index" => Some(ObjectKind::Index),
        "image" => Some(ObjectKind::Image),
        _ => None,
    }
}

fn valid_text<'a>(
    value: &'a str,
    max_chars: usize,
    message: &'static str,
    request_id: &str,
) -> Result<&'a str, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max_chars {
        return Err(ApiError::invalid(message, request_id.to_owned()));
    }
    Ok(value)
}

fn map_project_error(error: PersistenceError, request_id: String) -> ApiError {
    if matches!(error, PersistenceError::NotFound) {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "PROJECT_NOT_FOUND",
            "项目不存在",
            false,
            request_id,
        )
    } else {
        map_persistence(error, request_id)
    }
}

#[allow(clippy::too_many_lines)]
fn map_persistence(error: PersistenceError, request_id: String) -> ApiError {
    match error {
        PersistenceError::AccountDisabled => account_disabled(request_id),
        PersistenceError::DeviceRevoked => ApiError::new(
            StatusCode::FORBIDDEN,
            "DEVICE_REVOKED",
            "设备已被撤销",
            false,
            request_id,
        ),
        PersistenceError::InvalidRefreshToken | PersistenceError::RefreshTokenReuse => {
            ApiError::authentication(request_id)
        }
        PersistenceError::InvalidInvitation => ApiError::invalid("邀请无效或已过期", request_id),
        PersistenceError::AdminRequired => ApiError::authorization(request_id),
        PersistenceError::InvalidAccountTransition => ApiError::new(
            StatusCode::CONFLICT,
            "INVALID_ACCOUNT_STATUS_TRANSITION",
            "账号状态转换无效",
            false,
            request_id,
        ),
        PersistenceError::Conflict => ApiError::new(
            StatusCode::CONFLICT,
            "INVALID_REQUEST",
            "目标记录已存在",
            false,
            request_id,
        ),
        PersistenceError::NotFound => ApiError::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "资源不存在",
            false,
            request_id,
        ),
        PersistenceError::ProjectMaintenance => ApiError::new(
            StatusCode::LOCKED,
            "PROJECT_MAINTENANCE",
            "项目当前处于维护状态",
            true,
            request_id,
        ),
        PersistenceError::GenerationMismatch { expected, actual } => ApiError::new(
            StatusCode::CONFLICT,
            "GENERATION_MISMATCH",
            "项目 generation 已变化，请重新 bootstrap",
            false,
            request_id,
        )
        .with_details(serde_json::json!({
            "expectedGeneration": expected,
            "actualGeneration": actual
        })),
        PersistenceError::BootstrapRequired => ApiError::new(
            StatusCode::CONFLICT,
            "CURSOR_INVALID",
            "当前设备必须先完成 bootstrap",
            false,
            request_id,
        ),
        PersistenceError::IdempotencyConflict => ApiError::new(
            StatusCode::CONFLICT,
            "IDEMPOTENCY_CONFLICT",
            "幂等键已用于不同请求",
            false,
            request_id,
        ),
        PersistenceError::IdempotencyReplay { response, status } => {
            map_idempotency_replay(&response, status, request_id)
        }
        PersistenceError::PayloadNotFound => ApiError::new(
            StatusCode::NOT_FOUND,
            "PAYLOAD_NOT_FOUND",
            "payload 不存在",
            false,
            request_id,
        ),
        PersistenceError::InvalidPayload => {
            ApiError::invalid("payload 与对象类型不兼容", request_id)
        }
        PersistenceError::InvalidRestoreTarget => ApiError::invalid("恢复目标无效", request_id),
        PersistenceError::RestoreNotReopenable => ApiError::new(
            StatusCode::CONFLICT,
            "RESTORE_NOT_REOPENABLE",
            "项目尚未通过恢复失败校验，不能重新开放",
            false,
            request_id,
        ),
        PersistenceError::RestoreCancelled => ApiError::new(
            StatusCode::CONFLICT,
            "RESTORE_CANCELLED",
            "恢复任务已取消",
            false,
            request_id,
        ),
        PersistenceError::AccountPurgeNotReady => ApiError::new(
            StatusCode::CONFLICT,
            "ACCOUNT_PURGE_EXPORT_NOT_READY",
            "账号导出尚未完成或已过期",
            false,
            request_id,
        ),
        PersistenceError::Database(database_error) => {
            drop(database_error);
            ApiError::internal(request_id)
        }
    }
}

fn map_idempotency_replay(
    response: &serde_json::Value,
    _status: i16,
    request_id: String,
) -> ApiError {
    let code = response.get("code").and_then(serde_json::Value::as_str);
    let details = response.get("details").cloned();
    match code {
        Some("GENERATION_MISMATCH") => ApiError::new(
            StatusCode::CONFLICT,
            "GENERATION_MISMATCH",
            "项目 generation 已变化，请重新 bootstrap",
            false,
            request_id,
        )
        .with_details(details.unwrap_or_else(|| json!({}))),
        Some("CURSOR_INVALID") => ApiError::new(
            StatusCode::CONFLICT,
            "CURSOR_INVALID",
            "当前设备必须先完成 bootstrap",
            false,
            request_id,
        ),
        Some("PROJECT_MAINTENANCE") => ApiError::new(
            StatusCode::LOCKED,
            "PROJECT_MAINTENANCE",
            "项目当前处于维护状态",
            true,
            request_id,
        ),
        _ => ApiError::internal(request_id),
    }
}

fn account_disabled(request_id: String) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "ACCOUNT_DISABLED",
        "账号当前不可用",
        false,
        request_id,
    )
}

pub fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::auth_rate_limit_key;

    #[test]
    fn auth_rate_limit_key_normalizes_client_ip_and_hashes_identity() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("2001:0db8:0:0:0:0:0:1, 10.0.0.1"),
        );

        let key = auth_rate_limit_key(&headers, "person@example.com");

        assert!(key.starts_with("2001:db8::1:"));
        assert!(!key.contains("person@example.com"));
        assert_eq!(key.len(), "2001:db8::1:".len() + 64);
    }

    #[test]
    fn auth_rate_limit_key_rejects_unparseable_proxy_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("attacker-controlled-value"),
        );

        assert!(auth_rate_limit_key(&headers, "refresh").starts_with("unknown-client:"));
    }
}
