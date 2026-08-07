// P102: CMMN case start request — transientVariables / outcome /
// overrideDefinitionTenantId / returnVariables + validation.
//
// Java references:
// - CaseInstanceCreateRequest.java:37-48 (request fields)
// - CaseInstanceCollectionResource.java:314-429 (createCaseInstance)
//   :320-324 (id/key validation), :326-331 (tenantId + id → 400)
//   :357-365 (transient variables), :387-389 (override tenant), :410-416 (returnVariables)

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const CASE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="startCase" name="Start Case">
    <casePlanModel id="startPlan" name="Start Plan" autoComplete="false">
      <planItem id="planItemExpress" definitionRef="expressTask" />
      <humanTask id="expressTask" name="Express task" assignee="${assignee}" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-cmmn-case-start".to_string()));
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

async fn deploy_case(base_url: &str, client: &reqwest::Client) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P102 deployment",
            "resourceName": "start.cmmn",
            "resource": CASE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn cmmn_case_start_validates_id_key_and_tenant() {
    let (base_url, client) = spawn_server().await;
    let deployment_id = deploy_case(&base_url, &client).await;

    // Resolve a real case definition id.
    let defs = client
        .get(format!("{base_url}/cmmn-repository/case-definitions"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let case_definition_id = defs["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|definition| definition["deploymentId"] == deployment_id)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Neither id nor key → 400 (Java :316-318).
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // Both id and key → 400 (Java :320-324).
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionId": case_definition_id,
            "caseDefinitionKey": "startCase"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // tenantId with caseDefinitionId → 400 (Java :326-331).
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionId": case_definition_id,
            "tenantId": "tenant-x"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // Valid start by key returns 201.
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": "startCase" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
}

#[tokio::test]
async fn cmmn_case_start_transient_variables_visible_but_not_persisted() {
    let (base_url, client) = spawn_server().await;
    deploy_case(&base_url, &client).await;

    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "startCase",
            "variables": { "real": "kept" },
            "transientVariables": { "assignee": "alice", "temp": "gone" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let case_id = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The expression-resolved assignee saw the transient variable.
    let tasks = client
        .get(format!("{base_url}/cmmn-runtime/tasks?caseInstanceId={case_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(tasks["data"][0]["assignee"], "alice");

    // Transient variables are not persisted.
    let variables = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let names = variables
        .as_array()
        .unwrap()
        .iter()
        .map(|variable| variable["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["real".to_string()]);
}

#[tokio::test]
async fn cmmn_case_start_return_variables_outcome_and_override_tenant() {
    let (base_url, client) = spawn_server().await;
    deploy_case(&base_url, &client).await;

    // returnVariables=true includes the case variables in the response.
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "startCase",
            "variables": { "priority": "high" },
            "returnVariables": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let body: Value = response.json().await.unwrap();
    let variables = body["variables"].as_array().unwrap();
    let names = variables
        .iter()
        .map(|variable| variable["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["priority".to_string()]);

    // outcome is accepted (dropped without a form engine) → 201.
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "startCase",
            "outcome": "approved"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);

    // overrideDefinitionTenantId sets the case instance tenant.
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "startCase",
            "overrideDefinitionTenantId": "tenant-override"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["tenantId"], "tenant-override");
}
