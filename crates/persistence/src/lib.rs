use sqlx::PgPool;

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

    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// # Errors
    ///
    /// Returns the `SQLx` query error when the readiness query cannot complete.
    pub async fn is_ready(&self) -> Result<bool, sqlx::Error> {
        let value = sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(value == 1)
    }

    /// # Errors
    ///
    /// Returns the migration error when a migration cannot be applied.
    pub async fn migrate(&self) -> Result<(), sqlx::migrate::MigrateError> {
        sqlx::migrate!("../../migrations").run(&self.pool).await
    }
}
