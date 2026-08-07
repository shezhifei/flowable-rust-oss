use flowable_engine::engine::historical_migration::HistoricalMigrationRawDialect;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn unique_dump_path(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "flowable-historical-migration-raw-{test_name}-{}.sql",
        Uuid::new_v4()
    ))
}

fn unique_db_path(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "flowable-historical-migration-raw-target-{test_name}-{}.sqlite",
        Uuid::new_v4()
    ))
}

fn cleanup_file(path: &Path) {
    if path.exists() {
        fs::remove_file(path).unwrap();
    }
}

fn sample_bpmn_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?><definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples"><process id="historicalImportedProcess" isExecutable="true"><startEvent id="start"/><sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask"/><userTask id="approveTask" name="Approve Imported Request"/><sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end"/><endEvent id="end"/></process></definitions>"#
}

fn mysql_dump() -> String {
    format!(
        r#"
SET NAMES utf8mb4;
INSERT INTO `ACT_RE_DEPLOYMENT` (`ID_`, `NAME_`, `KEY_`, `ENGINE_VERSION_`, `DEPLOY_TIME_`) VALUES ('deployment-1', 'Imported Deployment', 'deployment-key', '8.0.0', 1710000000000);
INSERT INTO `ACT_GE_BYTEARRAY` (`ID_`, `NAME_`, `DEPLOYMENT_ID_`, `BYTES_`) VALUES ('resource-1', 'historicalImportedProcess.bpmn', 'deployment-1', '{bpmn}');
INSERT INTO `ACT_RE_PROCDEF` (`ID_`, `NAME_`, `KEY_`, `DESCRIPTION_`, `VERSION_`, `RESOURCE_NAME_`, `DEPLOYMENT_ID_`, `HAS_START_FORM_KEY_`, `HAS_GRAPHICAL_NOTATION_`, `SUSPENSION_STATE_`, `ENGINE_VERSION_`) VALUES ('historicalImportedProcess:1:source', 'Historical Imported Process', 'historicalImportedProcess', 'imported from historical source raw dump', 1, 'historicalImportedProcess.bpmn', 'deployment-1', 0, 0, 1, '8.0.0');
INSERT INTO `ACT_RU_EXECUTION` (`ID_`, `ROOT_PROC_INST_ID_`, `PROC_INST_ID_`, `PROC_DEF_ID_`, `ACT_ID_`, `IS_ACTIVE_`, `IS_CONCURRENT_`, `IS_SCOPE_`, `IS_MI_ROOT_`, `SUSPENSION_STATE_`, `NAME_`, `BUSINESS_KEY_`, `START_USER_ID_`, `START_TIME_`) VALUES ('proc-inst-1', 'proc-inst-1', 'proc-inst-1', 'historicalImportedProcess:1:source', 'approveTask', 1, 0, 1, 0, 1, 'Imported Root Execution', 'BUS-RAW-001', 'kermit', 1710000100000);
INSERT INTO `ACT_RU_TASK` (`ID_`, `PROC_INST_ID_`, `EXECUTION_ID_`, `TASK_DEF_KEY_`, `NAME_`, `CREATE_TIME_`) VALUES ('task-1', 'proc-inst-1', 'proc-inst-1', 'approveTask', 'Approve Imported Request', 1710000100500);
INSERT INTO `ACT_RU_VARIABLE` (`ID_`, `EXECUTION_ID_`, `PROC_INST_ID_`, `NAME_`, `TYPE_`, `TEXT_`, `LONG_`) VALUES
('var-1', 'proc-inst-1', 'proc-inst-1', 'customer', 'string', 'alice', NULL),
('var-2', 'proc-inst-1', 'proc-inst-1', 'approved', 'boolean', NULL, 0),
('var-3', 'proc-inst-1', 'proc-inst-1', 'unsupportedPayload', 'serializable', 'opaque', NULL);
INSERT INTO `ACT_HI_PROCINST` (`ID_`, `PROC_DEF_ID_`, `BUSINESS_KEY_`, `START_TIME_`, `END_TIME_`, `DURATION_`, `START_USER_ID_`) VALUES ('historic-proc-1', 'historicalImportedProcess:1:source', 'BUS-HIST-RAW-1', 1709999000000, 1709999100000, 100000, 'gonzo');
INSERT INTO `ACT_HI_VARINST` (`ID_`, `PROC_INST_ID_`, `EXECUTION_ID_`, `NAME_`, `VAR_TYPE_`, `TEXT_`, `CREATE_TIME_`, `LAST_UPDATED_TIME_`) VALUES ('hist-var-1', 'historic-proc-1', 'historic-proc-1', 'customer', 'string', 'imported-customer', 1709999000000, 1709999050000);
"#,
        bpmn = sample_bpmn_xml()
    )
}

fn postgres_dump() -> String {
    format!(
        r#"
BEGIN;
SET search_path TO public;
INSERT INTO public."ACT_RE_DEPLOYMENT" ("ID_", "NAME_", "KEY_", "ENGINE_VERSION_", "DEPLOY_TIME_") VALUES ('deployment-1', 'Imported Deployment', 'deployment-key', '8.0.0', 1710000000000::bigint);
INSERT INTO public."ACT_GE_BYTEARRAY" ("ID_", "NAME_", "DEPLOYMENT_ID_", "BYTES_") VALUES ('resource-1', 'historicalImportedProcess.bpmn', 'deployment-1', '{bpmn}'::text);
INSERT INTO public."ACT_RE_PROCDEF" ("ID_", "NAME_", "KEY_", "DESCRIPTION_", "VERSION_", "RESOURCE_NAME_", "DEPLOYMENT_ID_", "HAS_START_FORM_KEY_", "HAS_GRAPHICAL_NOTATION_", "SUSPENSION_STATE_", "ENGINE_VERSION_") VALUES ('historicalImportedProcess:1:source', 'Historical Imported Process', 'historicalImportedProcess', 'imported from postgres raw dump', 1, 'historicalImportedProcess.bpmn', 'deployment-1', FALSE, FALSE, 1, '8.0.0');
INSERT INTO public."ACT_RU_EXECUTION" ("ID_", "ROOT_PROC_INST_ID_", "PROC_INST_ID_", "PROC_DEF_ID_", "ACT_ID_", "IS_ACTIVE_", "IS_CONCURRENT_", "IS_SCOPE_", "IS_MI_ROOT_", "SUSPENSION_STATE_", "NAME_", "BUSINESS_KEY_", "START_USER_ID_", "START_TIME_") VALUES ('proc-inst-1', 'proc-inst-1', 'proc-inst-1', 'historicalImportedProcess:1:source', 'approveTask', TRUE, FALSE, TRUE, FALSE, 1, 'Imported Root Execution', 'BUS-RAW-002', 'kermit', 1710000100000::bigint);
INSERT INTO public."ACT_RU_TASK" ("ID_", "PROC_INST_ID_", "EXECUTION_ID_", "TASK_DEF_KEY_", "NAME_", "CREATE_TIME_") VALUES ('task-1', 'proc-inst-1', 'proc-inst-1', 'approveTask', 'Approve Imported Request', 1710000100500::bigint);
INSERT INTO public."ACT_RU_VARIABLE" ("ID_", "EXECUTION_ID_", "PROC_INST_ID_", "NAME_", "TYPE_", "TEXT_", "LONG_") VALUES ('var-1', 'proc-inst-1', 'proc-inst-1', 'customer', 'string', 'alice', NULL);
INSERT INTO public."ACT_RU_VARIABLE" ("ID_", "EXECUTION_ID_", "PROC_INST_ID_", "NAME_", "TYPE_", "LONG_") VALUES ('var-2', 'proc-inst-1', 'proc-inst-1', 'approved', 'boolean', 0::bigint);
INSERT INTO public."ACT_HI_PROCINST" ("ID_", "PROC_DEF_ID_", "BUSINESS_KEY_", "START_TIME_", "END_TIME_", "DURATION_", "START_USER_ID_") VALUES ('historic-proc-1', 'historicalImportedProcess:1:source', 'BUS-HIST-RAW-2', 1709999000000::bigint, 1709999100000::bigint, 100000::bigint, 'gonzo');
INSERT INTO public."ACT_HI_VARINST" ("ID_", "PROC_INST_ID_", "EXECUTION_ID_", "NAME_", "VAR_TYPE_", "TEXT_", "CREATE_TIME_", "LAST_UPDATED_TIME_") VALUES ('hist-var-1', 'historic-proc-1', 'historic-proc-1', 'customer', 'string', 'imported-customer', 1709999000000::bigint, 1709999050000::bigint);
COMMIT;
"#,
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
historicalImportedProcess:1:source\tHistorical Imported Process\thistoricalImportedProcess\timported from postgres copy dump\t1\thistoricalImportedProcess.bpmn\tdeployment-1\t0\t0\t1\t8.0.0\n\
\\.\n\
COPY public.\"ACT_RU_EXECUTION\" (\"ID_\", \"ROOT_PROC_INST_ID_\", \"PROC_INST_ID_\", \"PROC_DEF_ID_\", \"ACT_ID_\", \"IS_ACTIVE_\", \"IS_CONCURRENT_\", \"IS_SCOPE_\", \"IS_MI_ROOT_\", \"SUSPENSION_STATE_\", \"NAME_\", \"BUSINESS_KEY_\", \"START_USER_ID_\", \"START_TIME_\") FROM STDIN;\n\
proc-inst-1\tproc-inst-1\tproc-inst-1\thistoricalImportedProcess:1:source\tapproveTask\t1\t0\t1\t0\t1\tImported Root Execution\tBUS-RAW-COPY-1\tkermit\t1710000100000\n\
\\.\n\
COPY public.\"ACT_RU_TASK\" (\"ID_\", \"PROC_INST_ID_\", \"EXECUTION_ID_\", \"TASK_DEF_KEY_\", \"NAME_\", \"CREATE_TIME_\") FROM STDIN;\n\
task-1\tproc-inst-1\tproc-inst-1\tapproveTask\tApprove Imported Request\t1710000100500\n\
\\.\n\
COPY public.\"ACT_RU_VARIABLE\" (\"ID_\", \"EXECUTION_ID_\", \"PROC_INST_ID_\", \"NAME_\", \"TYPE_\", \"TEXT_\", \"LONG_\") FROM STDIN;\n\
var-1\tproc-inst-1\tproc-inst-1\tcustomer\tstring\talice\t\\N\n\
var-2\tproc-inst-1\tproc-inst-1\tapproved\tboolean\t\\N\t0\n\
\\.\n\
COPY public.\"ACT_HI_PROCINST\" (\"ID_\", \"PROC_DEF_ID_\", \"BUSINESS_KEY_\", \"START_TIME_\", \"END_TIME_\", \"DURATION_\", \"START_USER_ID_\") FROM STDIN;\n\
historic-proc-1\thistoricalImportedProcess:1:source\tBUS-HIST-COPY-1\t1709999000000\t1709999100000\t100000\tgonzo\n\
\\.\n\
COPY public.\"ACT_HI_VARINST\" (\"ID_\", \"PROC_INST_ID_\", \"EXECUTION_ID_\", \"NAME_\", \"VAR_TYPE_\", \"TEXT_\", \"CREATE_TIME_\", \"LAST_UPDATED_TIME_\") FROM STDIN;\n\
hist-var-1\thistoric-proc-1\thistoric-proc-1\tcustomer\tstring\timported-customer\t1709999000000\t1709999050000\n\
\\.\n\
COMMIT;\n",
        bpmn = sample_bpmn_xml()
    )
}

fn h2_dump() -> String {
    format!(
        r#"
-- H2 Raw Dump
INSERT INTO "ACT_RE_DEPLOYMENT" ("ID_", "NAME_", "KEY_", "ENGINE_VERSION_", "DEPLOY_TIME_") VALUES ('deployment-1', 'Imported Deployment', 'deployment-key', '8.0.0', 1710000000000);
INSERT INTO "ACT_GE_BYTEARRAY" ("ID_", "NAME_", "DEPLOYMENT_ID_", "BYTES_") VALUES ('resource-1', 'historicalImportedProcess.bpmn', 'deployment-1', '{bpmn}');
INSERT INTO "ACT_RE_PROCDEF" ("ID_", "NAME_", "KEY_", "DESCRIPTION_", "VERSION_", "RESOURCE_NAME_", "DEPLOYMENT_ID_", "HAS_START_FORM_KEY_", "HAS_GRAPHICAL_NOTATION_", "SUSPENSION_STATE_", "ENGINE_VERSION_") VALUES ('historicalImportedProcess:1:source', 'Historical Imported Process', 'historicalImportedProcess', 'imported from h2 raw dump', 1, 'historicalImportedProcess.bpmn', 'deployment-1', 0, 0, 1, '8.0.0');
INSERT INTO "ACT_RU_EXECUTION" ("ID_", "ROOT_PROC_INST_ID_", "PROC_INST_ID_", "PROC_DEF_ID_", "ACT_ID_", "IS_ACTIVE_", "IS_CONCURRENT_", "IS_SCOPE_", "IS_MI_ROOT_", "SUSPENSION_STATE_", "NAME_", "BUSINESS_KEY_", "START_USER_ID_", "START_TIME_") VALUES ('proc-inst-1', 'proc-inst-1', 'proc-inst-1', 'historicalImportedProcess:1:source', 'approveTask', 1, 0, 1, 0, 1, 'Imported Root Execution', 'BUS-RAW-003', 'kermit', 1710000100000);
INSERT INTO "ACT_RU_TASK" ("ID_", "PROC_INST_ID_", "EXECUTION_ID_", "TASK_DEF_KEY_", "NAME_", "CREATE_TIME_") VALUES ('task-1', 'proc-inst-1', 'proc-inst-1', 'approveTask', 'Approve Imported Request', 1710000100500);
INSERT INTO "ACT_RU_VARIABLE" ("ID_", "EXECUTION_ID_", "PROC_INST_ID_", "NAME_", "TYPE_", "TEXT_", "LONG_") VALUES ('var-1', 'proc-inst-1', 'proc-inst-1', 'customer', 'string', 'alice''s order', NULL);
INSERT INTO "ACT_RU_VARIABLE" ("ID_", "EXECUTION_ID_", "PROC_INST_ID_", "NAME_", "TYPE_", "TEXT_", "LONG_") VALUES ('var-2', 'proc-inst-1', 'proc-inst-1', 'approved', 'boolean', NULL, 0);
INSERT INTO "ACT_HI_PROCINST" ("ID_", "PROC_DEF_ID_", "BUSINESS_KEY_", "START_TIME_", "END_TIME_", "DURATION_", "START_USER_ID_") VALUES ('historic-proc-1', 'historicalImportedProcess:1:source', 'BUS-HIST-RAW-3', 1709999000000, 1709999100000, 100000, 'gonzo');
INSERT INTO "ACT_HI_VARINST" ("ID_", "PROC_INST_ID_", "EXECUTION_ID_", "NAME_", "VAR_TYPE_", "TEXT_", "CREATE_TIME_", "LAST_UPDATED_TIME_") VALUES ('hist-var-1', 'historic-proc-1', 'historic-proc-1', 'customer', 'string', 'imported''customer', 1709999000000, 1709999050000);
"#,
        bpmn = sample_bpmn_xml()
    )
}

#[test]
fn inspects_mysql_raw_dump_and_reports_bounded_normalization() {
    let dump_path = unique_dump_path("inspect-mysql");
    fs::write(&dump_path, mysql_dump()).unwrap();

    let report = ProcessEngine::inspect_historical_migration_sql_dump(
        &dump_path,
        HistoricalMigrationRawDialect::Mysql,
    )
    .unwrap();

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
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("temporary SQLite bridge"))
    );

    cleanup_file(&dump_path);
}

#[test]
fn imports_postgres_raw_dump_into_fresh_engine_baseline() {
    let dump_path = unique_dump_path("import-postgres");
    let target_db = unique_db_path("import-postgres");
    fs::write(&dump_path, postgres_dump()).unwrap();

    {
        let engine = ProcessEngine::new_with_db_path(
            "historical_migration_raw_postgres_import".to_string(),
            target_db.to_str().unwrap(),
        );
        let result = engine
            .import_historical_migration_from_sql_dump(
                &dump_path,
                HistoricalMigrationRawDialect::Postgres,
            )
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
                .any(|warning| warning.contains("temporary SQLite bridge"))
        );

        let tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id("proc-inst-1".to_string())
            .expect("tasks should list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "Approve Imported Request");

        let historic_variables = engine
            .get_history_service()
            .create_historic_variable_instance_query()
            .list()
            .expect("historic variable query should succeed");
        assert_eq!(historic_variables.len(), 1);
        assert_eq!(historic_variables[0].name, "customer");
    }

    cleanup_file(&dump_path);
    cleanup_file(&target_db);
}

#[test]
fn imports_copy_based_postgres_raw_dump_into_fresh_engine_baseline() {
    let dump_path = unique_dump_path("import-postgres-copy");
    let target_db = unique_db_path("import-postgres-copy");
    fs::write(&dump_path, postgres_copy_dump()).unwrap();

    {
        let engine = ProcessEngine::new_with_db_path(
            "historical_migration_raw_postgres_copy_import".to_string(),
            target_db.to_str().unwrap(),
        );
        let result = engine
            .import_historical_migration_from_sql_dump(
                &dump_path,
                HistoricalMigrationRawDialect::Postgres,
            )
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
                .any(|warning| warning.contains("temporary SQLite bridge"))
        );

        let tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id("proc-inst-1".to_string())
            .expect("tasks should list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "Approve Imported Request");
    }

    cleanup_file(&dump_path);
    cleanup_file(&target_db);
}

#[test]
fn imports_h2_raw_dump_into_fresh_engine_baseline() {
    let dump_path = unique_dump_path("import-h2");
    let target_db = unique_db_path("import-h2");
    fs::write(&dump_path, h2_dump()).unwrap();

    {
        let engine = ProcessEngine::new_with_db_path(
            "historical_migration_raw_h2_import".to_string(),
            target_db.to_str().unwrap(),
        );
        let result = engine
            .import_historical_migration_from_sql_dump(
                &dump_path,
                HistoricalMigrationRawDialect::H2,
            )
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
                .any(|warning| warning.contains("temporary SQLite bridge"))
        );

        let tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id("proc-inst-1".to_string())
            .expect("tasks should list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "Approve Imported Request");

        // 校验 H2 单引号 '' 转义后的字符串是否能正确恢复
        let variables = engine
            .get_runtime_service()
            .get_variables("proc-inst-1".to_string())
            .expect("variables should list");
        assert_eq!(
            variables.get("customer").and_then(|v| v.as_str()),
            Some("alice's order")
        );
    }

    cleanup_file(&dump_path);
    cleanup_file(&target_db);
}
