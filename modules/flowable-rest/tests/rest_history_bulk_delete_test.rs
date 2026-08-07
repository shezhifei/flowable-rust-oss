use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const SIMPLE_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="simpleProcess" name="Simple Process" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="userTask" />
        <userTask id="userTask" name="Simple Task" />
        <sequenceFlow id="flow2" sourceRef="userTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(
        "rest-history-bulk-delete-test".to_string(),
    ));
    engine
        .get_identity_service()
        .save_user(flowable_engine::identity::entities::User {
            id: "admin".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: Some("test".to_string()),
            tenant_id: None,
        });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let engine_for_server = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_process(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Simple process",
            "resourceName": "simple-process.bpmn20.xml",
            "resource": SIMPLE_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
}

async fn start_task(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
) -> (String, String) {
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), reqwest::StatusCode::OK);
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(tasks_response.status(), reqwest::StatusCode::OK);
    let tasks: Value = tasks_response.json().await.unwrap();
    let task_id = tasks["data"][0]["id"].as_str().unwrap().to_string();

    (process_instance_id, task_id)
}

async fn complete_task(client: &reqwest::Client, base_url: &str, task_id: &str) {
    let complete_response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete_response.status(), reqwest::StatusCode::OK);
}

async fn start_and_complete_task(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
) -> (String, String) {
    let (process_instance_id, task_id) = start_task(client, base_url, process_definition_id).await;
    complete_task(client, base_url, &task_id).await;
    (process_instance_id, task_id)
}

#[tokio::test]
async fn historic_task_instances_support_unfinished_get_and_post_queries() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("simpleProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let (_finished_process_id, finished_task_id) =
        start_task(&client, &base_url, &process_definition_id).await;
    let (unfinished_process_id, unfinished_task_id) =
        start_task(&client, &base_url, &process_definition_id).await;
    complete_task(&client, &base_url, &finished_task_id).await;

    let get_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?unfinished=true&sort=processInstanceId&order=asc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["start"], 0);
    assert_eq!(get_body["size"], 1);
    assert_eq!(get_body["total"], 1);
    assert_eq!(get_body["data"][0]["id"], unfinished_task_id);
    assert_eq!(
        get_body["data"][0]["processInstanceId"],
        unfinished_process_id
    );
    assert!(get_body["data"][0]["endTime"].is_null());

    let post_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "unfinished": true,
            "processInstanceId": unfinished_process_id,
            "sort": "processInstanceId",
            "order": "asc"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(post_response.status(), reqwest::StatusCode::OK);
    let post_body: Value = post_response.json().await.unwrap();
    assert_eq!(post_body["total"], 1);
    assert_eq!(post_body["data"][0]["id"], unfinished_task_id);
    assert_eq!(
        post_body["data"][0]["processInstanceId"],
        unfinished_process_id
    );
    assert!(post_body["data"][0]["endTime"].is_null());
}

#[tokio::test]
async fn bulk_delete_historic_task_instances() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("simpleProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let (_, task_id_1) = start_and_complete_task(&client, &base_url, &process_definition_id).await;
    let (_, task_id_2) = start_and_complete_task(&client, &base_url, &process_definition_id).await;

    let list_before = client
        .get(format!("{base_url}/history/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_before.status(), reqwest::StatusCode::OK);
    let body_before: Value = list_before.json().await.unwrap();
    assert!(body_before["total"].as_u64().unwrap() >= 2);

    let delete_response = client
        .post(format!("{base_url}/history/historic-task-instances/delete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "historicTaskInstanceIds": [task_id_1, task_id_2]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn bulk_delete_historic_process_instances() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("simpleProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let (process_instance_id_1, _) =
        start_and_complete_task(&client, &base_url, &process_definition_id).await;
    let (process_instance_id_2, _) =
        start_and_complete_task(&client, &base_url, &process_definition_id).await;

    let delete_response = client
        .post(format!(
            "{base_url}/history/historic-process-instances/delete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "instanceIds": [process_instance_id_1, process_instance_id_2]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let get_1 = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id_1}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_1.status(), reqwest::StatusCode::NOT_FOUND);

    let get_2 = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id_2}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_2.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bulk_delete_historic_tasks_missing_action_returns_400() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/historic-task-instances/delete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "historicTaskInstanceIds": ["id1"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bulk_delete_historic_tasks_invalid_action_returns_400() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/historic-task-instances/delete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "destroy",
            "historicTaskInstanceIds": ["id1"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bulk_delete_historic_tasks_empty_ids_returns_400() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/historic-task-instances/delete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "historicTaskInstanceIds": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}
