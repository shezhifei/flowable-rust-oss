use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use reqwest::{Client, StatusCode};
use serde_json::{Map, Value, json};
use std::path::Path;
use std::sync::Arc;
use tokio::{net::TcpListener, task::JoinHandle};

const ADMIN_USER_ID: &str = "admin";
const ADMIN_PASSWORD: &str = "dual-run-secret";
const OWNED_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="replacementDualRunProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Dual Run Review Task" />
        <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

fn normalize_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn value_shape(value: &Value) -> Value {
    match value {
        Value::Null => Value::String("null".to_string()),
        Value::Bool(_) => Value::String("bool".to_string()),
        Value::Number(_) => Value::String("number".to_string()),
        Value::String(_) => Value::String("string".to_string()),
        Value::Array(items) => Value::Array(items.iter().map(value_shape).collect()),
        Value::Object(map) => {
            let mut shaped = Map::new();
            for (key, value) in map {
                shaped.insert(key.clone(), value_shape(value));
            }
            Value::Object(shaped)
        }
    }
}

fn write_platform_config(
    config_path: &Path,
    database_path: &Path,
    engine_name: &str,
    admin_password: &str,
) {
    let config = format!(
        r#"[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "{engine_name}"
database_path = "{database_path}"

[security]
auth_mode = "basic"

[bootstrap]
create_default_admin = true
admin_user_id = "{admin_user_id}"
admin_password = "{admin_password}"
"#,
        engine_name = engine_name,
        database_path = normalize_path(database_path),
        admin_user_id = ADMIN_USER_ID,
        admin_password = admin_password,
    );

    std::fs::write(config_path, config).expect("platform config should be written");
}

async fn spawn_platform_server(
    config_path: &Path,
) -> (Arc<ProcessEngine>, String, Client, JoinHandle<()>) {
    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path.to_path_buf()))
        .expect("platform should bootstrap from config file");
    let engine = platform.process_engine();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let base_url = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("listener should expose address")
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
        .expect_err("aborted platform server should not complete normally");
    assert!(join_error.is_cancelled(), "server task should be cancelled");
}

async fn deploy_owned_process(client: &Client, base_url: &str) -> Value {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .json(&json!({
            "name": "Replacement Dual Run Deployment",
            "resourceName": "replacement_dual_run_process.bpmn20.xml",
            "resource": OWNED_PROCESS_BPMN
        }))
        .send()
        .await
        .expect("deployment request should succeed");

    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(response.status(), StatusCode::CREATED);
    response
        .json()
        .await
        .expect("deployment body should be json")
}

async fn start_process_instance(
    client: &Client,
    base_url: &str,
    process_definition_id: &str,
    business_key: &str,
) -> Value {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": business_key
        }))
        .send()
        .await
        .expect("start request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    response.json().await.expect("start body should be json")
}

async fn list_runtime_process_instances(client: &Client, base_url: &str) -> Value {
    let response = client
        .get(format!(
            "{base_url}/runtime/process-instances?start=0&size=10"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("runtime process query should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    response
        .json()
        .await
        .expect("runtime process query body should be json")
}

async fn list_runtime_tasks(client: &Client, base_url: &str, process_instance_id: &str) -> Value {
    let response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}&start=0&size=10"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("task query should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    response
        .json()
        .await
        .expect("task query body should be json")
}

async fn complete_task(client: &Client, base_url: &str, task_id: &str) -> StatusCode {
    let response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .expect("task completion should succeed");

    response.status()
}

async fn list_history_process_instances(client: &Client, base_url: &str) -> Value {
    let response = client
        .get(format!(
            "{base_url}/history/historic-process-instances?start=0&size=10"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("history query should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    response
        .json()
        .await
        .expect("history query body should be json")
}

#[tokio::test]
async fn replacement_dual_run_owned_rest_outputs_remain_stable() {
    let left_tempdir = tempfile::tempdir().expect("left tempdir should be created");
    let right_tempdir = tempfile::tempdir().expect("right tempdir should be created");

    let left_config_path = left_tempdir.path().join("flowable-platform.toml");
    let right_config_path = right_tempdir.path().join("flowable-platform.toml");
    write_platform_config(
        &left_config_path,
        &left_tempdir.path().join("left-process-engine.sqlite"),
        "replacement-dual-run-left",
        ADMIN_PASSWORD,
    );
    write_platform_config(
        &right_config_path,
        &right_tempdir.path().join("right-process-engine.sqlite"),
        "replacement-dual-run-right",
        ADMIN_PASSWORD,
    );

    let (left_engine, left_base_url, left_client, left_handle) =
        spawn_platform_server(&left_config_path).await;
    let (right_engine, right_base_url, right_client, right_handle) =
        spawn_platform_server(&right_config_path).await;

    let left_deploy = deploy_owned_process(&left_client, &left_base_url).await;
    let right_deploy = deploy_owned_process(&right_client, &right_base_url).await;

    assert_eq!(value_shape(&left_deploy), value_shape(&right_deploy));
    assert_eq!(left_deploy["name"], right_deploy["name"]);
    assert_eq!(left_deploy["name"], "Replacement Dual Run Deployment");
    assert!(left_deploy["deploymentTime"].is_string());
    assert!(right_deploy["deploymentTime"].is_string());

    let left_process_definition_id = left_engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("left process definition ids should be available")
        .into_iter()
        .next()
        .expect("left deployment should register one process definition");
    let right_process_definition_id = right_engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("right process definition ids should be available")
        .into_iter()
        .next()
        .expect("right deployment should register one process definition");

    let left_started = start_process_instance(
        &left_client,
        &left_base_url,
        &left_process_definition_id,
        "replacement-dual-run-instance",
    )
    .await;
    let right_started = start_process_instance(
        &right_client,
        &right_base_url,
        &right_process_definition_id,
        "replacement-dual-run-instance",
    )
    .await;

    assert_eq!(value_shape(&left_started), value_shape(&right_started));
    assert_eq!(left_started["businessKey"], right_started["businessKey"]);
    assert_eq!(left_started["businessKey"], "replacement-dual-run-instance");
    assert_eq!(left_started["isEnded"], false);
    assert_eq!(right_started["isEnded"], false);
    assert!(left_started["id"].is_string());
    assert!(right_started["id"].is_string());

    let left_process_instance_id = left_started["id"]
        .as_str()
        .expect("left start response should contain process instance id")
        .to_string();
    let right_process_instance_id = right_started["id"]
        .as_str()
        .expect("right start response should contain process instance id")
        .to_string();

    let left_runtime = list_runtime_process_instances(&left_client, &left_base_url).await;
    let right_runtime = list_runtime_process_instances(&right_client, &right_base_url).await;

    assert_eq!(value_shape(&left_runtime), value_shape(&right_runtime));
    assert_eq!(left_runtime["total"], 1);
    assert_eq!(right_runtime["total"], 1);
    assert_eq!(
        left_runtime["data"][0]["businessKey"],
        "replacement-dual-run-instance"
    );
    assert_eq!(
        right_runtime["data"][0]["businessKey"],
        "replacement-dual-run-instance"
    );
    assert_eq!(left_runtime["data"][0]["isEnded"], false);
    assert_eq!(right_runtime["data"][0]["isEnded"], false);

    let left_tasks =
        list_runtime_tasks(&left_client, &left_base_url, &left_process_instance_id).await;
    let right_tasks =
        list_runtime_tasks(&right_client, &right_base_url, &right_process_instance_id).await;

    assert_eq!(value_shape(&left_tasks), value_shape(&right_tasks));
    assert_eq!(left_tasks["total"], 1);
    assert_eq!(right_tasks["total"], 1);
    assert_eq!(left_tasks["data"][0]["name"], "Dual Run Review Task");
    assert_eq!(right_tasks["data"][0]["name"], "Dual Run Review Task");
    assert_eq!(
        left_tasks["data"][0]["processInstanceId"],
        left_process_instance_id
    );
    assert_eq!(
        right_tasks["data"][0]["processInstanceId"],
        right_process_instance_id
    );

    let left_task_id = left_tasks["data"][0]["id"]
        .as_str()
        .expect("left task id should be present")
        .to_string();
    let right_task_id = right_tasks["data"][0]["id"]
        .as_str()
        .expect("right task id should be present")
        .to_string();

    let left_completion = complete_task(&left_client, &left_base_url, &left_task_id).await;
    let right_completion = complete_task(&right_client, &right_base_url, &right_task_id).await;

    assert_eq!(left_completion, right_completion);
    assert_eq!(left_completion, StatusCode::OK);

    let left_history = list_history_process_instances(&left_client, &left_base_url).await;
    let right_history = list_history_process_instances(&right_client, &right_base_url).await;

    assert_eq!(value_shape(&left_history), value_shape(&right_history));
    assert_eq!(left_history["total"], 1);
    assert_eq!(right_history["total"], 1);
    assert!(left_history["data"][0]["startTime"].is_string());
    assert!(right_history["data"][0]["startTime"].is_string());
    assert!(left_history["data"][0]["endTime"].is_string());
    assert!(right_history["data"][0]["endTime"].is_string());
    assert_eq!(left_history["data"][0]["id"], left_process_instance_id);
    assert_eq!(right_history["data"][0]["id"], right_process_instance_id);

    abort_server(left_handle).await;
    abort_server(right_handle).await;
}
