use crate::error::PersistenceError;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions};
use std::str::FromStr;
use std::sync::Arc;
use tokio::runtime::Runtime;

pub struct MysqlAdapter {
    pool: MySqlPool,
    runtime: Arc<Runtime>,
}

impl MysqlAdapter {
    pub fn new(url: &str, pool_size: u32) -> Result<Self, PersistenceError> {
        let runtime =
            Arc::new(Runtime::new().map_err(|e| PersistenceError::Connection(e.to_string()))?);

        let pool = runtime.block_on(async {
            let options = MySqlConnectOptions::from_str(url)
                .map_err(|e| PersistenceError::Connection(e.to_string()))?;

            MySqlPoolOptions::new()
                .max_connections(pool_size)
                .connect_with(options)
                .await
                .map_err(|e| PersistenceError::Connection(e.to_string()))
        })?;

        Ok(Self { pool, runtime })
    }

    pub fn pool(&self) -> &MySqlPool {
        &self.pool
    }

    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }
}
