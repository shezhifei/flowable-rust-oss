use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const CMMN_WITH_FORM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="Examples">
    <case id="approvalCase" name="Approval Case">
        <casePlanModel id="casePlanModel" name="Approval">
            <planItem id="planItem_reviewTask" definitionRef="reviewTask" />
            <humanTask id="reviewTask" name="Review" formKey="reviewForm" />
        </casePlanModel>
    </case>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("cmmn-task-form-test".to_string()));
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

#[tokio::test]
async fn cmmn_task_form_returns_form_payload_for_human_task_with_form_key() {
    let (base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN Task Form Test",
            "resourceName": "approval-case.cmmn",
            "resource": CMMN_WITH_FORM
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let form_response = client
        .post(format!("{base_url}/form-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Review forms",
            "resources": [
                {
                    "resourceName": "review-form.form",
                    "resource": json!({
                        "key": "reviewForm",
                        "name": "Review form",
                        "resourceName": "review-form.form",
                        "fields": [
                            { "id": "approved", "name": "Approved", "type": "boolean", "required": true },
                            { "id": "comment", "name": "Comment", "type": "string" }
                        ]
                    }).to_string()
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert!(form_response.status().is_success());

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "approvalCase"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let case_instance: Value = start_response.json().await.unwrap();
    let case_instance_id = case_instance["id"].as_str().unwrap();

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_instance_id}&state=ACTIVE"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks: Value = tasks_response.json().await.unwrap();
    assert!(
        !tasks["data"].as_array().unwrap().is_empty(),
        "Expected at least one active plan item instance for case {case_instance_id}, got: {tasks}"
    );
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let form_data_response = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}/form"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        form_data_response.status().is_success(),
        "CMMN task form endpoint should return 200, got {}: {}",
        form_data_response.status(),
        form_data_response.text().await.unwrap_or_default()
    );

    let form_data: Value = form_data_response.json().await.unwrap();
    assert_eq!(form_data["key"], "reviewForm");
    assert_eq!(form_data["name"], "Review form");
    assert!(form_data["fields"].is_array());
    assert_eq!(form_data["fields"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn cmmn_task_form_returns_404_for_task_without_form_key() {
    let (base_url, client) = spawn_server().await;

    let cmmn_no_form = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
                 targetNamespace="Examples">
        <case id="noFormCase" name="No Form Case">
            <casePlanModel id="casePlanModel" name="No Form">
                <planItem id="planItem_simpleTask" definitionRef="simpleTask" />
                <humanTask id="simpleTask" name="Simple Task" />
            </casePlanModel>
        </case>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "No Form Case",
            "resourceName": "no-form-case.cmmn",
            "resource": cmmn_no_form
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "noFormCase"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let case_instance: Value = start_response.json().await.unwrap();
    let case_instance_id = case_instance["id"].as_str().unwrap();

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_instance_id}&state=ACTIVE"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks: Value = tasks_response.json().await.unwrap();
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let form_data_response = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}/form"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(form_data_response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cmmn_task_form_returns_404_for_nonexistent_task() {
    let (base_url, client) = spawn_server().await;

    let form_data_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks/nonexistent-task-id/form"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(form_data_response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cmmn_historic_task_form_returns_form_payload() {
    let (base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN Historic Task Form Test",
            "resourceName": "approval-case.cmmn",
            "resource": CMMN_WITH_FORM
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let form_response = client
        .post(format!("{base_url}/form-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Review forms",
            "resources": [
                {
                    "resourceName": "review-form.form",
                    "resource": json!({
                        "key": "reviewForm",
                        "name": "Review form",
                        "resourceName": "review-form.form",
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
    assert!(form_response.status().is_success());

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "approvalCase"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let case_instance: Value = start_response.json().await.unwrap();
    let case_instance_id = case_instance["id"].as_str().unwrap();

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_instance_id}&state=ACTIVE"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let tasks: Value = tasks_response.json().await.unwrap();
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let complete_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_id}/complete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(complete_response.status().is_success());

    let historic_form_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-task-instances/{task_id}/form"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        historic_form_response.status().is_success(),
        "Historic CMMN task form endpoint should return 200, got {}: {}",
        historic_form_response.status(),
        historic_form_response.text().await.unwrap_or_default()
    );

    let form_data: Value = historic_form_response.json().await.unwrap();
    assert_eq!(form_data["key"], "reviewForm");
    assert_eq!(form_data["name"], "Review form");
}
