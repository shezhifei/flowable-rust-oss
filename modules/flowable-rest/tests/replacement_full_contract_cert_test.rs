use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use reqwest::{Client, StatusCode, header::CONTENT_TYPE};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use tokio::{net::TcpListener, task::JoinHandle};

const ADMIN_USER_ID: &str = "admin";
const ADMIN_PASSWORD: &str = "historical-cert-secret";

fn normalize_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn write_historical_source_properties_config(
    config_path: &Path,
    process_database_path: &Path,
    engine_name: &str,
    admin_password: &str,
) {
    let config = format!(
        r#"
server.address=127.0.0.1
server.port=0
flowable.process.engine-name={engine_name}
flowable.process.datasource.url=jdbc:sqlite:{database_path}
flowable.security.auth-mode=basic
flowable.bootstrap.admin.enabled=true
flowable.bootstrap.admin.user-id={admin_user_id}
flowable.bootstrap.admin.password={admin_password}
"#,
        engine_name = engine_name,
        database_path = normalize_path(process_database_path),
        admin_user_id = ADMIN_USER_ID,
        admin_password = admin_password,
    );

    std::fs::write(config_path, config).expect("historical source properties should be written");
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
            "BUS-001",
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
        "INSERT INTO ACT_RU_VARIABLE (ID_, EXECUTION_ID_, PROC_INST_ID_, NAME_, TYPE_, TEXT_) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        ("var-1", "proc-inst-1", "proc-inst-1", "customer", "string", "alice"),
    )
    .expect("string variable should be inserted");
    conn.execute(
        "INSERT INTO ACT_RU_VARIABLE (ID_, EXECUTION_ID_, PROC_INST_ID_, NAME_, TYPE_, LONG_) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        ("var-2", "proc-inst-1", "proc-inst-1", "approved", "boolean", 0_i64),
    )
    .expect("boolean variable should be inserted");
    conn.execute(
        "INSERT INTO ACT_RU_VARIABLE (ID_, EXECUTION_ID_, PROC_INST_ID_, NAME_, TYPE_, TEXT_) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        ("var-3", "proc-inst-1", "proc-inst-1", "unsupportedPayload", "serializable", "opaque"),
    )
    .expect("unsupported variable should be inserted");
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
    .expect("historic process instance should be inserted");
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
    .expect("historic variable should be inserted");
}

async fn spawn_platform_server(
    properties_path: &Path,
) -> (Arc<ProcessEngine>, String, Client, JoinHandle<()>) {
    let platform = FlowablePlatform::bootstrap_from_sources(Some(properties_path.to_path_buf()))
        .expect("platform should bootstrap from historical source properties");
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
async fn replacement_full_contract_links_historical_import_bootstrap_and_native_rest_routes() {
    let tempdir = tempfile::tempdir().expect("tempdir should be created");
    let source_db_path = tempdir.path().join("historical-migration-source.sqlite");
    let config_path = tempdir.path().join("application.properties");
    let process_db_path = tempdir.path().join("replacement-full-contract.sqlite");
    create_historical_source_db(&source_db_path);

    let inspection = ProcessEngine::inspect_historical_migration_sqlite(&source_db_path)
        .expect("historical migration sqlite should be inspectable");
    assert_eq!(inspection.deployment_count, 1);
    assert_eq!(inspection.process_definition_count, 1);
    assert_eq!(inspection.process_instance_count, 1);
    assert_eq!(inspection.task_count, 1);
    assert_eq!(inspection.historic_process_instance_count, 1);
    assert_eq!(inspection.historic_variable_count, 1);
    assert!(
        inspection
            .unsupported_variable_types
            .contains(&"serializable".to_string())
    );

    write_historical_source_properties_config(
        &config_path,
        &process_db_path,
        "replacement-full-contract",
        ADMIN_PASSWORD,
    );

    let (engine, base_url, client, server_handle) = spawn_platform_server(&config_path).await;

    let import_result = engine
        .import_historical_migration_from_sqlite(&source_db_path)
        .expect("historical migration sqlite should import into empty bootstrap baseline");
    assert_eq!(import_result.imported_deployments, 1);
    assert_eq!(import_result.imported_process_definitions, 1);
    assert_eq!(import_result.imported_process_instances, 1);
    assert_eq!(import_result.imported_executions, 1);
    assert_eq!(import_result.imported_tasks, 1);
    assert_eq!(import_result.imported_variables, 3);
    assert_eq!(import_result.imported_historic_process_instances, 1);
    assert_eq!(import_result.imported_historic_variable_instances, 1);

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("imported process definition ids should be available")
        .into_iter()
        .next()
        .expect("imported process definition should be registered");
    assert_eq!(process_definition_id, "historicalImportedProcess:1:source");

    let health = client
        .get(format!("{base_url}/health"))
        .send()
        .await
        .expect("health request should succeed");
    assert_eq!(health.status(), StatusCode::OK);
    assert!(health.text().await.expect("health body").contains("UP"));

    let unauthorized = client
        .get(format!("{base_url}/runtime/tasks?start=0&size=10"))
        .send()
        .await
        .expect("unauthorized native request should return a response");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized
        .json()
        .await
        .expect("unauthorized response should be json");
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    let deprecated_business_alias = client
        .get(format!("{base_url}/service/runtime/tasks?start=0&size=10"))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("deprecated business alias request should return a response");
    assert_eq!(deprecated_business_alias.status(), StatusCode::NOT_FOUND);

    let image_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/image"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("native image route should respond");
    assert_eq!(image_response.status(), StatusCode::OK);
    assert_eq!(
        image_response
            .headers()
            .get(CONTENT_TYPE)
            .expect("image content type should be present")
            .to_str()
            .expect("image content type should be utf-8"),
        "image/svg+xml"
    );
    assert!(
        image_response
            .text()
            .await
            .expect("image response should be readable")
            .contains("<svg"),
        "native repository route should render imported BPMN"
    );

    let imported_tasks = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId=proc-inst-1&start=0&size=10"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("imported runtime task query should succeed");
    assert_eq!(imported_tasks.status(), StatusCode::OK);
    let imported_tasks_body: Value = imported_tasks
        .json()
        .await
        .expect("imported runtime task response should be json");
    assert_eq!(imported_tasks_body["total"], 1);
    assert_eq!(imported_tasks_body["data"][0]["id"], "task-1");
    assert_eq!(
        imported_tasks_body["data"][0]["name"],
        "Approve Imported Request"
    );
    assert_eq!(
        imported_tasks_body["data"][0]["processInstanceId"],
        "proc-inst-1"
    );

    let started = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .json(&serde_json::json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "replacement-full-contract-chain"
        }))
        .send()
        .await
        .expect("runtime start through native route should succeed");
    assert_eq!(started.status(), StatusCode::OK);
    let started_body: Value = started.json().await.expect("start response should be json");
    let started_process_instance_id = started_body["id"]
        .as_str()
        .expect("start response should include process instance id")
        .to_string();
    assert_eq!(
        started_body["processDefinitionId"],
        process_definition_id.clone()
    );
    assert_eq!(
        started_body["businessKey"],
        "replacement-full-contract-chain"
    );
    assert_eq!(started_body["isEnded"], false);

    let started_tasks = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={started_process_instance_id}&start=0&size=10"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("started instance task query should succeed");
    assert_eq!(started_tasks.status(), StatusCode::OK);
    let started_tasks_body: Value = started_tasks
        .json()
        .await
        .expect("started instance task response should be json");
    assert_eq!(started_tasks_body["total"], 1);
    let started_task_id = started_tasks_body["data"][0]["id"]
        .as_str()
        .expect("started instance task id should be present")
        .to_string();
    assert_eq!(
        started_tasks_body["data"][0]["name"],
        "Approve Imported Request"
    );

    let completion = client
        .post(format!(
            "{base_url}/runtime/tasks/{started_task_id}/complete"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .expect("started instance task completion should succeed");
    assert_eq!(completion.status(), StatusCode::OK);

    let started_tasks_after_completion = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={started_process_instance_id}&start=0&size=10"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("post-completion task query should succeed");
    assert_eq!(started_tasks_after_completion.status(), StatusCode::OK);
    let started_tasks_after_completion_body: Value = started_tasks_after_completion
        .json()
        .await
        .expect("post-completion task response should be json");
    assert_eq!(started_tasks_after_completion_body["total"], 0);

    let history = client
        .get(format!(
            "{base_url}/history/historic-process-instances?start=0&size=20"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("historic process query should succeed");
    assert_eq!(history.status(), StatusCode::OK);
    let history_body: Value = history
        .json()
        .await
        .expect("historic process response should be json");
    assert!(
        history_body["total"].as_i64().unwrap_or_default() >= 1,
        "history body: {history_body}"
    );

    let history_items = history_body["data"]
        .as_array()
        .expect("historic process response should contain data");
    let migrated_runtime_history = history_items
        .iter()
        .find(|item| item["id"] == started_process_instance_id)
        .expect("completed native-started instance should be recorded in history");
    assert_eq!(
        migrated_runtime_history["processDefinitionId"],
        process_definition_id
    );
    assert!(migrated_runtime_history["startTime"].is_string());
    assert!(migrated_runtime_history["endTime"].is_string());

    if let Some(preserved_historic_instance) = history_items
        .iter()
        .find(|item| item["id"] == "historic-proc-1")
    {
        assert_eq!(
            preserved_historic_instance["processDefinitionId"],
            "historicalImportedProcess:1:source"
        );
        assert!(preserved_historic_instance["startTime"].is_string());
        assert!(preserved_historic_instance["endTime"].is_string());
    }

    abort_server(server_handle).await;
}
