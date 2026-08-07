use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const ASYNC_SIGNAL_CATCH_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <signal id="testSignal" name="testSignal" />
    <process id="asyncSignalProcess" name="Async Signal Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="signalCatch" />
        <intermediateCatchEvent id="signalCatch" name="Wait for signal">
            <signalEventDefinition signalRef="testSignal" />
        </intermediateCatchEvent>
        <sequenceFlow id="f2" sourceRef="signalCatch" targetRef="task1" />
        <userTask id="task1" name="After signal" />
        <sequenceFlow id="f3" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const ASYNC_MESSAGE_CATCH_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <message id="testMsg" name="testMessage" />
    <process id="asyncMessageProcess" name="Async Message Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="msgCatch" />
        <intermediateCatchEvent id="msgCatch" name="Wait for message">
            <messageEventDefinition messageRef="testMsg" />
        </intermediateCatchEvent>
        <sequenceFlow id="f2" sourceRef="msgCatch" targetRef="task1" />
        <userTask id="task1" name="After message" />
        <sequenceFlow id="f3" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("async-signal-test".to_string()));
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

async fn deploy(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
    resource_name: &str,
    xml: &str,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": resource_name,
            "resource": xml
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
async fn async_signal_returns_202() {
    let (base_url, client) = spawn_server().await;
    deploy(
        &client,
        &base_url,
        "Async signal",
        "async-signal.bpmn20.xml",
        ASYNC_SIGNAL_CATCH_BPMN,
    )
    .await;

    // Start a process instance
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "asyncSignalProcess"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let instance: Value = start_response.json().await.unwrap();
    let instance_id = instance["id"].as_str().unwrap();

    // Send async signal
    let signal_response = client
        .post(format!("{base_url}/runtime/signals"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "testSignal",
            "async": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(signal_response.status(), 202);

    // Wait a bit for async execution
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify the signal was processed (task should be active)
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
}

#[tokio::test]
async fn async_message_returns_202() {
    let (base_url, client) = spawn_server().await;
    deploy(
        &client,
        &base_url,
        "Async message",
        "async-message.bpmn20.xml",
        ASYNC_MESSAGE_CATCH_BPMN,
    )
    .await;

    // Start a process instance
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "asyncMessageProcess"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let instance: Value = start_response.json().await.unwrap();
    let instance_id = instance["id"].as_str().unwrap();

    // Send async message
    let message_response = client
        .post(format!("{base_url}/runtime/messages"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "messageName": "testMessage",
            "async": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(message_response.status(), 202);

    // Wait a bit for async execution
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify the message was processed (task should be active)
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
}
