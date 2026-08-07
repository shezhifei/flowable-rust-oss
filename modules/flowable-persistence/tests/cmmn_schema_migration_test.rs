#[test]
fn cmmn_repository_metadata_has_a_versioned_migration_for_every_backend() {
    let scripts = flowable_persistence::get_all_scripts();

    for backend in ["sqlite", "postgres", "mysql"] {
        let migration = scripts.iter().find(|script| {
            script.database_type == backend
                && script.component == "ACT_CMMN_DEPLOYMENT"
                && script.version == "7.1.0"
        });
        let migration = migration.unwrap_or_else(|| {
            panic!("{backend} requires a new 7.1.0 CMMN repository metadata migration")
        });

        for required_sql in [
            "CATEGORY_",
            "KEY_",
            "PARENT_DEPLOYMENT_ID_",
            "DIAGRAM_RESOURCE_NAME_",
            "IDX_CMMN_DEPLOYMENT_KEY",
            "IDX_CMMN_CASE_DEFINITION_DEPLOYMENT",
            "IDX_CMMN_CASE_INSTANCE_DEFINITION",
            "IDX_CMMN_JOB_SCOPE",
        ] {
            assert!(
                migration.sql.contains(required_sql),
                "{backend} migration must contain {required_sql}"
            );
        }
    }
}

#[test]
fn cmmn_repository_metadata_upgrade_preserves_existing_rows_and_adds_indexes() {
    let mut executor = sqlite_executor();
    let mut previous = manager_through("7.0.0");
    previous
        .create_schema(executor.as_mut())
        .expect("7.0.0 schema");
    executor
        .execute(RenderedStatement::new(
            "INSERT INTO ACT_CMMN_DEPLOYMENT (ID_, NAME_, TENANT_ID_, DEPLOYED_AT_, DATA_) VALUES ('deployment', 'name', NULL, '2026-07-12T00:00:00Z', '{}')".to_string(),
            DbParams::new(),
        ))
        .expect("representative deployment");

    let mut current = manager_through("7.1.0");
    current
        .update_schema(executor.as_mut())
        .expect("7.1.0 upgrade");

    let deployment = executor
        .fetch_optional(RenderedStatement::new(
            "SELECT ID_, CATEGORY_, KEY_, PARENT_DEPLOYMENT_ID_ FROM ACT_CMMN_DEPLOYMENT WHERE ID_ = 'deployment'".to_string(),
            DbParams::new(),
        ))
        .expect("preserved deployment")
        .expect("deployment row");
    assert_eq!(deployment.get_text("ID_").as_deref(), Some("deployment"));
    assert_eq!(deployment.get_text("CATEGORY_"), None);
    assert_eq!(deployment.get_text("KEY_"), None);
    assert_eq!(deployment.get_text("PARENT_DEPLOYMENT_ID_"), None);

    let indexes = executor
        .fetch_all(RenderedStatement::new(
            "SELECT name FROM sqlite_master WHERE type = 'index'".to_string(),
            DbParams::new(),
        ))
        .expect("indexes");
    for name in [
        "IDX_CMMN_DEPLOYMENT_KEY",
        "IDX_CMMN_CASE_DEFINITION_KEY_TENANT_VERSION",
        "IDX_CMMN_CASE_HISTORY_DEFINITION_INSTANCE",
        "IDX_CMMN_IDENTITY_LINK_SCOPES",
        "IDX_CMMN_EVENT_SUBSCRIPTION_SCOPES",
    ] {
        assert!(
            indexes
                .iter()
                .any(|row| row.get_text("name").as_deref() == Some(name))
        );
    }
}
use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbParams, FlowableSchemaManager, RenderedStatement,
    SchemaManager, SchemaMode, SqlExecutor, SqlxExecutorFactory, get_all_scripts,
};
use std::sync::Arc;

fn sqlite_executor() -> Box<dyn SqlExecutor> {
    let config = DatabaseConfig {
        kind: DatabaseKind::Sqlite,
        url: "sqlite::memory:".to_string(),
        pool_size: 1,
        schema_mode: SchemaMode::Create,
        table_prefix: None,
        schema: None,
        catalog: None,
    };
    let factory = SqlxExecutorFactory::new(
        &config,
        Arc::new(tokio::runtime::Runtime::new().expect("runtime")),
    )
    .expect("factory");
    factory.create_executor().expect("executor")
}

fn manager_through(version: &str) -> FlowableSchemaManager {
    let mut manager = FlowableSchemaManager::new();
    for script in get_all_scripts()
        .into_iter()
        .filter(|script| script.version.as_str() <= version)
    {
        manager.add_script(script);
    }
    manager
}
