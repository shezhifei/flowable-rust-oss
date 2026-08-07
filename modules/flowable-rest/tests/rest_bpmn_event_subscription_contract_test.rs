use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::Value;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn bpmn_event_subscription_paths_are_available() {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-event-subscription-contract".to_string(),
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
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    let client = reqwest::Client::new();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="approvalMessage" name="approvalReceived" />
        <process id="eventSubscriptionProcess" name="Event Subscription Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="waitForApproval" />
            <intermediateCatchEvent id="waitForApproval">
                <messageEventDefinition messageRef="approvalMessage" />
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="waitForApproval" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&serde_json::json!({
            "key": "eventSubscriptionDeployment",
            "name": "event-subscription.bpmn20.xml",
            "resourceName": "event-subscription.bpmn20.xml",
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
        .json(&serde_json::json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());

    let list_response = client
        .get(format!("{base_url}/runtime/event-subscriptions"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list_response.status().is_success());
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["total"], 1);
    assert_eq!(list_body["data"][0]["eventType"], "message");
    let event_subscription_id = list_body["data"][0]["id"].as_str().unwrap().to_string();

    let get_response = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions/{event_subscription_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_response.status().is_success());
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["id"], event_subscription_id);
    assert_eq!(get_body["eventType"], "message");
}
