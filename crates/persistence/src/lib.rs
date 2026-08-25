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
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = 'instance_settings')",
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
}
