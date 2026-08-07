// P110: event-subscriptions query parameter surface.
//
// Java reference: `EventSubscriptionCollectionResource.java:99-147` — every
// query parameter is an equality filter; `without*` variants are boolean
// "null-value" filters. The Rust-side model only persists id/event_name/
// event_kind as first-class columns, so the REST layer reads the persisted
// row directly (extras columns + wait-state JSON) to expose activityId,
// executionId, processInstanceId and configuration.
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::Value;
use std::sync::Arc;
use tokio::net::TcpListener;

const MESSAGE_CATCH_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
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

#[tokio::test]
async fn event_subscription_query_params_filter_by_activity_execution_instance_and_configuration() {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-event-subscription-query".to_string(),
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

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&serde_json::json!({
            "key": "eventSubscriptionQueryDeployment",
            "name": "event-subscription-query.bpmn20.xml",
            "resourceName": "event-subscription-query.bpmn20.xml",
            "resource": MESSAGE_CATCH_XML
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

    // Baseline: one message subscription, response exposes activityId /
    // executionId / processInstanceId.
    let list_response = client
        .get(format!("{base_url}/runtime/event-subscriptions"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list_response.status().is_success());
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["total"], 1);
    let subscription = list_body["data"][0].clone();
    let event_subscription_id = subscription["id"].as_str().unwrap().to_string();
    let execution_id = subscription["executionId"].as_str().unwrap().to_string();
    let process_instance_id = subscription["processInstanceId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(subscription["eventType"], "message");
    assert_eq!(subscription["activityId"], "waitForApproval");
    assert!(subscription["configuration"].is_null());

    // activityId equality (Java EventSubscriptionCollectionResource.java:99).
    let resp = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions?activityId=waitForApproval"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], event_subscription_id);

    // activityId no-match.
    let resp = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions?activityId=otherActivity"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);

    // executionId equality (Java :102).
    let resp = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions?executionId={execution_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);

    // processInstanceId equality (Java :105).
    let resp = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);

    // configuration equality (Java :144) — the message subscription has no
    // configuration, so an exact value never matches.
    let resp = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions?configuration=some-correlation-key"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);

    // withoutConfiguration=true (Java :147) — matches the null configuration.
    let resp = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions?withoutConfiguration=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);

    // withoutProcessInstanceId=true (Java :108) — every persisted row carries a
    // process instance id, so this matches nothing.
    let resp = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions?withoutProcessInstanceId=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);

    // eventType / eventName equality unchanged (Java :93-97).
    let resp = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions?eventType=message"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);

    let resp = client
        .get(format!(
            "{base_url}/runtime/event-subscriptions?eventName=approvalReceived"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);
}
