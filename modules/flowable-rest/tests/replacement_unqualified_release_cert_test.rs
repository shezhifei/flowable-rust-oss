use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use reqwest::{Client, StatusCode};
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tokio::{net::TcpListener, task::JoinHandle};

const ADMIN_USER_ID: &str = "admin";
const ADMIN_PASSWORD: &str = "m29-platform-secret";

fn normalize_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn write_directory_bundle(path: &Path) {
    std::fs::write(
        path,
        r#"
[[users]]
id = "ldap-platform-admin"
first_name = "Platform"
last_name = "Admin"
email = "platform-admin@example.test"

[[groups]]
id = "platform-admins"
name = "Platform Admins"
group_type = "security-role"

[[memberships]]
user_id = "ldap-platform-admin"
group_id = "platform-admins"
"#,
    )
    .expect("directory bundle should be written");
}

struct PlatformConfigSpec<'a> {
    auth_mode: &'a str,
    embedding_mode: &'a str,
    embedding_profile: &'a str,
    adapters: &'a [&'a str],
    operations_exposure: &'a str,
}

fn write_platform_config(
    config_path: &Path,
    process_database_path: &Path,
    directory_bundle_path: &Path,
    spec: PlatformConfigSpec<'_>,
) {
    let adapter_entries = spec
        .adapters
        .iter()
        .map(|adapter| format!("\"{adapter}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "m29-unqualified-release"
database_path = "{database_path}"

[security]
auth_mode = "{auth_mode}"

[embedding]
mode = "{embedding_mode}"
profile = "{embedding_profile}"

[enterprise]
adapters = [{adapter_entries}]

[directory]
provider = "ldap-mirror"
sync_on_bootstrap = true
bundle_path = "{bundle_path}"

[operations]
exposure = "{operations_exposure}"
management_api_enabled = true

[bootstrap]
create_default_admin = true
admin_user_id = "{admin_user_id}"
admin_password = "{admin_password}"
"#,
        database_path = normalize_path(process_database_path),
        auth_mode = spec.auth_mode,
        embedding_mode = spec.embedding_mode,
        embedding_profile = spec.embedding_profile,
        adapter_entries = adapter_entries,
        bundle_path = normalize_path(directory_bundle_path),
        operations_exposure = spec.operations_exposure,
        admin_user_id = ADMIN_USER_ID,
        admin_password = ADMIN_PASSWORD,
    );
    std::fs::write(config_path, config).expect("platform config should be written");
}

fn create_historical_source_db(path: &Path) {
    let conn = Connection::open(path).expect("historical source db should open");
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
        ",
    )
    .expect("historical source schema should be created");

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
    .expect("deployment should be inserted");
    conn.execute(
        "INSERT INTO ACT_GE_BYTEARRAY (ID_, NAME_, DEPLOYMENT_ID_, BYTES_) VALUES (?1, ?2, ?3, ?4)",
        (
            "resource-1",
            "historicalImportedProcess.bpmn",
            "deployment-1",
            bpmn.as_bytes(),
        ),
    )
    .expect("deployment resource should be inserted");
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
    .expect("process definition should be inserted");
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
    .expect("runtime execution should be inserted");
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
    .expect("runtime task should be inserted");
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
    .expect("historic process instance should be inserted");
}

async fn spawn_platform_server(
    config_path: &Path,
) -> (Arc<ProcessEngine>, String, Client, JoinHandle<()>) {
    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path.to_path_buf()))
        .expect("platform should bootstrap from config");
    let engine = platform.process_engine();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let base_url = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("listener should expose local address")
    );
    let handle = tokio::spawn(async move {
        run_platform_server(platform, listener)
            .await
            .expect("platform server should start");
    });
    (engine, base_url, Client::new(), handle)
}

async fn abort_server(handle: JoinHandle<()>) {
    handle.abort();
    let join_error = handle
        .await
        .expect_err("aborted server should not complete normally");
    assert!(join_error.is_cancelled(), "server task should be cancelled");
}

#[tokio::test]
async fn replacement_unqualified_release_cert_chains_bundle_embedding_directory_and_deprecated_management()
 {
    let tempdir = tempfile::tempdir().expect("tempdir should be created");

    let source_db_path = tempdir.path().join("historical-source.sqlite");
    let bundle_path = tempdir.path().join("portable-bundle");
    create_historical_source_db(&source_db_path);
    let export_result =
        ProcessEngine::export_historical_migration_bundle(&source_db_path, &bundle_path)
            .expect("portable bundle export should succeed");
    assert_eq!(
        export_result.format,
        "portable-historical-migration-bundle/v1".to_string()
    );

    let standalone_directory_bundle = tempdir.path().join("ldap-directory.toml");
    write_directory_bundle(&standalone_directory_bundle);
    let standalone_config = tempdir.path().join("standalone.toml");
    write_platform_config(
        &standalone_config,
        &tempdir.path().join("standalone.sqlite"),
        &standalone_directory_bundle,
        PlatformConfigSpec {
            auth_mode: "basic",
            embedding_mode: "standalone",
            embedding_profile: "standalone-service",
            adapters: &["camel", "cxf"],
            operations_exposure: "jmx-bridge",
        },
    );

    let (standalone_engine, standalone_base_url, standalone_client, standalone_handle) =
        spawn_platform_server(&standalone_config).await;

    let bundle_inspection = ProcessEngine::inspect_historical_migration_bundle(&bundle_path)
        .expect("portable bundle inspection should succeed");
    assert_eq!(bundle_inspection.deployment_count, 1);

    let import_result = standalone_engine
        .import_historical_migration_from_bundle(&bundle_path)
        .expect("portable bundle import should succeed");
    assert_eq!(import_result.imported_deployments, 1);
    assert_eq!(import_result.imported_process_instances, 1);

    let unauthorized_platform_support = standalone_client
        .get(format!("{standalone_base_url}/management/platform/support"))
        .send()
        .await
        .expect("unauthorized support request should respond");
    assert_eq!(
        unauthorized_platform_support.status(),
        StatusCode::UNAUTHORIZED
    );

    let platform_support: Value = standalone_client
        .get(format!("{standalone_base_url}/management/platform/support"))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("platform support request should succeed")
        .json()
        .await
        .expect("platform support payload should be json");
    let deprecated_platform_support = standalone_client
        .get(format!(
            "{standalone_base_url}/service/management/platform/support"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("deprecated platform support request should respond");
    assert_eq!(deprecated_platform_support.status(), StatusCode::NOT_FOUND);
    assert_eq!(platform_support["embedding"]["mode"], "standalone");
    assert_eq!(
        platform_support["embedding"]["profile"],
        "standalone-service"
    );
    assert_eq!(platform_support["enterprise"]["adapterCount"], 2);
    assert_eq!(platform_support["directory"]["provider"], "ldap-mirror");
    assert_eq!(platform_support["directory"]["importedUserCount"], 1);
    assert_eq!(platform_support["operations"]["exposure"], "jmx-bridge");

    let runtime_tasks: Value = standalone_client
        .get(format!(
            "{standalone_base_url}/runtime/tasks?processInstanceId=proc-inst-1&start=0&size=10"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("runtime task query should succeed")
        .json()
        .await
        .expect("runtime task payload should be json");
    assert_eq!(runtime_tasks["total"], 1);
    assert_eq!(runtime_tasks["data"][0]["id"], "task-1");

    abort_server(standalone_handle).await;

    let embedded_directory_bundle = tempdir.path().join("embedded-directory.toml");
    write_directory_bundle(&embedded_directory_bundle);
    let cdi_config = tempdir.path().join("cdi-compatible.toml");
    write_platform_config(
        &cdi_config,
        &tempdir.path().join("cdi-compatible.sqlite"),
        &embedded_directory_bundle,
        PlatformConfigSpec {
            auth_mode: "disabled",
            embedding_mode: "embedded",
            embedding_profile: "cdi-compatible",
            adapters: &["camel", "cdi", "cxf"],
            operations_exposure: "jmx-bridge",
        },
    );
    let (_cdi_engine, cdi_base_url, cdi_client, cdi_handle) =
        spawn_platform_server(&cdi_config).await;
    let cdi_platform_support: Value = cdi_client
        .get(format!("{cdi_base_url}/management/platform/support"))
        .send()
        .await
        .expect("cdi support request should succeed")
        .json()
        .await
        .expect("cdi support payload should be json");
    assert_eq!(cdi_platform_support["embedding"]["mode"], "embedded");
    assert_eq!(
        cdi_platform_support["embedding"]["profile"],
        "cdi-compatible"
    );
    assert_eq!(cdi_platform_support["enterprise"]["adapterCount"], 3);
    abort_server(cdi_handle).await;

    let osgi_directory_bundle = tempdir.path().join("osgi-directory.toml");
    write_directory_bundle(&osgi_directory_bundle);
    let osgi_config = tempdir.path().join("osgi-managed.toml");
    write_platform_config(
        &osgi_config,
        &tempdir.path().join("osgi-managed.sqlite"),
        &osgi_directory_bundle,
        PlatformConfigSpec {
            auth_mode: "disabled",
            embedding_mode: "embedded",
            embedding_profile: "osgi-managed",
            adapters: &["camel", "cxf", "osgi"],
            operations_exposure: "jmx-bridge",
        },
    );
    let (_osgi_engine, osgi_base_url, osgi_client, osgi_handle) =
        spawn_platform_server(&osgi_config).await;
    let osgi_platform_support: Value = osgi_client
        .get(format!("{osgi_base_url}/management/platform/support"))
        .send()
        .await
        .expect("osgi support request should succeed")
        .json()
        .await
        .expect("osgi support payload should be json");
    assert_eq!(osgi_platform_support["embedding"]["mode"], "embedded");
    assert_eq!(
        osgi_platform_support["embedding"]["profile"],
        "osgi-managed"
    );
    assert_eq!(osgi_platform_support["enterprise"]["adapterCount"], 3);
    abort_server(osgi_handle).await;
}
