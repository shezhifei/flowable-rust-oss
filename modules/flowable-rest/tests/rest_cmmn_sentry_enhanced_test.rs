use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("cmmn-sentry-enhanced-test".to_string()));
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

const SENTRY_ENHANCED_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="Examples">
    <case id="sentryEnhancedCase" name="Sentry Enhanced Case">
        <casePlanModel id="casePlanModel">
            <planItem id="planItemA" name="Task A" definitionRef="humanTaskA" />
            <planItem id="planItemB" name="Task B" definitionRef="humanTaskB">
                <entryCriterion id="entryB">
                    <sentryRef>sentryB</sentryRef>
                </entryCriterion>
            </planItem>
            <humanTask id="humanTaskA" name="Task A" />
            <humanTask id="humanTaskB" name="Task B" />
            <sentry id="sentryB">
                <planItemOnPart id="onPartA" sourceRef="planItemA">
                    <standardEvent>complete</standardEvent>
                </planItemOnPart>
                <ifPart>
                    <condition>${(customer.age + 1 >= minAge) &amp;&amp; items[0].status == 'open' &amp;&amp; (approved ? true : false)}</condition>
                </ifPart>
            </sentry>
        </casePlanModel>
    </case>
</definitions>"#;

const MANUAL_ACTIVATION_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="Examples">
    <case id="manualActivationCase" name="Manual Activation Case">
        <casePlanModel id="casePlanModel">
            <planItem id="planItemA" name="Decision A" definitionRef="decisionTaskA">
                <itemControl>
                    <manualActivationRule>
                        <condition>${approved}</condition>
                    </manualActivationRule>
                </itemControl>
            </planItem>
            <decisionTask id="decisionTaskA" name="Decision A" decisionRef="myDecision" />
        </casePlanModel>
    </case>
</definitions>"#;

async fn deploy_cmmn(client: &reqwest::Client, base_url: &str, resource: &str, name: &str) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": "test-case.cmmn",
            "resource": resource
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
async fn test_advanced_sentry_evaluation() {
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(
        &client,
        &base_url,
        SENTRY_ENHANCED_CMMN,
        "Sentry Enhanced Test",
    )
    .await;

    // Start a case instance with variables satisfying the condition
    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "sentryEnhancedCase",
            "variables": {
                "customer": {
                    "age": 17
                },
                "minAge": 18,
                "items": [
                    { "status": "open" }
                ],
                "approved": true
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let case_instance: Value = start_response.json().await.unwrap();
    let case_id = case_instance["id"].as_str().unwrap();

    // Query active plan items, task A should be active, task B should be waiting
    let query_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(query_response.status().is_success());
    let plan_items: Value = query_response.json().await.unwrap();
    let plan_items_data = plan_items["data"].as_array().unwrap();

    let task_a = plan_items_data
        .iter()
        // Java PlanItemInstanceEntityManagerImpl.java:92-95 keeps the plan item
        // XML id (`elementId`) distinct from its definitionRef target.
        .find(|t| t["planItemDefinitionId"].as_str() == Some("humanTaskA"))
        .unwrap();
    assert_eq!(task_a["state"].as_str().unwrap(), "ACTIVE");

    let task_a_id = task_a["id"].as_str().unwrap();

    // Complete Task A
    let complete_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_a_id}/complete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(complete_response.status().is_success());

    // Query again. Since condition (17 + 1 >= 18) && ('open' == 'open') && (true) is true, Task B must become ACTIVE.
    let query_response_2 = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(query_response_2.status().is_success());
    let plan_items_2: Value = query_response_2.json().await.unwrap();
    let plan_items_data_2 = plan_items_2["data"].as_array().unwrap();

    let task_b_after = plan_items_data_2
        .iter()
        // Java PlanItemInstanceEntityManagerImpl.java:92-95.
        .find(|t| t["planItemDefinitionId"].as_str() == Some("humanTaskB"))
        .unwrap();
    assert_eq!(task_b_after["state"].as_str().unwrap(), "ACTIVE");
}

#[tokio::test]
async fn test_manual_activation_on_decision_task_deploys() {
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(
        &client,
        &base_url,
        MANUAL_ACTIVATION_CMMN,
        "Manual Activation Test",
    )
    .await;
}
