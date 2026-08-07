use crate::config::{DatabaseConfig, SchemaMode};
use crate::db_session::DbSession;
use crate::error::PersistenceError;
use crate::executor::SqlExecutor;
use crate::schema::SchemaManager;
use crate::statement::StatementCatalog;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

static SCHEMA_INITIALIZATION_LOCKS: OnceLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    OnceLock::new();

fn schema_initialization_lock(database_key: &str) -> Arc<Mutex<()>> {
    let locks = SCHEMA_INITIALIZATION_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(lock) = locks.get(database_key).and_then(Weak::upgrade) {
        return lock;
    }

    locks.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(Mutex::new(()));
    locks.insert(database_key.to_string(), Arc::downgrade(&lock));
    lock
}

pub struct DbSessionFactory {
    config: DatabaseConfig,
    catalog: Arc<dyn StatementCatalog>,
    executor_factory: Arc<dyn Fn() -> Result<Box<dyn SqlExecutor>, PersistenceError> + Send + Sync>,
    schema_manager: Option<Arc<std::sync::Mutex<dyn SchemaManager>>>,
}

impl DbSessionFactory {
    pub fn new<F>(
        config: DatabaseConfig,
        catalog: Arc<dyn StatementCatalog>,
        executor_factory: F,
    ) -> Self
    where
        F: Fn() -> Result<Box<dyn SqlExecutor>, PersistenceError> + Send + Sync + 'static,
    {
        Self {
            config,
            catalog,
            executor_factory: Arc::new(executor_factory),
            schema_manager: None,
        }
    }

    pub fn with_schema_manager(mut self, manager: impl SchemaManager + 'static) -> Self {
        self.schema_manager = Some(Arc::new(std::sync::Mutex::new(manager)));
        self
    }

    pub fn config(&self) -> &DatabaseConfig {
        &self.config
    }

    pub fn create_session(&self) -> Result<DbSession, PersistenceError> {
        let executor = (self.executor_factory)()?;
        let catalog = Arc::clone(&self.catalog);
        Ok(DbSession::new(executor, catalog))
    }

    /// Runs schema management according to `DatabaseConfig.schema_mode`.
    ///
    /// - `False`      — no-op.
    /// - `True`       — create schema if missing, then run versioned migrations
    ///                  (`update_schema`). Aligns with Java `databaseSchemaUpdate=true`:
    ///                  older DBs are upgraded; a schema version newer than the code
    ///                  refuses to start.
    /// - `Create`     — create schema if not already present (idempotent; no upgrade).
    /// - `DropCreate` — drop then create schema.
    /// - `CreateDrop` — create schema if not present (caller must drop on shutdown).
    pub fn ensure_schema(&self) -> Result<(), PersistenceError> {
        let manager = match &self.schema_manager {
            Some(m) => m,
            None => return Ok(()),
        };

        let initialization_lock = schema_initialization_lock(&format!(
            "{}\0{}\0{}\0{}\0{}",
            self.config.kind,
            self.config.url,
            self.config.schema.as_deref().unwrap_or_default(),
            self.config.catalog.as_deref().unwrap_or_default(),
            self.config.table_prefix.as_deref().unwrap_or_default()
        ));
        let _initialization_guard = initialization_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut executor = (self.executor_factory)()?;
        let mut guard = manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        match self.config.schema_mode {
            SchemaMode::False => Ok(()),
            SchemaMode::True => {
                // create_schema is a no-op when ACT_GE_PROPERTY already exists;
                // update_schema then applies any scripts newer than the recorded version.
                guard.create_schema(executor.as_mut())?;
                guard.update_schema(executor.as_mut())?;
                executor.commit()
            }
            SchemaMode::Create => {
                guard.create_schema(executor.as_mut())?;
                executor.commit()
            }
            SchemaMode::DropCreate => {
                guard.drop_schema(executor.as_mut())?;
                guard.create_schema(executor.as_mut())?;
                executor.commit()
            }
            SchemaMode::CreateDrop => {
                guard.create_schema(executor.as_mut())?;
                executor.commit()
            }
        }
    }
}
