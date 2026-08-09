pub mod memory;
#[cfg(feature = "mysql")]
pub mod mysql;
#[cfg(feature = "postgres")]
pub mod postgres;
pub mod rusqlite_executor;
pub mod rusqlite_pool;
pub mod sqlite;
pub mod sqlx_executor;

use crate::config::DatabaseConfig;
use crate::config::DatabaseKind;
use crate::db_session_factory::DbSessionFactory;
use crate::error::PersistenceError;
use crate::schema::{FlowableSchemaManager, get_all_scripts};
use crate::statement::StatementCatalog;
use crate::statement_catalog::FlowableStatementCatalog;
use std::sync::Arc;

pub fn create_session_factory(
    config: &DatabaseConfig,
) -> Result<DbSessionFactory, PersistenceError> {
    let dialect = sqlx_executor::dialect_for(config.kind);
    let catalog: Arc<dyn StatementCatalog> = Arc::new(FlowableStatementCatalog::new(dialect));

    match config.kind {
        DatabaseKind::Memory | DatabaseKind::Sqlite => {
            rusqlite_pool::create_sqlite_session_factory(config, catalog)
        }
        DatabaseKind::Postgres | DatabaseKind::Mysql => {
            let runtime = sqlx_executor::shared_runtime()?;
            let sqlx_factory =
                sqlx_executor::SqlxExecutorFactory::new(config, Arc::clone(&runtime))?;
            let sqlx_factory = Arc::new(sqlx_factory);
            let factory_clone = Arc::clone(&sqlx_factory);
            let executor_factory = move || factory_clone.create_executor();

            let mut schema_manager = FlowableSchemaManager::new();
            for script in get_all_scripts() {
                schema_manager.add_script(script);
            }

            let factory = DbSessionFactory::new(config.clone(), catalog, executor_factory)
                .with_schema_manager(schema_manager);

            factory.ensure_schema()?;

            Ok(factory)
        }
    }
}
