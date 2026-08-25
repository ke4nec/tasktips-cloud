use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::collections::HashSet;
use tasktips_domain::{AccountStatus, ObjectKind, UserRole};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("record already exists")]
    Conflict,
    #[error("record was not found")]
    NotFound,
    #[error("account is not active")]
    AccountDisabled,
    #[error("device is revoked")]
    DeviceRevoked,
    #[error("refresh token is invalid or expired")]
    InvalidRefreshToken,
    #[error("refresh token reuse was detected")]
    RefreshTokenReuse,
    #[error("invitation is invalid or expired")]
    InvalidInvitation,
    #[error("operation requires a system administrator")]
    AdminRequired,
    #[error("account status transition is not allowed")]
    InvalidAccountTransition,
    #[error("project is not active")]
    ProjectMaintenance,
    #[error("project generation does not match")]
    GenerationMismatch { expected: i64, actual: i64 },
    #[error("device must complete bootstrap before pushing")]
    BootstrapRequired,
    #[error("idempotency key was reused with different content")]
    IdempotencyConflict,
    #[error("payload was not found")]
    PayloadNotFound,
    #[error("payload is not valid for the object kind")]
    InvalidPayload,
    #[error("restore target is invalid")]
    InvalidRestoreTarget,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRecord {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: String,
    pub status: String,
    pub created_at: OffsetDateTime,
    pub last_login_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub name: String,
    pub generation: i64,
    pub status: String,
    pub change_sequence: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecord {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub display_name: String,
    pub platform: String,
    pub app_version: String,
    pub created_at: OffsetDateTime,
    pub last_seen_at: Option<OffsetDateTime>,
    pub last_login_at: Option<OffsetDateTime>,
    pub last_pull_at: Option<OffsetDateTime>,
    pub last_push_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub user: UserRecord,
    pub device_id: Uuid,
    pub family_id: Uuid,
}

#[derive(Clone, Debug)]
pub struct NewRefreshToken {
    pub token_hash: Vec<u8>,
    pub family_id: Uuid,
    pub expires_at: OffsetDateTime,
}

#[derive(Clone, Debug)]
pub struct NewInvitation {
    pub email_normalized: String,
    pub email_display: String,
    pub token_hash: Vec<u8>,
    pub expires_at: OffsetDateTime,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationRecord {
    pub id: Uuid,
    pub email: String,
    pub expires_at: OffsetDateTime,
}

#[derive(Clone, Debug)]
pub struct DeviceProfile {
    pub display_name: String,
    pub platform: String,
    pub app_version: String,
}

#[derive(Clone, Debug)]
pub struct StoredPayload {
    pub content_hash: String,
    pub bucket: String,
    pub object_key: String,
    pub size: i64,
    pub media_type: String,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRecord {
    pub kind: String,
    pub id: String,
    pub schema_version: Option<i32>,
    pub revision: i64,
    pub base_revision: Option<i64>,
    pub content_hash: Option<String>,
    pub changed_at: OffsetDateTime,
    pub device_id: Uuid,
    pub tombstone: bool,
    pub change_sequence: i64,
}

#[derive(Clone, Debug)]
pub struct NewSyncRevision {
    pub kind: ObjectKind,
    pub id: String,
    pub schema_version: Option<i32>,
    pub base_revision: Option<i64>,
    pub content_hash: Option<String>,
    pub changed_at: OffsetDateTime,
    pub tombstone: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum PushItemResult {
    Applied {
        kind: ObjectKind,
        id: String,
        revision: i64,
        change_sequence: i64,
        changed_at: OffsetDateTime,
    },
    Conflict {
        kind: ObjectKind,
        id: String,
        expected_revision: Option<i64>,
        actual_revision: Option<i64>,
    },
    Rejected {
        kind: ObjectKind,
        id: String,
        code: &'static str,
    },
}

#[derive(Clone, Debug)]
pub struct BootstrapPage {
    pub generation: i64,
    pub snapshot_sequence: i64,
    pub records: Vec<SyncRecord>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct PullPage {
    pub generation: i64,
    pub records: Vec<SyncRecord>,
    pub has_more: bool,
    pub next_sequence: i64,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRecord {
    pub id: Uuid,
    pub project_id: Uuid,
    pub generation: i64,
    pub change_sequence: i64,
    pub manifest_hash: String,
    pub status: String,
    pub created_by: Uuid,
    pub created_at: OffsetDateTime,
    #[serde(skip_serializing)]
    pub manifest_bucket: String,
    #[serde(skip_serializing)]
    pub manifest_key: String,
    #[serde(skip_serializing)]
    pub manifest: serde_json::Value,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreJobRecord {
    pub id: Uuid,
    pub project_id: Uuid,
    pub requested_by: Uuid,
    pub snapshot_id: Option<Uuid>,
    pub target_change_sequence: Option<i64>,
    pub reason: String,
    pub status: String,
    pub pre_restore_snapshot_id: Option<Uuid>,
    pub generation_before: Option<i64>,
    pub generation_after: Option<i64>,
    pub restored_objects: i32,
    pub restored_tombstones: i32,
    pub error_code: Option<String>,
    pub created_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminHistoryRecord {
    pub kind: String,
    pub object_id: String,
    pub revision: i64,
    pub changed_at: OffsetDateTime,
    pub device_id: Uuid,
    pub tombstone: bool,
    pub change_sequence: i64,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncAttemptRecord {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub project_id: Uuid,
    pub device_id: Option<Uuid>,
    pub operation: String,
    pub status: String,
    pub error_code: Option<String>,
    pub item_count: i32,
    pub latency_ms: Option<i32>,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEventRecord {
    pub id: i64,
    pub actor_user_id: Option<Uuid>,
    pub subject_user_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub action: String,
    pub metadata: serde_json::Value,
    pub request_id: Option<String>,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminOverview {
    pub users: i64,
    pub active_users: i64,
    pub projects: i64,
    pub devices: i64,
    pub revisions: i64,
    pub tombstones: i64,
    pub payload_bytes: i64,
    pub queued_restores: i64,
}

#[derive(FromRow)]
struct AdminOverviewRow {
    users: i64,
    active_users: i64,
    projects: i64,
    devices: i64,
    revisions: i64,
    tombstones: i64,
    payload_bytes: i64,
    queued_restores: i64,
}

#[derive(Clone, Debug, FromRow)]
struct ManifestRow {
    kind: String,
    id: String,
    schema_version: Option<i32>,
    revision: i64,
    base_revision: Option<i64>,
    content_hash: Option<String>,
    changed_at: OffsetDateTime,
    device_id: Uuid,
    tombstone: bool,
    change_sequence: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestItem {
    kind: String,
    id: String,
    schema_version: Option<i32>,
    revision: i64,
    content_hash: Option<String>,
    device_id: Uuid,
    tombstone: bool,
}

#[derive(Clone)]
pub struct Persistence {
    pool: PgPool,
}

impl Persistence {
    /// Connects without embedding credentials in logs or error context.
    ///
    /// # Errors
    ///
    /// Returns the `SQLx` connection error when `PostgreSQL` cannot be reached.
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self { pool })
    }

    /// Creates a pool without opening a network connection yet.
    ///
    /// # Errors
    ///
    /// Returns the `SQLx` configuration error when the URL is invalid.
    pub fn connect_lazy(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPool::connect_lazy(database_url)?;
        Ok(Self { pool })
    }

    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// # Errors
    ///
    /// Returns the `SQLx` query error when the readiness query cannot complete.
    pub async fn is_ready(&self) -> Result<bool, sqlx::Error> {
        let schema_ready = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM instance_settings \
             WHERE key = 'identity_schema_version')",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(schema_ready)
    }

    /// # Errors
    ///
    /// Returns the migration error when a migration cannot be applied.
    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        sqlx::migrate!("../../migrations").run(&self.pool).await
    }

    /// # Errors
    ///
    /// Returns an error when the account already exists or the database write fails.
    pub async fn create_initial_admin(
        &self,
        email_normalized: &str,
        email_display: &str,
        password_hash: &str,
    ) -> Result<UserRecord, PersistenceError> {
        let mut tx = self.begin_auth().await?;
        let result = sqlx::query_as::<_, UserRecord>(
            "INSERT INTO users (email_normalized, email_display, password_hash, role, status) \
             VALUES ($1, $2, $3, 'system_admin', 'active') \
             RETURNING id, email_display AS email, password_hash, role::text AS role, \
                       status::text AS status, created_at, last_login_at",
        )
        .bind(email_normalized)
        .bind(email_display)
        .bind(password_hash)
        .fetch_one(&mut *tx)
        .await;
        let admin = map_conflict(result)?;
        tx.commit().await?;
        Ok(admin)
    }

    /// # Errors
    ///
    /// Returns an error when the lookup cannot be completed.
    pub async fn find_login_user(
        &self,
        email_normalized: &str,
    ) -> Result<Option<UserRecord>, PersistenceError> {
        let mut tx = self.begin_auth().await?;
        let user = sqlx::query_as::<_, UserRecord>(
            "SELECT id, email_display AS email, password_hash, role::text AS role, \
                    status::text AS status, created_at, last_login_at \
             FROM users WHERE email_normalized = $1",
        )
        .bind(email_normalized)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(user)
    }

    /// # Errors
    ///
    /// Returns an error when account/device state rejects login or persistence fails.
    pub async fn create_login_session(
        &self,
        user_id: Uuid,
        device_id: Uuid,
        refresh: &NewRefreshToken,
        request_id: &str,
    ) -> Result<SessionRecord, PersistenceError> {
        let mut tx = self.begin_auth().await?;
        let mut user = lock_active_user(&mut tx, user_id).await?;
        ensure_device(&mut tx, user_id, device_id).await?;
        insert_refresh(&mut tx, user_id, device_id, refresh).await?;
        sqlx::query("UPDATE users SET last_login_at = CURRENT_TIMESTAMP WHERE id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE devices SET last_seen_at = CURRENT_TIMESTAMP, last_login_at = CURRENT_TIMESTAMP \
             WHERE id = $1 AND owner_user_id = $2",
        )
        .bind(device_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        user.last_login_at = Some(OffsetDateTime::now_utc());
        insert_audit(
            &mut tx,
            user_id,
            Some(user_id),
            "auth.login",
            json!({"deviceId": device_id}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(SessionRecord {
            user,
            device_id,
            family_id: refresh.family_id,
        })
    }

    /// # Errors
    ///
    /// Returns an error for invalid/reused credentials or database failures.
    pub async fn rotate_refresh_token(
        &self,
        old_token_hash: &[u8],
        replacement: &NewRefreshToken,
        request_id: &str,
    ) -> Result<SessionRecord, PersistenceError> {
        #[derive(FromRow)]
        struct TokenRow {
            user_id: Uuid,
            device_id: Uuid,
            family_id: Uuid,
            expires_at: OffsetDateTime,
            used_at: Option<OffsetDateTime>,
            revoked_at: Option<OffsetDateTime>,
        }

        let mut tx = self.begin_auth().await?;
        let token = sqlx::query_as::<_, TokenRow>(
            "SELECT user_id, device_id, family_id, expires_at, used_at, revoked_at \
             FROM refresh_tokens WHERE token_hash = $1 FOR UPDATE",
        )
        .bind(old_token_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::InvalidRefreshToken)?;

        if token.used_at.is_some() || token.revoked_at.is_some() {
            sqlx::query(
                "UPDATE refresh_tokens SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP) \
                 WHERE family_id = $1",
            )
            .bind(token.family_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Err(PersistenceError::RefreshTokenReuse);
        }
        if token.expires_at <= OffsetDateTime::now_utc() {
            return Err(PersistenceError::InvalidRefreshToken);
        }

        let user = lock_active_user(&mut tx, token.user_id).await?;
        ensure_existing_device(&mut tx, token.user_id, token.device_id).await?;
        sqlx::query(
            "UPDATE refresh_tokens SET used_at = CURRENT_TIMESTAMP, revoked_at = CURRENT_TIMESTAMP \
             WHERE token_hash = $1",
        )
        .bind(old_token_hash)
        .execute(&mut *tx)
        .await?;
        let mut replacement = replacement.clone();
        replacement.family_id = token.family_id;
        insert_refresh(&mut tx, token.user_id, token.device_id, &replacement).await?;
        sqlx::query("UPDATE devices SET last_seen_at = CURRENT_TIMESTAMP WHERE id = $1")
            .bind(token.device_id)
            .execute(&mut *tx)
            .await?;
        insert_audit(
            &mut tx,
            token.user_id,
            Some(token.user_id),
            "auth.refresh",
            json!({"deviceId": token.device_id}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(SessionRecord {
            user,
            device_id: token.device_id,
            family_id: token.family_id,
        })
    }

    /// # Errors
    ///
    /// Returns an error when revocation cannot be persisted.
    pub async fn logout_device(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
        request_id: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP) \
             WHERE user_id = $1 AND device_id = $2",
        )
        .bind(user_id)
        .bind(device_id)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            user_id,
            Some(user_id),
            "auth.logout",
            json!({"deviceId": device_id}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Validates mutable account and device state for an otherwise valid access token.
    ///
    /// # Errors
    ///
    /// Returns an error when the account/device was disabled or revoked.
    pub async fn validate_access(
        &self,
        user_id: Uuid,
        device_id: Uuid,
        token_role: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_auth().await?;
        let user = lock_active_user(&mut tx, user_id).await?;
        if user.role != token_role {
            return Err(PersistenceError::InvalidRefreshToken);
        }
        ensure_existing_device(&mut tx, user_id, device_id).await?;
        sqlx::query("UPDATE devices SET last_seen_at = CURRENT_TIMESTAMP WHERE id = $1")
            .bind(device_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when invitation creation is unauthorized or fails.
    pub async fn create_invitation(
        &self,
        actor_user_id: Uuid,
        invitation: &NewInvitation,
        request_id: &str,
    ) -> Result<InvitationRecord, PersistenceError> {
        let mut tx = self.begin_auth().await?;
        require_admin(&mut tx, actor_user_id).await?;
        let existing_status = sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM users WHERE email_normalized = $1",
        )
        .bind(&invitation.email_normalized)
        .fetch_optional(&mut *tx)
        .await?;
        if existing_status
            .as_deref()
            .is_some_and(|status| status != "pending")
        {
            return Err(PersistenceError::Conflict);
        }
        sqlx::query(
            "INSERT INTO users (email_normalized, email_display, password_hash, role, status) \
             VALUES ($1, $2, '', 'user', 'pending') \
             ON CONFLICT (email_normalized) DO UPDATE SET email_display = EXCLUDED.email_display",
        )
        .bind(&invitation.email_normalized)
        .bind(&invitation.email_display)
        .execute(&mut *tx)
        .await?;
        let record = sqlx::query_as::<_, InvitationRecord>(
            "INSERT INTO invitations (email_normalized, email_display, token_hash, role, expires_at) \
             VALUES ($1, $2, $3, 'user', $4) \
             ON CONFLICT (email_normalized) DO UPDATE SET token_hash = EXCLUDED.token_hash, \
                 email_display = EXCLUDED.email_display, expires_at = EXCLUDED.expires_at, used_at = NULL \
             RETURNING id, email_display AS email, expires_at",
        )
        .bind(&invitation.email_normalized)
        .bind(&invitation.email_display)
        .bind(&invitation.token_hash)
        .bind(invitation.expires_at)
        .fetch_one(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            actor_user_id,
            None,
            "invitation.created",
            json!({"invitationId": record.id, "email": invitation.email_normalized}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(record)
    }

    /// # Errors
    ///
    /// Returns an error when the invitation is invalid or activation cannot complete.
    pub async fn activate_invitation(
        &self,
        invitation_token_hash: &[u8],
        password_hash: &str,
        device_id: Uuid,
        refresh: &NewRefreshToken,
        request_id: &str,
    ) -> Result<SessionRecord, PersistenceError> {
        #[derive(FromRow)]
        struct InvitationRow {
            id: Uuid,
            email_normalized: String,
            expires_at: OffsetDateTime,
            used_at: Option<OffsetDateTime>,
        }

        let mut tx = self.begin_auth().await?;
        let invitation = sqlx::query_as::<_, InvitationRow>(
            "SELECT id, email_normalized, expires_at, used_at FROM invitations \
             WHERE token_hash = $1 FOR UPDATE",
        )
        .bind(invitation_token_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::InvalidInvitation)?;
        if invitation.used_at.is_some() || invitation.expires_at <= OffsetDateTime::now_utc() {
            return Err(PersistenceError::InvalidInvitation);
        }
        let user = sqlx::query_as::<_, UserRecord>(
            "UPDATE users SET password_hash = $2, status = 'active', \
                       last_login_at = CURRENT_TIMESTAMP \
             WHERE email_normalized = $1 AND status = 'pending' \
             RETURNING id, email_display AS email, password_hash, role::text AS role, \
                       status::text AS status, created_at, last_login_at",
        )
        .bind(&invitation.email_normalized)
        .bind(password_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::InvalidInvitation)?;
        sqlx::query("UPDATE invitations SET used_at = CURRENT_TIMESTAMP WHERE id = $1")
            .bind(invitation.id)
            .execute(&mut *tx)
            .await?;
        ensure_device(&mut tx, user.id, device_id).await?;
        sqlx::query(
            "UPDATE devices SET last_seen_at = CURRENT_TIMESTAMP, last_login_at = CURRENT_TIMESTAMP \
             WHERE id = $1 AND owner_user_id = $2",
        )
        .bind(device_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
        insert_refresh(&mut tx, user.id, device_id, refresh).await?;
        insert_audit(
            &mut tx,
            user.id,
            Some(user.id),
            "auth.invitation_activated",
            json!({"deviceId": device_id}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(SessionRecord {
            user,
            device_id,
            family_id: refresh.family_id,
        })
    }

    /// # Errors
    ///
    /// Returns an error when the user cannot be read through RLS.
    pub async fn current_user(
        &self,
        user_id: Uuid,
        role: &str,
    ) -> Result<UserRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let user = sqlx::query_as::<_, UserRecord>(
            "SELECT id, email_display AS email, password_hash, role::text AS role, \
                    status::text AS status, created_at, last_login_at FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        tx.commit().await?;
        Ok(user)
    }

    /// # Errors
    ///
    /// Returns an error when the password update cannot be persisted.
    pub async fn change_password(
        &self,
        user_id: Uuid,
        role: &str,
        password_hash: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let updated = sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(user_id)
            .bind(password_hash)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if updated != 1 {
            return Err(PersistenceError::NotFound);
        }
        sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP) \
             WHERE user_id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when project creation fails.
    pub async fn create_project(
        &self,
        user_id: Uuid,
        role: &str,
        name: &str,
    ) -> Result<ProjectRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let project = sqlx::query_as::<_, ProjectRecord>(
            "INSERT INTO projects (owner_user_id, name) VALUES ($1, $2) \
             RETURNING id, owner_user_id, name, generation, status::text AS status, \
                       change_seq AS change_sequence, created_at, updated_at",
        )
        .bind(user_id)
        .bind(name)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(project)
    }

    /// # Errors
    ///
    /// Returns an error when projects cannot be read.
    pub async fn list_projects(
        &self,
        user_id: Uuid,
        role: &str,
    ) -> Result<Vec<ProjectRecord>, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let projects = sqlx::query_as::<_, ProjectRecord>(
            "SELECT id, owner_user_id, name, generation, status::text AS status, \
                    change_seq AS change_sequence, created_at, updated_at \
             FROM projects ORDER BY created_at, id",
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(projects)
    }

    /// # Errors
    ///
    /// Returns an error when the project is absent or cannot be read.
    pub async fn get_project(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
    ) -> Result<ProjectRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let project = sqlx::query_as::<_, ProjectRecord>(
            "SELECT id, owner_user_id, name, generation, status::text AS status, \
                    change_seq AS change_sequence, created_at, updated_at \
             FROM projects WHERE id = $1",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        tx.commit().await?;
        Ok(project)
    }

    /// # Errors
    ///
    /// Returns an error when the project is absent or cannot be updated.
    pub async fn rename_project(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        name: &str,
    ) -> Result<ProjectRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let project = sqlx::query_as::<_, ProjectRecord>(
            "UPDATE projects SET name = $2, updated_at = CURRENT_TIMESTAMP WHERE id = $1 \
             RETURNING id, owner_user_id, name, generation, status::text AS status, \
                       change_seq AS change_sequence, created_at, updated_at",
        )
        .bind(project_id)
        .bind(name)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        tx.commit().await?;
        Ok(project)
    }

    /// # Errors
    ///
    /// Returns an error when the project is absent or cannot be disabled.
    pub async fn disable_project(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
    ) -> Result<ProjectRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let project = sqlx::query_as::<_, ProjectRecord>(
            "UPDATE projects SET status = 'disabled', updated_at = CURRENT_TIMESTAMP WHERE id = $1 \
             RETURNING id, owner_user_id, name, generation, status::text AS status, \
                       change_seq AS change_sequence, created_at, updated_at",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        tx.commit().await?;
        Ok(project)
    }

    /// Records a `RustFS` object only after its bytes have been fully written and verified.
    ///
    /// # Errors
    ///
    /// Returns an error when the project is unavailable or the payload catalog write fails.
    pub async fn register_payload(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        payload: &StoredPayload,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        require_active_project(&mut tx, project_id).await?;
        sqlx::query(
            "INSERT INTO payloads \
             (project_id, owner_user_id, content_hash, bucket, object_key, size_bytes, media_type) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (project_id, content_hash) DO NOTHING",
        )
        .bind(project_id)
        .bind(user_id)
        .bind(&payload.content_hash)
        .bind(&payload.bucket)
        .bind(&payload.object_key)
        .bind(payload.size)
        .bind(&payload.media_type)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Validates that an owned project currently accepts sync traffic.
    ///
    /// # Errors
    ///
    /// Returns an error when the project is inaccessible or not active.
    pub async fn validate_sync_project(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
    ) -> Result<(i64, i64), PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let state = require_active_project(&mut tx, project_id).await?;
        tx.commit().await?;
        Ok(state)
    }

    /// Removes catalog entries for completed payloads that never became revision references.
    /// Object bytes are deleted separately only after this transaction commits.
    ///
    /// # Errors
    ///
    /// Returns an error when the worker role cannot inspect or prune the catalog.
    pub async fn prune_unreferenced_payloads(
        &self,
        older_than: OffsetDateTime,
    ) -> Result<Vec<String>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let keys = sqlx::query_scalar::<_, String>(
            "DELETE FROM payloads p WHERE p.created_at < $1 \
             AND NOT EXISTS (SELECT 1 FROM object_revisions r WHERE r.payload_id = p.id) \
             RETURNING p.object_key",
        )
        .bind(older_than)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(keys)
    }

    /// Returns every payload key still referenced by the database catalog.
    ///
    /// # Errors
    ///
    /// Returns an error when the worker metadata query fails.
    pub async fn referenced_payload_keys(&self) -> Result<HashSet<String>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let keys = sqlx::query_scalar::<_, String>(
            "SELECT object_key FROM payloads \
             UNION ALL SELECT manifest_key FROM snapshots WHERE status = 'ready'",
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        tx.commit().await?;
        Ok(keys)
    }

    /// Builds an immutable metadata-only snapshot manifest at the current project sequence.
    /// Payload bytes are never included in the manifest.
    #[allow(clippy::missing_errors_doc)]
    pub async fn snapshot_manifest(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        snapshot_id: Uuid,
    ) -> Result<(i64, i64, serde_json::Value, Vec<u8>), PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let (generation, sequence) = require_active_project(&mut tx, project_id).await?;
        let rows = sqlx::query_as::<_, ManifestRow>(
            "SELECT r.kind::text AS kind, r.object_id AS id, r.schema_version, r.revision, \
                    r.base_revision, r.content_hash::text AS content_hash, r.changed_at, \
                    r.device_id, r.is_tombstone AS tombstone, c.sequence AS change_sequence \
             FROM object_heads h JOIN object_revisions r ON r.id = h.revision_id \
             JOIN change_log c ON c.revision_id = r.id AND c.project_id = r.project_id \
             WHERE h.project_id = $1 ORDER BY h.kind, h.object_id",
        )
        .bind(project_id)
        .fetch_all(&mut *tx)
        .await?;
        let items = rows.iter().map(manifest_item_json).collect::<Vec<_>>();
        let manifest = json!({
            "version": 1,
            "snapshotId": snapshot_id,
            "generation": generation,
            "changeSequence": sequence,
            "items": items
        });
        let bytes = serde_json::to_vec(&manifest).map_err(|_| PersistenceError::InvalidPayload)?;
        tx.commit().await?;
        Ok((generation, sequence, manifest, bytes))
    }

    /// Records a verified snapshot manifest after its immutable object has been written.
    #[allow(clippy::missing_errors_doc, clippy::too_many_arguments)]
    pub async fn record_snapshot(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        snapshot_id: Uuid,
        generation: i64,
        change_sequence: i64,
        manifest_hash: &str,
        manifest_bucket: &str,
        manifest_key: &str,
        manifest: serde_json::Value,
    ) -> Result<SnapshotRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        ensure_project_visible(&mut tx, project_id).await?;
        let snapshot = sqlx::query_as::<_, SnapshotRecord>(
            "INSERT INTO snapshots \
                 (id, project_id, owner_user_id, generation, change_sequence, manifest_hash, \
                  manifest_bucket, manifest_key, manifest, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $3) \
             RETURNING id, project_id, generation, change_sequence, manifest_hash, status, \
                       created_by, created_at, manifest_bucket, manifest_key, manifest",
        )
        .bind(snapshot_id)
        .bind(project_id)
        .bind(user_id)
        .bind(generation)
        .bind(change_sequence)
        .bind(manifest_hash)
        .bind(manifest_bucket)
        .bind(manifest_key)
        .bind(manifest)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx_conflict)?;
        insert_audit(
            &mut tx,
            user_id,
            Some(user_id),
            "snapshot.created",
            json!({"projectId": project_id, "snapshotId": snapshot_id, "generation": generation}),
            "snapshot",
        )
        .await?;
        tx.commit().await?;
        Ok(snapshot)
    }

    /// Lists metadata for owner-visible snapshots.
    #[allow(clippy::missing_errors_doc)]
    pub async fn list_snapshots(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
    ) -> Result<Vec<SnapshotRecord>, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        ensure_project_visible(&mut tx, project_id).await?;
        let snapshots = sqlx::query_as::<_, SnapshotRecord>(
            "SELECT id, project_id, generation, change_sequence, manifest_hash, status, \
                    created_by, created_at, manifest_bucket, manifest_key, manifest \
             FROM snapshots WHERE project_id = $1 ORDER BY created_at DESC, id DESC",
        )
        .bind(project_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(snapshots)
    }

    /// Lists immutable revision metadata, optionally scoped to one object.
    #[allow(clippy::missing_errors_doc, clippy::too_many_arguments)]
    pub async fn list_history(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        kind: Option<ObjectKind>,
        object_id: Option<&str>,
        after_sequence: i64,
        limit: i64,
    ) -> Result<(Vec<SyncRecord>, bool), PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        ensure_project_visible(&mut tx, project_id).await?;
        let mut records = if let Some(kind) = kind {
            sqlx::query_as::<_, SyncRecord>(
                "SELECT r.kind::text AS kind, r.object_id AS id, r.schema_version, r.revision, \
                        r.base_revision, r.content_hash::text AS content_hash, r.changed_at, \
                        r.device_id, r.is_tombstone AS tombstone, c.sequence AS change_sequence \
                 FROM change_log c JOIN object_revisions r ON r.id = c.revision_id \
                 WHERE c.project_id = $1 AND c.sequence > $2 AND r.kind = $3::sync_object_kind \
                   AND r.object_id = COALESCE($4, r.object_id) \
                 ORDER BY c.sequence LIMIT $5",
            )
            .bind(project_id)
            .bind(after_sequence)
            .bind(kind.as_str())
            .bind(object_id)
            .bind(limit + 1)
            .fetch_all(&mut *tx)
            .await?
        } else {
            sqlx::query_as::<_, SyncRecord>(
                "SELECT r.kind::text AS kind, r.object_id AS id, r.schema_version, r.revision, \
                        r.base_revision, r.content_hash::text AS content_hash, r.changed_at, \
                        r.device_id, r.is_tombstone AS tombstone, c.sequence AS change_sequence \
                 FROM change_log c JOIN object_revisions r ON r.id = c.revision_id \
                 WHERE c.project_id = $1 AND c.sequence > $2 \
                 ORDER BY c.sequence LIMIT $3",
            )
            .bind(project_id)
            .bind(after_sequence)
            .bind(limit + 1)
            .fetch_all(&mut *tx)
            .await?
        };
        let has_more = i64::try_from(records.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            records.pop();
        }
        tx.commit().await?;
        Ok((records, has_more))
    }

    /// Enqueues one snapshot- or sequence-targeted restore request.
    #[allow(clippy::missing_errors_doc, clippy::too_many_arguments)]
    pub async fn enqueue_restore(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        snapshot_id: Option<Uuid>,
        target_change_sequence: Option<i64>,
        reason: &str,
        request_id: &str,
    ) -> Result<RestoreJobRecord, PersistenceError> {
        if snapshot_id.is_none() == target_change_sequence.is_none()
            || target_change_sequence.is_some_and(|sequence| sequence < 0)
        {
            return Err(PersistenceError::InvalidRestoreTarget);
        }
        let mut tx = self.begin_user(user_id, role).await?;
        let (_, current_sequence) = require_active_project_for_update(&mut tx, project_id).await?;
        if target_change_sequence.is_some_and(|sequence| sequence > current_sequence) {
            return Err(PersistenceError::InvalidRestoreTarget);
        }
        if let Some(snapshot_id) = snapshot_id {
            let belongs = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM snapshots WHERE id = $1 AND project_id = $2 AND status = 'ready')",
            )
            .bind(snapshot_id)
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
            if !belongs {
                return Err(PersistenceError::NotFound);
            }
        }
        let job = sqlx::query_as::<_, RestoreJobRecord>(
            "INSERT INTO restore_jobs \
                 (id, project_id, owner_user_id, requested_by, snapshot_id, target_change_sequence, \
                  reason, request_id) \
             VALUES ($1, $2, $3, $3, $4, $5, $6, $7) \
             RETURNING id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                       status, pre_restore_snapshot_id, generation_before, generation_after, \
                       restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at",
        )
        .bind(Uuid::new_v4())
        .bind(project_id)
        .bind(user_id)
        .bind(snapshot_id)
        .bind(target_change_sequence)
        .bind(reason)
        .bind(request_id)
        .fetch_one(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            user_id,
            Some(user_id),
            "restore.requested",
            json!({"projectId": project_id, "restoreId": job.id}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(job)
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn get_restore_job(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        restore_id: Uuid,
    ) -> Result<RestoreJobRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let job = sqlx::query_as::<_, RestoreJobRecord>(
            "SELECT id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                    status, pre_restore_snapshot_id, generation_before, generation_after, \
                    restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at \
             FROM restore_jobs WHERE id = $1 AND project_id = $2",
        )
        .bind(restore_id)
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        tx.commit().await?;
        Ok(job)
    }

    /// Claims one queued restore and puts its project into the maintenance window.
    #[allow(clippy::missing_errors_doc)]
    pub async fn claim_restore_job(&self) -> Result<Option<RestoreJobRecord>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let Some(job) = sqlx::query_as::<_, RestoreJobRecord>(
            "SELECT id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                    status, pre_restore_snapshot_id, generation_before, generation_after, \
                    restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at \
             FROM restore_jobs WHERE status = 'queued' ORDER BY created_at, id \
             FOR UPDATE SKIP LOCKED LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let generation = sqlx::query_scalar::<_, i64>(
            "UPDATE projects SET status = 'maintenance', updated_at = CURRENT_TIMESTAMP \
             WHERE id = $1 AND status = 'active' RETURNING generation",
        )
        .bind(job.project_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(generation) = generation else {
            sqlx::query(
                "UPDATE restore_jobs SET status = 'failed', error_code = 'PROJECT_MAINTENANCE', \
                 finished_at = CURRENT_TIMESTAMP WHERE id = $1",
            )
            .bind(job.id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Err(PersistenceError::ProjectMaintenance);
        };
        let job = sqlx::query_as::<_, RestoreJobRecord>(
            "UPDATE restore_jobs SET status = 'running', generation_before = $2, \
                 started_at = CURRENT_TIMESTAMP WHERE id = $1 \
             RETURNING id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                       status, pre_restore_snapshot_id, generation_before, generation_after, \
                       restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at",
        )
        .bind(job.id)
        .bind(generation)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(job))
    }

    /// Creates the mandatory pre-restore metadata snapshot before any head changes.
    #[allow(clippy::missing_errors_doc)]
    pub async fn create_pre_restore_snapshot(
        &self,
        job: &RestoreJobRecord,
    ) -> Result<(Uuid, Uuid, Vec<u8>), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let (owner_user_id, generation, sequence) = sqlx::query_as::<_, (Uuid, i64, i64)>(
            "SELECT owner_user_id, generation, change_seq FROM projects WHERE id = $1 FOR UPDATE",
        )
        .bind(job.project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        let rows = sqlx::query_as::<_, ManifestRow>(
            "SELECT r.kind::text AS kind, r.object_id AS id, r.schema_version, r.revision, \
                    r.base_revision, r.content_hash::text AS content_hash, r.changed_at, \
                    r.device_id, r.is_tombstone AS tombstone, c.sequence AS change_sequence \
             FROM object_heads h JOIN object_revisions r ON r.id = h.revision_id \
             JOIN change_log c ON c.revision_id = r.id AND c.project_id = r.project_id \
             WHERE h.project_id = $1 ORDER BY h.kind, h.object_id",
        )
        .bind(job.project_id)
        .fetch_all(&mut *tx)
        .await?;
        let snapshot_id = Uuid::new_v4();
        let manifest = json!({
            "version": 1,
            "snapshotId": snapshot_id,
            "generation": generation,
            "changeSequence": sequence,
            "items": rows.iter().map(manifest_item_json).collect::<Vec<_>>()
        });
        let bytes = serde_json::to_vec(&manifest).map_err(|_| PersistenceError::InvalidPayload)?;
        let manifest_hash = hex::encode(Sha256::digest(&bytes));
        let manifest_key = format!(
            "users/{owner_user_id}/projects/{}/snapshots/{snapshot_id}/manifest.json",
            job.project_id
        );
        sqlx::query(
            "INSERT INTO snapshots \
                 (id, project_id, owner_user_id, generation, change_sequence, manifest_hash, \
                  manifest_bucket, manifest_key, manifest, status, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8, 'pending', $3)",
        )
        .bind(snapshot_id)
        .bind(job.project_id)
        .bind(owner_user_id)
        .bind(generation)
        .bind(sequence)
        .bind(&manifest_hash)
        .bind(&manifest_key)
        .bind(&manifest)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_conflict)?;
        sqlx::query("UPDATE restore_jobs SET pre_restore_snapshot_id = $2 WHERE id = $1")
            .bind(job.id)
            .bind(snapshot_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok((snapshot_id, owner_user_id, bytes))
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn mark_snapshot_ready(
        &self,
        snapshot_id: Uuid,
        bucket: &str,
        key: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        sqlx::query(
            "UPDATE snapshots SET manifest_bucket = $2, manifest_key = $3, status = 'ready' \
             WHERE id = $1 AND status = 'pending'",
        )
        .bind(snapshot_id)
        .bind(bucket)
        .bind(key)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    #[allow(clippy::missing_errors_doc)]
    pub async fn fail_restore_job(
        &self,
        job_id: Uuid,
        error_code: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        sqlx::query(
            "UPDATE restore_jobs SET status = 'failed', error_code = $2, \
                 finished_at = CURRENT_TIMESTAMP WHERE id = $1 AND status = 'running'",
        )
        .bind(job_id)
        .bind(error_code)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Applies a restore by appending revisions and advancing project generation.
    #[allow(clippy::missing_errors_doc, clippy::too_many_lines)]
    pub async fn execute_restore(&self, job_id: Uuid) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let job = sqlx::query_as::<_, RestoreJobRecord>(
            "SELECT id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                    status, pre_restore_snapshot_id, generation_before, generation_after, \
                    restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at \
             FROM restore_jobs WHERE id = $1 FOR UPDATE",
        )
        .bind(job_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if job.status != "running" {
            return Err(PersistenceError::Conflict);
        }
        let (owner_user_id, generation) = sqlx::query_as::<_, (Uuid, i64)>(
            "SELECT owner_user_id, generation FROM projects WHERE id = $1 AND status = 'maintenance' FOR UPDATE",
        )
        .bind(job.project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::ProjectMaintenance)?;

        let target = if let Some(snapshot_id) = job.snapshot_id {
            let manifest = sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT manifest FROM snapshots WHERE id = $1 AND project_id = $2 AND status = 'ready'",
            )
            .bind(snapshot_id)
            .bind(job.project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(PersistenceError::NotFound)?;
            manifest_items(&manifest)?
        } else if let Some(sequence) = job.target_change_sequence {
            sqlx::query_as::<_, ManifestRow>(
                "WITH ranked AS ( \
                     SELECT r.kind::text AS kind, r.object_id AS id, r.schema_version, r.revision, \
                            r.base_revision, r.content_hash::text AS content_hash, r.changed_at, \
                            r.device_id, r.is_tombstone AS tombstone, c.sequence AS change_sequence, \
                            ROW_NUMBER() OVER (PARTITION BY r.kind, r.object_id ORDER BY c.sequence DESC) AS position \
                     FROM object_revisions r JOIN change_log c ON c.revision_id = r.id \
                     WHERE r.project_id = $1 AND c.sequence <= $2 \
                 ) SELECT kind, id, schema_version, revision, base_revision, content_hash, changed_at, \
                          device_id, tombstone, change_sequence FROM ranked WHERE position = 1",
            )
            .bind(job.project_id)
            .bind(sequence)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .map(manifest_item_from_row)
            .collect::<Vec<_>>()
        } else {
            return Err(PersistenceError::InvalidRestoreTarget);
        };
        let current = sqlx::query_as::<_, ManifestRow>(
            "SELECT r.kind::text AS kind, r.object_id AS id, r.schema_version, r.revision, \
                    r.base_revision, r.content_hash::text AS content_hash, r.changed_at, \
                    r.device_id, r.is_tombstone AS tombstone, c.sequence AS change_sequence \
             FROM object_heads h JOIN object_revisions r ON r.id = h.revision_id \
             JOIN change_log c ON c.revision_id = r.id AND c.project_id = r.project_id \
             WHERE h.project_id = $1",
        )
        .bind(job.project_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut target = target;
        for row in &current {
            if !target
                .iter()
                .any(|item| item.kind == row.kind && item.id == row.id)
            {
                target.push(ManifestItem {
                    kind: row.kind.clone(),
                    id: row.id.clone(),
                    schema_version: None,
                    revision: 0,
                    content_hash: None,
                    device_id: row.device_id,
                    tombstone: true,
                });
            }
        }
        let mut restored_objects = 0_i32;
        let mut restored_tombstones = 0_i32;
        for item in target {
            let existing = current
                .iter()
                .find(|row| row.kind == item.kind && row.id == item.id);
            if existing.is_some_and(|row| same_manifest_state(row, &item)) {
                continue;
            }
            let actual_revision = existing.map_or(0, |row| row.revision);
            let payload_id = if item.tombstone {
                None
            } else {
                let hash = item
                    .content_hash
                    .as_deref()
                    .ok_or(PersistenceError::InvalidPayload)?;
                Some(
                    sqlx::query_scalar::<_, Uuid>(
                        "SELECT id FROM payloads WHERE project_id = $1 AND content_hash = $2",
                    )
                    .bind(job.project_id)
                    .bind(hash)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or(PersistenceError::PayloadNotFound)?,
                )
            };
            let next_sequence = sqlx::query_scalar::<_, i64>(
                "UPDATE projects SET change_seq = change_seq + 1, updated_at = CURRENT_TIMESTAMP \
                 WHERE id = $1 RETURNING change_seq",
            )
            .bind(job.project_id)
            .fetch_one(&mut *tx)
            .await?;
            let revision_id = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO object_revisions \
                 (project_id, owner_user_id, kind, object_id, revision, base_revision, schema_version, \
                  payload_id, content_hash, is_tombstone, changed_at, device_id) \
                 VALUES ($1, $2, $3::sync_object_kind, $4, $5, $6, $7, $8, $9, $10, CURRENT_TIMESTAMP, $11) \
                 RETURNING id",
            )
            .bind(job.project_id)
            .bind(owner_user_id)
            .bind(&item.kind)
            .bind(&item.id)
            .bind(actual_revision + 1)
            .bind((actual_revision > 0).then_some(actual_revision))
            .bind(item.schema_version)
            .bind(payload_id)
            .bind(item.content_hash.as_deref())
            .bind(item.tombstone)
            .bind(item.device_id)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO object_heads (project_id, owner_user_id, kind, object_id, revision, revision_id) \
                 VALUES ($1, $2, $3::sync_object_kind, $4, $5, $6) \
                 ON CONFLICT (project_id, kind, object_id) DO UPDATE SET revision = EXCLUDED.revision, revision_id = EXCLUDED.revision_id",
            )
            .bind(job.project_id)
            .bind(owner_user_id)
            .bind(&item.kind)
            .bind(&item.id)
            .bind(actual_revision + 1)
            .bind(revision_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO change_log (project_id, owner_user_id, sequence, revision_id) VALUES ($1, $2, $3, $4)",
            )
            .bind(job.project_id)
            .bind(owner_user_id)
            .bind(next_sequence)
            .bind(revision_id)
            .execute(&mut *tx)
            .await?;
            if item.tombstone {
                restored_tombstones += 1;
            } else {
                restored_objects += 1;
            }
        }
        let generation_after = generation + 1;
        sqlx::query(
            "UPDATE projects SET generation = $2, status = 'active', updated_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(job.project_id)
        .bind(generation_after)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE restore_jobs SET status = 'succeeded', generation_after = $2, restored_objects = $3, \
                 restored_tombstones = $4, finished_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(job_id)
        .bind(generation_after)
        .bind(restored_objects)
        .bind(restored_tombstones)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO audit_events (actor_user_id, subject_user_id, project_id, action, metadata) \
             VALUES ($1, $1, $2, 'restore.completed', $3)",
        )
        .bind(job.requested_by)
        .bind(job.project_id)
        .bind(json!({"restoreId": job_id, "generation": generation_after, "objects": restored_objects, "tombstones": restored_tombstones}))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Returns a stable page of heads as they existed at `snapshot_sequence`.
    ///
    /// # Errors
    ///
    /// Returns an error for inaccessible projects, changed generations, or database failures.
    #[allow(clippy::too_many_arguments)]
    pub async fn bootstrap_page(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
        project_id: Uuid,
        expected_generation: Option<i64>,
        snapshot_sequence: Option<i64>,
        offset: i64,
        limit: i64,
    ) -> Result<BootstrapPage, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let (generation, current_sequence) = require_active_project(&mut tx, project_id).await?;
        if let Some(expected) = expected_generation
            && expected != generation
        {
            return Err(PersistenceError::GenerationMismatch {
                expected,
                actual: generation,
            });
        }
        let snapshot_sequence = snapshot_sequence.unwrap_or(current_sequence);
        if snapshot_sequence > current_sequence || snapshot_sequence < 0 {
            return Err(PersistenceError::GenerationMismatch {
                expected: generation,
                actual: generation,
            });
        }
        let mut records = sqlx::query_as::<_, SyncRecord>(
            "WITH ranked AS ( \
                 SELECT r.kind::text AS kind, r.object_id AS id, r.schema_version, r.revision, \
                        r.base_revision, r.content_hash::text AS content_hash, \
                        r.changed_at, r.device_id, r.is_tombstone AS tombstone, \
                        c.sequence AS change_sequence, \
                        ROW_NUMBER() OVER (PARTITION BY r.kind, r.object_id \
                                           ORDER BY c.sequence DESC) AS position \
                 FROM object_revisions r \
                 JOIN change_log c ON c.revision_id = r.id AND c.project_id = r.project_id \
                 WHERE r.project_id = $1 AND c.sequence <= $2 \
             ) \
             SELECT kind, id, schema_version, revision, base_revision, content_hash, changed_at, \
                    device_id, tombstone, change_sequence \
             FROM ranked WHERE position = 1 \
             ORDER BY kind, id OFFSET $3 LIMIT $4",
        )
        .bind(project_id)
        .bind(snapshot_sequence)
        .bind(offset)
        .bind(limit + 1)
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(records.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            records.pop();
        } else {
            sqlx::query(
                "INSERT INTO bootstrap_completions \
                 (project_id, owner_user_id, device_id, generation, change_sequence) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (project_id, device_id, generation) DO UPDATE \
                 SET change_sequence = EXCLUDED.change_sequence, completed_at = CURRENT_TIMESTAMP",
            )
            .bind(project_id)
            .bind(user_id)
            .bind(device_id)
            .bind(generation)
            .bind(snapshot_sequence)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(BootstrapPage {
            generation,
            snapshot_sequence,
            records,
            has_more,
        })
    }

    /// Returns permanent change-log entries after the supplied sequence.
    ///
    /// # Errors
    ///
    /// Returns an error for inaccessible projects, changed generations, or database failures.
    #[allow(clippy::too_many_arguments)]
    pub async fn pull_page(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
        project_id: Uuid,
        expected_generation: i64,
        after_sequence: i64,
        limit: i64,
    ) -> Result<PullPage, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let (generation, current_sequence) = require_active_project(&mut tx, project_id).await?;
        if expected_generation != generation {
            return Err(PersistenceError::GenerationMismatch {
                expected: expected_generation,
                actual: generation,
            });
        }
        if after_sequence < 0 || after_sequence > current_sequence {
            return Err(PersistenceError::NotFound);
        }
        let mut records = sqlx::query_as::<_, SyncRecord>(
            "SELECT r.kind::text AS kind, r.object_id AS id, r.schema_version, r.revision, \
                    r.base_revision, r.content_hash::text AS content_hash, r.changed_at, \
                    r.device_id, r.is_tombstone AS tombstone, c.sequence AS change_sequence \
             FROM change_log c JOIN object_revisions r ON r.id = c.revision_id \
             WHERE c.project_id = $1 AND c.sequence > $2 \
             ORDER BY c.sequence LIMIT $3",
        )
        .bind(project_id)
        .bind(after_sequence)
        .bind(limit + 1)
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(records.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            records.pop();
        }
        let next_sequence = records
            .last()
            .map_or(after_sequence, |record| record.change_sequence);
        sqlx::query("UPDATE devices SET last_pull_at = CURRENT_TIMESTAMP WHERE id = $1")
            .bind(device_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(PullPage {
            generation,
            records,
            has_more,
            next_sequence,
        })
    }

    /// Applies a push with generation, bootstrap, object CAS, and request idempotency checks.
    /// Business conflicts are returned per item and do not prevent other items from applying.
    ///
    /// # Errors
    ///
    /// Returns an error when the request-level guards fail or persistence is unavailable.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub async fn push(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
        project_id: Uuid,
        expected_generation: i64,
        request_key: &str,
        request_hash: &str,
        revisions: &[NewSyncRevision],
    ) -> Result<serde_json::Value, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let lock_key = format!("{user_id}:{device_id}:{request_key}");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(lock_key)
            .execute(&mut *tx)
            .await?;
        if let Some((stored_hash, response)) = sqlx::query_as::<_, (String, serde_json::Value)>(
            "SELECT request_hash::text, response FROM idempotency_records \
                 WHERE owner_user_id = $1 AND device_id = $2 AND request_key = $3",
        )
        .bind(user_id)
        .bind(device_id)
        .bind(request_key)
        .fetch_optional(&mut *tx)
        .await?
        {
            if stored_hash == request_hash {
                tx.commit().await?;
                return Ok(response);
            }
            return Err(PersistenceError::IdempotencyConflict);
        }

        let (generation, _) = require_active_project_for_update(&mut tx, project_id).await?;
        if expected_generation != generation {
            return Err(PersistenceError::GenerationMismatch {
                expected: expected_generation,
                actual: generation,
            });
        }
        let bootstrapped = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM bootstrap_completions \
             WHERE project_id = $1 AND device_id = $2 AND generation = $3)",
        )
        .bind(project_id)
        .bind(device_id)
        .bind(generation)
        .fetch_one(&mut *tx)
        .await?;
        if !bootstrapped {
            return Err(PersistenceError::BootstrapRequired);
        }

        let mut results = Vec::with_capacity(revisions.len());
        for revision in revisions {
            let actual_revision = sqlx::query_scalar::<_, i64>(
                "SELECT revision FROM object_heads \
                 WHERE project_id = $1 AND kind = $2::sync_object_kind AND object_id = $3 \
                 FOR UPDATE",
            )
            .bind(project_id)
            .bind(revision.kind.as_str())
            .bind(&revision.id)
            .fetch_optional(&mut *tx)
            .await?;
            if revision.base_revision != actual_revision {
                results.push(PushItemResult::Conflict {
                    kind: revision.kind,
                    id: revision.id.clone(),
                    expected_revision: revision.base_revision,
                    actual_revision,
                });
                continue;
            }

            let payload_id = if revision.tombstone {
                None
            } else {
                let Some(content_hash) = revision.content_hash.as_deref() else {
                    results.push(PushItemResult::Rejected {
                        kind: revision.kind,
                        id: revision.id.clone(),
                        code: "PAYLOAD_NOT_FOUND",
                    });
                    continue;
                };
                let payload = sqlx::query_as::<_, (Uuid, i64, String)>(
                    "SELECT id, size_bytes, media_type FROM payloads \
                     WHERE project_id = $1 AND content_hash = $2",
                )
                .bind(project_id)
                .bind(content_hash)
                .fetch_optional(&mut *tx)
                .await?;
                let Some((payload_id, size, media_type)) = payload else {
                    results.push(PushItemResult::Rejected {
                        kind: revision.kind,
                        id: revision.id.clone(),
                        code: "PAYLOAD_NOT_FOUND",
                    });
                    continue;
                };
                let size_valid =
                    u64::try_from(size).is_ok_and(|size| size <= revision.kind.max_payload_bytes());
                if !size_valid || !valid_media_type(revision.kind, &media_type) {
                    results.push(PushItemResult::Rejected {
                        kind: revision.kind,
                        id: revision.id.clone(),
                        code: "INVALID_REQUEST",
                    });
                    continue;
                }
                Some(payload_id)
            };

            let next_revision = actual_revision.unwrap_or(0) + 1;
            let change_sequence = sqlx::query_scalar::<_, i64>(
                "UPDATE projects SET change_seq = change_seq + 1, updated_at = CURRENT_TIMESTAMP \
                 WHERE id = $1 RETURNING change_seq",
            )
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
            let revision_id = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO object_revisions \
                 (project_id, owner_user_id, kind, object_id, revision, base_revision, \
                  schema_version, payload_id, content_hash, is_tombstone, changed_at, device_id) \
                 VALUES ($1, $2, $3::sync_object_kind, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
                 RETURNING id",
            )
            .bind(project_id)
            .bind(user_id)
            .bind(revision.kind.as_str())
            .bind(&revision.id)
            .bind(next_revision)
            .bind(revision.base_revision)
            .bind(revision.schema_version)
            .bind(payload_id)
            .bind(revision.content_hash.as_deref())
            .bind(revision.tombstone)
            .bind(revision.changed_at)
            .bind(device_id)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO object_heads \
                 (project_id, owner_user_id, kind, object_id, revision, revision_id) \
                 VALUES ($1, $2, $3::sync_object_kind, $4, $5, $6) \
                 ON CONFLICT (project_id, kind, object_id) DO UPDATE \
                 SET revision = EXCLUDED.revision, revision_id = EXCLUDED.revision_id",
            )
            .bind(project_id)
            .bind(user_id)
            .bind(revision.kind.as_str())
            .bind(&revision.id)
            .bind(next_revision)
            .bind(revision_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO change_log (project_id, owner_user_id, sequence, revision_id) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(project_id)
            .bind(user_id)
            .bind(change_sequence)
            .bind(revision_id)
            .execute(&mut *tx)
            .await?;
            results.push(PushItemResult::Applied {
                kind: revision.kind,
                id: revision.id.clone(),
                revision: next_revision,
                change_sequence,
                changed_at: revision.changed_at,
            });
        }

        let response = json!({"generation": generation, "results": results});
        sqlx::query(
            "INSERT INTO idempotency_records \
             (owner_user_id, device_id, project_id, request_key, request_hash, response) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(user_id)
        .bind(device_id)
        .bind(project_id)
        .bind(request_key)
        .bind(request_hash)
        .bind(&response)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE devices SET last_push_at = CURRENT_TIMESTAMP WHERE id = $1")
            .bind(device_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(response)
    }

    /// # Errors
    ///
    /// Returns an error when devices cannot be read.
    pub async fn list_devices(
        &self,
        user_id: Uuid,
        role: &str,
    ) -> Result<Vec<DeviceRecord>, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let devices = sqlx::query_as::<_, DeviceRecord>(
            "SELECT id, owner_user_id, display_name, platform, app_version, created_at, \
                    last_seen_at, last_login_at, last_pull_at, last_push_at, revoked_at \
             FROM devices ORDER BY created_at, id",
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(devices)
    }

    /// # Errors
    ///
    /// Returns an error when registration conflicts or cannot be persisted.
    pub async fn register_device(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
        profile: &DeviceProfile,
    ) -> Result<DeviceRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let device = sqlx::query_as::<_, DeviceRecord>(
            "INSERT INTO devices (id, owner_user_id, display_name, platform, app_version, last_seen_at) \
             VALUES ($1, $2, $3, $4, $5, CURRENT_TIMESTAMP) \
             ON CONFLICT (id) DO UPDATE SET display_name = EXCLUDED.display_name, \
                 platform = EXCLUDED.platform, app_version = EXCLUDED.app_version, \
                 last_seen_at = CURRENT_TIMESTAMP \
             WHERE devices.owner_user_id = EXCLUDED.owner_user_id AND devices.revoked_at IS NULL \
             RETURNING id, owner_user_id, display_name, platform, app_version, created_at, \
                       last_seen_at, last_login_at, last_pull_at, last_push_at, revoked_at",
        )
        .bind(device_id)
        .bind(user_id)
        .bind(&profile.display_name)
        .bind(&profile.platform)
        .bind(&profile.app_version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_conflict)?
        .ok_or(PersistenceError::DeviceRevoked)?;
        tx.commit().await?;
        Ok(device)
    }

    /// # Errors
    ///
    /// Returns an error when the device is absent or cannot be updated.
    pub async fn update_device(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
        display_name: &str,
    ) -> Result<DeviceRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let device = sqlx::query_as::<_, DeviceRecord>(
            "UPDATE devices SET display_name = $2 WHERE id = $1 AND revoked_at IS NULL \
             RETURNING id, owner_user_id, display_name, platform, app_version, created_at, \
                       last_seen_at, last_login_at, last_pull_at, last_push_at, revoked_at",
        )
        .bind(device_id)
        .bind(display_name)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        tx.commit().await?;
        Ok(device)
    }

    /// # Errors
    ///
    /// Returns an error when the device is absent or revocation fails.
    pub async fn revoke_device(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
        request_id: &str,
    ) -> Result<DeviceRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let device = sqlx::query_as::<_, DeviceRecord>(
            "UPDATE devices SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP) WHERE id = $1 \
             RETURNING id, owner_user_id, display_name, platform, app_version, created_at, \
                       last_seen_at, last_login_at, last_pull_at, last_push_at, revoked_at",
        )
        .bind(device_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        sqlx::query(
            "UPDATE refresh_tokens SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP) \
             WHERE user_id = $1 AND device_id = $2",
        )
        .bind(user_id)
        .bind(device_id)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            user_id,
            Some(user_id),
            "device.revoked",
            json!({"deviceId": device_id, "source": "user"}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(device)
    }

    /// # Errors
    ///
    /// Returns an error when admin metadata cannot be read.
    pub async fn admin_list_users(
        &self,
        actor_user_id: Uuid,
    ) -> Result<Vec<UserRecord>, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let users = sqlx::query_as::<_, UserRecord>(
            "SELECT id, email_display AS email, '' AS password_hash, role::text AS role, \
                    status::text AS status, created_at, last_login_at \
             FROM admin_user_metadata ORDER BY created_at, id",
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(users)
    }

    /// # Errors
    ///
    /// Returns an error when admin project metadata cannot be read.
    pub async fn admin_list_projects(
        &self,
        actor_user_id: Uuid,
        subject_user_id: Uuid,
    ) -> Result<Vec<ProjectRecord>, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let projects = sqlx::query_as::<_, ProjectRecord>(
            "SELECT id, owner_user_id, name, generation, status::text AS status, \
                    change_seq AS change_sequence, created_at, updated_at \
             FROM admin_project_metadata WHERE owner_user_id = $1 ORDER BY created_at, id",
        )
        .bind(subject_user_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(projects)
    }

    /// # Errors
    ///
    /// Returns an error when admin device metadata cannot be read.
    pub async fn admin_list_devices(
        &self,
        actor_user_id: Uuid,
        subject_user_id: Uuid,
    ) -> Result<Vec<DeviceRecord>, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let devices = sqlx::query_as::<_, DeviceRecord>(
            "SELECT id, owner_user_id, display_name, platform, app_version, created_at, \
                    last_seen_at, last_login_at, last_pull_at, last_push_at, revoked_at \
             FROM admin_device_metadata WHERE owner_user_id = $1 ORDER BY created_at, id",
        )
        .bind(subject_user_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(devices)
    }

    /// Returns operational counts only; no payload body, hash, or storage key is selected.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_overview(
        &self,
        actor_user_id: Uuid,
    ) -> Result<AdminOverview, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let overview = sqlx::query_as::<_, AdminOverviewRow>(
            "SELECT \
                (SELECT count(*) FROM admin_user_metadata) AS users, \
                (SELECT count(*) FROM admin_user_metadata WHERE status = 'active') AS active_users, \
                (SELECT count(*) FROM admin_project_metadata) AS projects, \
                (SELECT count(*) FROM admin_device_metadata) AS devices, \
                (SELECT count(*) FROM object_revisions) AS revisions, \
                (SELECT count(*) FROM object_revisions WHERE is_tombstone) AS tombstones, \
                (SELECT COALESCE(sum(size_bytes), 0)::bigint FROM payloads) AS payload_bytes, \
                (SELECT count(*) FROM restore_jobs WHERE status IN ('queued', 'running')) AS queued_restores",
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(AdminOverview {
            users: overview.users,
            active_users: overview.active_users,
            projects: overview.projects,
            devices: overview.devices,
            revisions: overview.revisions,
            tombstones: overview.tombstones,
            payload_bytes: overview.payload_bytes,
            queued_restores: overview.queued_restores,
        })
    }

    /// Lists project history metadata without selecting content hashes or payload references.
    #[allow(clippy::missing_errors_doc, clippy::too_many_arguments)]
    pub async fn admin_history_metadata(
        &self,
        actor_user_id: Uuid,
        project_id: Uuid,
        after_sequence: i64,
        limit: i64,
    ) -> Result<(Vec<AdminHistoryRecord>, bool), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let mut rows = sqlx::query_as::<_, AdminHistoryRecord>(
            "SELECT r.kind::text AS kind, r.object_id, r.revision, r.changed_at, r.device_id, \
                    r.is_tombstone AS tombstone, c.sequence AS change_sequence \
             FROM change_log c JOIN object_revisions r ON r.id = c.revision_id \
             WHERE c.project_id = $1 AND c.sequence > $2 ORDER BY c.sequence LIMIT $3",
        )
        .bind(project_id)
        .bind(after_sequence)
        .bind(limit + 1)
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            rows.pop();
        }
        tx.commit().await?;
        Ok((rows, has_more))
    }

    /// Lists restore job metadata for operational monitoring.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_restore_jobs(
        &self,
        actor_user_id: Uuid,
        project_id: Option<Uuid>,
    ) -> Result<Vec<RestoreJobRecord>, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let jobs = sqlx::query_as::<_, RestoreJobRecord>(
            "SELECT id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                    status, pre_restore_snapshot_id, generation_before, generation_after, \
                    restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at \
             FROM restore_jobs WHERE project_id = COALESCE($1, project_id) \
             ORDER BY created_at DESC, id DESC LIMIT 200",
        )
        .bind(project_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(jobs)
    }

    /// Queues an owner-scoped restore on behalf of an administrator without exposing payloads.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_enqueue_restore(
        &self,
        actor_user_id: Uuid,
        project_id: Uuid,
        snapshot_id: Option<Uuid>,
        target_change_sequence: Option<i64>,
        reason: &str,
        request_id: &str,
    ) -> Result<RestoreJobRecord, PersistenceError> {
        if snapshot_id.is_none() == target_change_sequence.is_none()
            || target_change_sequence.is_some_and(|sequence| sequence < 0)
        {
            return Err(PersistenceError::InvalidRestoreTarget);
        }
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let (owner_user_id, current_sequence, status) = sqlx::query_as::<_, (Uuid, i64, String)>(
            "SELECT owner_user_id, change_seq, status::text FROM projects WHERE id = $1 FOR UPDATE",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if status != "active"
            || target_change_sequence.is_some_and(|sequence| sequence > current_sequence)
        {
            return Err(PersistenceError::InvalidRestoreTarget);
        }
        if let Some(snapshot_id) = snapshot_id {
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM snapshots WHERE id = $1 AND project_id = $2 AND status = 'ready')",
            )
            .bind(snapshot_id)
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
            if !exists {
                return Err(PersistenceError::NotFound);
            }
        }
        let job = sqlx::query_as::<_, RestoreJobRecord>(
            "INSERT INTO restore_jobs \
                 (id, project_id, owner_user_id, requested_by, snapshot_id, target_change_sequence, reason, request_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             RETURNING id, project_id, requested_by, snapshot_id, target_change_sequence, reason, status, \
                       pre_restore_snapshot_id, generation_before, generation_after, restored_objects, \
                       restored_tombstones, error_code, created_at, started_at, finished_at",
        )
        .bind(Uuid::new_v4())
        .bind(project_id)
        .bind(owner_user_id)
        .bind(actor_user_id)
        .bind(snapshot_id)
        .bind(target_change_sequence)
        .bind(reason)
        .bind(request_id)
        .fetch_one(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            actor_user_id,
            Some(owner_user_id),
            "restore.requested",
            json!({"projectId": project_id, "restoreId": job.id, "source": "admin"}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(job)
    }

    /// Lists recent sync diagnostics without payload data.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_sync_attempts(
        &self,
        actor_user_id: Uuid,
    ) -> Result<Vec<SyncAttemptRecord>, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let attempts = sqlx::query_as::<_, SyncAttemptRecord>(
            "SELECT id, owner_user_id, project_id, device_id, operation, status, error_code, \
                    item_count, latency_ms, created_at FROM sync_attempts \
             ORDER BY created_at DESC, id DESC LIMIT 200",
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(attempts)
    }

    /// Lists recent audit metadata. Audit metadata is never populated from payload bodies.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_audit_events(
        &self,
        actor_user_id: Uuid,
    ) -> Result<Vec<AuditEventRecord>, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let events = sqlx::query_as::<_, AuditEventRecord>(
            "SELECT id, actor_user_id, subject_user_id, project_id, action, metadata, request_id, created_at \
             FROM audit_events ORDER BY created_at DESC, id DESC LIMIT 200",
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(events)
    }

    /// Records an owner-scoped sync diagnostic without payload content.
    #[allow(clippy::missing_errors_doc)]
    pub async fn record_sync_attempt(
        &self,
        user_id: Uuid,
        role: &str,
        attempt: &SyncAttemptRecord,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        sqlx::query(
            "INSERT INTO sync_attempts (id, owner_user_id, project_id, device_id, operation, status, \
                 error_code, item_count, latency_ms) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(attempt.id)
        .bind(user_id)
        .bind(attempt.project_id)
        .bind(attempt.device_id)
        .bind(&attempt.operation)
        .bind(&attempt.status)
        .bind(&attempt.error_code)
        .bind(attempt.item_count)
        .bind(attempt.latency_ms)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when the actor is not an administrator or status update fails.
    pub async fn admin_set_account_status(
        &self,
        actor_user_id: Uuid,
        subject_user_id: Uuid,
        status: AccountStatus,
        reason: &str,
        request_id: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_auth().await?;
        require_admin(&mut tx, actor_user_id).await?;
        let current_status = sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM users WHERE id = $1 FOR UPDATE",
        )
        .bind(subject_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        let allowed_transition = matches!(
            (current_status.as_str(), status),
            ("active", AccountStatus::Disabled) | ("disabled", AccountStatus::Active)
        );
        if !allowed_transition {
            return Err(PersistenceError::InvalidAccountTransition);
        }
        let updated = sqlx::query("UPDATE users SET status = $2::account_status WHERE id = $1")
            .bind(subject_user_id)
            .bind(status.as_str())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if updated != 1 {
            return Err(PersistenceError::NotFound);
        }
        if status != AccountStatus::Active {
            sqlx::query(
                "UPDATE refresh_tokens SET revoked_at = COALESCE(revoked_at, CURRENT_TIMESTAMP) \
                 WHERE user_id = $1",
            )
            .bind(subject_user_id)
            .execute(&mut *tx)
            .await?;
        }
        insert_audit(
            &mut tx,
            actor_user_id,
            Some(subject_user_id),
            "account.status_changed",
            json!({"status": status.as_str(), "reason": reason}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn verify_admin(&self, actor_user_id: Uuid) -> Result<(), PersistenceError> {
        let mut tx = self.begin_auth().await?;
        require_admin(&mut tx, actor_user_id).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn begin_auth(&self) -> Result<Transaction<'_, Postgres>, PersistenceError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL ROLE tasktips_auth")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    async fn begin_admin(&self) -> Result<Transaction<'_, Postgres>, PersistenceError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL ROLE tasktips_admin")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    async fn begin_worker(&self) -> Result<Transaction<'_, Postgres>, PersistenceError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL ROLE tasktips_worker")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    async fn begin_user(
        &self,
        user_id: Uuid,
        role: &str,
    ) -> Result<Transaction<'_, Postgres>, PersistenceError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL ROLE tasktips_app")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT set_config('app.user_id', $1, true)")
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT set_config('app.role', $1, true)")
            .bind(role)
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
}

async fn lock_active_user(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<UserRecord, PersistenceError> {
    let user = sqlx::query_as::<_, UserRecord>(
        "SELECT id, email_display AS email, password_hash, role::text AS role, \
                status::text AS status, created_at, last_login_at \
         FROM users WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(PersistenceError::NotFound)?;
    if user.status != AccountStatus::Active.as_str() {
        return Err(PersistenceError::AccountDisabled);
    }
    Ok(user)
}

async fn require_active_project(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
) -> Result<(i64, i64), PersistenceError> {
    project_state(tx, project_id, false).await
}

async fn require_active_project_for_update(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
) -> Result<(i64, i64), PersistenceError> {
    project_state(tx, project_id, true).await
}

async fn project_state(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    for_update: bool,
) -> Result<(i64, i64), PersistenceError> {
    let query = if for_update {
        "SELECT generation, change_seq, status::text FROM projects WHERE id = $1 FOR UPDATE"
    } else {
        "SELECT generation, change_seq, status::text FROM projects WHERE id = $1"
    };
    let (generation, sequence, status) = sqlx::query_as::<_, (i64, i64, String)>(query)
        .bind(project_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
    if status != "active" {
        return Err(PersistenceError::ProjectMaintenance);
    }
    Ok((generation, sequence))
}

async fn ensure_project_visible(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
) -> Result<(), PersistenceError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM projects WHERE id = $1 AND status NOT IN ('disabled', 'deleting'))",
    )
    .bind(project_id)
    .fetch_one(&mut **tx)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(PersistenceError::NotFound)
    }
}

fn manifest_item_json(row: &ManifestRow) -> serde_json::Value {
    json!({
        "kind": row.kind,
        "id": row.id,
        "schemaVersion": row.schema_version,
        "revision": row.revision,
        "baseRevision": row.base_revision,
        "contentHash": row.content_hash,
        "changedAt": row.changed_at,
        "deviceId": row.device_id,
        "tombstone": row.tombstone,
        "changeSequence": row.change_sequence
    })
}

fn manifest_item_from_row(row: ManifestRow) -> ManifestItem {
    ManifestItem {
        kind: row.kind,
        id: row.id,
        schema_version: row.schema_version,
        revision: row.revision,
        content_hash: row.content_hash,
        device_id: row.device_id,
        tombstone: row.tombstone,
    }
}

fn manifest_items(manifest: &serde_json::Value) -> Result<Vec<ManifestItem>, PersistenceError> {
    serde_json::from_value(
        manifest
            .get("items")
            .cloned()
            .ok_or(PersistenceError::InvalidPayload)?,
    )
    .map_err(|_| PersistenceError::InvalidPayload)
}

fn same_manifest_state(row: &ManifestRow, item: &ManifestItem) -> bool {
    row.kind == item.kind
        && row.id == item.id
        && row.tombstone == item.tombstone
        && row.schema_version == item.schema_version
        && row.content_hash == item.content_hash
        && row.revision == item.revision
}

fn valid_media_type(kind: ObjectKind, media_type: &str) -> bool {
    let essence = media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match kind {
        ObjectKind::Todo => essence == "text/markdown" || essence == "text/plain",
        ObjectKind::Classification | ObjectKind::Index => essence == "application/json",
        ObjectKind::Image => essence.starts_with("image/"),
    }
}

async fn require_admin(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<(), PersistenceError> {
    let role_status = sqlx::query_as::<_, (String, String)>(
        "SELECT role::text, status::text FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(PersistenceError::AdminRequired)?;
    if role_status.0 != UserRole::SystemAdmin.as_str()
        || role_status.1 != AccountStatus::Active.as_str()
    {
        return Err(PersistenceError::AdminRequired);
    }
    Ok(())
}

async fn ensure_device(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    device_id: Uuid,
) -> Result<(), PersistenceError> {
    let existing = sqlx::query_as::<_, (Uuid, Option<OffsetDateTime>)>(
        "SELECT owner_user_id, revoked_at FROM devices WHERE id = $1",
    )
    .bind(device_id)
    .fetch_optional(&mut **tx)
    .await?;
    match existing {
        Some((owner_user_id, _)) if owner_user_id != user_id => Err(PersistenceError::NotFound),
        Some((_, Some(_))) => Err(PersistenceError::DeviceRevoked),
        Some((_, None)) => Ok(()),
        None => {
            sqlx::query(
                "INSERT INTO devices (id, owner_user_id, display_name, platform, app_version) \
                 VALUES ($1, $2, 'Unnamed device', 'unknown', 'unknown')",
            )
            .bind(device_id)
            .bind(user_id)
            .execute(&mut **tx)
            .await?;
            Ok(())
        }
    }
}

async fn ensure_existing_device(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    device_id: Uuid,
) -> Result<(), PersistenceError> {
    let revoked_at = sqlx::query_scalar::<_, Option<OffsetDateTime>>(
        "SELECT revoked_at FROM devices WHERE id = $1 AND owner_user_id = $2",
    )
    .bind(device_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(PersistenceError::InvalidRefreshToken)?;
    if revoked_at.is_some() {
        return Err(PersistenceError::DeviceRevoked);
    }
    Ok(())
}

async fn insert_refresh(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    device_id: Uuid,
    refresh: &NewRefreshToken,
) -> Result<(), PersistenceError> {
    sqlx::query(
        "INSERT INTO refresh_tokens (user_id, device_id, token_hash, family_id, expires_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(device_id)
    .bind(&refresh.token_hash)
    .bind(refresh.family_id)
    .bind(refresh.expires_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_audit(
    tx: &mut Transaction<'_, Postgres>,
    actor_user_id: Uuid,
    subject_user_id: Option<Uuid>,
    action: &str,
    metadata: serde_json::Value,
    request_id: &str,
) -> Result<(), PersistenceError> {
    sqlx::query(
        "INSERT INTO audit_events \
         (actor_user_id, subject_user_id, action, metadata, request_id) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(actor_user_id)
    .bind(subject_user_id)
    .bind(action)
    .bind(metadata)
    .bind(request_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn map_conflict<T>(result: Result<T, sqlx::Error>) -> Result<T, PersistenceError> {
    result.map_err(map_sqlx_conflict)
}

fn map_sqlx_conflict(error: sqlx::Error) -> PersistenceError {
    if error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "23505")
    {
        PersistenceError::Conflict
    } else {
        PersistenceError::Database(error)
    }
}

#[must_use]
pub fn invitation_expiry() -> OffsetDateTime {
    OffsetDateTime::now_utc() + Duration::days(7)
}

#[must_use]
pub fn refresh_expiry(ttl_seconds: i64) -> OffsetDateTime {
    OffsetDateTime::now_utc() + Duration::seconds(ttl_seconds)
}
