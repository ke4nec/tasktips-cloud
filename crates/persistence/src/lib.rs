use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction, pool::PoolConnection};
use std::collections::HashSet;
use tasktips_application::validate_restore_target;
use tasktips_domain::{AccountStatus, ObjectKind, UserRole};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const LEGACY_MIGRATION_VERSIONS: &[i64] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29,
];

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
    #[error("idempotent request replayed a deterministic failure")]
    IdempotencyReplay {
        response: serde_json::Value,
        status: i16,
    },
    #[error("payload was not found")]
    PayloadNotFound,
    #[error("payload is not valid for the object kind")]
    InvalidPayload,
    #[error("restore target is invalid")]
    InvalidRestoreTarget,
    #[error("restore project is not ready to reopen")]
    RestoreNotReopenable,
    #[error("restore was cancelled")]
    RestoreCancelled,
    #[error("account purge export is not ready or has expired")]
    AccountPurgeNotReady,
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

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminInvitationRecord {
    pub id: Uuid,
    pub email: String,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub used_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub status: String,
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

#[derive(Clone, Debug, FromRow, Deserialize, Serialize)]
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
        message: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<serde_json::Value>,
    },
}

#[derive(Clone, Debug)]
pub struct PushOutcome {
    pub response: serde_json::Value,
    pub replayed: bool,
}

#[derive(Clone, Debug)]
pub struct BootstrapPage {
    pub manifest_id: Uuid,
    pub generation: i64,
    pub snapshot_sequence: i64,
    pub records: Vec<SyncRecord>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct BootstrapManifest {
    pub id: Uuid,
    pub generation: i64,
    pub snapshot_sequence: i64,
    pub records: Vec<SyncRecord>,
    pub bytes: Vec<u8>,
    pub hash: String,
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
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub lease_token: Option<Uuid>,
    #[sqlx(default)]
    pub cancel_requested: bool,
    #[sqlx(default)]
    pub attempts: i32,
    #[sqlx(default)]
    pub run_after: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminHistoryRecord {
    pub kind: String,
    pub object_id: String,
    pub revision: i64,
    pub base_revision: Option<i64>,
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
pub struct AdminOperationRecord {
    pub id: Uuid,
    pub operation: String,
    pub status: String,
    pub attempts: i32,
    pub run_after: Option<OffsetDateTime>,
    pub cancel_requested: bool,
    pub project_id: Option<Uuid>,
    pub device_id: Option<Uuid>,
    pub item_count: Option<i32>,
    pub latency_ms: Option<i32>,
    pub error_code: Option<String>,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminTrendPoint {
    pub day: OffsetDateTime,
    pub attempts: i64,
    pub succeeded: i64,
    pub conflicts: i64,
    pub p50_latency_ms: Option<f64>,
    pub p99_latency_ms: Option<f64>,
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

#[derive(Clone, Debug, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRecord {
    pub id: Uuid,
    pub kind: String,
    pub owner_user_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub status: String,
    pub attempts: i32,
    pub run_after: OffsetDateTime,
    pub error_code: Option<String>,
    pub created_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub lease_token: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPurgeResponse {
    pub job_id: Uuid,
    pub export_id: Uuid,
    pub status: String,
    pub confirmation_required: bool,
}

#[derive(Clone, Debug)]
pub struct AccountExportPayload {
    pub object_key: String,
    pub archive_path: String,
}

#[derive(Clone, Debug)]
pub struct AccountExportData {
    pub export_id: Uuid,
    pub owner_user_id: Uuid,
    pub object_key: String,
    pub manifest: Vec<u8>,
    pub payloads: Vec<AccountExportPayload>,
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

/// Holds a transaction-scoped lock for one content-addressed payload key.
/// The lock must cover both object-store mutation and catalog registration/deletion.
pub struct PayloadLock {
    connection: PoolConnection<Postgres>,
    project_id: Uuid,
    content_hash: String,
}

#[allow(clippy::missing_errors_doc)]
impl PayloadLock {
    /// Registers the catalog row while retaining the object-store/catalog ordering guarantee.
    pub async fn register_payload(
        mut self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        payload: &StoredPayload,
    ) -> Result<(), PersistenceError> {
        sqlx::query("SET LOCAL ROLE tasktips_app")
            .execute(&mut *self.connection)
            .await?;
        sqlx::query("SELECT set_config('app.user_id', $1, true)")
            .bind(user_id.to_string())
            .execute(&mut *self.connection)
            .await?;
        sqlx::query("SELECT set_config('app.role', $1, true)")
            .bind(role)
            .execute(&mut *self.connection)
            .await?;
        let project_status = sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM projects WHERE id = $1 FOR UPDATE",
        )
        .bind(project_id)
        .fetch_optional(&mut *self.connection)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if project_status != "active" {
            return Err(PersistenceError::ProjectMaintenance);
        }
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
        .execute(&mut *self.connection)
        .await?;
        sqlx::query("COMMIT").execute(&mut *self.connection).await?;
        Ok(())
    }

    /// Returns the catalog key if it is still old and unreferenced.
    pub async fn unreferenced_payload_key(
        &mut self,
        older_than: OffsetDateTime,
    ) -> Result<Option<String>, PersistenceError> {
        sqlx::query("SET LOCAL ROLE tasktips_worker")
            .execute(&mut *self.connection)
            .await?;
        let key = sqlx::query_scalar::<_, String>(
            "SELECT p.object_key FROM payloads p \
             WHERE p.project_id = $1 AND p.content_hash = $2 AND p.created_at < $3 \
               AND NOT EXISTS (SELECT 1 FROM object_revisions r WHERE r.payload_id = p.id) \
             FOR UPDATE",
        )
        .bind(self.project_id)
        .bind(&self.content_hash)
        .bind(older_than)
        .fetch_optional(&mut *self.connection)
        .await?;
        Ok(key)
    }

    /// Checks whether the payload catalog currently contains this project/hash pair.
    pub async fn payload_is_referenced(&mut self) -> Result<bool, PersistenceError> {
        sqlx::query("SET LOCAL ROLE tasktips_worker")
            .execute(&mut *self.connection)
            .await?;
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM payloads \
             WHERE project_id = $1 AND content_hash = $2)",
        )
        .bind(self.project_id)
        .bind(&self.content_hash)
        .fetch_one(&mut *self.connection)
        .await?)
    }

    /// Deletes the catalog row after its immutable object has been deleted.
    pub async fn delete_payload_row(mut self) -> Result<(), PersistenceError> {
        sqlx::query(
            "DELETE FROM payloads p WHERE p.project_id = $1 AND p.content_hash = $2 \
             AND NOT EXISTS (SELECT 1 FROM object_revisions r WHERE r.payload_id = p.id)",
        )
        .bind(self.project_id)
        .bind(&self.content_hash)
        .execute(&mut *self.connection)
        .await?;
        sqlx::query("COMMIT").execute(&mut *self.connection).await?;
        Ok(())
    }

    /// Releases the lock without changing the catalog.
    pub async fn finish(mut self) -> Result<(), PersistenceError> {
        sqlx::query("COMMIT").execute(&mut *self.connection).await?;
        Ok(())
    }

    /// Rolls back the lock transaction after an object-store failure.
    pub async fn rollback(mut self) {
        let _ = sqlx::query("ROLLBACK").execute(&mut *self.connection).await;
    }
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

    /// Consumes one shared fixed-window rate-limit token.
    ///
    /// The bucket key is hashed before it is persisted, so client addresses
    /// and account identifiers are not retained in operational tables.
    ///
    /// # Errors
    ///
    /// Returns an error when the shared limiter cannot be updated.
    pub async fn consume_distributed_rate_limit(
        &self,
        bucket_key: &str,
        limit: u32,
        period: Duration,
    ) -> Result<bool, PersistenceError> {
        let mut hasher = Sha256::new();
        hasher.update(b"tasktips-rate-limit:");
        hasher.update(bucket_key.as_bytes());
        let bucket_hash = hasher.finalize().to_vec();
        let mut tx = self.begin_auth().await?;
        let allowed = sqlx::query_scalar::<_, bool>(
            "INSERT INTO distributed_rate_limit_buckets (bucket_hash, window_started, request_count) \
             VALUES ($1, CURRENT_TIMESTAMP, 1) \
             ON CONFLICT (bucket_hash) DO UPDATE SET \
               window_started = CASE \
                 WHEN CURRENT_TIMESTAMP >= distributed_rate_limit_buckets.window_started \
                      + ($3::double precision * INTERVAL '1 second') \
                 THEN CURRENT_TIMESTAMP ELSE distributed_rate_limit_buckets.window_started END, \
               request_count = CASE \
                 WHEN CURRENT_TIMESTAMP >= distributed_rate_limit_buckets.window_started \
                      + ($3::double precision * INTERVAL '1 second') \
                 THEN 1 ELSE distributed_rate_limit_buckets.request_count + 1 END \
             RETURNING request_count <= $2",
        )
        .bind(bucket_hash)
        .bind(i32::try_from(limit.max(1)).unwrap_or(i32::MAX))
        .bind(period.whole_seconds().max(1))
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(allowed)
    }

    /// Removes buckets that have been idle for at least ten minutes.
    ///
    /// # Errors
    ///
    /// Returns an error when the cleanup query cannot complete.
    pub async fn prune_distributed_rate_limits(&self, limit: i64) -> Result<u64, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let deleted = sqlx::query(
            "WITH expired AS ( \
                 SELECT bucket_hash FROM distributed_rate_limit_buckets \
                 WHERE window_started < CURRENT_TIMESTAMP - INTERVAL '10 minutes' \
                 ORDER BY window_started LIMIT $1 \
             ) DELETE FROM distributed_rate_limit_buckets b \
             USING expired WHERE b.bucket_hash = expired.bucket_hash",
        )
        .bind(limit.max(1))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(deleted)
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

    /// Stores a short-lived administrator re-authentication nonce.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not an active administrator or the nonce cannot be
    /// persisted.
    pub async fn create_reauth_nonce(
        &self,
        actor_user_id: Uuid,
        device_id: Uuid,
        nonce_hash: &[u8],
        expires_at: OffsetDateTime,
    ) -> Result<(), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        sqlx::query("DELETE FROM admin_reauth_nonces WHERE expires_at <= CURRENT_TIMESTAMP")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO admin_reauth_nonces (nonce_hash, user_id, device_id, expires_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(nonce_hash)
        .bind(actor_user_id)
        .bind(device_id)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_conflict)?;
        tx.commit().await?;
        Ok(())
    }

    /// Atomically consumes a nonce for the authenticated administrator session.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not an active administrator or the database operation
    /// fails.
    pub async fn consume_reauth_nonce(
        &self,
        actor_user_id: Uuid,
        device_id: Uuid,
        nonce_hash: &[u8],
    ) -> Result<bool, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let consumed = sqlx::query(
            "DELETE FROM admin_reauth_nonces \
             WHERE nonce_hash = $1 AND user_id = $2 AND device_id = $3 \
               AND expires_at > CURRENT_TIMESTAMP",
        )
        .bind(nonce_hash)
        .bind(actor_user_id)
        .bind(device_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        tx.commit().await?;
        Ok(consumed)
    }

    /// Revokes outstanding re-authentication nonces for an administrator session.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not an active administrator or the database operation
    /// fails.
    pub async fn revoke_reauth_nonces(
        &self,
        actor_user_id: Uuid,
        device_id: Uuid,
    ) -> Result<(), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        sqlx::query("DELETE FROM admin_reauth_nonces WHERE user_id = $1 AND device_id = $2")
            .bind(actor_user_id)
            .bind(device_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns the migration error when a migration cannot be applied.
    ///
    /// # Panics
    ///
    /// Panics if the consolidated init migration is missing from the build.
    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        let migrator = sqlx::migrate!("../../migrations");
        let migration = migrator
            .iter()
            .next()
            .expect("the consolidated init migration must be present");
        let (migration_table_exists, needs_baseline) = self
            .migration_table_state(migration)
            .await
            .map_err(sqlx::migrate::MigrateError::Execute)?;
        let already_initialized = self
            .schema_is_initialized()
            .await
            .map_err(sqlx::migrate::MigrateError::Execute)?;
        if already_initialized {
            self.ensure_reauth_nonce_schema()
                .await
                .map_err(sqlx::migrate::MigrateError::Execute)?;
            if !migration_table_exists || needs_baseline {
                self.baseline_migration(migration)
                    .await
                    .map_err(sqlx::migrate::MigrateError::Execute)?;
            }
        }
        migrator.run(&self.pool).await
    }

    #[allow(clippy::too_many_lines)]
    async fn schema_is_initialized(&self) -> Result<bool, sqlx::Error> {
        let instance_settings_exists = sqlx::query_scalar::<_, bool>(
            "SELECT to_regclass('public.instance_settings') IS NOT NULL",
        )
        .fetch_one(&self.pool)
        .await?;
        if !instance_settings_exists {
            return Ok(false);
        }
        sqlx::query_scalar::<_, bool>(
            r#"SELECT to_regclass('public.users') IS NOT NULL
                 AND to_regclass('public.invitations') IS NOT NULL
                 AND to_regclass('public.projects') IS NOT NULL
                 AND to_regclass('public.devices') IS NOT NULL
                 AND to_regclass('public.refresh_tokens') IS NOT NULL
                 AND to_regclass('public.audit_events') IS NOT NULL
                 AND to_regclass('public.payloads') IS NOT NULL
                 AND to_regclass('public.object_revisions') IS NOT NULL
                 AND to_regclass('public.object_heads') IS NOT NULL
                 AND to_regclass('public.change_log') IS NOT NULL
                 AND to_regclass('public.bootstrap_completions') IS NOT NULL
                 AND to_regclass('public.bootstrap_manifests') IS NOT NULL
                 AND to_regclass('public.idempotency_records') IS NOT NULL
                 AND to_regclass('public.snapshots') IS NOT NULL
                 AND to_regclass('public.restore_jobs') IS NOT NULL
                 AND to_regclass('public.sync_attempts') IS NOT NULL
                 AND to_regclass('public.jobs') IS NOT NULL
                 AND to_regclass('public.account_purge_exports') IS NOT NULL
                 AND to_regclass('public.distributed_rate_limit_buckets') IS NOT NULL
                 AND to_regprocedure('public.app_current_user_id()') IS NOT NULL
                 AND to_regprocedure('public.admin_lock_project(uuid)') IS NOT NULL
                 AND to_regclass('public.admin_account_purge_export_metadata') IS NOT NULL
                 AND to_regclass('public.admin_device_metadata') IS NOT NULL
                 AND to_regclass('public.admin_history_metadata') IS NOT NULL
                 AND to_regclass('public.admin_job_metadata') IS NOT NULL
                 AND to_regclass('public.admin_project_metadata') IS NOT NULL
                 AND to_regclass('public.admin_user_metadata') IS NOT NULL
                 AND to_regclass('public.admin_operational_counts') IS NOT NULL
                 AND to_regclass('public.admin_restore_job_metadata') IS NOT NULL
                 AND to_regclass('public.admin_snapshot_metadata') IS NOT NULL
                 AND (
                     SELECT bool_and(
                         (
                             SELECT COUNT(*)
                             FROM information_schema.columns
                             WHERE table_schema = 'public'
                               AND table_name = required.table_name
                         ) >= required.minimum_columns
                     )
                     FROM (VALUES
                         ('users', 8),
                         ('invitations', 9),
                         ('projects', 8),
                         ('devices', 11),
                         ('refresh_tokens', 9),
                         ('audit_events', 8),
                         ('payloads', 9),
                         ('object_revisions', 14),
                         ('object_heads', 6),
                         ('change_log', 5),
                         ('bootstrap_completions', 6),
                         ('bootstrap_manifests', 11),
                         ('idempotency_records', 9),
                         ('snapshots', 12),
                         ('restore_jobs', 23),
                         ('sync_attempts', 10),
                         ('jobs', 14),
                         ('account_purge_exports', 10),
                         ('distributed_rate_limit_buckets', 3),
                         ('instance_settings', 3)
                     ) AS required(table_name, minimum_columns)
                 )
                 AND (
                     SELECT COUNT(*) >= 24
                            AND COUNT(*) FILTER (
                                WHERE value = '{"version": 1}'::jsonb
                            ) >= 24
                     FROM public.instance_settings
                     WHERE key IN (
                         'identity_schema_version',
                         'sync_schema_version',
                         'history_schema_version',
                         'admin_schema_version',
                         'admin_safe_views_schema_version',
                         'restore_lease_schema_version',
                         'admin_project_lock_schema_version',
                         'restore_requeue_schema_version',
                         'restore_job_guard_schema_version',
                         'retention_cleanup_schema_version',
                         'invitation_operations_schema_version',
                         'idempotency_expiry_permissions_schema_version',
                         'worker_rls_function_access_schema_version',
                         'idempotency_project_scope_schema_version',
                         'admin_project_reopen_schema_version',
                         'bootstrap_manifest_schema_version',
                         'jobs_project_purge_schema_version',
                         'worker_purge_rls_schema_version',
                         'restore_retry_schema_version',
                         'account_purge_schema_version',
                         'account_purge_admin_rls_schema_version',
                         'restore_cancellation_schema_version',
                         'distributed_rate_limit_schema_version',
                         'admin_restore_progress_schema_version'
                     )
                 )
                 AND (
                     SELECT COUNT(*) >= 62
                     FROM pg_policies
                     WHERE schemaname = 'public'
                 )
                 AND EXISTS (
                     SELECT 1 FROM public.instance_settings
                     WHERE key = 'admin_restore_progress_schema_version'
                 )"#,
        )
        .fetch_one(&self.pool)
        .await
    }

    async fn migration_table_state(
        &self,
        migration: &sqlx::migrate::Migration,
    ) -> Result<(bool, bool), sqlx::Error> {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT to_regclass('public._sqlx_migrations') IS NOT NULL",
        )
        .fetch_one(&self.pool)
        .await?;
        if !exists {
            return Ok((false, false));
        }
        let current_applied = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                 SELECT 1 FROM public._sqlx_migrations
                 WHERE version = $1 AND checksum = $2 AND success
             )",
        )
        .bind(migration.version)
        .bind(migration.checksum.as_ref())
        .fetch_one(&self.pool)
        .await?;
        let (versions, descriptions, all_success) =
            sqlx::query_as::<_, (Vec<i64>, Vec<String>, bool)>(
                "SELECT COALESCE(array_agg(version ORDER BY version), ARRAY[]::BIGINT[]),
                    COALESCE(array_agg(description ORDER BY version), ARRAY[]::TEXT[]),
                    COALESCE(bool_and(success), true)
             FROM public._sqlx_migrations",
            )
            .fetch_one(&self.pool)
            .await?;
        let is_known_legacy_history = all_success && versions == LEGACY_MIGRATION_VERSIONS;
        let is_prior_consolidated_history = all_success
            && versions.as_slice() == [migration.version]
            && descriptions.as_slice() == [migration.description.as_ref()];
        let is_empty_history = all_success && versions.is_empty();
        Ok((
            true,
            (is_known_legacy_history || is_prior_consolidated_history || is_empty_history)
                && !current_applied,
        ))
    }

    async fn ensure_reauth_nonce_schema(&self) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        // Compatibility DDL for databases created before the consolidated init script.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS public.admin_reauth_nonces (
                 nonce_hash BYTEA PRIMARY KEY,
                 user_id UUID NOT NULL REFERENCES public.users(id),
                 device_id UUID NOT NULL,
                 expires_at TIMESTAMPTZ NOT NULL,
                 FOREIGN KEY (device_id, user_id)
                     REFERENCES public.devices(id, owner_user_id)
             )",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("ALTER TABLE public.admin_reauth_nonces ENABLE ROW LEVEL SECURITY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("ALTER TABLE public.admin_reauth_nonces FORCE ROW LEVEL SECURITY")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS admin_reauth_nonces_expiry_idx
             ON public.admin_reauth_nonces (expires_at)",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DO $$
             BEGIN
                 IF NOT EXISTS (
                     SELECT 1 FROM pg_policy
                     WHERE polname = 'admin_reauth_nonces_admin'
                       AND polrelid = 'public.admin_reauth_nonces'::regclass
                 ) THEN
                     CREATE POLICY admin_reauth_nonces_admin
                         ON public.admin_reauth_nonces
                         USING (CURRENT_USER = 'tasktips_admin'::name)
                         WITH CHECK (CURRENT_USER = 'tasktips_admin'::name);
                 END IF;
             END
             $$",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "GRANT SELECT, INSERT, DELETE ON TABLE public.admin_reauth_nonces TO tasktips_admin",
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    async fn baseline_migration(
        &self,
        migration: &sqlx::migrate::Migration,
    ) -> Result<(), sqlx::Error> {
        // Existing development databases with the exact old 1..29 history (or
        // databases initialized directly from SQL) only need bookkeeping
        // reconciliation; application tables and data are never rewritten.
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS public._sqlx_migrations (
                 version BIGINT PRIMARY KEY,
                 description TEXT NOT NULL,
                 installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
                 success BOOLEAN NOT NULL,
                 checksum BYTEA NOT NULL,
                 execution_time BIGINT NOT NULL
             )",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("LOCK TABLE public._sqlx_migrations IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await?;
        let rows = sqlx::query_as::<_, (i64, String, bool, Vec<u8>)>(
            "SELECT version, description, success, checksum
             FROM public._sqlx_migrations ORDER BY version",
        )
        .fetch_all(&mut *tx)
        .await?;
        let current_applied = rows.iter().any(|(version, _, success, checksum)| {
            *version == migration.version && *success && checksum == migration.checksum.as_ref()
        });
        let all_success = rows.iter().all(|(_, _, success, _)| *success);
        let versions = rows
            .iter()
            .map(|(version, _, _, _)| *version)
            .collect::<Vec<_>>();
        let descriptions = rows
            .iter()
            .map(|(_, description, _, _)| description.as_str())
            .collect::<Vec<_>>();
        let can_baseline = rows.is_empty()
            || (!current_applied
                && all_success
                && (versions.as_slice() == LEGACY_MIGRATION_VERSIONS
                    || (versions.as_slice() == [migration.version]
                        && descriptions.as_slice() == [migration.description.as_ref()])));
        if !can_baseline {
            tx.commit().await?;
            return Ok(());
        }
        sqlx::query("DELETE FROM public._sqlx_migrations")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO public._sqlx_migrations
                 (version, description, installed_on, success, checksum, execution_time)
             VALUES ($1, $2, CURRENT_TIMESTAMP, true, $3, 0)",
        )
        .bind(migration.version)
        .bind(migration.description.as_ref())
        .bind(migration.checksum.as_ref())
        .execute(&mut *tx)
        .await?;
        tx.commit().await
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
        self.rotate_refresh_token_inner(old_token_hash, replacement, None, request_id)
            .await
    }

    /// Rotates a refresh token only when it belongs to the expected account role.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid/reused credentials, a role mismatch, or database failures.
    pub async fn rotate_refresh_token_for_role(
        &self,
        old_token_hash: &[u8],
        replacement: &NewRefreshToken,
        expected_role: &str,
        request_id: &str,
    ) -> Result<SessionRecord, PersistenceError> {
        self.rotate_refresh_token_inner(
            old_token_hash,
            replacement,
            Some(expected_role),
            request_id,
        )
        .await
    }

    async fn rotate_refresh_token_inner(
        &self,
        old_token_hash: &[u8],
        replacement: &NewRefreshToken,
        expected_role: Option<&str>,
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
        if expected_role.is_some_and(|expected| user.role != expected) {
            return Err(PersistenceError::InvalidRefreshToken);
        }
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
                 email_display = EXCLUDED.email_display, expires_at = EXCLUDED.expires_at, \
                 used_at = NULL, revoked_at = NULL \
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

    /// Lists invitation metadata without returning invitation tokens.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_invitations(
        &self,
        actor_user_id: Uuid,
    ) -> Result<Vec<AdminInvitationRecord>, PersistenceError> {
        Ok(self
            .admin_list_invitations_page(actor_user_id, 200, 0)
            .await?
            .0)
    }

    /// Lists one page of invitation metadata without invitation tokens.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_invitations_page(
        &self,
        actor_user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<AdminInvitationRecord>, bool), PersistenceError> {
        let mut tx = self.begin_auth().await?;
        require_admin(&mut tx, actor_user_id).await?;
        let mut records = sqlx::query_as::<_, AdminInvitationRecord>(
            "SELECT id, email_display AS email, created_at, expires_at, used_at, revoked_at, \
                    CASE WHEN revoked_at IS NOT NULL THEN 'revoked' \
                         WHEN used_at IS NOT NULL THEN 'used' \
                         WHEN expires_at <= CURRENT_TIMESTAMP THEN 'expired' \
                         ELSE 'pending' END AS status \
             FROM invitations ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit.saturating_add(1))
        .bind(offset.max(0))
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(records.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            records.pop();
        }
        tx.commit().await?;
        Ok((records, has_more))
    }

    /// Revokes an unused invitation and records the operator reason.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_revoke_invitation(
        &self,
        actor_user_id: Uuid,
        invitation_id: Uuid,
        reason: &str,
        request_id: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_auth().await?;
        require_admin(&mut tx, actor_user_id).await?;
        let updated = sqlx::query(
            "UPDATE invitations SET revoked_at = CURRENT_TIMESTAMP \
             WHERE id = $1 AND used_at IS NULL AND revoked_at IS NULL",
        )
        .bind(invitation_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(PersistenceError::NotFound);
        }
        insert_audit(
            &mut tx,
            actor_user_id,
            None,
            "invitation.revoked",
            json!({"invitationId": invitation_id, "reason": reason}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Reissues a token for an existing, unused invitation.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_resend_invitation(
        &self,
        actor_user_id: Uuid,
        invitation_id: Uuid,
        token_hash: &[u8],
        expires_at: OffsetDateTime,
        request_id: &str,
    ) -> Result<InvitationRecord, PersistenceError> {
        let mut tx = self.begin_auth().await?;
        require_admin(&mut tx, actor_user_id).await?;
        let record = sqlx::query_as::<_, InvitationRecord>(
            "UPDATE invitations SET token_hash = $2, expires_at = $3, revoked_at = NULL \
             WHERE id = $1 AND used_at IS NULL \
             RETURNING id, email_display AS email, expires_at",
        )
        .bind(invitation_id)
        .bind(token_hash)
        .bind(expires_at)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        insert_audit(
            &mut tx,
            actor_user_id,
            None,
            "invitation.resent",
            json!({"invitationId": invitation_id}),
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
            revoked_at: Option<OffsetDateTime>,
        }

        let mut tx = self.begin_auth().await?;
        let invitation = sqlx::query_as::<_, InvitationRow>(
            "SELECT id, email_normalized, expires_at, used_at, revoked_at FROM invitations \
             WHERE token_hash = $1 FOR UPDATE",
        )
        .bind(invitation_token_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::InvalidInvitation)?;
        if invitation.used_at.is_some()
            || invitation.revoked_at.is_some()
            || invitation.expires_at <= OffsetDateTime::now_utc()
        {
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

    /// Moves a project into the irreversible deleting state and queues its purge job.
    #[allow(clippy::missing_errors_doc)]
    pub async fn enqueue_project_purge(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        reason: &str,
        request_id: &str,
    ) -> Result<JobRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM projects WHERE id = $1 FOR UPDATE",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if !matches!(status.as_str(), "active" | "disabled") {
            return Err(PersistenceError::Conflict);
        }
        let has_active_restore = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM restore_jobs \
             WHERE project_id = $1 AND status IN ('queued', 'running'))",
        )
        .bind(project_id)
        .fetch_one(&mut *tx)
        .await?;
        if has_active_restore {
            return Err(PersistenceError::Conflict);
        }
        sqlx::query(
            "UPDATE projects SET status = 'deleting', updated_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        let job = sqlx::query_as::<_, JobRecord>(
            "INSERT INTO jobs (id, kind, owner_user_id, project_id, payload) \
             VALUES ($1, 'project_purge', $2, $3, $4) \
             RETURNING id, kind, owner_user_id, project_id, status, attempts, run_after, \
                       error_code, created_at, started_at, finished_at",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(project_id)
        .bind(json!({"reason": reason, "requestedBy": user_id}))
        .fetch_one(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            user_id,
            Some(user_id),
            "project.purge_requested",
            json!({"projectId": project_id, "jobId": job.id, "reason": reason}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(job)
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
        let lock = self
            .acquire_payload_lock(project_id, &payload.content_hash)
            .await?;
        lock.register_payload(user_id, role, project_id, payload)
            .await
    }

    /// Acquires the transaction-scoped lock used to serialize object and catalog changes.
    #[allow(clippy::missing_errors_doc)]
    pub async fn acquire_payload_lock(
        &self,
        project_id: Uuid,
        content_hash: &str,
    ) -> Result<PayloadLock, PersistenceError> {
        let mut connection = self.pool.acquire().await?;
        sqlx::query("BEGIN").execute(&mut *connection).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("{project_id}:{content_hash}"))
            .execute(&mut *connection)
            .await?;
        Ok(PayloadLock {
            connection,
            project_id,
            content_hash: content_hash.to_owned(),
        })
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
    pub async fn unreferenced_payload_keys(
        &self,
        older_than: OffsetDateTime,
    ) -> Result<Vec<(Uuid, String)>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let keys = sqlx::query_as::<_, (Uuid, String)>(
            "SELECT p.project_id, p.content_hash::text FROM payloads p WHERE p.created_at < $1 \
             AND NOT EXISTS (SELECT 1 FROM object_revisions r WHERE r.payload_id = p.id) \
             ORDER BY p.created_at, p.id",
        )
        .bind(older_than)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(keys)
    }

    /// Marks stale pending snapshot manifests as failed and returns their object keys.
    /// The caller may delete those objects only after this transaction commits.
    #[allow(clippy::missing_errors_doc)]
    pub async fn expire_pending_snapshots(
        &self,
        older_than: OffsetDateTime,
    ) -> Result<Vec<String>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let keys = sqlx::query_scalar::<_, String>(
            "UPDATE snapshots SET status = 'failed' \
             WHERE status = 'pending' AND created_at < $1 \
             RETURNING manifest_key",
        )
        .bind(older_than)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(keys)
    }

    /// Deletes idempotency records outside the configured replay window.
    #[allow(clippy::missing_errors_doc)]
    pub async fn purge_expired_idempotency_records(
        &self,
        limit: i64,
    ) -> Result<u64, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let deleted = sqlx::query(
            "WITH expired AS ( \
                 SELECT owner_user_id, device_id, project_id, request_key \
                 FROM idempotency_records \
                 WHERE expires_at <= CURRENT_TIMESTAMP \
                 ORDER BY expires_at, request_key \
                 LIMIT $1 \
             ) \
             DELETE FROM idempotency_records r \
             USING expired e \
             WHERE r.owner_user_id = e.owner_user_id \
               AND r.device_id = e.device_id \
               AND r.project_id = e.project_id \
               AND r.request_key = e.request_key",
        )
        .bind(limit.max(1))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(deleted)
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
             UNION ALL SELECT manifest_key FROM snapshots WHERE status IN ('pending', 'ready') \
             UNION ALL SELECT manifest_key FROM bootstrap_manifests \
                 WHERE expires_at > CURRENT_TIMESTAMP",
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
        let (generation, sequence) = require_active_project_for_update(&mut tx, project_id).await?;
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
        let manifest_hash = hex::encode(Sha256::digest(&bytes));
        let manifest_key =
            format!("users/{user_id}/projects/{project_id}/snapshots/{snapshot_id}/manifest.json");
        sqlx::query(
            "INSERT INTO snapshots \
                 (id, project_id, owner_user_id, generation, change_sequence, manifest_hash, \
                  manifest_bucket, manifest_key, manifest, status, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8, 'pending', $3)",
        )
        .bind(snapshot_id)
        .bind(project_id)
        .bind(user_id)
        .bind(generation)
        .bind(sequence)
        .bind(&manifest_hash)
        .bind(&manifest_key)
        .bind(&manifest)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_conflict)?;
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
            "UPDATE snapshots SET manifest_bucket = $6, manifest_key = $7, manifest = $8, \
                    status = 'ready' \
             WHERE id = $1 AND project_id = $2 AND owner_user_id = $3 \
               AND generation = $4 AND change_sequence = $5 \
               AND manifest_hash = $9 AND status = 'pending' \
             RETURNING id, project_id, generation, change_sequence, manifest_hash, status, \
                       created_by, created_at, manifest_bucket, manifest_key, manifest",
        )
        .bind(snapshot_id)
        .bind(project_id)
        .bind(user_id)
        .bind(generation)
        .bind(change_sequence)
        .bind(manifest_bucket)
        .bind(manifest_key)
        .bind(manifest)
        .bind(manifest_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
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
        validate_restore_target(snapshot_id, target_change_sequence)
            .map_err(|_| PersistenceError::InvalidRestoreTarget)?;
        let mut tx = self.begin_user(user_id, role).await?;
        let (_, current_sequence) = require_active_project_for_update(&mut tx, project_id).await?;
        let active_restore = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM restore_jobs WHERE project_id = $1 AND status IN ('queued', 'running'))",
        )
        .bind(project_id)
        .fetch_one(&mut *tx)
        .await?;
        if active_restore {
            return Err(PersistenceError::Conflict);
        }
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
                    restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at, lease_token, \
                    cancel_requested \
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

    /// Cancels a queued restore immediately or requests cancellation at the next safe worker
    /// boundary for a running restore.
    #[allow(clippy::missing_errors_doc)]
    pub async fn cancel_restore(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        restore_id: Uuid,
        reason: &str,
        request_id: &str,
    ) -> Result<RestoreJobRecord, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM restore_jobs \
             WHERE id = $1 AND project_id = $2 FOR UPDATE",
        )
        .bind(restore_id)
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        let action = match status.as_str() {
            "queued" => {
                sqlx::query(
                    "UPDATE restore_jobs SET status = 'cancelled', finished_at = CURRENT_TIMESTAMP \
                     WHERE id = $1 AND status = 'queued'",
                )
                .bind(restore_id)
                .execute(&mut *tx)
                .await?;
                "restore.cancelled"
            }
            "running" => {
                sqlx::query(
                    "UPDATE restore_jobs SET cancel_requested = true \
                     WHERE id = $1 AND status = 'running'",
                )
                .bind(restore_id)
                .execute(&mut *tx)
                .await?;
                "restore.cancel_requested"
            }
            _ => return Err(PersistenceError::Conflict),
        };
        insert_audit(
            &mut tx,
            user_id,
            Some(user_id),
            action,
            json!({"projectId": project_id, "restoreId": restore_id, "reason": reason}),
            request_id,
        )
        .await?;
        let job = sqlx::query_as::<_, RestoreJobRecord>(
            "SELECT id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                    status, pre_restore_snapshot_id, generation_before, generation_after, \
                    restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at, \
                    lease_token, cancel_requested \
             FROM restore_jobs WHERE id = $1 AND project_id = $2",
        )
        .bind(restore_id)
        .bind(project_id)
        .fetch_one(&mut *tx)
        .await?;
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
             FROM restore_jobs WHERE status = 'queued' AND run_after <= CURRENT_TIMESTAMP \
                OR (status = 'running' AND lease_expires_at < CURRENT_TIMESTAMP) \
             ORDER BY created_at, id \
             FOR UPDATE SKIP LOCKED LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let generation = if job.status == "queued" {
            sqlx::query_scalar::<_, i64>(
                "UPDATE projects SET status = 'maintenance', updated_at = CURRENT_TIMESTAMP \
                 WHERE id = $1 AND status IN ('active', 'maintenance') RETURNING generation",
            )
            .bind(job.project_id)
            .fetch_optional(&mut *tx)
            .await?
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT generation FROM projects WHERE id = $1 AND status = 'maintenance' FOR UPDATE",
            )
            .bind(job.project_id)
            .fetch_optional(&mut *tx)
            .await?
        };
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
        let lease_token = Uuid::new_v4();
        let job = sqlx::query_as::<_, RestoreJobRecord>(
            "UPDATE restore_jobs SET status = 'running', generation_before = COALESCE(generation_before, $2), \
                 lease_token = $3, lease_expires_at = CURRENT_TIMESTAMP + INTERVAL '5 minutes', \
                 started_at = COALESCE(started_at, CURRENT_TIMESTAMP) WHERE id = $1 \
             RETURNING id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                       status, pre_restore_snapshot_id, generation_before, generation_after, \
                       restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at, lease_token",
        )
        .bind(job.id)
        .bind(generation)
        .bind(lease_token)
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
        let (owner_user_id, generation, sequence, cancel_requested) = sqlx::query_as::<_, (Uuid, i64, i64, bool)>(
            "SELECT p.owner_user_id, p.generation, p.change_seq, j.cancel_requested \
             FROM projects p JOIN restore_jobs j ON j.project_id = p.id \
             WHERE p.id = $1 AND j.id = $2 AND j.status = 'running' AND j.lease_token = $3 FOR UPDATE",
        )
        .bind(job.project_id)
        .bind(job.id)
        .bind(job.lease_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if cancel_requested {
            return Err(PersistenceError::RestoreCancelled);
        }
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
        sqlx::query(
            "UPDATE restore_jobs SET pre_restore_snapshot_id = $2 \
             WHERE id = $1 AND status = 'running' AND lease_token = $3",
        )
        .bind(job.id)
        .bind(snapshot_id)
        .bind(job.lease_token)
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

    /// Finalizes a pre-restore snapshot only for the worker holding its lease.
    #[allow(clippy::missing_errors_doc)]
    pub async fn mark_snapshot_ready_for_job(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        snapshot_id: Uuid,
        bucket: &str,
        key: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        sqlx::query(
            "UPDATE snapshots SET manifest_bucket = $2, manifest_key = $3, status = 'ready' \
             WHERE id = $1 AND status = 'pending' \
               AND EXISTS (SELECT 1 FROM restore_jobs WHERE id = $4 AND lease_token = $5 AND status = 'running')",
        )
        .bind(snapshot_id)
        .bind(bucket)
        .bind(key)
        .bind(job_id)
        .bind(lease_token)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Extends a restore lease while a worker is performing object-store work.
    #[allow(clippy::missing_errors_doc)]
    pub async fn renew_restore_lease(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let updated = sqlx::query(
            "UPDATE restore_jobs SET lease_expires_at = CURRENT_TIMESTAMP + INTERVAL '5 minutes' \
             WHERE id = $1 AND status = 'running' AND lease_token = $2",
        )
        .bind(job_id)
        .bind(lease_token)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        if updated != 1 {
            return Err(PersistenceError::Conflict);
        }
        Ok(())
    }

    /// Completes a running cancellation request while holding the worker lease.
    #[allow(clippy::missing_errors_doc)]
    pub async fn cancel_restore_with_lease(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let (project_id, owner_user_id, requested_by, cancel_requested) =
            sqlx::query_as::<_, (Uuid, Uuid, Uuid, bool)>(
                "SELECT project_id, owner_user_id, requested_by, cancel_requested \
                 FROM restore_jobs WHERE id = $1 AND status = 'running' AND lease_token = $2 FOR UPDATE",
            )
            .bind(job_id)
            .bind(lease_token)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(PersistenceError::NotFound)?;
        if !cancel_requested {
            return Err(PersistenceError::Conflict);
        }
        sqlx::query(
            "UPDATE restore_jobs SET status = 'cancelled', finished_at = CURRENT_TIMESTAMP, \
                 lease_token = NULL, lease_expires_at = NULL \
             WHERE id = $1 AND status = 'running' AND lease_token = $2",
        )
        .bind(job_id)
        .bind(lease_token)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE projects SET status = 'active', updated_at = CURRENT_TIMESTAMP \
             WHERE id = $1 AND status = 'maintenance'",
        )
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            requested_by,
            Some(owner_user_id),
            "restore.cancelled",
            json!({"projectId": project_id, "restoreId": job_id}),
            "",
        )
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

    /// Fails a restore only when the worker still owns its lease.
    #[allow(clippy::missing_errors_doc)]
    pub async fn fail_restore_job_with_lease(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        error_code: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let _project_id = sqlx::query_scalar::<_, Uuid>(
            "UPDATE restore_jobs SET status = 'failed', error_code = $3, \
                 lease_token = NULL, lease_expires_at = NULL, finished_at = CURRENT_TIMESTAMP \
             WHERE id = $1 AND status = 'running' AND lease_token = $2 \
             RETURNING project_id",
        )
        .bind(job_id)
        .bind(lease_token)
        .bind(error_code)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Requeues a transient restore failure while retaining the project's maintenance window.
    /// After three attempts the job becomes failed and requires operator verification.
    #[allow(clippy::missing_errors_doc)]
    pub async fn retry_restore_job_with_lease(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        error_code: &str,
    ) -> Result<bool, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let requeued = sqlx::query_scalar::<_, bool>(
            "WITH updated AS (\
                 UPDATE restore_jobs SET attempts = attempts + 1, error_code = $3, \
                    status = CASE WHEN attempts + 1 < 3 THEN 'queued' ELSE 'failed' END, \
                    run_after = CURRENT_TIMESTAMP + INTERVAL '1 minute', \
                    lease_token = NULL, lease_expires_at = NULL, \
                    finished_at = CASE WHEN attempts + 1 < 3 THEN NULL ELSE CURRENT_TIMESTAMP END \
                 WHERE id = $1 AND status = 'running' AND lease_token = $2 \
                 RETURNING attempts \
             ) SELECT COALESCE((SELECT attempts < 3 FROM updated), false)",
        )
        .bind(job_id)
        .bind(lease_token)
        .bind(error_code)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(requeued)
    }

    /// Reopens a project after an operator has verified a failed restore.
    #[allow(clippy::missing_errors_doc)]
    pub async fn reopen_failed_restore(
        &self,
        actor_user_id: Uuid,
        project_id: Uuid,
        reason: &str,
        request_id: &str,
    ) -> Result<(), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM projects WHERE id = $1 FOR UPDATE",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if status != "maintenance" {
            return Err(PersistenceError::RestoreNotReopenable);
        }
        let failed_restore = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM admin_restore_job_metadata \
             WHERE project_id = $1 AND status = 'failed')",
        )
        .bind(project_id)
        .fetch_one(&mut *tx)
        .await?;
        if !failed_restore {
            return Err(PersistenceError::RestoreNotReopenable);
        }
        sqlx::query(
            "UPDATE projects SET status = 'active', updated_at = CURRENT_TIMESTAMP WHERE id = $1",
        )
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            actor_user_id,
            None,
            "restore.reopened",
            json!({"projectId": project_id, "reason": reason}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Applies a restore by appending revisions and advancing project generation.
    #[allow(clippy::missing_errors_doc, clippy::too_many_lines)]
    pub async fn execute_restore(&self, job_id: Uuid) -> Result<(), PersistenceError> {
        let lease_token = sqlx::query_scalar::<_, Uuid>(
            "SELECT lease_token FROM restore_jobs \
             WHERE id = $1 AND status = 'running' AND lease_token IS NOT NULL",
        )
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(PersistenceError::Conflict)?;
        self.execute_restore_with_lease(job_id, lease_token).await
    }

    /// Applies a restore while fencing workers that lost their lease.
    #[allow(clippy::missing_errors_doc, clippy::too_many_lines)]
    pub async fn execute_restore_with_lease(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let job = sqlx::query_as::<_, RestoreJobRecord>(
            "SELECT id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                    status, pre_restore_snapshot_id, generation_before, generation_after, \
                    restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at, \
                    cancel_requested \
             FROM restore_jobs WHERE id = $1 AND status = 'running' AND lease_token = $2 FOR UPDATE",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if job.status != "running" {
            return Err(PersistenceError::Conflict);
        }
        if job.cancel_requested {
            return Err(PersistenceError::RestoreCancelled);
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
                 restored_tombstones = $4, lease_token = NULL, lease_expires_at = NULL, \
                 finished_at = CURRENT_TIMESTAMP WHERE id = $1 AND status = 'running' AND lease_token = $5",
        )
        .bind(job_id)
        .bind(generation_after)
        .bind(restored_objects)
        .bind(restored_tombstones)
        .bind(lease_token)
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
            manifest_id: Uuid::nil(),
            generation,
            snapshot_sequence,
            records,
            has_more,
        })
    }

    /// Captures the current project heads into an immutable bootstrap manifest.
    /// The returned bytes must be written and verified in object storage before
    /// `record_bootstrap_manifest` is called.
    #[allow(clippy::missing_errors_doc)]
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_bootstrap_manifest(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        manifest_id: Uuid,
    ) -> Result<BootstrapManifest, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let (generation, sequence) = require_active_project_for_update(&mut tx, project_id).await?;
        let records = sqlx::query_as::<_, SyncRecord>(
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
             ORDER BY kind, id",
        )
        .bind(project_id)
        .bind(sequence)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let manifest = json!({
            "version": 1,
            "manifestId": manifest_id,
            "projectId": project_id,
            "generation": generation,
            "changeSequence": sequence,
            "items": records,
        });
        let bytes = serde_json::to_vec(&manifest).map_err(|_| PersistenceError::InvalidPayload)?;
        let hash = hex::encode(Sha256::digest(&bytes));
        Ok(BootstrapManifest {
            id: manifest_id,
            generation,
            snapshot_sequence: sequence,
            records,
            bytes,
            hash,
        })
    }

    /// Commits the database reference after an immutable bootstrap manifest has been verified.
    #[allow(clippy::missing_errors_doc)]
    #[allow(clippy::too_many_arguments)]
    pub async fn record_bootstrap_manifest(
        &self,
        user_id: Uuid,
        role: &str,
        project_id: Uuid,
        manifest: &BootstrapManifest,
        bucket: &str,
        key: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        sqlx::query(
            "INSERT INTO bootstrap_manifests \
                 (id, project_id, owner_user_id, generation, change_sequence, manifest_hash, \
                  manifest_bucket, manifest_key, manifest, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, CURRENT_TIMESTAMP + INTERVAL '24 hours')",
        )
        .bind(manifest.id)
        .bind(project_id)
        .bind(user_id)
        .bind(manifest.generation)
        .bind(manifest.snapshot_sequence)
        .bind(&manifest.hash)
        .bind(bucket)
        .bind(key)
        .bind(json!({
            "version": 1,
            "manifestId": manifest.id,
            "projectId": project_id,
            "generation": manifest.generation,
            "changeSequence": manifest.snapshot_sequence,
            "items": manifest.records,
        }))
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_conflict)?;
        tx.commit().await?;
        Ok(())
    }

    /// Reads a page from a persisted manifest. Pagination is performed over the fixed JSON
    /// snapshot, so no database OFFSET scan or moving head set is involved.
    #[allow(clippy::missing_errors_doc)]
    #[allow(clippy::too_many_arguments)]
    pub async fn bootstrap_manifest_page(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
        project_id: Uuid,
        manifest_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> Result<BootstrapPage, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let (generation, current_sequence) = require_active_project(&mut tx, project_id).await?;
        let (manifest_generation, snapshot_sequence, items) =
            sqlx::query_as::<_, (i64, i64, serde_json::Value)>(
                "SELECT generation, change_sequence, manifest->'items' \
             FROM bootstrap_manifests \
             WHERE id = $1 AND project_id = $2 AND owner_user_id = $3 \
               AND expires_at > CURRENT_TIMESTAMP",
            )
            .bind(manifest_id)
            .bind(project_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(PersistenceError::NotFound)?;
        if manifest_generation != generation {
            return Err(PersistenceError::GenerationMismatch {
                expected: manifest_generation,
                actual: generation,
            });
        }
        if snapshot_sequence > current_sequence || offset < 0 {
            return Err(PersistenceError::NotFound);
        }
        let all_records: Vec<SyncRecord> =
            serde_json::from_value(items).map_err(|_| PersistenceError::InvalidPayload)?;
        let start = usize::try_from(offset).map_err(|_| PersistenceError::InvalidPayload)?;
        let page_limit =
            usize::try_from(limit.max(0)).map_err(|_| PersistenceError::InvalidPayload)?;
        let records = all_records
            .into_iter()
            .skip(start)
            .take(page_limit.saturating_add(1))
            .collect::<Vec<_>>();
        let has_more = records.len() > page_limit;
        let records = if has_more {
            records.into_iter().take(page_limit).collect()
        } else {
            records
        };
        if !has_more {
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
            manifest_id,
            generation,
            snapshot_sequence,
            records,
            has_more,
        })
    }

    /// Deletes expired bootstrap manifest references and returns their object keys.
    #[allow(clippy::missing_errors_doc)]
    pub async fn expire_bootstrap_manifests(
        &self,
        limit: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let keys = sqlx::query_scalar::<_, String>(
            "WITH expired AS ( \
                 SELECT id FROM bootstrap_manifests \
                 WHERE expires_at <= CURRENT_TIMESTAMP \
                 ORDER BY expires_at, id LIMIT $1 \
             ) \
             DELETE FROM bootstrap_manifests b USING expired e \
             WHERE b.id = e.id RETURNING b.manifest_key",
        )
        .bind(limit.max(1))
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(keys)
    }

    /// Marks expired account exports and returns their detached object keys for deletion.
    #[allow(clippy::missing_errors_doc)]
    pub async fn expire_account_purge_exports(
        &self,
        limit: i64,
    ) -> Result<Vec<String>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let keys = sqlx::query_scalar::<_, String>(
            "WITH expired AS (\
                 SELECT e.id FROM account_purge_exports e\
                 WHERE e.expires_at <= CURRENT_TIMESTAMP\
                   AND e.status IN ('pending', 'ready', 'failed')\
                 ORDER BY e.expires_at, e.id LIMIT $1\
             )\
             UPDATE account_purge_exports e SET status = 'expired'\
             FROM expired WHERE e.id = expired.id RETURNING e.object_key",
        )
        .bind(limit.max(1))
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(keys)
    }

    /// Claims one project purge using a renewable lease.
    #[allow(clippy::missing_errors_doc)]
    pub async fn claim_project_purge_job(&self) -> Result<Option<JobRecord>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let Some(job_id) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM jobs \
             WHERE kind = 'project_purge' AND run_after <= CURRENT_TIMESTAMP \
               AND (status = 'queued' OR \
                    (status = 'running' AND lease_expires_at < CURRENT_TIMESTAMP)) \
             ORDER BY run_after, created_at, id \
             FOR UPDATE SKIP LOCKED LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let lease_token = Uuid::new_v4();
        let job = sqlx::query_as::<_, JobRecord>(
            "UPDATE jobs SET status = 'running', attempts = attempts + 1, lease_token = $2, \
                 lease_expires_at = CURRENT_TIMESTAMP + INTERVAL '5 minutes', \
                 started_at = COALESCE(started_at, CURRENT_TIMESTAMP), error_code = NULL \
             WHERE id = $1 \
             RETURNING id, kind, owner_user_id, project_id, status, attempts, run_after, \
                       error_code, created_at, started_at, finished_at, lease_token",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(job))
    }

    /// Claims one account purge job using the same bounded lease as project purge.
    #[allow(clippy::missing_errors_doc)]
    pub async fn claim_account_purge_job(&self) -> Result<Option<JobRecord>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let Some(job_id) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM jobs \
             WHERE kind = 'account_purge' AND run_after <= CURRENT_TIMESTAMP \
               AND (status = 'queued' OR \
                    (status = 'running' AND lease_expires_at < CURRENT_TIMESTAMP)) \
             ORDER BY run_after, created_at, id \
             FOR UPDATE SKIP LOCKED LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let lease_token = Uuid::new_v4();
        let job = sqlx::query_as::<_, JobRecord>(
            "UPDATE jobs SET status = 'running', attempts = attempts + 1, lease_token = $2, \
                 lease_expires_at = CURRENT_TIMESTAMP + INTERVAL '5 minutes', \
                 started_at = COALESCE(started_at, CURRENT_TIMESTAMP), error_code = NULL \
             WHERE id = $1 \
             RETURNING id, kind, owner_user_id, project_id, status, attempts, run_after, \
                       error_code, created_at, started_at, finished_at, lease_token",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(job))
    }

    /// Returns the phase of a claimed account purge without exposing its payload to callers.
    #[allow(clippy::missing_errors_doc)]
    pub async fn account_purge_phase(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<String, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let phase = sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(payload->>'phase', '') FROM jobs \
             WHERE id = $1 AND kind = 'account_purge' AND status = 'running' \
               AND lease_token = $2 AND lease_expires_at > CURRENT_TIMESTAMP",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        tx.commit().await?;
        if matches!(phase.as_str(), "export" | "purge") {
            Ok(phase)
        } else {
            Err(PersistenceError::InvalidPayload)
        }
    }

    /// Extends an account purge lease while the worker is downloading and packaging payloads.
    #[allow(clippy::missing_errors_doc)]
    pub async fn renew_account_purge_lease(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let updated = sqlx::query(
            "UPDATE jobs SET lease_expires_at = CURRENT_TIMESTAMP + INTERVAL '5 minutes' \
             WHERE id = $1 AND kind = 'account_purge' AND status = 'running' \
               AND lease_token = $2 AND lease_expires_at > CURRENT_TIMESTAMP",
        )
        .bind(job_id)
        .bind(lease_token)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(PersistenceError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    /// Collects an account export manifest and the payload objects that belong in its archive.
    #[allow(clippy::missing_errors_doc, clippy::too_many_lines)]
    pub async fn prepare_account_export(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<AccountExportData, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let (owner_user_id, payload) = sqlx::query_as::<_, (Uuid, serde_json::Value)>(
            "SELECT owner_user_id, payload FROM jobs \
             WHERE id = $1 AND kind = 'account_purge' AND status = 'running' \
               AND lease_token = $2 AND lease_expires_at > CURRENT_TIMESTAMP FOR UPDATE",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        let export_id = payload
            .get("exportId")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(PersistenceError::InvalidPayload)?;
        let export_object_key = sqlx::query_scalar::<_, String>(
            "SELECT object_key FROM account_purge_exports \
             WHERE id = $1 AND user_id = $2 AND status = 'pending' \
               AND expires_at > CURRENT_TIMESTAMP FOR UPDATE",
        )
        .bind(export_id)
        .bind(owner_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::AccountPurgeNotReady)?;
        let projects = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT COALESCE(jsonb_agg(jsonb_build_object(\
                 'id', id, 'name', name, 'generation', generation, 'status', status::text, \
                 'changeSequence', change_seq, 'createdAt', created_at, 'updatedAt', updated_at)\
                 ORDER BY id), '[]'::jsonb) FROM projects WHERE owner_user_id = $1",
        )
        .bind(owner_user_id)
        .fetch_one(&mut *tx)
        .await?;
        let devices = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT COALESCE(jsonb_agg(jsonb_build_object(\
                 'id', id, 'displayName', display_name, 'platform', platform, \
                 'appVersion', app_version, 'createdAt', created_at, 'lastSeenAt', last_seen_at)\
                 ORDER BY id), '[]'::jsonb) FROM devices WHERE owner_user_id = $1",
        )
        .bind(owner_user_id)
        .fetch_one(&mut *tx)
        .await?;
        let payload_metadata = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT COALESCE(jsonb_agg(jsonb_build_object(\
                 'projectId', project_id, 'contentHash', content_hash, 'sizeBytes', size_bytes, \
                 'mediaType', media_type, 'createdAt', created_at) ORDER BY project_id, content_hash), \
                 '[]'::jsonb) FROM payloads WHERE owner_user_id = $1",
        )
        .bind(owner_user_id)
        .fetch_one(&mut *tx)
        .await?;
        let revisions = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT COALESCE(jsonb_agg(jsonb_build_object(\
                 'projectId', project_id, 'kind', kind::text, 'objectId', object_id, \
                 'revision', revision, 'baseRevision', base_revision, 'schemaVersion', schema_version, \
                 'contentHash', content_hash, 'isTombstone', is_tombstone, 'changedAt', changed_at, \
                 'deviceId', device_id) ORDER BY project_id, kind, object_id, revision), '[]'::jsonb) \
             FROM object_revisions WHERE owner_user_id = $1",
        )
        .bind(owner_user_id)
        .fetch_one(&mut *tx)
        .await?;
        let change_log = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT COALESCE(jsonb_agg(jsonb_build_object(\
                 'projectId', project_id, 'sequence', sequence, 'revisionId', revision_id, \
                 'createdAt', created_at) ORDER BY project_id, sequence), '[]'::jsonb) \
             FROM change_log WHERE owner_user_id = $1",
        )
        .bind(owner_user_id)
        .fetch_one(&mut *tx)
        .await?;
        let snapshots = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT COALESCE(jsonb_agg(jsonb_build_object(\
                 'id', id, 'projectId', project_id, 'generation', generation, \
                 'changeSequence', change_sequence, 'manifestHash', manifest_hash, \
                 'status', status, 'createdBy', created_by, 'createdAt', created_at, \
                 'manifest', manifest) ORDER BY project_id, created_at, id), '[]'::jsonb) \
             FROM snapshots WHERE owner_user_id = $1",
        )
        .bind(owner_user_id)
        .fetch_one(&mut *tx)
        .await?;
        let payload_rows = sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT project_id, content_hash, object_key FROM payloads \
             WHERE owner_user_id = $1 ORDER BY project_id, content_hash",
        )
        .bind(owner_user_id)
        .fetch_all(&mut *tx)
        .await?;
        let manifest = serde_json::to_vec(&json!({
            "format": "tasktips-account-export-v1",
            "userId": owner_user_id,
            "projects": projects,
            "devices": devices,
            "payloads": payload_metadata,
            "revisions": revisions,
            "changeLog": change_log,
            "snapshots": snapshots,
        }))
        .map_err(|_| PersistenceError::InvalidPayload)?;
        let payloads = payload_rows
            .into_iter()
            .map(
                |(project_id, content_hash, object_key)| AccountExportPayload {
                    object_key,
                    archive_path: format!("payloads/{project_id}/{content_hash}"),
                },
            )
            .collect();
        tx.commit().await?;
        Ok(AccountExportData {
            export_id,
            owner_user_id,
            object_key: export_object_key,
            manifest,
            payloads,
        })
    }

    /// Commits the verified export catalog entry after the archive is in `RustFS`.
    #[allow(clippy::missing_errors_doc)]
    pub async fn complete_account_export(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        export_id: Uuid,
        content_hash: &str,
        size: u64,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let (owner_user_id, payload) = sqlx::query_as::<_, (Uuid, serde_json::Value)>(
            "SELECT owner_user_id, payload FROM jobs WHERE id = $1 AND kind = 'account_purge' \
             AND status = 'running' AND lease_token = $2 AND lease_expires_at > CURRENT_TIMESTAMP FOR UPDATE",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        let requested_by = payload
            .get("requestedBy")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .unwrap_or(owner_user_id);
        sqlx::query(
            "UPDATE account_purge_exports SET status = 'ready', content_hash = $2, size_bytes = $3, \
                 completed_at = CURRENT_TIMESTAMP WHERE id = $1 AND user_id = $4 AND status = 'pending'",
        )
        .bind(export_id)
        .bind(content_hash)
        .bind(i64::try_from(size).map_err(|_| PersistenceError::InvalidPayload)?)
        .bind(owner_user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE jobs SET status = 'succeeded', lease_token = NULL, lease_expires_at = NULL, \
                 finished_at = CURRENT_TIMESTAMP WHERE id = $1 AND lease_token = $2",
        )
        .bind(job_id)
        .bind(lease_token)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            requested_by,
            Some(owner_user_id),
            "account.purge_export_ready",
            json!({"jobId": job_id, "exportId": export_id}),
            "",
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Detaches all account-owned database rows and returns object keys for deletion from `RustFS`.
    #[allow(clippy::missing_errors_doc, clippy::too_many_lines)]
    pub async fn prepare_account_purge(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<Vec<String>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let (owner_user_id, payload) = sqlx::query_as::<_, (Uuid, serde_json::Value)>(
            "SELECT owner_user_id, payload FROM jobs \
             WHERE id = $1 AND kind = 'account_purge' AND status = 'running' \
               AND lease_token = $2 AND lease_expires_at > CURRENT_TIMESTAMP FOR UPDATE",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if payload
            .get("detached")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            let keys = payload
                .get("objectKeys")
                .and_then(serde_json::Value::as_array)
                .ok_or(PersistenceError::InvalidPayload)?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or(PersistenceError::InvalidPayload)
                })
                .collect::<Result<Vec<_>, _>>()?;
            tx.commit().await?;
            return Ok(keys);
        }
        let export_id = payload
            .get("exportId")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(PersistenceError::InvalidPayload)?;
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM users WHERE id = $1 FOR UPDATE",
        )
        .bind(owner_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if status != AccountStatus::Deleting.as_str() {
            return Err(PersistenceError::InvalidAccountTransition);
        }
        let mut keys = sqlx::query_scalar::<_, String>(
            "SELECT object_key FROM payloads WHERE owner_user_id = $1 \
             UNION SELECT manifest_key FROM snapshots WHERE owner_user_id = $1 \
             UNION SELECT manifest_key FROM bootstrap_manifests WHERE owner_user_id = $1 \
             UNION SELECT object_key FROM account_purge_exports WHERE id = $2",
        )
        .bind(owner_user_id)
        .bind(export_id)
        .fetch_all(&mut *tx)
        .await?;
        keys.sort_unstable();
        for statement in [
            "DELETE FROM bootstrap_completions WHERE owner_user_id = $1",
            "DELETE FROM idempotency_records WHERE owner_user_id = $1",
            "DELETE FROM sync_attempts WHERE owner_user_id = $1",
            "DELETE FROM restore_jobs WHERE owner_user_id = $1",
            "DELETE FROM snapshots WHERE owner_user_id = $1",
            "DELETE FROM bootstrap_manifests WHERE owner_user_id = $1",
            "DELETE FROM change_log WHERE owner_user_id = $1",
            "DELETE FROM object_heads WHERE owner_user_id = $1",
            "DELETE FROM object_revisions WHERE owner_user_id = $1",
            "DELETE FROM payloads WHERE owner_user_id = $1",
            "DELETE FROM projects WHERE owner_user_id = $1",
        ] {
            sqlx::query(statement)
                .bind(owner_user_id)
                .execute(&mut *tx)
                .await?;
        }
        let email_normalized =
            sqlx::query_scalar::<_, String>("SELECT email_normalized FROM users WHERE id = $1")
                .bind(owner_user_id)
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
            .bind(owner_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM devices WHERE owner_user_id = $1")
            .bind(owner_user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM invitations WHERE email_normalized = $1")
            .bind(email_normalized)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE jobs SET payload = $3 WHERE id = $1 AND status = 'running' AND lease_token = $2",
        )
        .bind(job_id)
        .bind(lease_token)
        .bind(json!({"detached": true, "exportId": export_id, "objectKeys": keys}))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(keys)
    }

    /// Marks an account deleted after all detached object-store keys have been removed.
    #[allow(clippy::missing_errors_doc)]
    pub async fn complete_account_purge(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let (owner_user_id, payload) = sqlx::query_as::<_, (Uuid, serde_json::Value)>(
            "SELECT owner_user_id, payload FROM jobs \
             WHERE id = $1 AND kind = 'account_purge' AND status = 'running' \
               AND lease_token = $2 AND lease_expires_at > CURRENT_TIMESTAMP FOR UPDATE",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        let export_id = payload
            .get("exportId")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(PersistenceError::InvalidPayload)?;
        let requested_by = payload
            .get("requestedBy")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .unwrap_or(owner_user_id);
        sqlx::query(
            "UPDATE users SET status = 'deleted', email_normalized = $2, email_display = 'deleted', \
                 password_hash = $3, last_login_at = NULL WHERE id = $1 AND status = 'deleting'",
        )
        .bind(owner_user_id)
        .bind(format!("deleted+{owner_user_id}@invalid.tasktips"))
        .bind(format!("!deleted!{owner_user_id}"))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE account_purge_exports SET status = 'completed', completed_at = CURRENT_TIMESTAMP \
             WHERE id = $1 AND user_id = $2",
        )
        .bind(export_id)
        .bind(owner_user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE jobs SET status = 'succeeded', lease_token = NULL, lease_expires_at = NULL, \
                 finished_at = CURRENT_TIMESTAMP WHERE id = $1 AND lease_token = $2",
        )
        .bind(job_id)
        .bind(lease_token)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            requested_by,
            Some(owner_user_id),
            "account.purge_completed",
            json!({"jobId": job_id, "exportId": export_id}),
            "",
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records a bounded account purge retry. A failed export remains confirmable only after a
    /// later retry succeeds; a failed purge deliberately leaves the account in `deleting`.
    #[allow(clippy::missing_errors_doc)]
    pub async fn fail_account_purge(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        error_code: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let phase = sqlx::query_scalar::<_, String>(
            "SELECT COALESCE(payload->>'phase', '') FROM jobs \
             WHERE id = $1 AND kind = 'account_purge' AND status = 'running' AND lease_token = $2",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        let (job_status, _) = sqlx::query_as::<_, (String, i32)>(
            "UPDATE jobs SET status = CASE WHEN attempts < 5 THEN 'queued' ELSE 'failed' END, \
                 error_code = $3, lease_token = NULL, lease_expires_at = NULL, \
                 run_after = CURRENT_TIMESTAMP + INTERVAL '1 minute', \
                 finished_at = CASE WHEN attempts < 5 THEN NULL ELSE CURRENT_TIMESTAMP END \
             WHERE id = $1 AND status = 'running' AND lease_token = $2 \
             RETURNING status, attempts",
        )
        .bind(job_id)
        .bind(lease_token)
        .bind(error_code)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if phase == "export" {
            sqlx::query(
                "UPDATE account_purge_exports SET status = CASE WHEN $2 = 'failed' THEN 'failed' ELSE 'pending' END \
                 WHERE job_id = $1 AND status = 'pending'",
            )
            .bind(job_id)
            .bind(&job_status)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Atomically removes database references and persists the detached object keys on the job.
    /// A retry after commit reads the same key list without touching the deleted project again.
    #[allow(clippy::missing_errors_doc)]
    pub async fn prepare_project_purge(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<Vec<String>, PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let (owner_user_id, project_id, payload) =
            sqlx::query_as::<_, (Uuid, Uuid, serde_json::Value)>(
                "SELECT owner_user_id, project_id, payload FROM jobs \
             WHERE id = $1 AND kind = 'project_purge' AND status = 'running' \
               AND lease_token = $2 AND lease_expires_at > CURRENT_TIMESTAMP FOR UPDATE",
            )
            .bind(job_id)
            .bind(lease_token)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(PersistenceError::NotFound)?;
        if payload
            .get("detached")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            let keys = payload
                .get("objectKeys")
                .and_then(serde_json::Value::as_array)
                .ok_or(PersistenceError::InvalidPayload)?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or(PersistenceError::InvalidPayload)
                })
                .collect::<Result<Vec<_>, _>>()?;
            tx.commit().await?;
            return Ok(keys);
        }
        let project_status = sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM projects \
             WHERE id = $1 AND owner_user_id = $2 FOR UPDATE",
        )
        .bind(project_id)
        .bind(owner_user_id)
        .fetch_optional(&mut *tx)
        .await?;
        if project_status.as_deref() != Some("deleting") {
            return Err(PersistenceError::Conflict);
        }
        let keys = sqlx::query_scalar::<_, String>(
            "SELECT object_key FROM payloads WHERE project_id = $1 \
             UNION SELECT manifest_key FROM snapshots WHERE project_id = $1 \
             UNION SELECT manifest_key FROM bootstrap_manifests WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_all(&mut *tx)
        .await?;
        for statement in [
            "DELETE FROM bootstrap_completions WHERE project_id = $1",
            "DELETE FROM idempotency_records WHERE project_id = $1",
            "DELETE FROM sync_attempts WHERE project_id = $1",
            "DELETE FROM restore_jobs WHERE project_id = $1",
            "DELETE FROM snapshots WHERE project_id = $1",
            "DELETE FROM bootstrap_manifests WHERE project_id = $1",
            "DELETE FROM change_log WHERE project_id = $1",
            "DELETE FROM object_heads WHERE project_id = $1",
            "DELETE FROM object_revisions WHERE project_id = $1",
            "DELETE FROM payloads WHERE project_id = $1",
        ] {
            sqlx::query(statement)
                .bind(project_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM projects WHERE id = $1")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE jobs SET payload = $3 \
             WHERE id = $1 AND status = 'running' AND lease_token = $2",
        )
        .bind(job_id)
        .bind(lease_token)
        .bind(json!({
            "detached": true,
            "objectKeys": keys,
        }))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(keys)
    }

    /// Marks a detached purge complete after all object-store keys are gone.
    #[allow(clippy::missing_errors_doc)]
    pub async fn complete_project_purge(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let (owner_user_id, project_id) = sqlx::query_as::<_, (Uuid, Uuid)>(
            "SELECT owner_user_id, project_id FROM jobs \
             WHERE id = $1 AND kind = 'project_purge' AND status = 'running' \
               AND lease_token = $2 AND lease_expires_at > CURRENT_TIMESTAMP FOR UPDATE",
        )
        .bind(job_id)
        .bind(lease_token)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        sqlx::query(
            "UPDATE jobs SET status = 'succeeded', lease_token = NULL, lease_expires_at = NULL, \
                 finished_at = CURRENT_TIMESTAMP WHERE id = $1 AND lease_token = $2",
        )
        .bind(job_id)
        .bind(lease_token)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO audit_events \
                 (actor_user_id, subject_user_id, action, metadata) \
             VALUES ($1, $1, 'project.purge_completed', $2)",
        )
        .bind(owner_user_id)
        .bind(json!({"projectId": project_id, "jobId": job_id}))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records a purge failure and schedules bounded automatic retry.
    #[allow(clippy::missing_errors_doc)]
    pub async fn fail_project_purge(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        error_code: &str,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.begin_worker().await?;
        let updated = sqlx::query(
            "UPDATE jobs SET status = CASE WHEN attempts < 5 THEN 'queued' ELSE 'failed' END, \
                 error_code = $3, lease_token = NULL, lease_expires_at = NULL, \
                 run_after = CURRENT_TIMESTAMP + INTERVAL '1 minute', \
                 finished_at = CASE WHEN attempts < 5 THEN NULL ELSE CURRENT_TIMESTAMP END \
             WHERE id = $1 AND status = 'running' AND lease_token = $2",
        )
        .bind(job_id)
        .bind(lease_token)
        .bind(error_code)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(PersistenceError::NotFound);
        }
        tx.commit().await?;
        Ok(())
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
        self.push_with_outcome(
            user_id,
            role,
            device_id,
            project_id,
            expected_generation,
            request_key,
            request_hash,
            revisions,
        )
        .await
        .map(|outcome| outcome.response)
    }

    /// Applies a push and reports whether the response came from an idempotency replay.
    ///
    /// # Errors
    ///
    /// Returns an error when request-level guards fail or persistence is unavailable.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub async fn push_with_outcome(
        &self,
        user_id: Uuid,
        role: &str,
        device_id: Uuid,
        project_id: Uuid,
        expected_generation: i64,
        request_key: &str,
        request_hash: &str,
        revisions: &[NewSyncRevision],
    ) -> Result<PushOutcome, PersistenceError> {
        let mut tx = self.begin_user(user_id, role).await?;
        let lock_key = format!("{user_id}:{device_id}:{request_key}");
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(lock_key)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM idempotency_records WHERE owner_user_id = $1 AND device_id = $2 \
             AND project_id = $3 AND request_key = $4 AND expires_at <= CURRENT_TIMESTAMP",
        )
        .bind(user_id)
        .bind(device_id)
        .bind(project_id)
        .bind(request_key)
        .execute(&mut *tx)
        .await?;
        if let Some((stored_hash, response, response_status)) =
            sqlx::query_as::<_, (String, serde_json::Value, i16)>(
                "SELECT request_hash::text, response, response_status FROM idempotency_records \
                 WHERE owner_user_id = $1 AND device_id = $2 AND project_id = $3 \
                   AND request_key = $4",
            )
            .bind(user_id)
            .bind(device_id)
            .bind(project_id)
            .bind(request_key)
            .fetch_optional(&mut *tx)
            .await?
        {
            if stored_hash == request_hash {
                tx.commit().await?;
                if response_status == 200 {
                    return Ok(PushOutcome {
                        response,
                        replayed: true,
                    });
                }
                return Err(PersistenceError::IdempotencyReplay {
                    response,
                    status: response_status,
                });
            }
            return Err(PersistenceError::IdempotencyConflict);
        }

        let (generation, _) = match require_active_project_for_update(&mut tx, project_id).await {
            Ok(state) => state,
            Err(error) => {
                if store_push_failure(
                    &mut tx,
                    user_id,
                    device_id,
                    project_id,
                    request_key,
                    request_hash,
                    &error,
                )
                .await?
                {
                    tx.commit().await?;
                }
                return Err(error);
            }
        };
        if expected_generation != generation {
            let error = PersistenceError::GenerationMismatch {
                expected: expected_generation,
                actual: generation,
            };
            store_push_failure(
                &mut tx,
                user_id,
                device_id,
                project_id,
                request_key,
                request_hash,
                &error,
            )
            .await?;
            tx.commit().await?;
            return Err(error);
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
            let error = PersistenceError::BootstrapRequired;
            store_push_failure(
                &mut tx,
                user_id,
                device_id,
                project_id,
                request_key,
                request_hash,
                &error,
            )
            .await?;
            tx.commit().await?;
            return Err(error);
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
                        message: "payload 不存在",
                        details: None,
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
                        message: "payload 不存在",
                        details: None,
                    });
                    continue;
                };
                let size_valid =
                    u64::try_from(size).is_ok_and(|size| size <= revision.kind.max_payload_bytes());
                if !size_valid {
                    results.push(PushItemResult::Rejected {
                        kind: revision.kind,
                        id: revision.id.clone(),
                        code: "PAYLOAD_TOO_LARGE",
                        message: "payload 超过对象类型限制",
                        details: Some(json!({
                            "kind": revision.kind,
                            "maxBytes": revision.kind.max_payload_bytes(),
                            "actualBytes": size,
                        })),
                    });
                    continue;
                }
                if !valid_media_type(revision.kind, &media_type) {
                    results.push(PushItemResult::Rejected {
                        kind: revision.kind,
                        id: revision.id.clone(),
                        code: "INVALID_REQUEST",
                        message: "payload 媒体类型与对象类型不兼容",
                        details: None,
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
        Ok(PushOutcome {
            response,
            replayed: false,
        })
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
        Ok(self.admin_list_users_page(actor_user_id, 500, 0).await?.0)
    }

    /// Lists one page of account metadata without payload fields.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_users_page(
        &self,
        actor_user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<UserRecord>, bool), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let mut users = sqlx::query_as::<_, UserRecord>(
            "SELECT id, email_display AS email, '' AS password_hash, role::text AS role, \
                    status::text AS status, created_at, last_login_at \
             FROM admin_user_metadata ORDER BY created_at, id LIMIT $1 OFFSET $2",
        )
        .bind(limit.saturating_add(1))
        .bind(offset.max(0))
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(users.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            users.pop();
        }
        tx.commit().await?;
        Ok((users, has_more))
    }

    /// # Errors
    ///
    /// Returns an error when admin project metadata cannot be read.
    pub async fn admin_list_projects(
        &self,
        actor_user_id: Uuid,
        subject_user_id: Uuid,
    ) -> Result<Vec<ProjectRecord>, PersistenceError> {
        Ok(self
            .admin_list_projects_page(actor_user_id, subject_user_id, 500, 0)
            .await?
            .0)
    }

    /// Lists one page of project metadata across all accounts.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_all_projects_page(
        &self,
        actor_user_id: Uuid,
        limit: i64,
        offset: i64,
        search: Option<&str>,
    ) -> Result<(Vec<ProjectRecord>, bool), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let mut projects = sqlx::query_as::<_, ProjectRecord>(
            "SELECT id, owner_user_id, name, generation, status::text AS status, \
                    change_seq AS change_sequence, created_at, updated_at \
             FROM admin_project_metadata \
             WHERE ($1::text IS NULL OR name ILIKE '%' || $1 || '%' OR id::text ILIKE '%' || $1 || '%') \
             ORDER BY created_at, id LIMIT $2 OFFSET $3",
        )
        .bind(search)
        .bind(limit.saturating_add(1))
        .bind(offset.max(0))
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(projects.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            projects.pop();
        }
        tx.commit().await?;
        Ok((projects, has_more))
    }

    /// Lists one page of project metadata for an account.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_projects_page(
        &self,
        actor_user_id: Uuid,
        subject_user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<ProjectRecord>, bool), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let mut projects = sqlx::query_as::<_, ProjectRecord>(
            "SELECT id, owner_user_id, name, generation, status::text AS status, \
                    change_seq AS change_sequence, created_at, updated_at \
             FROM admin_project_metadata WHERE owner_user_id = $1 \
             ORDER BY created_at, id LIMIT $2 OFFSET $3",
        )
        .bind(subject_user_id)
        .bind(limit.saturating_add(1))
        .bind(offset.max(0))
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(projects.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            projects.pop();
        }
        tx.commit().await?;
        Ok((projects, has_more))
    }

    /// # Errors
    ///
    /// Returns an error when admin device metadata cannot be read.
    pub async fn admin_list_devices(
        &self,
        actor_user_id: Uuid,
        subject_user_id: Uuid,
    ) -> Result<Vec<DeviceRecord>, PersistenceError> {
        Ok(self
            .admin_list_devices_page(actor_user_id, subject_user_id, 500, 0)
            .await?
            .0)
    }

    /// Lists one page of device metadata across all accounts.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_all_devices_page(
        &self,
        actor_user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<DeviceRecord>, bool), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let mut devices = sqlx::query_as::<_, DeviceRecord>(
            "SELECT id, owner_user_id, display_name, platform, app_version, created_at, \
                    last_seen_at, last_login_at, last_pull_at, last_push_at, revoked_at \
             FROM admin_device_metadata \
             ORDER BY created_at, id LIMIT $1 OFFSET $2",
        )
        .bind(limit.saturating_add(1))
        .bind(offset.max(0))
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(devices.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            devices.pop();
        }
        tx.commit().await?;
        Ok((devices, has_more))
    }

    /// Lists one page of device metadata for an account.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_devices_page(
        &self,
        actor_user_id: Uuid,
        subject_user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<DeviceRecord>, bool), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let mut devices = sqlx::query_as::<_, DeviceRecord>(
            "SELECT id, owner_user_id, display_name, platform, app_version, created_at, \
                    last_seen_at, last_login_at, last_pull_at, last_push_at, revoked_at \
             FROM admin_device_metadata WHERE owner_user_id = $1 \
             ORDER BY created_at, id LIMIT $2 OFFSET $3",
        )
        .bind(subject_user_id)
        .bind(limit.saturating_add(1))
        .bind(offset.max(0))
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(devices.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            devices.pop();
        }
        tx.commit().await?;
        Ok((devices, has_more))
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
                users, active_users, projects, devices, revisions, tombstones, payload_bytes, queued_restores \
             FROM admin_operational_counts",
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
            "SELECT kind, object_id, revision, base_revision, changed_at, device_id, \
                    tombstone, change_sequence \
             FROM admin_history_metadata \
             WHERE project_id = $1 AND change_sequence > $2 \
             ORDER BY change_sequence LIMIT $3",
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
        validate_restore_target(snapshot_id, target_change_sequence)
            .map_err(|_| PersistenceError::InvalidRestoreTarget)?;
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let (owner_user_id, current_sequence, status) = sqlx::query_as::<_, (Uuid, i64, String)>(
            "SELECT owner_user_id, change_seq, status::text FROM admin_lock_project($1)",
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
        let active_restore = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM restore_jobs WHERE project_id = $1 AND status IN ('queued', 'running'))",
        )
        .bind(project_id)
        .fetch_one(&mut *tx)
        .await?;
        if active_restore {
            return Err(PersistenceError::Conflict);
        }
        if let Some(snapshot_id) = snapshot_id {
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM admin_snapshot_metadata \
                 WHERE id = $1 AND project_id = $2 AND status = 'ready')",
            )
            .bind(snapshot_id)
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
            if !exists {
                return Err(PersistenceError::NotFound);
            }
        }
        let job_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO restore_jobs \
                 (id, project_id, owner_user_id, requested_by, snapshot_id, target_change_sequence, reason, request_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(job_id)
        .bind(project_id)
        .bind(owner_user_id)
        .bind(actor_user_id)
        .bind(snapshot_id)
        .bind(target_change_sequence)
        .bind(reason)
        .bind(request_id)
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            actor_user_id,
            Some(owner_user_id),
            "restore.requested",
            json!({"projectId": project_id, "restoreId": job_id, "source": "admin"}),
            request_id,
        )
        .await?;
        let job = sqlx::query_as::<_, RestoreJobRecord>(
            "SELECT id, project_id, requested_by, snapshot_id, target_change_sequence, reason, \
                    status, pre_restore_snapshot_id, generation_before, generation_after, \
                    restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at \
             FROM admin_restore_job_metadata WHERE id = $1",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(job)
    }

    /// Lists a globally ordered, privacy-safe stream of operational metadata.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_operations_page(
        &self,
        actor_user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<AdminOperationRecord>, bool), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let mut operations = sqlx::query_as::<_, AdminOperationRecord>(
            "SELECT id, operation, status, attempts, run_after, cancel_requested, project_id, \
                    device_id, item_count, latency_ms, error_code, created_at \
             FROM ( \
                 SELECT id, operation, status, 0::integer AS attempts, \
                        NULL::timestamptz AS run_after, false AS cancel_requested, \
                        project_id, device_id, item_count, latency_ms, error_code, created_at \
                 FROM sync_attempts \
                 UNION ALL \
                 SELECT id, 'restore'::text AS operation, status, attempts, run_after, \
                        cancel_requested, project_id, NULL::uuid AS device_id, \
                        restored_objects AS item_count, NULL::integer AS latency_ms, error_code, created_at \
                 FROM admin_restore_job_metadata \
                 UNION ALL \
                 SELECT id, kind AS operation, status, attempts, run_after, \
                        false AS cancel_requested, project_id, NULL::uuid AS device_id, \
                        NULL::integer AS item_count, NULL::integer AS latency_ms, error_code, created_at \
                 FROM admin_job_metadata \
             ) operations \
             ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit.saturating_add(1))
        .bind(offset.max(0))
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(operations.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            operations.pop();
        }
        tx.commit().await?;
        Ok((operations, has_more))
    }

    /// Lists recent audit metadata. Audit metadata is never populated from payload bodies.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_audit_events(
        &self,
        actor_user_id: Uuid,
    ) -> Result<Vec<AuditEventRecord>, PersistenceError> {
        Ok(self
            .admin_list_audit_events_page(actor_user_id, 200, 0)
            .await?
            .0)
    }

    /// Lists one page of audit metadata without payload content.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_list_audit_events_page(
        &self,
        actor_user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<AuditEventRecord>, bool), PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let mut events = sqlx::query_as::<_, AuditEventRecord>(
            "SELECT id, actor_user_id, subject_user_id, project_id, action, metadata, request_id, created_at \
             FROM audit_events ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit.saturating_add(1))
        .bind(offset.max(0))
        .fetch_all(&mut *tx)
        .await?;
        let has_more = i64::try_from(events.len()).unwrap_or(i64::MAX) > limit;
        if has_more {
            events.pop();
        }
        tx.commit().await?;
        Ok((events, has_more))
    }

    /// Returns daily synchronization outcome and latency trends without payload data.
    #[allow(clippy::missing_errors_doc)]
    pub async fn admin_sync_trends(
        &self,
        actor_user_id: Uuid,
        days: i32,
    ) -> Result<Vec<AdminTrendPoint>, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        let mut tx = self.begin_admin().await?;
        let points = sqlx::query_as::<_, AdminTrendPoint>(
            "SELECT date_trunc('day', created_at) AS day, COUNT(*)::bigint AS attempts, \
                    COUNT(*) FILTER (WHERE status = 'succeeded')::bigint AS succeeded, \
                    COUNT(*) FILTER (WHERE status = 'conflict')::bigint AS conflicts, \
                    percentile_cont(0.50) WITHIN GROUP (ORDER BY latency_ms) \
                        FILTER (WHERE latency_ms IS NOT NULL) AS p50_latency_ms, \
                    percentile_cont(0.99) WITHIN GROUP (ORDER BY latency_ms) \
                        FILTER (WHERE latency_ms IS NOT NULL) AS p99_latency_ms \
             FROM sync_attempts \
             WHERE created_at >= CURRENT_TIMESTAMP - ($1::double precision * INTERVAL '1 day') \
             GROUP BY date_trunc('day', created_at) ORDER BY day",
        )
        .bind(f64::from(days.max(1)))
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(points)
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

    /// Starts the first phase of an account purge. The worker creates a final export and the
    /// account remains usable until an administrator confirms that export.
    #[allow(clippy::missing_errors_doc)]
    pub async fn request_account_purge_export(
        &self,
        actor_user_id: Uuid,
        subject_user_id: Uuid,
        reason: &str,
        request_id: &str,
    ) -> Result<AccountPurgeResponse, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        if actor_user_id == subject_user_id {
            return Err(PersistenceError::InvalidAccountTransition);
        }
        let mut tx = self.begin_admin().await?;
        let (role, status) = sqlx::query_as::<_, (String, String)>(
            "SELECT role::text, status::text FROM users WHERE id = $1 FOR UPDATE",
        )
        .bind(subject_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if role != UserRole::User.as_str() || !matches!(status.as_str(), "active" | "disabled") {
            return Err(PersistenceError::InvalidAccountTransition);
        }
        let has_active_project_purge = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM admin_job_metadata \
             WHERE owner_user_id = $1 AND kind = 'project_purge' AND status IN ('queued', 'running'))",
        )
        .bind(subject_user_id)
        .fetch_one(&mut *tx)
        .await?;
        if has_active_project_purge {
            return Err(PersistenceError::Conflict);
        }

        if let Some((export_id, job_id, export_status, job_status)) =
            sqlx::query_as::<_, (Uuid, Uuid, String, String)>(
                "SELECT e.id, e.job_id, e.status, j.status \
                 FROM admin_account_purge_export_metadata e \
                 JOIN admin_job_metadata j ON j.id = e.job_id \
                 WHERE e.user_id = $1 AND e.expires_at > CURRENT_TIMESTAMP \
                   AND e.status IN ('pending', 'ready') \
                 ORDER BY e.created_at DESC LIMIT 1",
            )
            .bind(subject_user_id)
            .fetch_optional(&mut *tx)
            .await?
        {
            let status = if export_status == "ready" {
                "ready".to_owned()
            } else {
                job_status
            };
            tx.commit().await?;
            return Ok(AccountPurgeResponse {
                job_id,
                export_id,
                status,
                confirmation_required: true,
            });
        }

        let job_id = Uuid::new_v4();
        let export_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO jobs (id, kind, owner_user_id, payload) \
             VALUES ($1, 'account_purge', $2, $3)",
        )
        .bind(job_id)
        .bind(subject_user_id)
        .bind(json!({
            "phase": "export",
            "exportId": export_id,
            "requestedBy": actor_user_id,
            "reason": reason,
        }))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO account_purge_exports \
             (id, user_id, job_id, object_key, expires_at) \
             VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP + INTERVAL '7 days')",
        )
        .bind(export_id)
        .bind(subject_user_id)
        .bind(job_id)
        .bind(format!(
            "users/{subject_user_id}/exports/{export_id}.tar.zst"
        ))
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            actor_user_id,
            Some(subject_user_id),
            "account.purge_export_requested",
            json!({"jobId": job_id, "exportId": export_id, "reason": reason}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(AccountPurgeResponse {
            job_id,
            export_id,
            status: "queued".to_owned(),
            confirmation_required: true,
        })
    }

    /// Confirms a completed export and transitions the account into irreversible deletion.
    #[allow(clippy::missing_errors_doc)]
    pub async fn confirm_account_purge(
        &self,
        actor_user_id: Uuid,
        subject_user_id: Uuid,
        export_id: Uuid,
        reason: &str,
        request_id: &str,
    ) -> Result<AccountPurgeResponse, PersistenceError> {
        self.verify_admin(actor_user_id).await?;
        if actor_user_id == subject_user_id {
            return Err(PersistenceError::InvalidAccountTransition);
        }
        let mut tx = self.begin_admin().await?;
        let (role, status) = sqlx::query_as::<_, (String, String)>(
            "SELECT role::text, status::text FROM users WHERE id = $1 FOR UPDATE",
        )
        .bind(subject_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::NotFound)?;
        if role != UserRole::User.as_str() || !matches!(status.as_str(), "active" | "disabled") {
            return Err(PersistenceError::InvalidAccountTransition);
        }
        let export = sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT e.job_id, e.status, j.status \
             FROM admin_account_purge_export_metadata e \
             JOIN admin_job_metadata j ON j.id = e.job_id \
             WHERE e.id = $1 AND e.user_id = $2 AND e.expires_at > CURRENT_TIMESTAMP",
        )
        .bind(export_id)
        .bind(subject_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PersistenceError::AccountPurgeNotReady)?;
        if export.1 != "ready" || export.2 != "succeeded" {
            return Err(PersistenceError::AccountPurgeNotReady);
        }
        let has_active_restore = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM admin_restore_job_metadata r \
             JOIN admin_project_metadata p ON p.id = r.project_id \
             WHERE p.owner_user_id = $1 AND r.status IN ('queued', 'running'))",
        )
        .bind(subject_user_id)
        .fetch_one(&mut *tx)
        .await?;
        if has_active_restore {
            return Err(PersistenceError::Conflict);
        }
        let has_active_project_purge = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM admin_job_metadata \
             WHERE owner_user_id = $1 AND kind = 'project_purge' AND status IN ('queued', 'running'))",
        )
        .bind(subject_user_id)
        .fetch_one(&mut *tx)
        .await?;
        if has_active_project_purge {
            return Err(PersistenceError::Conflict);
        }
        sqlx::query(
            "UPDATE users SET status = 'deleting' WHERE id = $1 AND status IN ('active', 'disabled')",
        )
        .bind(subject_user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE projects SET status = 'deleting', updated_at = CURRENT_TIMESTAMP \
             WHERE owner_user_id = $1 AND status IN ('active', 'disabled', 'maintenance')",
        )
        .bind(subject_user_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE account_purge_exports SET status = 'purging' WHERE id = $1")
            .bind(export_id)
            .execute(&mut *tx)
            .await?;
        let job_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO jobs (id, kind, owner_user_id, payload) \
             VALUES ($1, 'account_purge', $2, $3)",
        )
        .bind(job_id)
        .bind(subject_user_id)
        .bind(json!({
            "phase": "purge",
            "exportId": export_id,
            "requestedBy": actor_user_id,
            "reason": reason,
        }))
        .execute(&mut *tx)
        .await?;
        insert_audit(
            &mut tx,
            actor_user_id,
            Some(subject_user_id),
            "account.purge_requested",
            json!({"jobId": job_id, "exportId": export_id, "reason": reason}),
            request_id,
        )
        .await?;
        tx.commit().await?;
        Ok(AccountPurgeResponse {
            job_id,
            export_id,
            status: "queued".to_owned(),
            confirmation_required: false,
        })
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
        ObjectKind::Image => essence.starts_with("image/") || essence == "application/octet-stream",
    }
}

async fn store_push_failure(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    device_id: Uuid,
    project_id: Uuid,
    request_key: &str,
    request_hash: &str,
    error: &PersistenceError,
) -> Result<bool, PersistenceError> {
    let Some((status, response)) = push_failure_response(error) else {
        return Ok(false);
    };
    sqlx::query(
        "INSERT INTO idempotency_records \
         (owner_user_id, device_id, project_id, request_key, request_hash, response, response_status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(user_id)
    .bind(device_id)
    .bind(project_id)
    .bind(request_key)
    .bind(request_hash)
    .bind(response)
    .bind(status)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx_conflict)?;
    Ok(true)
}

fn push_failure_response(error: &PersistenceError) -> Option<(i16, serde_json::Value)> {
    match error {
        PersistenceError::ProjectMaintenance => Some((
            423,
            json!({
                "code": "PROJECT_MAINTENANCE",
                "message": "项目当前处于维护状态",
                "retryable": true
            }),
        )),
        PersistenceError::GenerationMismatch { expected, actual } => Some((
            409,
            json!({
                "code": "GENERATION_MISMATCH",
                "message": "项目 generation 已变化，请重新 bootstrap",
                "retryable": false,
                "details": {"expectedGeneration": expected, "actualGeneration": actual}
            }),
        )),
        PersistenceError::BootstrapRequired => Some((
            409,
            json!({
                "code": "CURSOR_INVALID",
                "message": "当前设备必须先完成 bootstrap",
                "retryable": false
            }),
        )),
        _ => None,
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
