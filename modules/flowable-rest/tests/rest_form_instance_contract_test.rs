//! REST form instance lifecycle extensions + query tenant/values contract.
//!
//! Values GET and instance DELETE are Rust-owned Form REST extensions
//! (Java FormService engine APIs; no stable Java Form REST path in workspace
//! truth sources). Query tenant filters run at the engine layer.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;

const PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="formInstanceLifecycleProcess" name="Form Instance Lifecycle" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="travelRequest" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="approveTask" />
        <userTask id="approveTask" name="Approve" flowable:formKey="expenseApproval" />
        <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-form-instance".to_string()));
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

async fn deploy_forms(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/form-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Lifecycle forms",
            "resources": [
                {
                    "resourceName": "travel-request.form",
                    "resource": json!({
                        "key": "travelRequest",
                        "name": "Travel request",
                        "resourceName": "travel-request.form",
                        "fields": [
                            { "id": "requester", "name": "Requester", "type": "string", "required": true },
                            { "id": "amount", "name": "Amount", "type": "number", "required": true }
                        ]
                    }).to_string()
                },
                {
                    "resourceName": "expense-approval.form",
                    "resource": json!({
                        "key": "expenseApproval",
                        "name": "Expense approval",
                        "resourceName": "expense-approval.form",
                        "fields": [
                            { "id": "approved", "name": "Approved", "type": "boolean", "required": true }
                        ]
                    }).to_string()
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
}

#[tokio::test]
async fn form_instance_values_and_delete_lifecycle_extension() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_forms(&client, &base_url).await;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Form instance lifecycle process".to_string())
                .tenant_id("tenant-form".to_string())
                .add_string(
                    "form-instance-lifecycle.bpmn20.xml".to_string(),
                    PROCESS_BPMN.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("formInstanceLifecycleProcess", Some("tenant-form"))
        .unwrap()
        .unwrap()
        .id;

    let start = client
        .post(format!("{base_url}/form/form-data"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "fi-1",
            "properties": [
                { "id": "requester", "value": "bob" },
                { "id": "amount", "value": 42 }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start.status(), reqwest::StatusCode::OK);

    let list = client
        .get(format!(
            "{base_url}/form/form-instances?tenantId=tenant-form"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let list_body: Value = list.json().await.unwrap();
    assert_eq!(list_body["total"], 1);
    let form_instance_id = list_body["data"][0]["id"].as_str().unwrap().to_string();
    assert!(
        list_body["data"][0]["formValuesId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert_eq!(list_body["data"][0]["tenantId"], "tenant-form");

    // Rust-owned extension: GET values bytes
    let values = client
        .get(format!(
            "{base_url}/form/form-instances/{form_instance_id}/values"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(values.status(), reqwest::StatusCode::OK);
    let values_body: Value = values.json().await.unwrap();
    assert_eq!(values_body["requester"], "bob");

    // Rust-owned extension: DELETE instance
    let delete = client
        .delete(format!(
            "{base_url}/form/form-instances/{form_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let missing = client
        .get(format!(
            "{base_url}/form/form-instances/{form_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}
