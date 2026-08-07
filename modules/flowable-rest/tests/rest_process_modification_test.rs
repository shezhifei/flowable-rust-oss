use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const MODIFY_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="modifyProcess" name="Modify Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Task 1" />
        <sequenceFlow id="f2" sourceRef="task1" targetRef="task2" />
        <userTask id="task2" name="Task 2" />
        <sequenceFlow id="f3" sourceRef="task2" targetRef="task3" />
        <userTask id="task3" name="Task 3" />
        <sequenceFlow id="f4" sourceRef="task3" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("modify-test".to_string()));
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

    (base_url, reqwest::Client::new())
}

async fn deploy(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Modify Test",
            "resourceName": "modify.bpmn20.xml",
            "resource": MODIFY_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "deploy failed: {}",
        response.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn modify_process_instance_cancel_and_start() {
    let (base_url, client) = spawn_server().await;
    deploy(&client, &base_url).await;

    // Start a process instance
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "modifyProcess"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        start_response.status().is_success(),
        "start failed: {}",
        start_response.text().await.unwrap_or_default()
    );
    let instance: Value = start_response.json().await.unwrap();
    let instance_id = instance["id"].as_str().unwrap();

    // Verify task 1 is active
    let tasks_response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let tasks: Value = tasks_response.json().await.unwrap();
    assert_eq!(tasks["data"].as_array().unwrap().len(), 1);
    assert_eq!(tasks["data"][0]["taskDefinitionKey"], "task1");

    // Modify: cancel task1 and start task3
    let modify_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{instance_id}/modification"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["task1"],
            "startBeforeActivityIds": ["task3"]
        }))
        .send()
        .await
        .unwrap();
    assert!(
        modify_response.status().is_success(),
        "modify failed: {}",
        modify_response.text().await.unwrap_or_default()
    );

    // Verify task3 is now active
    let tasks_response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let tasks: Value = tasks_response.json().await.unwrap();
    assert_eq!(tasks["data"].as_array().unwrap().len(), 1);
    assert_eq!(tasks["data"][0]["taskDefinitionKey"], "task3");
}

#[tokio::test]
async fn modify_process_instance_empty_request_returns_400() {
    let (base_url, client) = spawn_server().await;
    deploy(&client, &base_url).await;

    // Start a process instance
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "modifyProcess"
        }))
        .send()
        .await
        .unwrap();
    let instance: Value = start_response.json().await.unwrap();
    let instance_id = instance["id"].as_str().unwrap();

    // Try to modify with empty request
    let modify_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{instance_id}/modification"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(modify_response.status(), 400);
}

#[tokio::test]
async fn modify_process_instance_unknown_instance_returns_404() {
    let (base_url, client) = spawn_server().await;

    let modify_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/unknown-instance/modification"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["task1"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(modify_response.status(), 404);
}
