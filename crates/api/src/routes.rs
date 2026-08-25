use axum::{
    Json,
    body::to_bytes,
    extract::{
        FromRequest, FromRequestParts, Path, Request, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::net::IpAddr;
use tasktips_application::{AccountStatus, normalize_email, valid_password};
use tasktips_persistence::{
    DeviceProfile, NewInvitation, NewRefreshToken, Persistence, PersistenceError, UserRecord,
    invitation_expiry, refresh_expiry,
};
use uuid::Uuid;

use crate::{
    AppState,
    auth::{
        AccessClaims, AuthError, AuthOperation, AuthService, hash_password, opaque_token_hash,
        verify_password,
    },
};

const ADMIN_REFRESH_COOKIE: &str = "tasktips_refresh";
const MAX_OPTIONAL_JSON_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponse {
    code: &'static str,
    message: &'static str,
    retryable: bool,
    request_id: String,
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
            },
        }
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
