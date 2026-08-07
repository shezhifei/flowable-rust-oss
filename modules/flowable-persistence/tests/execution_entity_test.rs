use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbSession, DbSessionFactory, ExecutionDataManager,
    ExecutionEntity, PersistenceError, PropertyStatementCatalog, SqlExecutor, SqliteDialect,
    SqlxExecutorFactory,
};
use std::sync::Arc;

fn create_test_session() -> Result<DbSession, PersistenceError> {
    let config = DatabaseConfig {
        kind: DatabaseKind::Sqlite,
        url: "sqlite::memory:".to_string(),
        pool_size: 1,
        schema_mode: flowable_persistence::SchemaMode::Create,
        table_prefix: None,
        schema: None,
        catalog: None,
    };

    let runtime = Arc::new(
        tokio::runtime::Runtime::new().map_err(|e| PersistenceError::Connection(e.to_string()))?,
    );
    let pool_factory = SqlxExecutorFactory::new(&config, runtime)?;

    let dialect = Box::new(SqliteDialect);
    let catalog = Arc::new(PropertyStatementCatalog::new(dialect));

    let factory = DbSessionFactory::new(
        config,
        catalog,
        move || -> Result<Box<dyn SqlExecutor>, PersistenceError> {
            pool_factory.create_executor()
        },
    );

    let mut session = factory.create_session()?;

    // Create execution table
    let create_sql = r#"
        CREATE TABLE IF NOT EXISTS ACT_RU_EXECUTION (
            ID_ TEXT PRIMARY KEY,
            REV_ INTEGER DEFAULT 1,
            PROC_INST_ID_ TEXT,
            BUSINESS_KEY_ TEXT,
            PARENT_ID_ TEXT,
            PROC_DEF_ID_ TEXT,
            SUPER_EXEC_ TEXT,
            ROOT_PROC_INST_ID_ TEXT,
            ACT_ID_ TEXT,
            IS_ACTIVE_ INTEGER,
            IS_CONCURRENT_ INTEGER,
            IS_SCOPE_ INTEGER,
            IS_EVENT_SCOPE_ INTEGER,
            IS_MI_ROOT_ INTEGER,
            SUSPENSION_STATE_ INTEGER DEFAULT 1,
            CACHED_ENT_STATE_ INTEGER,
            TENANT_ID_ TEXT DEFAULT '',
            NAME_ TEXT,
            START_ACT_ID_ TEXT,
            START_TIME_ INTEGER,
            START_USER_ID_ TEXT,
            LOCK_TIME_ INTEGER,
            IS_COUNT_ENABLED_ INTEGER,
            EVT_SUBSCR_COUNT_ INTEGER,
            TASK_COUNT_ INTEGER,
            JOB_COUNT_ INTEGER,
            TIMER_JOB_COUNT_ INTEGER,
            SUSP_JOB_COUNT_ INTEGER,
            DEADLETTER_JOB_COUNT_ INTEGER,
            VAR_COUNT_ INTEGER,
            ID_LINK_COUNT_ INTEGER
        )
    "#;

    let rendered = flowable_persistence::RenderedStatement::new(
        create_sql.to_string(),
        flowable_persistence::DbParams::new(),
    );
    session.execute_raw(rendered)?;
    session.commit()?;

    Ok(session)
}

#[test]
fn test_execution_entity_insert_and_find() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    let manager = ExecutionDataManager::new();

    let mut execution = ExecutionEntity::new("exec-1".to_string());
    execution.process_instance_id = Some("proc-inst-1".to_string());
    execution.business_key = Some("business-key-1".to_string());
    execution.process_definition_id = Some("proc-def-1".to_string());
    execution.is_active = true;
    execution.is_concurrent = false;
    execution.is_scope = true;
    execution.suspension_state = 1;
    execution.tenant_id = Some("tenant-1".to_string());
    execution.name = Some("Test Execution".to_string());

    manager.insert(&mut session, execution.clone())?;
    session.commit()?;

    let found = manager.find_by_id(&mut session, "exec-1")?;
    assert!(found.is_some());

    let found = found.unwrap();
    assert_eq!(found.id, "exec-1");
    assert_eq!(found.process_instance_id, Some("proc-inst-1".to_string()));
    assert_eq!(found.business_key, Some("business-key-1".to_string()));
    assert_eq!(found.process_definition_id, Some("proc-def-1".to_string()));
    assert!(found.is_active);
    assert!(!found.is_concurrent);
    assert!(found.is_scope);
    assert_eq!(found.suspension_state, 1);
    assert_eq!(found.tenant_id, Some("tenant-1".to_string()));
    assert_eq!(found.name, Some("Test Execution".to_string()));

    Ok(())
}

#[test]
fn test_execution_entity_update() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    let manager = ExecutionDataManager::new();

    let mut execution = ExecutionEntity::new("exec-2".to_string());
    execution.is_active = true;
    execution.suspension_state = 1;

    manager.insert(&mut session, execution.clone())?;
    session.commit()?;

    let mut execution = manager.find_by_id(&mut session, "exec-2")?.unwrap();
    execution.is_active = false;
    execution.suspension_state = 2;
    execution.name = Some("Updated Execution".to_string());

    manager.update(&mut session, execution)?;
    session.commit()?;

    let found = manager.find_by_id(&mut session, "exec-2")?.unwrap();
    assert!(!found.is_active);
    assert_eq!(found.suspension_state, 2);
    assert_eq!(found.name, Some("Updated Execution".to_string()));

    Ok(())
}

#[test]
fn test_execution_entity_delete() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    let manager = ExecutionDataManager::new();

    let execution = ExecutionEntity::new("exec-3".to_string());
    manager.insert(&mut session, execution)?;
    session.commit()?;

    let found = manager.find_by_id(&mut session, "exec-3")?;
    assert!(found.is_some());

    let execution = found.unwrap();
    manager.delete(&mut session, &execution)?;
    session.commit()?;

    let found = manager.find_by_id(&mut session, "exec-3")?;
    assert!(found.is_none());

    Ok(())
}

#[test]
fn test_execution_entity_find_by_process_instance_id() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    let manager = ExecutionDataManager::new();

    let mut exec1 = ExecutionEntity::new("exec-4".to_string());
    exec1.process_instance_id = Some("proc-inst-2".to_string());

    let mut exec2 = ExecutionEntity::new("exec-5".to_string());
    exec2.process_instance_id = Some("proc-inst-2".to_string());

    let mut exec3 = ExecutionEntity::new("exec-6".to_string());
    exec3.process_instance_id = Some("proc-inst-3".to_string());

    manager.insert(&mut session, exec1)?;
    manager.insert(&mut session, exec2)?;
    manager.insert(&mut session, exec3)?;
    session.commit()?;

    let found = manager.find_by_process_instance_id(&mut session, "proc-inst-2")?;
    assert_eq!(found.len(), 2);

    let ids: Vec<String> = found.iter().map(|e| e.id.clone()).collect();
    assert!(ids.contains(&"exec-4".to_string()));
    assert!(ids.contains(&"exec-5".to_string()));

    Ok(())
}

#[test]
fn test_execution_entity_find_by_parent_execution_id() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    let manager = ExecutionDataManager::new();

    let mut parent = ExecutionEntity::new("exec-parent".to_string());
    parent.is_scope = true;

    let mut child1 = ExecutionEntity::new("exec-child-1".to_string());
    child1.parent_id = Some("exec-parent".to_string());
    child1.is_concurrent = true;

    let mut child2 = ExecutionEntity::new("exec-child-2".to_string());
    child2.parent_id = Some("exec-parent".to_string());
    child2.is_concurrent = true;

    manager.insert(&mut session, parent)?;
    manager.insert(&mut session, child1)?;
    manager.insert(&mut session, child2)?;
    session.commit()?;

    let found = manager.find_by_parent_execution_id(&mut session, "exec-parent")?;
    assert_eq!(found.len(), 2);

    let ids: Vec<String> = found.iter().map(|e| e.id.clone()).collect();
    assert!(ids.contains(&"exec-child-1".to_string()));
    assert!(ids.contains(&"exec-child-2".to_string()));

    Ok(())
}
