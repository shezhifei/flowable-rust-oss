use flowable_engine::engine::process_engine::ProcessEngine;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn unique_path(prefix: &str, extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.{}", Uuid::new_v4(), extension))
}

fn cleanup_file(path: &Path) {
    if path.exists() {
        fs::remove_file(path).unwrap();
    }
}

fn sample_bpmn_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?><definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples"><process id="historicalImportedProcess" isExecutable="true"><startEvent id="start"/><sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask"/><userTask id="approveTask" name="Approve Imported Request"/><sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end"/><endEvent id="end"/></process></definitions>"#
}

fn sqlite_dump() -> String {
    format!(
        "BEGIN TRANSACTION;\n\
CREATE TABLE ACT_RE_DEPLOYMENT (ID_ TEXT PRIMARY KEY, NAME_ TEXT, KEY_ TEXT, ENGINE_VERSION_ TEXT, DEPLOY_TIME_ INTEGER);\n\
CREATE TABLE ACT_GE_BYTEARRAY (ID_ TEXT PRIMARY KEY, NAME_ TEXT, DEPLOYMENT_ID_ TEXT, BYTES_ BLOB);\n\
CREATE TABLE ACT_RE_PROCDEF (ID_ TEXT PRIMARY KEY, NAME_ TEXT, KEY_ TEXT, DESCRIPTION_ TEXT, VERSION_ INTEGER, RESOURCE_NAME_ TEXT, DEPLOYMENT_ID_ TEXT, HAS_START_FORM_KEY_ INTEGER, HAS_GRAPHICAL_NOTATION_ INTEGER, SUSPENSION_STATE_ INTEGER, ENGINE_VERSION_ TEXT);\n\
CREATE TABLE ACT_RU_EXECUTION (ID_ TEXT PRIMARY KEY, ROOT_PROC_INST_ID_ TEXT, PROC_INST_ID_ TEXT, PROC_DEF_ID_ TEXT, ACT_ID_ TEXT, IS_ACTIVE_ INTEGER, IS_CONCURRENT_ INTEGER, IS_SCOPE_ INTEGER, IS_MI_ROOT_ INTEGER, SUSPENSION_STATE_ INTEGER, NAME_ TEXT, BUSINESS_KEY_ TEXT, START_USER_ID_ TEXT, START_TIME_ INTEGER);\n\
CREATE TABLE ACT_RU_TASK (ID_ TEXT PRIMARY KEY, PROC_INST_ID_ TEXT, EXECUTION_ID_ TEXT, TASK_DEF_KEY_ TEXT, NAME_ TEXT, CREATE_TIME_ INTEGER);\n\
CREATE TABLE ACT_HI_PROCINST (ID_ TEXT PRIMARY KEY, PROC_DEF_ID_ TEXT, BUSINESS_KEY_ TEXT, START_TIME_ INTEGER, END_TIME_ INTEGER, DURATION_ INTEGER, START_USER_ID_ TEXT);\n\
CREATE TABLE ACT_HI_VARINST (ID_ TEXT PRIMARY KEY, PROC_INST_ID_ TEXT, EXECUTION_ID_ TEXT, NAME_ TEXT, VAR_TYPE_ TEXT, TEXT_ TEXT, CREATE_TIME_ INTEGER, LAST_UPDATED_TIME_ INTEGER);\n\
INSERT INTO ACT_RE_DEPLOYMENT (ID_, NAME_, KEY_, ENGINE_VERSION_, DEPLOY_TIME_) VALUES ('deployment-1', 'Imported Deployment', 'deployment-key', '8.0.0', 1710000000000);\n\
INSERT INTO ACT_GE_BYTEARRAY (ID_, NAME_, DEPLOYMENT_ID_, BYTES_) VALUES ('resource-1', 'historicalImportedProcess.bpmn', 'deployment-1', '{bpmn}');\n\
INSERT INTO ACT_RE_PROCDEF (ID_, NAME_, KEY_, DESCRIPTION_, VERSION_, RESOURCE_NAME_, DEPLOYMENT_ID_, HAS_START_FORM_KEY_, HAS_GRAPHICAL_NOTATION_, SUSPENSION_STATE_, ENGINE_VERSION_) VALUES ('historicalImportedProcess:1:source', 'Historical Imported Process', 'historicalImportedProcess', 'imported from sqlite dump', 1, 'historicalImportedProcess.bpmn', 'deployment-1', 0, 0, 1, '8.0.0');\n\
INSERT INTO ACT_RU_EXECUTION (ID_, ROOT_PROC_INST_ID_, PROC_INST_ID_, PROC_DEF_ID_, ACT_ID_, IS_ACTIVE_, IS_CONCURRENT_, IS_SCOPE_, IS_MI_ROOT_, SUSPENSION_STATE_, NAME_, BUSINESS_KEY_, START_USER_ID_, START_TIME_) VALUES ('proc-inst-1', 'proc-inst-1', 'proc-inst-1', 'historicalImportedProcess:1:source', 'approveTask', 1, 0, 1, 0, 1, 'Imported Root Execution', 'BUS-SQLITE-DUMP-1', 'kermit', 1710000100000);\n\
INSERT INTO ACT_RU_TASK (ID_, PROC_INST_ID_, EXECUTION_ID_, TASK_DEF_KEY_, NAME_, CREATE_TIME_) VALUES ('task-1', 'proc-inst-1', 'proc-inst-1', 'approveTask', 'Approve Imported Request', 1710000100500);\n\
INSERT INTO ACT_HI_PROCINST (ID_, PROC_DEF_ID_, BUSINESS_KEY_, START_TIME_, END_TIME_, DURATION_, START_USER_ID_) VALUES ('historic-proc-1', 'historicalImportedProcess:1:source', 'BUS-HIST-SQLITE-1', 1709999000000, 1709999100000, 100000, 'gonzo');\n\
INSERT INTO ACT_HI_VARINST (ID_, PROC_INST_ID_, EXECUTION_ID_, NAME_, VAR_TYPE_, TEXT_, CREATE_TIME_, LAST_UPDATED_TIME_) VALUES ('hist-var-1', 'historic-proc-1', 'historic-proc-1', 'customer', 'string', 'imported-customer', 1709999000000, 1709999050000);\n\
COMMIT;\n",
        bpmn = sample_bpmn_xml()
    )
}

fn postgres_copy_dump() -> String {
    format!(
        "BEGIN;\n\
COPY public.\"ACT_RE_DEPLOYMENT\" (\"ID_\", \"NAME_\", \"KEY_\", \"ENGINE_VERSION_\", \"DEPLOY_TIME_\") FROM STDIN;\n\
deployment-1\tImported Deployment\tdeployment-key\t8.0.0\t1710000000000\n\
\\.\n\
COPY public.\"ACT_GE_BYTEARRAY\" (\"ID_\", \"NAME_\", \"DEPLOYMENT_ID_\", \"BYTES_\") FROM STDIN;\n\
resource-1\thistoricalImportedProcess.bpmn\tdeployment-1\t{bpmn}\n\
\\.\n\
COPY public.\"ACT_RE_PROCDEF\" (\"ID_\", \"NAME_\", \"KEY_\", \"DESCRIPTION_\", \"VERSION_\", \"RESOURCE_NAME_\", \"DEPLOYMENT_ID_\", \"HAS_START_FORM_KEY_\", \"HAS_GRAPHICAL_NOTATION_\", \"SUSPENSION_STATE_\", \"ENGINE_VERSION_\") FROM STDIN;\n\
historicalImportedProcess:1:source\tHistorical Imported Process\thistoricalImportedProcess\timported from postgres copy manifest\t1\thistoricalImportedProcess.bpmn\tdeployment-1\t0\t0\t1\t8.0.0\n\
\\.\n\
COPY public.\"ACT_RU_EXECUTION\" (\"ID_\", \"ROOT_PROC_INST_ID_\", \"PROC_INST_ID_\", \"PROC_DEF_ID_\", \"ACT_ID_\", \"IS_ACTIVE_\", \"IS_CONCURRENT_\", \"IS_SCOPE_\", \"IS_MI_ROOT_\", \"SUSPENSION_STATE_\", \"NAME_\", \"BUSINESS_KEY_\", \"START_USER_ID_\", \"START_TIME_\") FROM STDIN;\n\
proc-inst-1\tproc-inst-1\tproc-inst-1\thistoricalImportedProcess:1:source\tapproveTask\t1\t0\t1\t0\t1\tImported Root Execution\tBUS-COPY-MANIFEST-1\tkermit\t1710000100000\n\
\\.\n\
COPY public.\"ACT_RU_TASK\" (\"ID_\", \"PROC_INST_ID_\", \"EXECUTION_ID_\", \"TASK_DEF_KEY_\", \"NAME_\", \"CREATE_TIME_\") FROM STDIN;\n\
task-1\tproc-inst-1\tproc-inst-1\tapproveTask\tApprove Imported Request\t1710000100500\n\
\\.\n\
COPY public.\"ACT_HI_PROCINST\" (\"ID_\", \"PROC_DEF_ID_\", \"BUSINESS_KEY_\", \"START_TIME_\", \"END_TIME_\", \"DURATION_\", \"START_USER_ID_\") FROM STDIN;\n\
historic-proc-1\thistoricalImportedProcess:1:source\tBUS-HIST-COPY-MANIFEST-1\t1709999000000\t1709999100000\t100000\tgonzo\n\
\\.\n\
COPY public.\"ACT_HI_VARINST\" (\"ID_\", \"PROC_INST_ID_\", \"EXECUTION_ID_\", \"NAME_\", \"VAR_TYPE_\", \"TEXT_\", \"CREATE_TIME_\", \"LAST_UPDATED_TIME_\") FROM STDIN;\n\
hist-var-1\thistoric-proc-1\thistoric-proc-1\tcustomer\tstring\timported-customer\t1709999000000\t1709999050000\n\
\\.\n\
COMMIT;\n",
        bpmn = sample_bpmn_xml()
    )
}

fn write_source_manifest(path: &Path, source_json: &str) {
    let manifest = format!(
        "{{\n  \"format\": \"historical-migration-source-manifest/v1\",\n  \"source\": {source}\n}}\n",
        source = source_json
    );
    fs::write(path, manifest).unwrap();
}

#[test]
fn inspects_sqlite_dump_via_source_manifest() {
    let dump_path = unique_path("flowable-historical-migration-source-sqlite", "sql");
    let manifest_path = unique_path("flowable-historical-migration-source-sqlite", "json");
    fs::write(&dump_path, sqlite_dump()).unwrap();
    write_source_manifest(
        &manifest_path,
        &format!(
            "{{\"kind\":\"sqlite-dump\",\"path\":\"{}\"}}",
            dump_path.display().to_string().replace('\\', "\\\\")
        ),
    );

    let report =
        ProcessEngine::inspect_historical_migration_source_manifest(&manifest_path).unwrap();

    assert_eq!(report.deployment_count, 1);
    assert_eq!(report.process_definition_count, 1);
    assert_eq!(report.process_instance_count, 1);
    assert_eq!(report.task_count, 1);
    assert_eq!(report.historic_process_instance_count, 1);
    assert_eq!(report.historic_variable_count, 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("source manifest"))
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("sqlite dump"))
    );

    cleanup_file(&dump_path);
    cleanup_file(&manifest_path);
}

#[test]
fn imports_postgres_copy_dump_via_source_manifest() {
    let dump_path = unique_path("flowable-historical-migration-source-copy", "sql");
    let manifest_path = unique_path("flowable-historical-migration-source-copy", "json");
    let target_db = unique_path("flowable-historical-source-copy-target", "sqlite");
    fs::write(&dump_path, postgres_copy_dump()).unwrap();
    write_source_manifest(
        &manifest_path,
        &format!(
            "{{\"kind\":\"postgres-copy-dump\",\"path\":\"{}\"}}",
            dump_path.display().to_string().replace('\\', "\\\\")
        ),
    );

    {
        let engine = ProcessEngine::new_with_db_path(
            "historical_migration_source_import".to_string(),
            target_db.to_str().unwrap(),
        );
        let result = engine
            .import_historical_migration_from_source_manifest(&manifest_path)
            .unwrap();

        assert_eq!(result.imported_deployments, 1);
        assert_eq!(result.imported_process_definitions, 1);
        assert_eq!(result.imported_process_instances, 1);
        assert_eq!(result.imported_tasks, 1);
        assert_eq!(result.imported_historic_process_instances, 1);
        assert_eq!(result.imported_historic_variable_instances, 1);
        assert!(
            result
                .report
                .warnings
                .iter()
                .any(|warning| warning.contains("source manifest"))
        );
        let tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id("proc-inst-1".to_string())
            .expect("tasks should list");
        assert_eq!(tasks.len(), 1);
    }

    cleanup_file(&dump_path);
    cleanup_file(&manifest_path);
    cleanup_file(&target_db);
}
