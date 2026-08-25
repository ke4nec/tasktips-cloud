use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{
        FromRequest, FromRequestParts, Path, Request, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use bytes::BytesMut;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use tasktips_application::{AccountStatus, ObjectKind, normalize_email, valid_password};
use tasktips_object_store::ObjectStoreError;
use tasktips_persistence::{
    DeviceProfile, NewInvitation, NewRefreshToken, NewSyncRevision, Persistence, PersistenceError,
    StoredPayload, SyncRecord, UserRecord, invitation_expiry, refresh_expiry,
};
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

const ADMIN_REFRESH_COOKIE: &str = "tasktips_refresh";
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

pub struct ApiOptionalJson<T>(pub Option<T>);

impl<S, T> FromRequest<S> for ApiOptionalJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let request_id = request_id(request.headers());
        let bytes = to_bytes(request.into_body(), MAX_OPTIONAL_JSON_BYTES)
            .await
            .map_err(|_| ApiError::invalid("请求 JSON 格式或字段无效", request_id.clone()))?;
        if bytes.is_empty() {
            return Ok(Self(None));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
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
pub struct RefreshRequest {
    refresh_token: Option<String>,
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
    database
        .validate_sync_project(claims.sub, &claims.role, project_id)
        .await
        .map_err(|error| map_project_error(error, request_id.clone()))?;
    let object_store = sync_object_store(&state, &request_id)?;

    let mut stream = request.into_body().into_data_stream();
    let mut bytes = BytesMut::with_capacity(content_length);
    let mut hasher = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| ApiError::invalid("payload 请求体读取失败", request_id.clone()))?;
        if bytes.len().saturating_add(chunk.len()) > content_length
            || bytes.len().saturating_add(chunk.len()) > MAX_PAYLOAD_BYTES
        {
            return Err(ApiError::invalid(
                "payload 大小与 Content-Length 不一致",
                request_id,
            ));
        }
        hasher.update(&chunk);
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != content_length {
        return Err(ApiError::invalid(
            "payload 大小与 Content-Length 不一致",
            request_id,
        ));
    }
    if hex::encode(hasher.finalize()) != content_hash {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "CONTENT_HASH_MISMATCH",
            "payload 的 SHA-256 与路径不一致",
            false,
            request_id,
        ));
    }
    let payload = object_store
        .put_payload(
            claims.sub,
            project_id,
            &content_hash,
            &media_type,
            bytes.freeze(),
        )
        .await
        .map_err(|error| map_object_store(&error, request_id.clone()))?;
    database
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
    let page = database
        .bootstrap_page(
            claims.sub,
            &claims.role,
            claims.device_id,
            project_id,
            page_claims.as_ref().map(|claims| claims.generation),
            page_claims.as_ref().map(|claims| claims.change_sequence),
            page_claims.as_ref().map_or(0, |claims| claims.offset),
            limit,
        )
        .await
        .map_err(|error| map_project_error(error, request_id.clone()))?;
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
        ))
        .map_err(|_| ApiError::internal(request_id))?;
    Ok(Json(serde_json::json!({
        "generation": page.generation,
        "changes": page.records.iter().map(sync_record_json).collect::<Vec<_>>(),
        "nextCursor": next_cursor,
        "hasMore": page.has_more
    })))
}

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
    let response = database
        .push(
            claims.sub,
            &claims.role,
            claims.device_id,
            project_id,
            request.generation,
            &request.request_id,
            &request_hash,
            &revisions,
        )
        .await
        .map_err(|error| map_project_error(error, request_id))?;
    Ok(Json(response))
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
    token_response(
        auth,
        &session.user,
        session.device_id,
        opaque.raw,
        &request_id,
    )
}

pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiOptionalJson(request): ApiOptionalJson<RefreshRequest>,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);
    let (database, auth) = services(&state, &request_id)?;
    if !auth.allow_auth_request(
        AuthOperation::Refresh,
        &auth_rate_limit_key(&headers, "refresh"),
    ) {
        return Err(ApiError::rate_limited(request_id));
    }
    let refresh_token = request
        .and_then(|request| request.refresh_token)
        .filter(|token| !token.is_empty())
        .or_else(|| refresh_token_from_cookie(&headers))
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
        .rotate_refresh_token(
            &opaque_token_hash(&refresh_token),
            &replacement,
            &request_id,
        )
        .await
        .map_err(|error| map_persistence(error, request_id.clone()))?;
    token_response(
        auth,
        &session.user,
        session.device_id,
        opaque.raw,
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
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        refresh_cookie_header("", 0, &request_id)?,
    );
    Ok(response)
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
    token_response(
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

pub async fn admin_list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let users = database
        .admin_list_users(claims.sub)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(serde_json::json!({"items": users})))
}

pub async fn admin_list_projects(
    State(state): State<AppState>,
    ApiPath(user_id): ApiPath<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let projects = database
        .admin_list_projects(claims.sub, user_id)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(serde_json::json!({"items": projects})))
}

pub async fn admin_list_devices(
    State(state): State<AppState>,
    ApiPath(user_id): ApiPath<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request_id = request_id(&headers);
    let (database, claims) = authenticate_admin(&state, &headers, &request_id).await?;
    let devices = database
        .admin_list_devices(claims.sub, user_id)
        .await
        .map_err(|error| map_persistence(error, request_id))?;
    Ok(Json(serde_json::json!({"items": devices})))
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

async fn set_account_status(
    state: AppState,
    headers: HeaderMap,
    user_id: Uuid,
    request: AccountStatusRequest,
    status: AccountStatus,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id(&headers);
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
    let (database, claims) = authenticate(state, headers, request_id).await?;
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

fn token_response(
    auth: &AuthService,
    user: &UserRecord,
    device_id: Uuid,
    refresh_token: String,
    request_id: &str,
) -> Result<Response, ApiError> {
    let access = auth
        .issue_access_token(user.id, &user.role, device_id)
        .map_err(|_| ApiError::internal(request_id.to_owned()))?;
    if user.role == "system_admin" {
        let mut response = Json(AdminTokenResponse {
            access_token: access.token,
            expires_in: access.expires_in,
        })
        .into_response();
        response.headers_mut().insert(
            header::SET_COOKIE,
            refresh_cookie_header(&refresh_token, auth.refresh_ttl_seconds(), request_id)?,
        );
        Ok(response)
    } else {
        Ok(Json(TokenResponse {
            access_token: access.token,
            refresh_token,
            expires_in: access.expires_in,
        })
        .into_response())
    }
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

fn refresh_cookie_header(
    token: &str,
    max_age_seconds: i64,
    request_id: &str,
) -> Result<HeaderValue, ApiError> {
    let value = format!(
        "{ADMIN_REFRESH_COOKIE}={token}; Max-Age={max_age_seconds}; Path=/api/v1/auth; HttpOnly; Secure; SameSite=Strict"
    );
    HeaderValue::from_str(&value).map_err(|_| ApiError::internal(request_id.to_owned()))
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

fn cursor_claims(
    kind: CursorKind,
    project_id: Uuid,
    owner_user_id: Uuid,
    generation: i64,
    change_sequence: i64,
    offset: i64,
) -> CursorClaims {
    CursorClaims {
        schema_version: 1,
        kind,
        project_id,
        owner_user_id,
        generation,
        change_sequence,
        offset,
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
        PersistenceError::Database(database_error) => {
            drop(database_error);
            ApiError::internal(request_id)
        }
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
