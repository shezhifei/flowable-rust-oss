// P103: CMMN POST variable conditions —
// case `variables`, plan-item `caseInstanceVariables`/`variables`,
// task `taskVariables`.
//
// Java:
// - QueryVariable.java:74-96 (operation enum)
// - CaseInstanceQueryRequest.java:75 + BaseCaseInstanceResource.java:204-206, :292-376
// - PlanItemInstanceQueryRequest.java:49-50 + PlanItemInstanceBaseResource.java:118-124
// - TaskQueryRequest.java:87 + TaskBaseResource.java:308-310, :360-444
// - processVariables: NOT on CMMN TaskQueryRequest (TaskQueryRequest.java:32-90)

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const CASE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="p103VarCase" name="P103 Variable Case">
    <casePlanModel id="p103Plan" name="P103 Plan" autoComplete="false">
      <planItem id="planItemA" definitionRef="taskA" />
      <humanTask id="taskA" name="Review A" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-cmmn-var-query".to_string()));
    engine.get_identity_service().save_user(User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });
    (base_url, reqwest::Client::new())
}

async fn deploy_and_start_two(base_url: &str, client: &reqwest::Client) -> (String, String) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "p103-var-query",
            "resourceName": "p103.cmmn",
            "resource": CASE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    let hit = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "p103VarCase",
            "name": "HitCase",
            "variables": {
                "amount": 100,
                "label": "Alpha",
                "flag": true
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(hit.status().is_success(), "{}", hit.text().await.unwrap());
    let hit_id = hit.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let miss = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "p103VarCase",
            "name": "MissCase",
            "variables": {
                "amount": 10,
                "label": "Beta"
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(miss.status().is_success());
    let miss_id = miss.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    (hit_id, miss_id)
}

fn data_ids(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn case_post_variables_hit_miss_and_operation_samples() {
    let (base_url, client) = spawn_server().await;
    let (hit_id, _miss_id) = deploy_and_start_two(&base_url, &client).await;

    // equals hit.
    let response = client
        .post(format!("{base_url}/cmmn-query/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [{ "name": "amount", "operation": "equals", "value": 100 }]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(data_ids(&body), vec![hit_id.clone()]);

    // equals miss.
    let response = client
        .post(format!("{base_url}/cmmn-query/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [{ "name": "amount", "operation": "equals", "value": 999 }]
        }))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert!(data_ids(&body).is_empty());

    // greaterThan.
    let response = client
        .post(format!("{base_url}/cmmn-query/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [{ "name": "amount", "operation": "greaterThan", "value": 50 }]
        }))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(data_ids(&body), vec![hit_id.clone()]);

    // like.
    let response = client
        .post(format!("{base_url}/cmmn-query/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [{ "name": "label", "operation": "like", "value": "Alp%" }]
        }))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(data_ids(&body), vec![hit_id.clone()]);

    // equalsIgnoreCase.
    let response = client
        .post(format!("{base_url}/cmmn-query/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [{ "name": "label", "operation": "equalsIgnoreCase", "value": "alpha" }]
        }))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(data_ids(&body), vec![hit_id]);
}

#[tokio::test]
async fn case_post_variables_illegal_operation_and_nameless_non_equals_return_400() {
    let (base_url, client) = spawn_server().await;
    deploy_and_start_two(&base_url, &client).await;

    let response = client
        .post(format!("{base_url}/cmmn-query/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [{ "name": "amount", "operation": "bogusOp", "value": 1 }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // nameLess non-equals → 400 (BaseCaseInstanceResource.java:306-308).
    let response = client
        .post(format!("{base_url}/cmmn-query/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [{ "operation": "notEquals", "value": 1 }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // nameLess equals is accepted (value-only query).
    let response = client
        .post(format!("{base_url}/cmmn-query/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [{ "operation": "equals", "value": 100 }]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn plan_item_case_instance_variables_hit_and_local_variables_always_empty() {
    let (base_url, client) = spawn_server().await;
    let (hit_id, _miss_id) = deploy_and_start_two(&base_url, &client).await;

    // caseInstanceVariables equals → plan items of the hit case only.
    let response = client
        .post(format!("{base_url}/cmmn-query/plan-item-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseInstanceVariables": [
                { "name": "amount", "operation": "equals", "value": 100 }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let case_ids: Vec<_> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["caseInstanceId"].as_str().unwrap().to_string())
        .collect();
    assert!(!case_ids.is_empty());
    assert!(case_ids.iter().all(|id| id == &hit_id));

    // Local plan-item `variables` → empty-local convention → empty result.
    let response = client
        .post(format!("{base_url}/cmmn-query/plan-item-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                { "name": "amount", "operation": "equals", "value": 100 }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert!(
        body["data"].as_array().unwrap().is_empty(),
        "local plan-item variables are empty → no match"
    );
}

#[tokio::test]
async fn task_task_variables_always_empty_and_illegal_op_400() {
    let (base_url, client) = spawn_server().await;
    deploy_and_start_two(&base_url, &client).await;

    // taskVariables → empty-local convention → empty result
    // (TaskBaseResource.java:308-310; Rust has no task-local store).
    let response = client
        .post(format!("{base_url}/cmmn-query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskVariables": [
                { "name": "amount", "operation": "equals", "value": 100 }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert!(body["data"].as_array().unwrap().is_empty());

    // Without taskVariables, tasks are returned.
    let response = client
        .post(format!("{base_url}/cmmn-query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert!(!body["data"].as_array().unwrap().is_empty());

    // Illegal operation on taskVariables → 400.
    let response = client
        .post(format!("{base_url}/cmmn-query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskVariables": [
                { "name": "amount", "operation": "regex", "value": "x" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // processVariables is not a CMMN TaskQueryRequest field → deny_unknown_fields 400.
    let response = client
        .post(format!("{base_url}/cmmn-query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processVariables": [
                { "name": "amount", "operation": "equals", "value": 100 }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        400,
        "CMMN TaskQueryRequest has no processVariables filter field"
    );
}
