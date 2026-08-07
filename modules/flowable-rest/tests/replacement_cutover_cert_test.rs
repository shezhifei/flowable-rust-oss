use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use tokio::{net::TcpListener, task::JoinHandle};

const ADMIN_USER_ID: &str = "admin";
const ADMIN_PASSWORD: &str = "cutover-secret";
const OWNED_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="replacementCutoverProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Replacement Approval Task" />
        <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

fn normalize_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
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
            "name": "Replacement Cutover Deployment",
            "resourceName": "replacement_cutover_process.bpmn20.xml",
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
async fn replacement_cutover_restart_preserves_owned_runtime_and_history_state() {
    let tempdir = tempfile::tempdir().expect("tempdir should be created");
    let config_path = tempdir.path().join("flowable-platform.toml");
    let database_path = tempdir.path().join("process-engine.sqlite");
    write_platform_config(
        &config_path,
        &database_path,
        "replacement-cutover-cert",
        ADMIN_PASSWORD,
    );

    let (first_engine, first_base_url, first_client, first_handle) =
        spawn_platform_server(&config_path).await;

    let deployment = deploy_owned_process(&first_client, &first_base_url).await;
    assert_eq!(deployment["name"], "Replacement Cutover Deployment");
    assert!(deployment["id"].is_string());
    assert!(deployment["deploymentTime"].is_string());

    let process_definition_id = first_engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("process definition ids should be available")
        .into_iter()
        .next()
        .expect("deployment should register one process definition");

    let started = start_process_instance(
        &first_client,
        &first_base_url,
        &process_definition_id,
        "replacement-cutover-instance",
    )
    .await;
    let process_instance_id = started["id"]
        .as_str()
        .expect("start response should contain process instance id")
        .to_string();

    assert_eq!(started["businessKey"], "replacement-cutover-instance");
    assert_eq!(started["processDefinitionId"], process_definition_id);
    assert_eq!(started["isEnded"], false);

    let runtime_before_restart =
        list_runtime_process_instances(&first_client, &first_base_url).await;
    assert_eq!(runtime_before_restart["total"], 1);
    assert_eq!(runtime_before_restart["data"][0]["id"], process_instance_id);
    assert_eq!(
        runtime_before_restart["data"][0]["businessKey"],
        "replacement-cutover-instance"
    );
    assert_eq!(runtime_before_restart["data"][0]["isEnded"], false);

    let history_before_restart =
        list_history_process_instances(&first_client, &first_base_url).await;
    assert_eq!(history_before_restart["total"], 1);
    assert_eq!(history_before_restart["data"][0]["id"], process_instance_id);
    assert_eq!(
        history_before_restart["data"][0]["processDefinitionId"],
        process_definition_id
    );
    assert!(history_before_restart["data"][0]["startTime"].is_string());
    assert!(history_before_restart["data"][0]["endTime"].is_null());

    let tasks_before_restart =
        list_runtime_tasks(&first_client, &first_base_url, &process_instance_id).await;
    assert_eq!(tasks_before_restart["total"], 1);
    assert_eq!(
        tasks_before_restart["data"][0]["processInstanceId"],
        process_instance_id
    );
    assert_eq!(
        tasks_before_restart["data"][0]["name"],
        "Replacement Approval Task"
    );
    let task_id = tasks_before_restart["data"][0]["id"]
        .as_str()
        .expect("task id should be present")
        .to_string();

    drop(first_engine);
    abort_server(first_handle).await;

    let (_second_engine, second_base_url, second_client, second_handle) =
        spawn_platform_server(&config_path).await;

    let runtime_after_restart =
        list_runtime_process_instances(&second_client, &second_base_url).await;
    assert_eq!(runtime_after_restart["total"], 1);
    assert_eq!(runtime_after_restart["data"][0]["id"], process_instance_id);
    assert_eq!(
        runtime_after_restart["data"][0]["businessKey"],
        "replacement-cutover-instance"
    );
    assert_eq!(runtime_after_restart["data"][0]["isEnded"], false);

    let history_after_restart =
        list_history_process_instances(&second_client, &second_base_url).await;
    assert_eq!(history_after_restart["total"], 1);
    assert_eq!(history_after_restart["data"][0]["id"], process_instance_id);
    assert!(history_after_restart["data"][0]["startTime"].is_string());
    assert!(history_after_restart["data"][0]["endTime"].is_null());

    let tasks_after_restart =
        list_runtime_tasks(&second_client, &second_base_url, &process_instance_id).await;
    assert_eq!(tasks_after_restart["total"], 1);
    assert_eq!(tasks_after_restart["data"][0]["id"], task_id);
    assert_eq!(
        tasks_after_restart["data"][0]["name"],
        "Replacement Approval Task"
    );

    let completion = complete_task(&second_client, &second_base_url, &task_id).await;
    assert_eq!(completion, StatusCode::OK);

    let tasks_after_completion =
        list_runtime_tasks(&second_client, &second_base_url, &process_instance_id).await;
    assert_eq!(tasks_after_completion["total"], 0);
    assert!(
        tasks_after_completion["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let history_after_completion =
        list_history_process_instances(&second_client, &second_base_url).await;
    assert_eq!(history_after_completion["total"], 1);
    assert_eq!(
        history_after_completion["data"][0]["id"],
        process_instance_id
    );
    assert!(history_after_completion["data"][0]["startTime"].is_string());
    assert!(history_after_completion["data"][0]["endTime"].is_string());

    abort_server(second_handle).await;
}
