//! Bounded live historical-migration inspect tests.
//!
//! SQLite live inspect always runs. Postgres cases skip gracefully when the
//! database is unreachable or the `postgres` feature is disabled (mirrors
//! postgres engine integration tests).

use flowable_engine::engine::process_engine::ProcessEngine;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn unique_path(prefix: &str, extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.{}", Uuid::new_v4(), extension))
}

fn cleanup_file(path: &Path) {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

fn create_minimal_historical_sqlite(path: &Path) {
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
            PARENT_TASK_ID_ TEXT,
            CREATE_TIME_ INTEGER
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
        INSERT INTO ACT_RE_DEPLOYMENT (ID_, NAME_, KEY_, ENGINE_VERSION_, DEPLOY_TIME_)
            VALUES ('d1', 'Live Deploy', 'live-key', '8.0.0', 1);
        INSERT INTO ACT_GE_BYTEARRAY (ID_, NAME_, DEPLOYMENT_ID_, BYTES_)
            VALUES ('b1', 'p.bpmn', 'd1', X'00');
        INSERT INTO ACT_RE_PROCDEF (ID_, NAME_, KEY_, VERSION_, RESOURCE_NAME_, DEPLOYMENT_ID_)
            VALUES ('p:1:src', 'Live Process', 'liveProcess', 1, 'p.bpmn', 'd1');
        INSERT INTO ACT_RU_EXECUTION (ID_, PROC_INST_ID_, PROC_DEF_ID_, ACT_ID_)
            VALUES ('pi-1', 'pi-1', 'p:1:src', 'task');
        INSERT INTO ACT_RU_TASK (ID_, PROC_INST_ID_, EXECUTION_ID_, TASK_DEF_KEY_, NAME_)
            VALUES ('t1', 'pi-1', 'pi-1', 'task', 'Task');
        INSERT INTO ACT_HI_PROCINST (ID_, PROC_DEF_ID_, BUSINESS_KEY_)
            VALUES ('h1', 'p:1:src', 'BK-1');
        ",
    )
    .unwrap();
}

#[test]
fn inspects_live_sqlite_url_and_reports_table_counts() {
    let db_path = unique_path("flowable-historical-live-sqlite", "sqlite");
    create_minimal_historical_sqlite(&db_path);

    let report =
        ProcessEngine::inspect_historical_migration_live_url(db_path.to_str().unwrap(), "sqlite")
            .expect("live sqlite inspect should succeed");

    assert_eq!(report.deployment_count, 1);
    assert_eq!(report.deployment_resource_count, 1);
    assert_eq!(report.process_definition_count, 1);
    assert_eq!(report.process_instance_count, 1);
    assert_eq!(report.task_count, 1);
    assert_eq!(report.historic_process_instance_count, 1);
    assert!(
        report
            .present_tables
            .iter()
            .any(|table| table == "ACT_RE_DEPLOYMENT")
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("live SQLite"))
    );

    cleanup_file(&db_path);
}

#[test]
fn inspects_live_sqlite_via_source_manifest() {
    let db_path = unique_path("flowable-historical-live-manifest", "sqlite");
    let manifest_path = unique_path("flowable-historical-live-manifest", "json");
    create_minimal_historical_sqlite(&db_path);

    let escaped = db_path.display().to_string().replace('\\', "\\\\");
    let manifest = format!(
        r#"{{
  "format": "historical-migration-source-manifest/v1",
  "source": {{
    "kind": "live-sqlx",
    "url": "{escaped}",
    "database_kind": "sqlite"
  }}
}}"#
    );
    fs::write(&manifest_path, manifest).unwrap();

    let report = ProcessEngine::inspect_historical_migration_source_manifest(&manifest_path)
        .expect("live-sqlx source manifest inspect should succeed");

    assert_eq!(report.deployment_count, 1);
    assert_eq!(report.process_definition_count, 1);
    assert_eq!(report.process_instance_count, 1);
    assert_eq!(report.task_count, 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("live-sqlx") || warning.contains("source manifest"))
    );

    cleanup_file(&db_path);
    cleanup_file(&manifest_path);
}

#[test]
fn rejects_unknown_live_database_kind() {
    let err = ProcessEngine::inspect_historical_migration_live_url(
        "postgres://localhost/flowable",
        "oracle",
    )
    .expect_err("unknown kind should fail");
    let message = err.to_string();
    assert!(
        message.contains("unsupported live historical migration database kind"),
        "unexpected error: {message}"
    );
}

#[test]
fn live_sqlx_import_via_manifest_imports_owned_tables() {
    let db_path = unique_path("flowable-historical-live-import", "sqlite");
    let manifest_path = unique_path("flowable-historical-live-import", "json");
    let target_db = unique_path("flowable-historical-live-import-target", "sqlite");
    create_minimal_historical_sqlite(&db_path);

    let escaped = db_path.display().to_string().replace('\\', "\\\\");
    fs::write(
        &manifest_path,
        format!(
            r#"{{
  "format": "historical-migration-source-manifest/v1",
  "source": {{
    "kind": "live-sqlx",
    "url": "{escaped}",
    "database_kind": "sqlite"
  }}
}}"#
        ),
    )
    .unwrap();

    let engine = ProcessEngine::new_with_db_path(
        "historical_migration_live_import".to_string(),
        target_db.to_str().unwrap(),
    );
    let result = engine
        .import_historical_migration_from_source_manifest(&manifest_path)
        .expect("live-sqlx import should import through the live extraction pipeline");
    assert_eq!(result.imported_deployments, 1);
    assert_eq!(result.imported_process_definitions, 1);
    assert_eq!(result.imported_process_instances, 1);
    assert_eq!(result.imported_tasks, 1);
    assert_eq!(result.imported_historic_process_instances, 1);

    cleanup_file(&db_path);
    cleanup_file(&manifest_path);
    cleanup_file(&target_db);
}

#[cfg(feature = "postgres")]
mod postgres_live {
    use super::*;
    use flowable_engine::engine::time_source::SystemTimeSource;
    use flowable_engine::service::config::{
        DatabaseConfiguration, EngineDatabaseKind, ProcessEngineConfiguration,
    };
    use std::sync::{Arc, Mutex, OnceLock};

    static PG_TEST_LOCK: Mutex<()> = Mutex::new(());
    static PG_AVAILABLE: OnceLock<bool> = OnceLock::new();

    fn postgres_url() -> String {
        std::env::var("FLOWABLE_TEST_POSTGRES_URL").unwrap_or_else(|_| {
            "postgres://postgres:postgres@localhost:5432/flowable_test".to_string()
        })
    }

    fn postgres_available() -> bool {
        *PG_AVAILABLE.get_or_init(|| {
            let config = ProcessEngineConfiguration {
                database: DatabaseConfiguration {
                    kind: EngineDatabaseKind::Postgres,
                    url: postgres_url(),
                    pool_size: 1,
                    busy_timeout_ms: 2000,
                    journal_mode: Default::default(),
                },
                ..Default::default()
            };
            match ProcessEngine::build_with_config(
                "pg-historical-live-probe".to_string(),
                Arc::new(SystemTimeSource),
                config,
            ) {
                Ok(_) => true,
                Err(err) => {
                    eprintln!(
                        "Skipping live postgres historical inspect test: database unreachable ({err}). \
                         Set FLOWABLE_TEST_POSTGRES_URL to a live instance to run it."
                    );
                    false
                }
            }
        })
    }

    #[test]
    fn inspects_live_postgres_url_when_available() {
        let _guard = PG_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !postgres_available() {
            return;
        }

        let report = match ProcessEngine::inspect_historical_migration_live_url(
            &postgres_url(),
            "postgres",
        ) {
            Ok(report) => report,
            Err(err) => {
                eprintln!(
                    "Skipping live postgres historical inspect: connection/inspect failed ({err})"
                );
                return;
            }
        };

        assert_eq!(report.source_path, postgres_url());
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("bounded") || warning.contains("live")),
            "expected bounded live warning, got {:?}",
            report.warnings
        );
        // Schema created by the availability probe should include ACT_RE_DEPLOYMENT.
        assert!(
            report
                .present_tables
                .iter()
                .any(|table| table.eq_ignore_ascii_case("ACT_RE_DEPLOYMENT")),
            "expected ACT_RE_DEPLOYMENT among {:?}",
            report.present_tables
        );
    }
}
