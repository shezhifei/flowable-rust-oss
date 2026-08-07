//! P71 contract: historic activity `deleteReason` for event-based gateway cancel.
//!
//! Java parity: after one branch of an event-based gateway fires, remaining
//! sibling catch historic activities end with
//! `DeleteReason.EVENT_BASED_GATEWAY_CANCEL` ("event based gateway cancel").
//! Normally completed activities keep `deleteReason: null`.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::history::delete_reason::EVENT_BASED_GATEWAY_CANCEL;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const EVENT_GATEWAY_MESSAGE_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p71EventGatewayDeleteReason" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="gw" />
        <eventBasedGateway id="gw" />
        <sequenceFlow id="flow2" sourceRef="gw" targetRef="catchMessage" />
        <sequenceFlow id="flow3" sourceRef="gw" targetRef="catchTimer" />
        <intermediateCatchEvent id="catchMessage">
            <messageEventDefinition messageRef="msg1" />
        </intermediateCatchEvent>
        <intermediateCatchEvent id="catchTimer">
            <timerEventDefinition>
                <timeDuration>PT1S</timeDuration>
            </timerEventDefinition>
        </intermediateCatchEvent>
        <sequenceFlow id="flow4" sourceRef="catchMessage" targetRef="taskAfterMsg" />
        <sequenceFlow id="flow5" sourceRef="catchTimer" targetRef="taskAfterTimer" />
        <userTask id="taskAfterMsg" name="Task After Message" />
        <userTask id="taskAfterTimer" name="Task After Timer" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
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

#[tokio::test]
async fn event_gateway_cancel_exposes_delete_reason_on_historic_activity_rest() {
    let (engine, base_url, client) = spawn_server("rest-p71-hai-delete-reason").await;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .add_string(
                    "p71_event_gateway_delete_reason.bpmn20.xml".to_string(),
                    EVENT_GATEWAY_MESSAGE_TIMER_XML.to_string(),
                ),
        )
        .unwrap();

    let process_def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_def_id, None)
        .unwrap();
    let process_instance_id = process_instance.id.clone();

    // Before trigger: open catch activities report deleteReason null via REST.
    let pre = client
        .get(format!(
            "{base_url}/history/historic-activity-instances?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(pre.status().is_success());
    let pre_body: Value = pre.json().await.unwrap();
    let pre_data = pre_body["data"].as_array().expect("data array");
    for activity_id in ["catchMessage", "catchTimer"] {
        let row = pre_data
            .iter()
            .find(|a| a["activityId"] == activity_id)
            .unwrap_or_else(|| panic!("pre-trigger historic activity for {activity_id}"));
        assert!(
            row["deleteReason"].is_null(),
            "{activity_id} deleteReason must be null before cancel; got {}",
            row["deleteReason"]
        );
    }

    let wait_states = engine
        .get_task_service()
        .get_event_wait_states_by_process_instance_id(process_instance_id.clone());
    let msg_wait = wait_states
        .iter()
        .find(|ws| ws.event_ref.as_deref() == Some("msg1"))
        .expect("message wait state");

    engine.get_runtime_service().trigger_event_intermediate_catch(
        EventSubscriptionKind::Message,
        "msg1".to_string(),
        msg_wait.execution_id.clone(),
    );

    let post = client
        .get(format!(
            "{base_url}/history/historic-activity-instances?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(post.status().is_success());
    let post_body: Value = post.json().await.unwrap();
    let post_data = post_body["data"].as_array().expect("data array");

    let cancelled = post_data
        .iter()
        .find(|a| a["activityId"] == "catchTimer")
        .expect("historic activity for cancelled catchTimer");
    assert_eq!(
        cancelled["deleteReason"].as_str(),
        Some(EVENT_BASED_GATEWAY_CANCEL),
        "cancelled sibling REST deleteReason must match Java EVENT_BASED_GATEWAY_CANCEL"
    );
    assert!(
        cancelled["endTime"].is_string(),
        "cancelled sibling must have endTime set"
    );

    let winner = post_data
        .iter()
        .find(|a| a["activityId"] == "catchMessage" && a["endTime"].is_string())
        .expect("ended historic activity for winning catchMessage");
    assert!(
        winner["deleteReason"].is_null(),
        "normally completed catch must keep deleteReason null; got {}",
        winner["deleteReason"]
    );

    // Query endpoint should surface the same field.
    let query = client
        .post(format!("{base_url}/query/historic-activity-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processInstanceId": process_instance_id }))
        .send()
        .await
        .unwrap();
    assert!(query.status().is_success());
    let query_body: Value = query.json().await.unwrap();
    let query_cancelled = query_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["activityId"] == "catchTimer")
        .expect("query cancelled catchTimer");
    assert_eq!(
        query_cancelled["deleteReason"].as_str(),
        Some(EVENT_BASED_GATEWAY_CANCEL)
    );
}
