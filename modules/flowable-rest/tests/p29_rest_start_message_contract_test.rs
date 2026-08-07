//! P29: REST start process instance by `message` + `transientVariables`.
//!
//! Java evidence (ProcessInstanceCollectionResource.java):
//! - :320-322 either processDefinitionId, processDefinitionKey or message is required
//! - :324-328 only one of the three may be set
//! - :381-382 message starts via ProcessInstanceBuilder.messageName
//! - :360-368,402-403 transientVariables accepted alongside variables
//! - :439-441 FlowableObjectNotFoundException surfaces as 400 (illegal argument)

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

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

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

fn message_start_process_xml(process_id: &str, message_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
            <message id="startMsg" name="{message_name}" />
            <process id="{process_id}" name="{process_id}" isExecutable="true">
                <startEvent id="msgStart">
                    <messageEventDefinition messageRef="startMsg" />
                </startEvent>
                <sequenceFlow id="flow1" sourceRef="msgStart" targetRef="task1" />
                <userTask id="task1" name="Task 1" />
                <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
                <endEvent id="end" />
            </process>
        </definitions>"#
    )
}

fn one_task_process_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
            <process id="{process_id}" name="{process_id}" isExecutable="true">
                <startEvent id="start" />
                <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
                <userTask id="task1" name="Task 1" />
                <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
                <endEvent id="end" />
            </process>
        </definitions>"#
    )
}

async fn deploy_xml(client: &reqwest::Client, base_url: &str, resource_name: &str, xml: String) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{resource_name} deployment"),
            "resourceName": format!("{resource_name}.bpmn20.xml"),
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn start_by_message_starts_message_start_event_instance() {
    // Java ProcessInstanceCollectionResource.java:381-382.
    let (engine, base_url, client) = spawn_server("p29-rest-start-by-message").await;
    deploy_xml(
        &client,
        &base_url,
        "msgStartProcess",
        message_start_process_xml("msgStartProcess", "newInvoice"),
    )
    .await;

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "message": "newInvoice",
            "businessKey": "BK-MSG-1",
            "returnVariables": true,
            "variables": [
                { "name": "amount", "type": "integer", "value": 42 }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["businessKey"], "BK-MSG-1");
    assert_eq!(body["variables"].as_array().unwrap().len(), 1);
    assert_eq!(body["variables"][0]["name"], "amount");
    assert_eq!(body["variables"][0]["value"], 42);

    // The started instance belongs to the message-start process definition.
    let pi_id = body["id"].as_str().unwrap().to_string();
    let identity_links_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{pi_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(identity_links_response.status().is_success());
    let identity_links: Value = identity_links_response.json().await.unwrap();
    assert!(
        identity_links
            .as_array()
            .unwrap()
            .iter()
            .any(|link| { link["user"] == "admin" && link["type"] == "starter" })
    );

    let expected_definition = engine
        .get_repository_service()
        .latest_process_definition_by_key("msgStartProcess", None)
        .unwrap()
        .expect("definition should exist");
    assert_eq!(body["processDefinitionId"], expected_definition.id.as_str());

    // Persistent start variable is applied.
    let amount = engine
        .get_runtime_service()
        .get_variable(pi_id, "amount".to_string())
        .unwrap();
    assert_eq!(amount, Some(json!(42)));
}

#[tokio::test]
async fn start_by_unknown_message_returns_400() {
    // Java ProcessInstanceCollectionResource.java:439-441: object-not-found
    // during start is rethrown as an illegal argument (400), not 404.
    let (_engine, base_url, client) = spawn_server("p29-rest-start-unknown-message").await;

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "message": "noSuchMessage" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn start_with_message_and_key_together_rejected() {
    // Java ProcessInstanceCollectionResource.java:324-328.
    let (_engine, base_url, client) = spawn_server("p29-rest-start-message-key-conflict").await;

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "message": "newInvoice",
            "processDefinitionKey": "someKey"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("Only one of processDefinitionId, processDefinitionKey or message")
    );
}

#[tokio::test]
async fn start_without_id_key_or_message_rejected_with_java_message() {
    // Java ProcessInstanceCollectionResource.java:320-322.
    let (_engine, base_url, client) = spawn_server("p29-rest-start-nothing-set").await;

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("Either processDefinitionId, processDefinitionKey or message is required")
    );
}

#[tokio::test]
async fn transient_variables_accepted_and_excluded_from_return_variables() {
    // Java ProcessInstanceCollectionResource.java:360-368,402-403; the
    // returnVariables response reads back persistent variables only (:416-423).
    let (engine, base_url, client) = spawn_server("p29-rest-start-transient").await;
    deploy_xml(
        &client,
        &base_url,
        "transientStartProcess",
        one_task_process_xml("transientStartProcess"),
    )
    .await;

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "transientStartProcess",
            "returnVariables": true,
            "variables": [
                { "name": "persistentVar", "type": "string", "value": "keep" }
            ],
            "transientVariables": [
                { "name": "transientVar", "type": "string", "value": "temp" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let names: Vec<&str> = body["variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["persistentVar"]);

    // The persistent variable is stored on the instance.
    let pi_id = body["id"].as_str().unwrap().to_string();
    let value = engine
        .get_runtime_service()
        .get_variable(pi_id, "persistentVar".to_string())
        .unwrap();
    assert_eq!(value, Some(json!("keep")));
}

#[tokio::test]
async fn start_by_message_with_transient_variables_succeeds() {
    // Java combination: builder.messageName + transientVariables
    // (ProcessInstanceCollectionResource.java:381-382,402-403).
    let (engine, base_url, client) = spawn_server("p29-rest-start-message-transient").await;
    deploy_xml(
        &client,
        &base_url,
        "msgTransientProcess",
        message_start_process_xml("msgTransientProcess", "orderReceived"),
    )
    .await;

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "message": "orderReceived",
            "transientVariables": [
                { "name": "payload", "type": "string", "value": "raw" }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let expected_definition = engine
        .get_repository_service()
        .latest_process_definition_by_key("msgTransientProcess", None)
        .unwrap()
        .expect("definition should exist");
    assert_eq!(body["processDefinitionId"], expected_definition.id.as_str());
}
