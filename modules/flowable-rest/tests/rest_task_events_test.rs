use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const EVENT_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="eventProcess" name="Event Process" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="userTask" />
        <userTask id="userTask" name="Review Task" />
        <sequenceFlow id="flow2" sourceRef="userTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-task-events-test".to_string()));
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

async fn deploy_and_start(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
) -> Value {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<Value>().await.unwrap()
}

async fn active_task(client: &reqwest::Client, base_url: &str, process_instance_id: &str) -> Value {
    let response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    body["data"][0].clone()
}

#[tokio::test]
async fn list_task_events_returns_events_for_task() {
    let (engine, base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Event process",
            "resourceName": "event-process.bpmn20.xml",
            "resource": EVENT_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("eventProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let instance = deploy_and_start(&client, &base_url, &process_definition_id).await;
    let process_instance_id = instance["id"].as_str().unwrap();
    let task = active_task(&client, &base_url, process_instance_id).await;
    let task_id = task["id"].as_str().unwrap();

    let response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let events: Value = response.json().await.unwrap();
    assert!(!events.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_task_event_by_id_returns_event() {
    let (engine, base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Event process",
            "resourceName": "event-process.bpmn20.xml",
            "resource": EVENT_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("eventProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let instance = deploy_and_start(&client, &base_url, &process_definition_id).await;
    let process_instance_id = instance["id"].as_str().unwrap();
    let task = active_task(&client, &base_url, process_instance_id).await;
    let task_id = task["id"].as_str().unwrap();

    let list_response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let events: Value = list_response.json().await.unwrap();
    let event_id = events[0]["id"].as_str().unwrap();

    let get_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/events/{event_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let event: Value = get_response.json().await.unwrap();
    assert_eq!(event["id"], event_id);
}

#[tokio::test]
async fn task_events_for_nonexistent_task_returns_404() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .get(format!("{base_url}/runtime/tasks/nonexistent-task/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn task_event_by_nonexistent_id_returns_404() {
    let (engine, base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Event process",
            "resourceName": "event-process.bpmn20.xml",
            "resource": EVENT_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("eventProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let instance = deploy_and_start(&client, &base_url, &process_definition_id).await;
    let process_instance_id = instance["id"].as_str().unwrap();
    let task = active_task(&client, &base_url, process_instance_id).await;
    let task_id = task["id"].as_str().unwrap();

    let response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/events/nonexistent-event"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}
