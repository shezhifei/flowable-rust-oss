use crate::error::PersistenceError;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use std::str::FromStr;
use std::sync::Arc;
use tokio::runtime::Runtime;

pub struct PostgresAdapter {
    pool: PgPool,
    runtime: Arc<Runtime>,
}

impl PostgresAdapter {
    pub fn new(url: &str, pool_size: u32) -> Result<Self, PersistenceError> {
        let runtime =
            Arc::new(Runtime::new().map_err(|e| PersistenceError::Connection(e.to_string()))?);

        let pool = runtime.block_on(async {
            let options = PgConnectOptions::from_str(url)
                .map_err(|e| PersistenceError::Connection(e.to_string()))?;

            PgPoolOptions::new()
                .max_connections(pool_size)
                .connect_with(options)
                .await
                .map_err(|e| PersistenceError::Connection(e.to_string()))
        })?;

        Ok(Self { pool, runtime })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }
}
