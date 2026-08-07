use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const MESSAGE_CATCH_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <message id="paymentReceived" name="paymentReceived" />
    <process id="orderProcess" name="Order Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="receivePayment" />
        <intermediateCatchEvent id="receivePayment" name="Wait for payment">
            <messageEventDefinition messageRef="paymentReceived" />
        </intermediateCatchEvent>
        <sequenceFlow id="f2" sourceRef="receivePayment" targetRef="processOrder" />
        <userTask id="processOrder" name="Process order" />
        <sequenceFlow id="f3" sourceRef="processOrder" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const MESSAGE_START_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <message id="newOrder" name="newOrder" />
    <process id="autoStartProcess" name="Auto Start Process" isExecutable="true">
        <startEvent id="start">
            <messageEventDefinition messageRef="newOrder" />
        </startEvent>
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Handle order" />
        <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const MESSAGE_BOUNDARY_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <message id="cancelOrder" name="cancelOrder" />
    <process id="boundaryProcess" name="Boundary Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Work on order" />
        <boundaryEvent id="cancelBoundary" attachedToRef="task1">
            <messageEventDefinition messageRef="cancelOrder" />
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="cancelBoundary" targetRef="cancelled" />
        <endEvent id="cancelled" name="Cancelled" />
        <sequenceFlow id="f3" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("message-correlation-test".to_string()));
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
async fn message_correlation_triggers_intermediate_catch_event() {
    let (base_url, client) = spawn_server().await;
    deploy(
        &client,
        &base_url,
        "Message catch",
        "message-catch.bpmn20.xml",
        MESSAGE_CATCH_BPMN,
    )
    .await;

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "orderProcess"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let instance: Value = start_response.json().await.unwrap();
    let instance_id = instance["id"].as_str().unwrap();

    let msg_response = client
        .post(format!("{base_url}/runtime/messages"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "messageName": "paymentReceived"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        msg_response.status().is_success(),
        "message correlation should succeed, got {}: {}",
        msg_response.status(),
        msg_response.text().await.unwrap_or_default()
    );

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
    assert_eq!(tasks["data"][0]["name"], "Process order");
}

#[tokio::test]
async fn message_correlation_starts_process_by_message() {
    let (base_url, client) = spawn_server().await;
    deploy(
        &client,
        &base_url,
        "Message start",
        "message-start.bpmn20.xml",
        MESSAGE_START_BPMN,
    )
    .await;

    let msg_response = client
        .post(format!("{base_url}/runtime/messages"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "messageName": "newOrder",
            "variables": [
                {"name": "orderId", "value": "ORD-001"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert!(
        msg_response.status().is_success(),
        "message start should succeed, got {}: {}",
        msg_response.status(),
        msg_response.text().await.unwrap_or_default()
    );

    let tasks_response = client
        .get(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let tasks: Value = tasks_response.json().await.unwrap();
    assert_eq!(tasks["data"].as_array().unwrap().len(), 1);
    assert_eq!(tasks["data"][0]["name"], "Handle order");
}

#[tokio::test]
async fn message_correlation_triggers_boundary_event() {
    let engine = Arc::new(ProcessEngine::new("boundary-debug-test".to_string()));
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
    let client = reqwest::Client::new();

    deploy(
        &client,
        &base_url,
        "Boundary msg",
        "boundary-msg.bpmn20.xml",
        MESSAGE_BOUNDARY_BPMN,
    )
    .await;

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "boundaryProcess"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let instance: Value = start_response.json().await.unwrap();
    let instance_id = instance["id"].as_str().unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let boundary_states = store.snapshot_boundary_event_states(&mut session);
    println!("Boundary event states count: {}", boundary_states.len());
    for (key, state) in &boundary_states {
        println!(
            "  key={key} boundary_event={} process_instance={} subscription={:?}",
            state.boundary_event_id, state.process_instance_id, state.event_subscription
        );
    }

    let wait_states = store.snapshot_event_wait_states(&mut session);
    println!("Wait states count: {}", wait_states.len());
    for (key, state) in &wait_states {
        println!(
            "  key={key} kind={:?} process_instance={} execution={} subscription={:?}",
            state.wait_kind,
            state.process_instance_id,
            state.execution_id,
            state.event_subscription
        );
    }
    let _ = session.rollback();

    let msg_response = client
        .post(format!("{base_url}/runtime/messages"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "messageName": "cancelOrder"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        msg_response.status().is_success(),
        "boundary message should succeed, got {}: {}",
        msg_response.status(),
        msg_response.text().await.unwrap_or_default()
    );

    let tasks_response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let tasks: Value = tasks_response.json().await.unwrap();
    assert_eq!(
        tasks["data"].as_array().unwrap().len(),
        0,
        "task should be cancelled by boundary message"
    );
}

#[tokio::test]
async fn message_correlation_returns_404_for_unknown_message() {
    let (base_url, client) = spawn_server().await;

    let msg_response = client
        .post(format!("{base_url}/runtime/messages"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "messageName": "nonexistentMessage"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(msg_response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn message_correlation_rejects_missing_message_name() {
    let (base_url, client) = spawn_server().await;

    let msg_response = client
        .post(format!("{base_url}/runtime/messages"))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(msg_response.status(), reqwest::StatusCode::BAD_REQUEST);
}
