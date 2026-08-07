//! P74 REST contract: remaining delete reasons exposed on historic activities.
//!
//! Complements engine `p74_delete_reason_contract_test` and P71 REST coverage
//! for event-based gateway cancel. One representative path (interrupting
//! boundary) is exercised via REST to confirm `deleteReason` is serialised.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::history::delete_reason;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const BOUNDARY_INTERRUPT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p74RestBoundaryDeleteReason" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="hostTask" />
        <userTask id="hostTask" name="Host Task" />
        <boundaryEvent id="boundaryCancel" attachedToRef="hostTask" cancelActivity="true">
            <messageEventDefinition messageRef="cancelMsg" />
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="hostTask" targetRef="normalEnd" />
        <sequenceFlow id="f3" sourceRef="boundaryCancel" targetRef="afterBoundary" />
        <userTask id="afterBoundary" name="After Boundary" />
        <sequenceFlow id="f4" sourceRef="afterBoundary" targetRef="end" />
        <endEvent id="normalEnd" />
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
async fn boundary_interrupt_exposes_delete_reason_on_historic_activity_rest() {
    let (engine, base_url, client) = spawn_server("rest-p74-boundary-delete-reason").await;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .add_string(
                    "p74_rest_boundary_delete_reason.bpmn20.xml".to_string(),
                    BOUNDARY_INTERRUPT_XML.to_string(),
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

    engine
        .get_runtime_service()
        .trigger_boundary_event("boundaryCancel".to_string(), process_instance_id.clone())
        .unwrap();

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

    let expected = delete_reason::boundary_event_interrupting("boundaryCancel");
    let host = post_data
        .iter()
        .find(|a| a["activityId"] == "hostTask")
        .expect("historic activity for interrupted hostTask");
    assert_eq!(
        host["deleteReason"].as_str(),
        Some(expected.as_str()),
        "REST deleteReason must match Java boundary interrupt string"
    );
    assert!(
        host["endTime"].is_string(),
        "interrupted host must have endTime set"
    );

    let query = client
        .post(format!("{base_url}/query/historic-activity-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processInstanceId": process_instance_id }))
        .send()
        .await
        .unwrap();
    assert!(query.status().is_success());
    let query_body: Value = query.json().await.unwrap();
    let query_host = query_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["activityId"] == "hostTask")
        .expect("query hostTask");
    assert_eq!(
        query_host["deleteReason"].as_str(),
        Some(expected.as_str())
    );
}
