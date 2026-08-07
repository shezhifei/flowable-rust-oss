use crate::error::CmmnError;
use flowable_persistence::config::{DatabaseConfig, DatabaseKind, SchemaMode};
use flowable_persistence::create_session_factory;
use flowable_persistence::db_session::DbSession;
use flowable_persistence::db_session_factory::DbSessionFactory;
use flowable_persistence::{DbParams, RenderedStatement};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

const CMMN_INDEXES: &str = "
    CREATE INDEX IF NOT EXISTS idx_cmmn_case_definitions_key_version
        ON ACT_CMMN_CASE_DEFINITION (CASE_KEY_, VERSION_ DESC);
    CREATE INDEX IF NOT EXISTS idx_cmmn_case_definitions_deployment
        ON ACT_CMMN_CASE_DEFINITION (DEPLOYMENT_ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_case_instances_key_state
        ON ACT_CMMN_CASE_INSTANCE (CASE_KEY_, STATE_, STARTED_AT_, ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_stage_instances_case_parent
        ON ACT_CMMN_STAGE_INSTANCE (CASE_INSTANCE_ID_, PARENT_STAGE_INSTANCE_ID_, STATE_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_stage_history_case
        ON ACT_CMMN_STAGE_HISTORY (CASE_INSTANCE_ID_, ACTIVATED_AT_, ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_human_tasks_case_state
        ON ACT_CMMN_HUMAN_TASK (CASE_INSTANCE_ID_, STATE_, ACTIVATED_AT_, ID_);
    CREATE INDEX IF NOT EXISTS idx_cmmn_human_tasks_key_state
        ON ACT_CMMN_HUMAN_TASK (CASE_KEY_, STATE_, ACTIVATED_AT_, ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_case_history_key_started
        ON ACT_CMMN_CASE_HISTORY (CASE_KEY_, STARTED_AT_, CASE_INSTANCE_ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_human_task_history_case
        ON ACT_CMMN_HUMAN_TASK_HISTORY (CASE_INSTANCE_ID_, ACTIVATED_AT_, TASK_ID_);
    CREATE INDEX IF NOT EXISTS idx_cmmn_human_task_history_key
        ON ACT_CMMN_HUMAN_TASK_HISTORY (CASE_KEY_, ACTIVATED_AT_, TASK_ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_milestone_history_case
        ON ACT_CMMN_MILESTONE_HISTORY (CASE_INSTANCE_ID_, TIME_, ID_);
    CREATE INDEX IF NOT EXISTS idx_cmmn_milestone_history_key
        ON ACT_CMMN_MILESTONE_HISTORY (CASE_KEY_, TIME_, ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_identity_links_scope
        ON ACT_CMMN_IDENTITY_LINK (SCOPE_TYPE_, SCOPE_ID_);
    CREATE INDEX IF NOT EXISTS idx_cmmn_identity_links_user
        ON ACT_CMMN_IDENTITY_LINK (USER_ID_);
    CREATE INDEX IF NOT EXISTS idx_cmmn_identity_links_group
        ON ACT_CMMN_IDENTITY_LINK (GROUP_ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_jobs_family_state_created
        ON ACT_CMMN_JOB (FAMILY_, STATE_, CREATED_AT_, ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_event_subscriptions_case
        ON ACT_CMMN_EVENT_SUBSCRIPTION (CASE_INSTANCE_ID_, CREATED_AT_, ID_);
    CREATE INDEX IF NOT EXISTS idx_cmmn_event_subscriptions_definition
        ON ACT_CMMN_EVENT_SUBSCRIPTION (CASE_DEFINITION_ID_, CREATED_AT_, ID_);
    CREATE INDEX IF NOT EXISTS idx_cmmn_event_subscriptions_event
        ON ACT_CMMN_EVENT_SUBSCRIPTION (EVENT_TYPE_, EVENT_NAME_, CREATED_AT_, ID_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_task_assoc_case_state
        ON ACT_CMMN_TASK_INSTANCE_ASSOCIATION (CASE_INSTANCE_ID_, STATE_, CREATED_AT_, ID_);
    CREATE INDEX IF NOT EXISTS idx_cmmn_task_assoc_child
        ON ACT_CMMN_TASK_INSTANCE_ASSOCIATION (CHILD_INSTANCE_ID_, STATE_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_plan_item_events_case_item_event
        ON ACT_CMMN_PLAN_ITEM_EVENT (CASE_INSTANCE_ID_, PLAN_ITEM_ID_, STANDARD_EVENT_);

    CREATE INDEX IF NOT EXISTS idx_cmmn_plan_item_instances_case_type_state
        ON ACT_CMMN_RU_PLAN_ITEM_INST (CASE_INST_ID_, ITEM_DEFINITION_TYPE_, STATE_);
    CREATE INDEX IF NOT EXISTS idx_cmmn_plan_item_instances_element
        ON ACT_CMMN_RU_PLAN_ITEM_INST (CASE_INST_ID_, ELEMENT_ID_);
";

#[derive(Clone)]
pub struct CmmnStore {
    session_factory: Arc<DbSessionFactory>,
}

impl CmmnStore {
    pub fn in_memory() -> Result<Self, CmmnError> {
        // Honor FLOWABLE_TEST_ENGINE_DATABASE_URL so full multi-backend matrices can
        // drive CmmnEngine::new_in_memory without rewriting every test constructor.
        if let Ok(url) = std::env::var("FLOWABLE_TEST_ENGINE_DATABASE_URL") {
            if url.starts_with("postgres://") || url.starts_with("postgresql://") {
                return Self::from_config(DatabaseConfig {
                    kind: DatabaseKind::Postgres,
                    url,
                    pool_size: 8,
                    schema_mode: SchemaMode::True,
                    table_prefix: None,
                    schema: None,
                    catalog: None,
                });
            }
            if url.starts_with("mysql://") {
                return Self::from_config(DatabaseConfig {
                    kind: DatabaseKind::Mysql,
                    url,
                    pool_size: 8,
                    schema_mode: SchemaMode::True,
                    table_prefix: None,
                    schema: None,
                    catalog: None,
                });
            }
        }
        let unique_id = Uuid::new_v4();
        let db_url = format!("file:cmmn_{}?mode=memory&cache=shared", unique_id);
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

    pub fn sqlite(path: impl AsRef<Path>) -> Result<Self, CmmnError> {
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

    pub fn from_config(config: DatabaseConfig) -> Result<Self, CmmnError> {
        let factory = create_session_factory(&config)
            .map_err(|e| CmmnError::storage(format!("Failed to create session factory: {e}")))?;

        if matches!(config.kind, DatabaseKind::Memory | DatabaseKind::Sqlite) {
            let mut session = factory
                .create_session()
                .map_err(|e| CmmnError::storage(format!("Failed to create session: {e}")))?;
            session
                .execute_raw(RenderedStatement::new(
                    CMMN_INDEXES.to_string(),
                    DbParams::new(),
                ))
                .map_err(|e| CmmnError::storage(format!("Failed to create indexes: {e}")))?;
        }

        Ok(Self {
            session_factory: Arc::new(factory),
        })
    }

    pub fn create_session(&self) -> Result<DbSession, CmmnError> {
        // A CMMN session corresponds to one command/transaction. Reset the
        // ambient command-scoped event set so `onEvent` trigger-mode sentries
        // only observe onParts satisfied within this command, matching Java's
        // per-command in-memory SentryPartInstance collection
        // (AbstractEvaluationCriteriaOperation.java:707-713).
        crate::runtime::reset_command_event_scope();
        self.session_factory
            .create_session()
            .map_err(|e| CmmnError::storage(format!("Failed to create session: {e}")))
    }
}
