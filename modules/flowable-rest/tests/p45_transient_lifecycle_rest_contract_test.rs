//! P45 REST: transient variables must not appear on collection GET after the
//! command that wrote them commits (Java VariableScopeImpl pure-memory parity).
//!
//! Also guards internal marker names such as `__flowable_pending_future_id`
//! from leaking into `/runtime/process-instances/{id}/variables`.

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

fn one_task_xml(process_id: &str) -> String {
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
    assert!(response.status().is_success(), "deploy failed");
}

async fn variable_names(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> Vec<String> {
    let response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "variables GET failed: {}",
        response.status()
    );
    let body: Value = response.json().await.unwrap();
    body.as_array()
        .expect("variables collection is an array")
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn rest_collection_get_excludes_start_transient_variables() {
    let (_engine, base_url, client) = spawn_server("p45-rest-transient-start").await;
    deploy_xml(
        &client,
        &base_url,
        "p45RestTransient",
        one_task_xml("p45RestTransient"),
    )
    .await;

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "p45RestTransient",
            "variables": [
                { "name": "keepMe", "type": "string", "value": "durable" }
            ],
            "transientVariables": [
                { "name": "ghost", "type": "string", "value": "ephemeral" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let pi_id = body["id"].as_str().unwrap();

    let names = variable_names(&client, &base_url, pi_id).await;
    assert!(
        names.contains(&"keepMe".to_string()),
        "durable start variable must appear in REST collection: {names:?}"
    );
    assert!(
        !names.contains(&"ghost".to_string()),
        "transient start variable must not leak into REST collection GET: {names:?}"
    );
}

#[tokio::test]
async fn rest_collection_get_excludes_pending_future_id_marker() {
    // Recon gap #1 point 4: PENDING_FUTURE_ID_VARIABLE written as transient
    // must not surface on REST collection GET after the writing command commits.
    let (_engine, base_url, client) = spawn_server("p45-rest-pending-future").await;
    deploy_xml(
        &client,
        &base_url,
        "p45PendingFuture",
        one_task_xml("p45PendingFuture"),
    )
    .await;

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "p45PendingFuture",
            "variables": [
                { "name": "ok", "type": "string", "value": "1" }
            ],
            "transientVariables": [
                {
                    "name": "__flowable_pending_future_id",
                    "type": "string",
                    "value": "future-should-not-leak"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let pi_id = body["id"].as_str().unwrap();

    let names = variable_names(&client, &base_url, pi_id).await;
    assert!(
        !names.iter().any(|n| n == "__flowable_pending_future_id"),
        "PENDING_FUTURE_ID marker must not appear on REST variables GET: {names:?}"
    );
    assert!(names.contains(&"ok".to_string()));
}

#[tokio::test]
async fn rest_durable_put_not_shadowed_by_prior_transient() {
    let (_engine, base_url, client) = spawn_server("p45-rest-shadow").await;
    deploy_xml(
        &client,
        &base_url,
        "p45Shadow",
        one_task_xml("p45Shadow"),
    )
    .await;

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "p45Shadow",
            "transientVariables": [
                { "name": "shared", "type": "string", "value": "from-transient" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let pi_id = body["id"].as_str().unwrap();

    // Create durable variable with the same name after start.
    let put = client
        .post(format!(
            "{base_url}/runtime/process-instances/{pi_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "shared", "type": "string", "value": "from-durable" }
        ]))
        .send()
        .await
        .unwrap();
    assert!(
        put.status().is_success(),
        "durable create should succeed: {}",
        put.status()
    );

    let get = client
        .get(format!(
            "{base_url}/runtime/process-instances/{pi_id}/variables/shared"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get.status().is_success());
    let var: Value = get.json().await.unwrap();
    assert_eq!(
        var["value"],
        json!("from-durable"),
        "durable write must not be shadowed by a stripped prior transient"
    );
}
