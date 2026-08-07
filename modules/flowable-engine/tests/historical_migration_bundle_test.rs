use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use uuid::Uuid;

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
            "imported from historical source bundle",
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
            "BUNDLE-001",
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
        "INSERT INTO ACT_HI_PROCINST (ID_, PROC_DEF_ID_, BUSINESS_KEY_, START_TIME_, END_TIME_, DURATION_, START_USER_ID_) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            "historic-proc-1",
            "historicalImportedProcess:1:source",
            "BUNDLE-HIST-1",
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

fn create_portable_bundle() -> (TempDir, PathBuf) {
    let bundle_dir = tempfile::tempdir().unwrap();
    let source_db = bundle_dir.path().join("historical-source.sqlite");
    create_historical_source_db(&source_db);
    let output = run_snapshot_tool(&[
        "export-historical-bundle",
        "--source-db",
        source_db.to_str().unwrap(),
        "--output-bundle",
        bundle_dir.path().to_str().unwrap(),
    ]);
    assert_success(&output);

    (bundle_dir, source_db)
}

fn unique_target_db_path(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "flowable-historical-source-bundle-target-{test_name}-{}.sqlite",
        Uuid::new_v4()
    ))
}

fn run_snapshot_tool(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_flowable_snapshot_tool"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_stdout_json(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_start = stdout.find('{').unwrap_or(0);

    serde_json::from_str(&stdout[json_start..]).unwrap_or_else(|error| {
        panic!(
            "failed to parse stdout as json: {}\nstdout={}\nstderr={}",
            error,
            stdout,
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn inspect_historical_migration_bundle_reads_manifested_source_without_direct_sqlite_argument() {
    let (bundle_dir, _source_db) = create_portable_bundle();

    let output = run_snapshot_tool(&[
        "inspect-historical-bundle",
        "--source-bundle",
        bundle_dir.path().to_str().unwrap(),
    ]);

    assert_success(&output);

    let report = parse_stdout_json(&output);
    assert_eq!(
        report["source_path"].as_str().unwrap(),
        bundle_dir.path().to_str().unwrap()
    );
    assert_eq!(report["deployment_count"], 1);
    assert_eq!(report["process_definition_count"], 1);
    assert_eq!(report["task_count"], 1);
    assert_eq!(report["historic_process_instance_count"], 1);
}

#[test]
fn export_historical_migration_bundle_emits_manifest_and_embedded_sqlite_source() {
    let (bundle_dir, source_db) = create_portable_bundle();

    let manifest_path = bundle_dir.path().join("manifest.json");
    assert!(manifest_path.exists());
    let manifest = serde_json::from_slice::<Value>(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(
        manifest["format"].as_str().unwrap(),
        "portable-historical-migration-bundle/v1"
    );
    assert_eq!(
        manifest["sqlite_source"].as_str().unwrap(),
        source_db.file_name().unwrap().to_str().unwrap()
    );
}

#[test]
fn import_historical_migration_bundle_populates_target_engine_from_bundle_source() {
    let (bundle_dir, _source_db) = create_portable_bundle();
    let target_db = unique_target_db_path("cli-import");

    let output = run_snapshot_tool(&[
        "import-historical-bundle",
        "--source-bundle",
        bundle_dir.path().to_str().unwrap(),
        "--target-db",
        target_db.to_str().unwrap(),
        "--engine-name",
        "historical_source_cli_import",
    ]);

    assert_success(&output);

    let report = parse_stdout_json(&output);
    assert_eq!(report["imported_deployments"], 1);
    assert_eq!(report["imported_process_instances"], 1);
    assert_eq!(report["imported_historic_variable_instances"], 1);

    {
        let engine = ProcessEngine::new_with_db_path(
            "historical_source_verify".to_string(),
            target_db.to_str().unwrap(),
        );
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

        let historic_variables = engine
            .get_history_service()
            .create_historic_variable_instance_query()
            .process_instance_id("historic-proc-1".to_string())
            .list()
            .unwrap();
        assert_eq!(historic_variables.len(), 1);
        assert_eq!(historic_variables[0].name, "customer");
    }

    if target_db.exists() {
        fs::remove_file(&target_db).unwrap();
    }
}
