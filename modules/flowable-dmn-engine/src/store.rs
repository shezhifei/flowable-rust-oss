use crate::error::DmnError;
use flowable_persistence::config::{DatabaseConfig, DatabaseKind, SchemaMode};
use flowable_persistence::create_session_factory;
use flowable_persistence::db_session::DbSession;
use flowable_persistence::db_session_factory::DbSessionFactory;
use flowable_persistence::{DbParams, RenderedStatement};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

const DMN_INDEXES: &str = "
    CREATE INDEX IF NOT EXISTS idx_dmn_deployment_name
        ON ACT_DMN_DEPLOYMENT (NAME_);
    CREATE INDEX IF NOT EXISTS idx_dmn_deployment_name_category
        ON ACT_DMN_DEPLOYMENT (NAME_, CATEGORY_);
    CREATE INDEX IF NOT EXISTS idx_dmn_deployment_category
        ON ACT_DMN_DEPLOYMENT (CATEGORY_);
    CREATE INDEX IF NOT EXISTS idx_dmn_deployment_parent
        ON ACT_DMN_DEPLOYMENT (PARENT_DEPLOYMENT_ID_);
    CREATE INDEX IF NOT EXISTS idx_dmn_deployment_tenant
        ON ACT_DMN_DEPLOYMENT (TENANT_ID_);
    CREATE INDEX IF NOT EXISTS idx_dmn_deployment_name_tenant
        ON ACT_DMN_DEPLOYMENT (NAME_, TENANT_ID_);
    CREATE INDEX IF NOT EXISTS idx_dmn_deployment_parent_tenant
        ON ACT_DMN_DEPLOYMENT (PARENT_DEPLOYMENT_ID_, TENANT_ID_);

    CREATE INDEX IF NOT EXISTS idx_dmn_decision_key_version
        ON ACT_DMN_DECISION (DECISION_KEY_, VERSION_ DESC);
    CREATE INDEX IF NOT EXISTS idx_dmn_decision_key_tenant_version
        ON ACT_DMN_DECISION (DECISION_KEY_, TENANT_ID_, VERSION_ DESC);
    CREATE INDEX IF NOT EXISTS idx_dmn_decision_deployment
        ON ACT_DMN_DECISION (DEPLOYMENT_ID_);
    CREATE INDEX IF NOT EXISTS idx_dmn_decision_tenant
        ON ACT_DMN_DECISION (TENANT_ID_);
    CREATE INDEX IF NOT EXISTS idx_dmn_decision_deployment_tenant
        ON ACT_DMN_DECISION (DEPLOYMENT_ID_, TENANT_ID_);

    CREATE INDEX IF NOT EXISTS idx_dmn_drd_deployment
        ON ACT_DMN_DRD (DEPLOYMENT_ID_);

    CREATE INDEX IF NOT EXISTS idx_dmn_hi_exec_decision_key
        ON ACT_DMN_HI_EXECUTION (DECISION_KEY_, EXECUTED_AT_, EXECUTION_ID_);
    CREATE INDEX IF NOT EXISTS idx_dmn_hi_exec_definition
        ON ACT_DMN_HI_EXECUTION (DECISION_DEFINITION_ID_);
    CREATE INDEX IF NOT EXISTS idx_dmn_hi_exec_tenant
        ON ACT_DMN_HI_EXECUTION (TENANT_ID_);
";

#[derive(Clone)]
pub struct DmnStore {
    session_factory: Arc<DbSessionFactory>,
}

impl DmnStore {
    pub fn in_memory() -> Result<Self, DmnError> {
        let unique_id = Uuid::new_v4();
        let db_url = format!("file:dmn_{}?mode=memory&cache=shared", unique_id);
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

    pub fn sqlite(path: impl AsRef<Path>) -> Result<Self, DmnError> {
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

    pub fn from_config(config: DatabaseConfig) -> Result<Self, DmnError> {
        let factory = create_session_factory(&config)
            .map_err(|e| DmnError::storage(format!("Failed to create session factory: {}", e)))?;

        if matches!(config.kind, DatabaseKind::Memory | DatabaseKind::Sqlite) {
            let mut session = factory
                .create_session()
                .map_err(|e| DmnError::storage(format!("Failed to create session: {}", e)))?;
            session
                .execute_raw(RenderedStatement::new(
                    DMN_INDEXES.to_string(),
                    DbParams::new(),
                ))
                .map_err(|e| DmnError::storage(format!("Failed to create indexes: {}", e)))?;
        }

        Ok(Self {
            session_factory: Arc::new(factory),
        })
    }

    pub fn create_session(&self) -> Result<DbSession, DmnError> {
        self.session_factory
            .create_session()
            .map_err(|e| DmnError::storage(format!("Failed to create session: {}", e)))
    }
}
