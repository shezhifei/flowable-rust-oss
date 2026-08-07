use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const SIGNAL_CATCH_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <signal id="alertSignal" name="Alert Signal" />
    <process id="signalCatchProcess" name="Signal Catch Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="catchAlert" />
        <intermediateCatchEvent id="catchAlert" name="Catch Alert">
            <signalEventDefinition signalRef="alertSignal" />
        </intermediateCatchEvent>
        <sequenceFlow id="f2" sourceRef="catchAlert" targetRef="afterSignalTask" />
        <userTask id="afterSignalTask" name="Task After Signal" />
        <sequenceFlow id="f3" sourceRef="afterSignalTask" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const MESSAGE_CATCH_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <message id="paymentMsg" name="Payment Received" />
    <process id="messageCatchProcess" name="Message Catch Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="catchPayment" />
        <intermediateCatchEvent id="catchPayment" name="Catch Payment">
            <messageEventDefinition messageRef="paymentMsg" />
        </intermediateCatchEvent>
        <sequenceFlow id="f2" sourceRef="catchPayment" targetRef="afterMessageTask" />
        <userTask id="afterMessageTask" name="Task After Message" />
        <sequenceFlow id="f3" sourceRef="afterMessageTask" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(
        "rest-execution-triggers-test".to_string(),
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

async fn find_waiting_execution(
    engine: &Arc<ProcessEngine>,
    process_instance_id: &str,
) -> Option<String> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let result = store
        .snapshot_event_wait_states(&mut session)
        .into_values()
        .find(|ws| ws.process_instance_id == process_instance_id)
        .map(|ws| ws.execution_id);
    let _ = session.rollback();
    result
}

#[tokio::test]
async fn signal_execution_triggers_catch_event() {
    let (engine, base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Signal Catch Deployment",
            "resourceName": "signal_catch_process.bpmn20.xml",
            "resource": SIGNAL_CATCH_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("signalCatchProcess", None)
        .unwrap()
        .unwrap()
        .id;

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
    let process_instance_id = start_body["id"].as_str().unwrap();

    let execution_id = find_waiting_execution(&engine, process_instance_id)
        .await
        .unwrap();

    let trigger_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/signal-event-received"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "Alert Signal"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trigger_response.status(), reqwest::StatusCode::NO_CONTENT);

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
    assert_eq!(tasks["total"], 1);
    assert_eq!(tasks["data"][0]["name"], "Task After Signal");
}

#[tokio::test]
async fn signal_execution_with_variables() {
    let (engine, base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Signal Catch Deployment",
            "resourceName": "signal_catch_process.bpmn20.xml",
            "resource": SIGNAL_CATCH_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("signalCatchProcess", None)
        .unwrap()
        .unwrap()
        .id;

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
    let process_instance_id = start_body["id"].as_str().unwrap();

    let execution_id = find_waiting_execution(&engine, process_instance_id)
        .await
        .unwrap();

    let trigger_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/signal-event-received"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "Alert Signal",
            "variables": [
                { "name": "approved", "value": true }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trigger_response.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn signal_execution_missing_signal_name_returns_400() {
    let (_engine, base_url, client) = spawn_server().await;

    let trigger_response = client
        .post(format!(
            "{base_url}/runtime/executions/some-execution/signal-event-received"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(trigger_response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn signal_execution_nonexistent_execution_returns_404() {
    let (_engine, base_url, client) = spawn_server().await;

    let trigger_response = client
        .post(format!(
            "{base_url}/runtime/executions/nonexistent-execution/signal-event-received"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "Alert Signal"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trigger_response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn message_execution_triggers_catch_event() {
    let (engine, base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Message Catch Deployment",
            "resourceName": "message_catch_process.bpmn20.xml",
            "resource": MESSAGE_CATCH_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("messageCatchProcess", None)
        .unwrap()
        .unwrap()
        .id;

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
    let process_instance_id = start_body["id"].as_str().unwrap();

    let execution_id = find_waiting_execution(&engine, process_instance_id)
        .await
        .unwrap();

    let trigger_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/message-event-received"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "messageName": "Payment Received"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trigger_response.status(), reqwest::StatusCode::NO_CONTENT);

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
    assert_eq!(tasks["total"], 1);
    assert_eq!(tasks["data"][0]["name"], "Task After Message");
}

#[tokio::test]
async fn message_execution_with_variables() {
    let (engine, base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Message Catch Deployment",
            "resourceName": "message_catch_process.bpmn20.xml",
            "resource": MESSAGE_CATCH_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("messageCatchProcess", None)
        .unwrap()
        .unwrap()
        .id;

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
    let process_instance_id = start_body["id"].as_str().unwrap();

    let execution_id = find_waiting_execution(&engine, process_instance_id)
        .await
        .unwrap();

    let trigger_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/message-event-received"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "messageName": "Payment Received",
            "variables": [
                { "name": "amount", "value": 100 }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trigger_response.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn message_execution_missing_message_name_returns_400() {
    let (_engine, base_url, client) = spawn_server().await;

    let trigger_response = client
        .post(format!(
            "{base_url}/runtime/executions/some-execution/message-event-received"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(trigger_response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn message_execution_nonexistent_execution_returns_404() {
    let (_engine, base_url, client) = spawn_server().await;

    let trigger_response = client
        .post(format!(
            "{base_url}/runtime/executions/nonexistent-execution/message-event-received"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "messageName": "Payment Received"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trigger_response.status(), reqwest::StatusCode::NOT_FOUND);
}
