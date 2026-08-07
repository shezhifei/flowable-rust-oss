use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("cmmn-reactivate-test".to_string()));
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

const REACTIVATE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="Examples">
    <case id="reactivateCase" name="Reactivate Case">
        <casePlanModel id="casePlanModel">
            <planItem id="planItem1" name="Task 1" definitionRef="humanTask1" />
            <planItem id="planItem2" name="Task 2" definitionRef="humanTask2" />
            <humanTask id="humanTask1" name="Task 1" />
            <humanTask id="humanTask2" name="Task 2" />
        </casePlanModel>
    </case>
</definitions>"#;

async fn deploy_cmmn(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Reactivate Test",
            "resourceName": "reactivate-case.cmmn",
            "resource": REACTIVATE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "deploy failed: {}",
        response.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn reactivate_completed_plan_item_returns_200() {
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(&client, &base_url).await;

    // Start a case instance
    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "reactivateCase"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        start_response.status().is_success(),
        "start case failed: {}",
        start_response.text().await.unwrap_or_default()
    );
    let case_instance: Value = start_response.json().await.unwrap();
    let case_id = case_instance["id"].as_str().unwrap();

    // List plan item instances to get the task ID
    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks: Value = tasks_response.json().await.unwrap();
    let tasks_data = tasks["data"].as_array().unwrap();
    assert!(!tasks_data.is_empty(), "should have plan item instances");
    let task_id = tasks_data[0]["id"].as_str().unwrap();

    // Complete the task
    let complete_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_id}/complete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(
        complete_response.status().is_success(),
        "complete failed: {}",
        complete_response.text().await.unwrap_or_default()
    );

    // Reactivate the task
    let reactivate_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_id}/reactivate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(
        reactivate_response.status().is_success(),
        "reactivate failed: {}",
        reactivate_response.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn reactivate_unknown_plan_item_returns_404() {
    let (base_url, client) = spawn_server().await;

    let response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/unknown-task/reactivate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
