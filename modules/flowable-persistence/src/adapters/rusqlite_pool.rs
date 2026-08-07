use crate::adapters::rusqlite_executor::RusqliteExecutor;
use crate::config::{DatabaseConfig, DatabaseKind};
use crate::db_session_factory::DbSessionFactory;
use crate::error::PersistenceError;
use crate::schema::{FlowableSchemaManager, get_all_scripts};
use crate::statement::StatementCatalog;
use r2d2::ManageConnection;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum SqliteTarget {
    Memory(String),
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct SqliteConnectionManager {
    target: SqliteTarget,
}

impl SqliteConnectionManager {
    pub fn memory(uri: String) -> Self {
        Self {
            target: SqliteTarget::Memory(uri),
        }
    }

    pub fn file(path: PathBuf) -> Self {
        Self {
            target: SqliteTarget::File(path),
        }
    }
}

impl ManageConnection for SqliteConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> Result<Connection, Self::Error> {
        let conn = match &self.target {
            SqliteTarget::Memory(uri) => Connection::open(uri)?,
            SqliteTarget::File(path) => Connection::open(path)?,
        };

        match &self.target {
            SqliteTarget::Memory(_) => {
                conn.execute_batch(
                    "PRAGMA journal_mode=MEMORY; PRAGMA synchronous=OFF; PRAGMA busy_timeout=5000;",
                )?;
            }
            SqliteTarget::File(_) => {
                conn.execute_batch(
                    "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;",
                )?;
            }
        }

        Ok(conn)
    }

    fn is_valid(&self, conn: &mut Connection) -> Result<(), Self::Error> {
        conn.execute_batch("SELECT 1")?;
        Ok(())
    }

    fn has_broken(&self, _conn: &mut Connection) -> bool {
        false
    }
}

pub type SqlitePool = r2d2::Pool<SqliteConnectionManager>;
pub type SqlitePooledConnection = r2d2::PooledConnection<SqliteConnectionManager>;

pub fn create_sqlite_session_factory(
    config: &DatabaseConfig,
    catalog: Arc<dyn StatementCatalog>,
) -> Result<DbSessionFactory, PersistenceError> {
    let target = match config.kind {
        DatabaseKind::Memory => {
            let uri = if config.url.is_empty() || config.url == "memory" || config.url == ":memory:"
            {
                format!("file:flowable_{}?mode=memory&cache=shared", Uuid::new_v4())
            } else {
                config.url.clone()
            };
            SqliteTarget::Memory(uri)
        }
        DatabaseKind::Sqlite => {
            let path =
                if config.url.is_empty() || config.url == "memory" || config.url == ":memory:" {
                    PathBuf::from("flowable.db")
                } else {
                    PathBuf::from(&config.url)
                };
            SqliteTarget::File(path)
        }
        _ => {
            return Err(PersistenceError::Connection(
                "create_sqlite_session_factory only supports Memory or Sqlite kinds".to_string(),
            ));
        }
    };

    let manager = SqliteConnectionManager { target };
    let pool = r2d2::Pool::builder()
        .max_size(config.pool_size)
        .build(manager)?;

    let pool = Arc::new(pool);

    {
        let _warmup_conn = pool.get()?;
    }

    let pool_for_factory = Arc::clone(&pool);
    let executor_factory =
        move || -> Result<Box<dyn crate::executor::SqlExecutor>, PersistenceError> {
            let conn = pool_for_factory.get()?;
            let executor = RusqliteExecutor::new_pooled(conn)?;
            Ok(Box::new(executor))
        };

    let mut schema_manager = FlowableSchemaManager::new();
    for script in get_all_scripts() {
        schema_manager.add_script(script);
    }

    let factory = DbSessionFactory::new(config.clone(), catalog, executor_factory)
        .with_schema_manager(schema_manager);

    factory.ensure_schema()?;

    Ok(factory)
}
