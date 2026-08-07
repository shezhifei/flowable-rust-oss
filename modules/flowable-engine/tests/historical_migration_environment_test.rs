use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn unique_db_path(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "flowable-historical-migration-{test_name}-{}.sqlite",
        Uuid::new_v4()
    ))
}

fn cleanup_file(path: &Path) {
    if path.exists() {
        fs::remove_file(path).unwrap();
    }
}

fn create_historical_source_db(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE ACT_RE_DEPLOYMENT (
            ID_ TEXT PRIMARY KEY,
            NAME_ TEXT,
            CATEGORY_ TEXT,
            KEY_ TEXT,
            TENANT_ID_ TEXT,
            PARENT_DEPLOYMENT_ID_ TEXT,
            DERIVED_FROM_ TEXT,
            DERIVED_FROM_ROOT_ TEXT,
            ENGINE_VERSION_ TEXT,
            DEPLOY_TIME_ INTEGER
        );

        CREATE TABLE ACT_GE_BYTEARRAY (
            ID_ TEXT PRIMARY KEY,
            NAME_ TEXT,
            DEPLOYMENT_ID_ TEXT,
            BYTES_ BLOB
        );

        CREATE TABLE ACT_RE_PROCDEF (
            ID_ TEXT PRIMARY KEY,
            CATEGORY_ TEXT,
            NAME_ TEXT,
            KEY_ TEXT,
            DESCRIPTION_ TEXT,
            VERSION_ INTEGER,
            RESOURCE_NAME_ TEXT,
            DEPLOYMENT_ID_ TEXT,
            DGRM_RESOURCE_NAME_ TEXT,
            HAS_START_FORM_KEY_ INTEGER,
            HAS_GRAPHICAL_NOTATION_ INTEGER,
            SUSPENSION_STATE_ INTEGER,
            TENANT_ID_ TEXT,
            ENGINE_VERSION_ TEXT,
            APP_VERSION_ INTEGER
        );

        CREATE TABLE ACT_RU_EXECUTION (
            ID_ TEXT PRIMARY KEY,
            PARENT_ID_ TEXT,
            SUPER_EXEC_ TEXT,
            ROOT_PROC_INST_ID_ TEXT,
            PROC_INST_ID_ TEXT,
            PROC_DEF_ID_ TEXT,
            ACT_ID_ TEXT,
            IS_ACTIVE_ INTEGER,
            IS_CONCURRENT_ INTEGER,
            IS_SCOPE_ INTEGER,
            IS_MI_ROOT_ INTEGER,
            SUSPENSION_STATE_ INTEGER,
            TENANT_ID_ TEXT,
            NAME_ TEXT,
            BUSINESS_KEY_ TEXT,
            START_USER_ID_ TEXT,
            START_TIME_ INTEGER
        );

        CREATE TABLE ACT_RU_TASK (
            ID_ TEXT PRIMARY KEY,
            PROC_INST_ID_ TEXT,
            EXECUTION_ID_ TEXT,
            TASK_DEF_KEY_ TEXT,
            NAME_ TEXT,
            CREATE_TIME_ INTEGER
        );

        CREATE TABLE ACT_RU_VARIABLE (
            ID_ TEXT PRIMARY KEY,
            EXECUTION_ID_ TEXT,
            PROC_INST_ID_ TEXT,
            NAME_ TEXT,
            TYPE_ TEXT,
            TEXT_ TEXT,
            TEXT2_ TEXT,
            LONG_ INTEGER,
            DOUBLE_ REAL
        );

        CREATE TABLE ACT_HI_PROCINST (
            ID_ TEXT PRIMARY KEY,
            PROC_DEF_ID_ TEXT,
            BUSINESS_KEY_ TEXT,
            START_TIME_ INTEGER,
            END_TIME_ INTEGER,
            DURATION_ INTEGER,
            START_USER_ID_ TEXT,
            DELETE_REASON_ TEXT
        );

        CREATE TABLE ACT_HI_VARINST (
            ID_ TEXT PRIMARY KEY,
            PROC_INST_ID_ TEXT,
            EXECUTION_ID_ TEXT,
            TASK_ID_ TEXT,
            NAME_ TEXT,
            VAR_TYPE_ TEXT,
            TEXT_ TEXT,
            TEXT2_ TEXT,
            LONG_ INTEGER,
            DOUBLE_ REAL,
            CREATE_TIME_ INTEGER,
            LAST_UPDATED_TIME_ INTEGER
        );
        ",
    )
    .unwrap();

    let bpmn = r#"<?xml version="1.0" encoding="UTF-8"?>
        <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
          <process id="historicalImportedProcess" isExecutable="true">
            <startEvent id="start"/>
            <sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask"/>
            <userTask id="approveTask" name="Approve Imported Request"/>
            <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end"/>
            <endEvent id="end"/>
          </process>
        </definitions>"#;

    conn.execute(
        "INSERT INTO ACT_RE_DEPLOYMENT (ID_, NAME_, KEY_, ENGINE_VERSION_, DEPLOY_TIME_) VALUES (?1, ?2, ?3, ?4, ?5)",
        ("deployment-1", "Imported Deployment", "deployment-key", "8.0.0", 1_710_000_000_000_i64),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ACT_GE_BYTEARRAY (ID_, NAME_, DEPLOYMENT_ID_, BYTES_) VALUES (?1, ?2, ?3, ?4)",
        (
            "resource-1",
            "historicalImportedProcess.bpmn",
            "deployment-1",
            bpmn.as_bytes(),
        ),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ACT_RE_PROCDEF (ID_, NAME_, KEY_, DESCRIPTION_, VERSION_, RESOURCE_NAME_, DEPLOYMENT_ID_, HAS_START_FORM_KEY_, HAS_GRAPHICAL_NOTATION_, SUSPENSION_STATE_, ENGINE_VERSION_) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        (
            "historicalImportedProcess:1:source",
            "Historical Imported Process",
            "historicalImportedProcess",
            "imported from historical source db",
            1_i32,
            "historicalImportedProcess.bpmn",
            "deployment-1",
            0_i64,
            0_i64,
            1_i64,
            "8.0.0",
        ),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ACT_RU_EXECUTION (ID_, ROOT_PROC_INST_ID_, PROC_INST_ID_, PROC_DEF_ID_, ACT_ID_, IS_ACTIVE_, IS_CONCURRENT_, IS_SCOPE_, IS_MI_ROOT_, SUSPENSION_STATE_, NAME_, BUSINESS_KEY_, START_USER_ID_, START_TIME_) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        (
            "proc-inst-1",
            "proc-inst-1",
            "proc-inst-1",
            "historicalImportedProcess:1:source",
            "approveTask",
            1_i64,
            0_i64,
            1_i64,
            0_i64,
            1_i64,
            "Imported Root Execution",
            "BUS-001",
            "kermit",
            1_710_000_100_000_i64,
        ),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ACT_RU_TASK (ID_, PROC_INST_ID_, EXECUTION_ID_, TASK_DEF_KEY_, NAME_, CREATE_TIME_) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (
            "task-1",
            "proc-inst-1",
            "proc-inst-1",
            "approveTask",
            "Approve Imported Request",
            1_710_000_100_500_i64,
        ),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ACT_RU_VARIABLE (ID_, EXECUTION_ID_, PROC_INST_ID_, NAME_, TYPE_, TEXT_) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        ("var-1", "proc-inst-1", "proc-inst-1", "customer", "string", "alice"),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ACT_RU_VARIABLE (ID_, EXECUTION_ID_, PROC_INST_ID_, NAME_, TYPE_, LONG_) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        ("var-2", "proc-inst-1", "proc-inst-1", "approved", "boolean", 0_i64),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ACT_RU_VARIABLE (ID_, EXECUTION_ID_, PROC_INST_ID_, NAME_, TYPE_, TEXT_) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        ("var-3", "proc-inst-1", "proc-inst-1", "unsupportedPayload", "serializable", "opaque"),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ACT_HI_PROCINST (ID_, PROC_DEF_ID_, BUSINESS_KEY_, START_TIME_, END_TIME_, DURATION_, START_USER_ID_) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            "historic-proc-1",
            "historicalImportedProcess:1:source",
            "BUS-HIST-1",
            1_709_999_000_000_i64,
            1_709_999_100_000_i64,
            100_000_i64,
            "gonzo",
        ),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO ACT_HI_VARINST (ID_, PROC_INST_ID_, EXECUTION_ID_, NAME_, VAR_TYPE_, TEXT_, CREATE_TIME_, LAST_UPDATED_TIME_) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        (
            "hist-var-1",
            "historic-proc-1",
            "historic-proc-1",
            "customer",
            "string",
            "imported-customer",
            1_709_999_000_000_i64,
            1_709_999_050_000_i64,
        ),
    )
    .unwrap();
}

#[test]
fn inspects_historical_migration_sqlite_and_reports_supported_boundary() {
    let source_db = unique_db_path("inspect");
    create_historical_source_db(&source_db);

    let report = ProcessEngine::inspect_historical_migration_sqlite(&source_db).unwrap();

    assert_eq!(report.deployment_count, 1);
    assert_eq!(report.process_definition_count, 1);
    assert_eq!(report.process_instance_count, 1);
    assert_eq!(report.task_count, 1);
    assert_eq!(report.variable_count, 3);
    assert_eq!(report.historic_process_instance_count, 1);
    assert_eq!(report.historic_variable_count, 1);
    assert!(
        report
            .unsupported_variable_types
            .contains(&"serializable".to_string())
    );

    cleanup_file(&source_db);
}

#[test]
fn imports_historical_migration_sqlite_into_fresh_engine_baseline() {
    let source_db = unique_db_path("import-source");
    create_historical_source_db(&source_db);

    let engine = ProcessEngine::new("historical_migration_import_engine".to_string());
    let result = engine
        .import_historical_migration_from_sqlite(&source_db)
        .unwrap();

    assert_eq!(result.imported_deployments, 1);
    assert_eq!(result.imported_process_definitions, 1);
    assert_eq!(result.imported_process_instances, 1);
    assert_eq!(result.imported_executions, 1);
    assert_eq!(result.imported_tasks, 1);
    assert_eq!(result.imported_historic_process_instances, 1);
    assert_eq!(result.imported_historic_variable_instances, 1);

    let process_definition_ids = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap();
    assert_eq!(
        process_definition_ids,
        vec!["historicalImportedProcess:1:source".to_string()]
    );

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id("proc-inst-1".to_string())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "approveTask");

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .find_execution("proc-inst-1", &mut session)
        .expect("execution should be imported");
    assert_eq!(
        execution.variables.get("customer"),
        Some(&serde_json::Value::String("alice".to_string()))
    );
    assert_eq!(
        execution.variables.get("approved"),
        Some(&serde_json::Value::Bool(false))
    );
    assert!(!execution.variables.contains_key("unsupportedPayload"));
    drop(session);

    let historic_instances = engine
        .get_history_service()
        .create_historic_process_instance_query()
        .list()
        .unwrap();
    assert_eq!(historic_instances.len(), 1);
    assert_eq!(historic_instances[0].id, "historic-proc-1");

    let historic_variables = engine
        .get_history_service()
        .create_historic_variable_instance_query()
        .process_instance_id("historic-proc-1".to_string())
        .list()
        .unwrap();
    assert_eq!(historic_variables.len(), 1);
    assert_eq!(historic_variables[0].name, "customer");

    engine
        .get_task_service()
        .complete_task_by_id("task-1".to_string())
        .unwrap();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let process_instance = runtime_store
        .find_process_instance("proc-inst-1", &mut session)
        .unwrap();
    assert!(process_instance.is_ended);

    cleanup_file(&source_db);
}
