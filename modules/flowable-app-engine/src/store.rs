use crate::error::AppError;
use flowable_persistence::config::{DatabaseConfig, DatabaseKind, SchemaMode};
use flowable_persistence::create_session_factory;
use flowable_persistence::db_session::DbSession;
use flowable_persistence::db_session_factory::DbSessionFactory;
use flowable_persistence::{DbParams, RenderedStatement};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

const APP_INDEXES: &str = "
    CREATE INDEX IF NOT EXISTS idx_app_deployment_name
        ON ACT_APP_DEPLOYMENT (NAME_);
    CREATE INDEX IF NOT EXISTS idx_app_deployment_category
        ON ACT_APP_DEPLOYMENT (CATEGORY_);
    CREATE INDEX IF NOT EXISTS idx_app_deployment_tenant
        ON ACT_APP_DEPLOYMENT (TENANT_ID_);
    CREATE INDEX IF NOT EXISTS idx_app_deployment_name_tenant
        ON ACT_APP_DEPLOYMENT (NAME_, TENANT_ID_);

    CREATE INDEX IF NOT EXISTS idx_app_definitions_key_version
        ON ACT_APP_DEFINITION (APP_KEY_, VERSION_ DESC);
    CREATE INDEX IF NOT EXISTS idx_app_definitions_key_tenant_version
        ON ACT_APP_DEFINITION (APP_KEY_, TENANT_ID_, VERSION_ DESC);
    CREATE INDEX IF NOT EXISTS idx_app_definitions_deployment
        ON ACT_APP_DEFINITION (DEPLOYMENT_ID_);
    CREATE INDEX IF NOT EXISTS idx_app_definitions_deployment_tenant
        ON ACT_APP_DEFINITION (DEPLOYMENT_ID_, TENANT_ID_);

    CREATE INDEX IF NOT EXISTS idx_app_compositions_definition
        ON ACT_APP_RESOLVED_COMPOSITION (APP_DEFINITION_ID_);
    CREATE INDEX IF NOT EXISTS idx_app_compositions_key
        ON ACT_APP_RESOLVED_COMPOSITION (APP_KEY_, APP_DEFINITION_ID_);
    CREATE INDEX IF NOT EXISTS idx_app_compositions_deployment
        ON ACT_APP_RESOLVED_COMPOSITION (DEPLOYMENT_ID_);

    CREATE INDEX IF NOT EXISTS idx_app_deployment_resources_deployment
        ON ACT_APP_DEPLOYMENT_RESOURCE (DEPLOYMENT_ID_);
";

#[derive(Clone)]
pub struct AppStore {
    session_factory: Arc<DbSessionFactory>,
}

impl AppStore {
    pub fn in_memory() -> Result<Self, AppError> {
        let unique_id = Uuid::new_v4();
        let db_url = format!("file:app_{}?mode=memory&cache=shared", unique_id);
        let config = DatabaseConfig {
            kind: DatabaseKind::Memory,
            url: db_url,
            pool_size: 1,
            schema_mode: SchemaMode::True,
            table_prefix: None,
            schema: None,
            catalog: None,
        };
        Self::from_config(config)
    }

    pub fn sqlite(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let config = DatabaseConfig {
            kind: DatabaseKind::Sqlite,
            url: path.as_ref().to_string_lossy().to_string(),
            pool_size: 1,
            schema_mode: SchemaMode::True,
            table_prefix: None,
            schema: None,
            catalog: None,
        };
        Self::from_config(config)
    }

    pub fn from_config(config: DatabaseConfig) -> Result<Self, AppError> {
        let factory = create_session_factory(&config)
            .map_err(|e| AppError::storage(format!("Failed to create session factory: {e}")))?;

        if matches!(config.kind, DatabaseKind::Memory | DatabaseKind::Sqlite) {
            let mut session = factory
                .create_session()
                .map_err(|e| AppError::storage(format!("Failed to create session: {e}")))?;
            session
                .execute_raw(RenderedStatement::new(
                    APP_INDEXES.to_string(),
                    DbParams::new(),
                ))
                .map_err(|e| AppError::storage(format!("Failed to create indexes: {e}")))?;
        }

        Ok(Self {
            session_factory: Arc::new(factory),
        })
    }

    pub fn create_session(&self) -> Result<DbSession, AppError> {
        self.session_factory
            .create_session()
            .map_err(|e| AppError::storage(format!("Failed to create session: {e}")))
    }
}
