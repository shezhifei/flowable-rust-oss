use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn runtime_signals_trigger_waiting_signal_events_and_validate_payloads() {
    let engine = Arc::new(ProcessEngine::new("rest-bpmn-runtime-signals".to_string()));

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
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    let client = reqwest::Client::new();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="Alert Signal" />
        <process id="runtimeSignalProcess" name="Runtime Signal Process" isExecutable="true">
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

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Runtime Signal Deployment",
            "resourceName": "runtime_signal_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "Runtime Signal Instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let waiting_tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(waiting_tasks_response.status().is_success());
    let waiting_tasks: Value = waiting_tasks_response.json().await.unwrap();
    assert_eq!(waiting_tasks["total"], 0);

    let missing_name_response = client
        .post(format!("{base_url}/runtime/signals"))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_name_response.status(), 400);

    let async_with_variables_response = client
        .post(format!("{base_url}/runtime/signals"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "Alert Signal",
            "async": true,
            "variables": [
                { "name": "approved", "value": true }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(async_with_variables_response.status(), 400);

    let signal_response = client
        .post(format!("{base_url}/runtime/signals"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "Alert Signal"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(signal_response.status(), 204);

    let tasks_after_signal_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(tasks_after_signal_response.status().is_success());
    let tasks_after_signal: Value = tasks_after_signal_response.json().await.unwrap();
    assert_eq!(tasks_after_signal["total"], 1);
    assert_eq!(tasks_after_signal["data"][0]["name"], "Task After Signal");

    let async_signal_response = client
        .post(format!("{base_url}/runtime/signals"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "unmatchedSignal",
            "async": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(async_signal_response.status(), 202);
}
