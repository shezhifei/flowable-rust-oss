use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("cmmn-user-event-test".to_string()));
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

const USER_EVENT_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="Examples">
    <case id="userEventCase" name="User Event Case">
        <casePlanModel id="casePlanModel">
            <planItem id="planItem1" name="Review application" definitionRef="humanTask1" />
            <planItem id="planItem2" name="Approve event" definitionRef="userEventListener1" />
            <humanTask id="humanTask1" name="Review application" flowable:formKey="reviewForm" />
            <eventListener id="userEventListener1" name="approveEvent" eventType="user" eventName="approveEvent" />
        </casePlanModel>
    </case>
</definitions>"#;

const USER_EVENT_WITH_SENTRY_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="Examples">
    <case id="userEventVariableCase" name="User Event Variable Case">
        <casePlanModel id="casePlanModel" autoComplete="false">
            <planItem id="planItemApprovalEvent" name="Approval event" definitionRef="approvalEventListener" />
            <planItem id="planItemApproveTask" name="Approve task" definitionRef="approveTask">
                <entryCriterion id="entryApproveTask">
                    <sentryRef>sentryAfterApprovalEvent</sentryRef>
                </entryCriterion>
            </planItem>
            <eventListener id="approvalEventListener" name="Approval event" eventType="user" eventName="approvalReceived" />
            <humanTask id="approveTask" name="Approve task" />
            <sentry id="sentryAfterApprovalEvent">
                <planItemOnPart id="onApprovalEventOccur" sourceRef="planItemApprovalEvent">
                    <standardEvent>occur</standardEvent>
                </planItemOnPart>
                <ifPart>
                    <condition>${approved == true}</condition>
                </ifPart>
            </sentry>
        </casePlanModel>
    </case>
</definitions>"#;

async fn deploy_cmmn(client: &reqwest::Client, base_url: &str) {
    deploy_cmmn_resource(
        client,
        base_url,
        "User Event Test",
        "user-event-case.cmmn",
        USER_EVENT_CMMN,
    )
    .await;
}

async fn deploy_cmmn_resource(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
    resource_name: &str,
    resource: &str,
) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": resource_name,
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

async fn start_case_instance(
    client: &reqwest::Client,
    base_url: &str,
    case_definition_key: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": case_definition_key
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "start case failed: {}",
        response.text().await.unwrap_or_default()
    );
    let case_instance: Value = response.json().await.unwrap();
    case_instance["id"].as_str().unwrap().to_string()
}

async fn plan_item_state(
    client: &reqwest::Client,
    base_url: &str,
    case_id: &str,
    plan_item_definition_id: &str,
) -> Option<String> {
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "list plan items failed: {}",
        response.text().await.unwrap_or_default()
    );
    let plan_items: Value = response.json().await.unwrap();
    plan_items["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["planItemDefinitionId"].as_str() == Some(plan_item_definition_id))
        .and_then(|item| item["state"].as_str())
        .map(str::to_string)
}

async fn event_subscription_count(
    client: &reqwest::Client,
    base_url: &str,
    case_id: &str,
) -> usize {
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/event-subscriptions?caseInstanceId={case_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "list event subscriptions failed: {}",
        response.text().await.unwrap_or_default()
    );
    let subscriptions: Value = response.json().await.unwrap();
    subscriptions["data"].as_array().unwrap().len()
}

async fn case_variable_value(
    client: &reqwest::Client,
    base_url: &str,
    case_id: &str,
    variable_name: &str,
) -> Option<Value> {
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables/{variable_name}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    if !response.status().is_success() {
        return None;
    }
    let variable: Value = response.json().await.unwrap();
    Some(variable["value"].clone())
}

#[tokio::test]
async fn trigger_user_event_returns_204() {
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(&client, &base_url).await;

    // Start a case instance
    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "userEventCase"
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

    // List event subscriptions to verify they exist
    let subs_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/event-subscriptions?caseInstanceId={case_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(subs_response.status().is_success());
    let subs: Value = subs_response.json().await.unwrap();
    let subs_data = subs["data"].as_array().unwrap();
    assert!(!subs_data.is_empty(), "should have event subscriptions");

    // Trigger the user event
    let trigger_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/events"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventName": "approveEvent"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        trigger_response.status().is_success(),
        "trigger event failed: {}",
        trigger_response.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn trigger_user_event_filters_by_type_and_applies_variables_before_sentry() {
    let (base_url, client) = spawn_server().await;
    deploy_cmmn_resource(
        &client,
        &base_url,
        "User Event Variable Test",
        "user-event-variable-case.cmmn",
        USER_EVENT_WITH_SENTRY_CMMN,
    )
    .await;

    let wrong_type_case_id = start_case_instance(&client, &base_url, "userEventVariableCase").await;
    let wrong_type_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{wrong_type_case_id}/events"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventName": "approvalReceived",
            "eventType": "message",
            "variables": [
                { "name": "approved", "value": true }
            ]
        }))
        .send()
        .await
        .unwrap();
    let wrong_type_status = wrong_type_response.status();
    let wrong_type_body = wrong_type_response.text().await.unwrap_or_default();
    let wrong_type_remaining_subscriptions =
        event_subscription_count(&client, &base_url, &wrong_type_case_id).await;
    let wrong_type_task_state = plan_item_state(
        &client,
        &base_url,
        &wrong_type_case_id,
        // Java PlanItemInstanceEntityManagerImpl.java:92-95.
        "approveTask",
    )
    .await;

    let matching_case_id = start_case_instance(&client, &base_url, "userEventVariableCase").await;
    let matching_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{matching_case_id}/events"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventName": "approvalReceived",
            "eventType": "user",
            "variables": [
                { "name": "approved", "value": true }
            ]
        }))
        .send()
        .await
        .unwrap();
    let matching_status = matching_response.status();
    let matching_body = matching_response.text().await.unwrap_or_default();
    // Java PlanItemInstanceEntityManagerImpl.java:92-95.
    let matching_task_state =
        plan_item_state(&client, &base_url, &matching_case_id, "approveTask").await;
    let approved_value =
        case_variable_value(&client, &base_url, &matching_case_id, "approved").await;

    let mut failures = Vec::new();
    if wrong_type_status.as_u16() != 404 {
        failures.push(format!(
            "wrong eventType should return 404, got {wrong_type_status}: {wrong_type_body}"
        ));
    }
    if wrong_type_remaining_subscriptions != 1 {
        failures.push(format!(
            "wrong eventType should leave subscription intact, found {wrong_type_remaining_subscriptions}"
        ));
    }
    if wrong_type_task_state.as_deref() == Some("ACTIVE") {
        failures.push("wrong eventType should not activate the gated task".to_string());
    }
    if matching_status.as_u16() != 204 {
        failures.push(format!(
            "matching user event should return 204, got {matching_status}: {matching_body}"
        ));
    }
    if matching_task_state.as_deref() != Some("ACTIVE") {
        failures.push(format!(
            "matching user event variables should satisfy sentry before occurrence, task state was {matching_task_state:?}"
        ));
    }
    if approved_value != Some(json!(true)) {
        failures.push(format!(
            "trigger variables should be persisted before occurrence, approved was {approved_value:?}"
        ));
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn trigger_user_event_unknown_case_returns_404() {
    let (base_url, client) = spawn_server().await;

    let response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/unknown-case/events"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventName": "approveEvent"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
