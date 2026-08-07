use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbSession, DbSessionFactory, ExecutionDataManager,
    ExecutionEntity, HistoryProcessInstanceDataManager, HistoryProcessInstanceEntity,
    HistoryTaskDataManager, HistoryTaskEntity, HistoryVariableDataManager, HistoryVariableEntity,
    JobDataManager, JobEntity, PersistenceError, PropertyStatementCatalog, SqlExecutor,
    SqliteDialect, SqlxExecutorFactory, TaskDataManager, TaskEntity, VariableDataManager,
    VariableEntity,
};
use std::sync::Arc;

fn create_test_session() -> Result<DbSession, PersistenceError> {
    create_test_session_with_name("memdb_bpmn_test")
}

fn create_test_session_with_name(db_name: &str) -> Result<DbSession, PersistenceError> {
    let db_url = format!("sqlite:file:{}?mode=memory&cache=shared", db_name);

    let config = DatabaseConfig {
        kind: DatabaseKind::Sqlite,
        url: db_url.to_string(),
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

    // Create all necessary tables
    let create_tables = vec![
        r#"
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
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS ACT_RU_TASK (
            ID_ TEXT PRIMARY KEY,
            REV_ INTEGER DEFAULT 1,
            EXECUTION_ID_ TEXT,
            PROC_INST_ID_ TEXT,
            PROC_DEF_ID_ TEXT,
            NAME_ TEXT,
            BUSINESS_KEY_ TEXT,
            PARENT_TASK_ID_ TEXT,
            DESCRIPTION_ TEXT,
            TASK_DEF_KEY_ TEXT,
            OWNER_ TEXT,
            ASSIGNEE_ TEXT,
            DELEGATION_ TEXT,
            PRIORITY_ INTEGER,
            CREATE_TIME_ INTEGER,
            DUE_DATE_ INTEGER,
            CATEGORY_ TEXT,
            SUSPENSION_STATE_ INTEGER DEFAULT 1,
            TENANT_ID_ TEXT DEFAULT '',
            FORM_KEY_ TEXT,
            CLAIM_TIME_ INTEGER,
            APP_VERSION_ INTEGER
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS ACT_RU_VARIABLE (
            ID_ TEXT PRIMARY KEY,
            REV_ INTEGER DEFAULT 1,
            TYPE_ TEXT,
            NAME_ TEXT,
            EXECUTION_ID_ TEXT,
            PROC_INST_ID_ TEXT,
            TASK_ID_ TEXT,
            SCOPE_TYPE_ TEXT,
            SCOPE_ID_ TEXT,
            SUB_SCOPE_ID_ TEXT,
            BYTEARRAY_ID_ TEXT,
            DOUBLE_ REAL,
            LONG_ INTEGER,
            TEXT_ TEXT,
            TEXT2_ TEXT,
            IS_INITIAL_ INTEGER
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS ACT_RU_JOB (
            ID_ TEXT PRIMARY KEY,
            REV_ INTEGER DEFAULT 1,
            TYPE_ TEXT,
            PROC_DEF_ID_ TEXT,
            PROC_INST_ID_ TEXT,
            EXECUTION_ID_ TEXT,
            NAME_ TEXT,
            SCOPE_TYPE_ TEXT,
            SCOPE_ID_ TEXT,
            SUB_SCOPE_ID_ TEXT,
            CREATE_TIME_ INTEGER,
            LOCK_OWNER_ TEXT,
            LOCK_TIME_ INTEGER,
            EXCLUSIVE_ INTEGER,
            EXECUTION_ TEXT,
            PROCESS_DEFINITION_ TEXT,
            RETRIES_ INTEGER,
            EXCEPTION_STACK_ID_ TEXT,
            EXCEPTION_MSG_ TEXT,
            DUEDATE_ INTEGER,
            REPEAT_ TEXT,
            HISTORY_URL_ TEXT,
            HANDLER_TYPE_ TEXT,
            CUSTOM_VALUES_ID_ TEXT
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS ACT_HI_PROCINST (
            ID_ TEXT PRIMARY KEY,
            REV_ INTEGER DEFAULT 1,
            PROC_DEF_ID_ TEXT,
            PROC_DEF_KEY_ TEXT,
            PROC_DEF_NAME_ TEXT,
            PROC_DEF_VERSION_ INTEGER,
            BUSINESS_KEY_ TEXT,
            START_TIME_ INTEGER,
            END_TIME_ INTEGER,
            DURATION_ INTEGER,
            START_USER_ID_ TEXT,
            START_ACT_ID_ TEXT,
            END_ACT_ID_ TEXT,
            SUPER_PROCESS_INSTANCE_ID_ TEXT,
            DELETE_REASON_ TEXT,
            TENANT_ID_ TEXT DEFAULT '',
            NAME_ TEXT,
            DESCRIPTION_ TEXT,
            CALLBACK_ID_ TEXT,
            CALLBACK_TYPE_ TEXT,
            REFERENCE_ID_ TEXT,
            REFERENCE_TYPE_ TEXT
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS ACT_HI_TASKINST (
            ID_ TEXT PRIMARY KEY,
            REV_ INTEGER DEFAULT 1,
            PROC_DEF_ID_ TEXT,
            PROC_INST_ID_ TEXT,
            EXECUTION_ID_ TEXT,
            NAME_ TEXT,
            PARENT_TASK_ID_ TEXT,
            DESCRIPTION_ TEXT,
            OWNER_ TEXT,
            ASSIGNEE_ TEXT,
            START_TIME_ INTEGER,
            CLAIM_TIME_ INTEGER,
            END_TIME_ INTEGER,
            DURATION_ INTEGER,
            DELETE_REASON_ TEXT,
            PRIORITY_ INTEGER,
            DUE_DATE_ INTEGER,
            TASK_DEF_KEY_ TEXT,
            CATEGORY_ TEXT,
            FORM_KEY_ TEXT,
            TENANT_ID_ TEXT DEFAULT '',
            APP_VERSION_ INTEGER
        )
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS ACT_HI_VARINST (
            ID_ TEXT PRIMARY KEY,
            REV_ INTEGER DEFAULT 1,
            PROC_INST_ID_ TEXT,
            EXECUTION_ID_ TEXT,
            TASK_ID_ TEXT,
            CREATE_TIME_ INTEGER,
            LAST_UPDATED_TIME_ INTEGER,
            NAME_ TEXT,
            VAR_TYPE_ TEXT,
            SCOPE_TYPE_ TEXT,
            SCOPE_ID_ TEXT,
            SUB_SCOPE_ID_ TEXT,
            BYTEARRAY_ID_ TEXT,
            DOUBLE_ REAL,
            LONG_ INTEGER,
            TEXT_ TEXT,
            TEXT2_ TEXT
        )
        "#,
    ];

    for create_sql in create_tables {
        let rendered = flowable_persistence::RenderedStatement::new(
            create_sql.to_string(),
            flowable_persistence::DbParams::new(),
        );
        session.execute_raw(rendered)?;
    }

    session.commit()?;
    Ok(session)
}

#[test]
fn test_bpmn_process_lifecycle() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;

    // 1. Create execution (process instance)
    let execution_manager = ExecutionDataManager::new();
    let mut process_instance = ExecutionEntity::new("proc-inst-1".to_string());
    process_instance.process_instance_id = Some("proc-inst-1".to_string());
    process_instance.process_definition_id = Some("proc-def-1".to_string());
    process_instance.is_active = true;
    process_instance.is_scope = true;
    process_instance.start_time = Some(1000);
    process_instance.start_user_id = Some("user1".to_string());

    execution_manager.insert(&mut session, process_instance)?;
    session.commit()?;

    // 2. Create task
    let task_manager = TaskDataManager::new();
    let mut task = TaskEntity::new("task-1".to_string());
    task.execution_id = Some("proc-inst-1".to_string());
    task.process_instance_id = Some("proc-inst-1".to_string());
    task.name = Some("Review Document".to_string());
    task.assignee = Some("user2".to_string());
    task.priority = 50;
    task.create_time = Some(1001);

    task_manager.insert(&mut session, task)?;
    session.commit()?;

    // 3. Create variable
    let variable_manager = VariableDataManager::new();
    let mut variable = VariableEntity::new("var-1".to_string());
    variable.execution_id = Some("proc-inst-1".to_string());
    variable.process_instance_id = Some("proc-inst-1".to_string());
    variable.name = Some("documentId".to_string());
    variable.variable_type = Some("string".to_string());
    variable.text_value = Some("doc-123".to_string());
    variable.is_initial = Some(true);

    variable_manager.insert(&mut session, variable)?;
    session.commit()?;

    // 4. Create job (async task)
    let job_manager = JobDataManager::new();
    let mut job = JobEntity::new("job-1".to_string());
    job.process_instance_id = Some("proc-inst-1".to_string());
    job.execution_id = Some("proc-inst-1".to_string());
    job.name = Some("async-task".to_string());
    job.job_type = Some("async".to_string());
    job.create_time = Some(1002);
    job.retries = Some(3);

    job_manager.insert(&mut session, job)?;
    session.commit()?;

    // 5. Complete task and create history
    let mut task = task_manager.find_by_id(&mut session, "task-1")?.unwrap();
    task.assignee = None; // Task completed

    let history_task_manager = HistoryTaskDataManager::new();
    let mut history_task = HistoryTaskEntity::new("task-1".to_string());
    history_task.process_instance_id = task.process_instance_id.clone();
    history_task.execution_id = task.execution_id.clone();
    history_task.name = task.name.clone();
    history_task.assignee = Some("user2".to_string());
    history_task.start_time = task.create_time;
    history_task.end_time = Some(2000);
    history_task.durationInMillis = Some(999);

    history_task_manager.insert(&mut session, history_task)?;
    task_manager.delete(&mut session, &task)?;
    session.commit()?;

    // 6. End process instance and create history
    let mut process_instance = execution_manager
        .find_by_id(&mut session, "proc-inst-1")?
        .unwrap();
    process_instance.is_active = false;

    let history_proc_manager = HistoryProcessInstanceDataManager::new();
    let mut history_proc = HistoryProcessInstanceEntity::new("proc-inst-1".to_string());
    history_proc.process_definition_id = process_instance.process_definition_id.clone();
    history_proc.business_key = process_instance.business_key.clone();
    history_proc.start_time = process_instance.start_time;
    history_proc.end_time = Some(2001);
    history_proc.durationInMillis = Some(1001);
    history_proc.start_user_id = process_instance.start_user_id.clone();

    history_proc_manager.insert(&mut session, history_proc)?;
    execution_manager.delete(&mut session, &process_instance)?;
    session.commit()?;

    // 7. Create history variable
    let history_var_manager = HistoryVariableDataManager::new();
    let mut history_var = HistoryVariableEntity::new("var-1".to_string());
    history_var.process_instance_id = Some("proc-inst-1".to_string());
    history_var.execution_id = Some("proc-inst-1".to_string());
    history_var.name = Some("documentId".to_string());
    history_var.variable_type = Some("string".to_string());
    history_var.text_value = Some("doc-123".to_string());
    history_var.create_time = Some(1001);
    history_var.last_updated_time = Some(2002);

    history_var_manager.insert(&mut session, history_var)?;
    let variable_to_delete = variable_manager.find_by_id(&mut session, "var-1")?.unwrap();
    variable_manager.delete(&mut session, &variable_to_delete)?;
    session.commit()?;

    // 8. Verify history records
    let history_proc = history_proc_manager
        .find_by_id(&mut session, "proc-inst-1")?
        .expect("History process instance should exist");
    assert_eq!(history_proc.start_time, Some(1000));
    assert_eq!(history_proc.end_time, Some(2001));
    assert_eq!(history_proc.durationInMillis, Some(1001));

    let history_task = history_task_manager
        .find_by_id(&mut session, "task-1")?
        .expect("History task should exist");
    assert_eq!(history_task.assignee, Some("user2".to_string()));
    assert_eq!(history_task.end_time, Some(2000));
    assert_eq!(history_task.durationInMillis, Some(999));

    let history_var = history_var_manager
        .find_by_id(&mut session, "var-1")?
        .expect("History variable should exist");
    assert_eq!(history_var.name, Some("documentId".to_string()));
    assert_eq!(history_var.text_value, Some("doc-123".to_string()));

    // 9. Verify runtime records are deleted
    assert!(
        execution_manager
            .find_by_id(&mut session, "proc-inst-1")?
            .is_none()
    );
    assert!(task_manager.find_by_id(&mut session, "task-1")?.is_none());
    assert!(
        variable_manager
            .find_by_id(&mut session, "var-1")?
            .is_none()
    );

    Ok(())
}

#[test]
fn test_concurrent_tasks_with_variables() -> Result<(), PersistenceError> {
    let mut session = create_test_session_with_name("memdb_concurrent_tasks")?;

    // Create process instance
    let execution_manager = ExecutionDataManager::new();
    let mut process_instance = ExecutionEntity::new("proc-inst-2".to_string());
    process_instance.process_instance_id = Some("proc-inst-2".to_string());
    process_instance.process_definition_id = Some("proc-def-2".to_string());
    process_instance.is_active = true;
    process_instance.is_scope = true;

    execution_manager.insert(&mut session, process_instance)?;
    session.commit()?;

    // Create multiple tasks
    let task_manager = TaskDataManager::new();
    for i in 1..=3 {
        let mut task = TaskEntity::new(format!("task-{}", i));
        task.execution_id = Some("proc-inst-2".to_string());
        task.process_instance_id = Some("proc-inst-2".to_string());
        task.name = Some(format!("Task {}", i));
        task.assignee = Some(format!("user{}", i));
        task.priority = 50 + i;
        task.create_time = Some(1000 + i as i64);

        task_manager.insert(&mut session, task)?;
    }
    session.commit()?;

    // Create variables for each task
    let variable_manager = VariableDataManager::new();
    for i in 1..=3 {
        let mut variable = VariableEntity::new(format!("var-{}", i));
        variable.execution_id = Some("proc-inst-2".to_string());
        variable.process_instance_id = Some("proc-inst-2".to_string());
        variable.task_id = Some(format!("task-{}", i));
        variable.name = Some(format!("taskVar{}", i));
        variable.variable_type = Some("integer".to_string());
        variable.long_value = Some(i as i64 * 10);
        variable.is_initial = Some(true);

        variable_manager.insert(&mut session, variable)?;
    }
    session.commit()?;

    // Verify all tasks and variables
    let tasks = task_manager.find_by_process_instance_id(&mut session, "proc-inst-2")?;
    assert_eq!(tasks.len(), 3);

    let variables = variable_manager.find_by_execution_id(&mut session, "proc-inst-2")?;
    assert_eq!(variables.len(), 3);

    // Verify task-specific variables
    for i in 1..=3 {
        let task_vars = variable_manager.find_by_task_id(&mut session, &format!("task-{}", i))?;
        assert_eq!(task_vars.len(), 1);
        assert_eq!(task_vars[0].long_value, Some(i as i64 * 10));
    }

    Ok(())
}

#[test]
fn test_job_retry_mechanism() -> Result<(), PersistenceError> {
    let mut session = create_test_session_with_name("memdb_job_retry")?;

    // Create a job with retries
    let job_manager = JobDataManager::new();
    let mut job = JobEntity::new("job-retry-1".to_string());
    job.process_instance_id = Some("proc-inst-3".to_string());
    job.name = Some("failing-job".to_string());
    job.job_type = Some("async".to_string());
    job.retries = Some(3);
    job.create_time = Some(1000);

    job_manager.insert(&mut session, job)?;
    session.commit()?;

    // Simulate job execution failure and retry
    for attempt in 1..=3 {
        let mut job = job_manager
            .find_by_id(&mut session, "job-retry-1")?
            .expect("Job should exist");

        assert_eq!(job.retries, Some(4 - attempt));

        // Simulate failure
        job.retries = job.retries.map(|r| r - 1);
        job.exception_msg = Some(format!("Failed on attempt {}", attempt));

        job_manager.update(&mut session, job)?;
        session.commit()?;
    }

    // Verify job has no retries left
    let job = job_manager
        .find_by_id(&mut session, "job-retry-1")?
        .expect("Job should still exist");
    assert_eq!(job.retries, Some(0));
    assert_eq!(job.exception_msg, Some("Failed on attempt 3".to_string()));

    Ok(())
}
