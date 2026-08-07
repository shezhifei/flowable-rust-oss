//! P140 — CMMN historic plan-item-instance response fields: `elementId`,
//! `planItemDefinitionType`, and `stageInstanceId`.
//!
//! After P131, `planItemDefinitionId` holds the definitionRef target (not the
//! plan item XML id). Historic responses previously had no migration path for
//! the old value; this test pins Java parity with
//! `HistoricPlanItemInstanceResponse.java:39-43` and the P131 migration path.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const P140_REST_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="p140PlainCase" name="P140 plain case">
    <casePlanModel id="p140PlainPlan" name="P140 plain plan" autoComplete="true">
      <planItem id="planItemReview" name="Review application" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review application" />
    </casePlanModel>
  </case>
  <case id="p140StageCase" name="P140 stage case">
    <casePlanModel id="p140StagePlan" name="P140 stage plan" autoComplete="true">
      <planItem id="planItemStage" name="Stage A" definitionRef="stageA" />
      <stage id="stageA" name="Stage A" autoComplete="true">
        <planItem id="planItemInner" name="Inner Task" definitionRef="innerTask" />
        <humanTask id="innerTask" name="Inner Task" />
      </stage>
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server(test_name: &str) -> (String, reqwest::Client) {
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
    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

async fn get_ok(client: &reqwest::Client, base_url: &str, path_and_query: &str) -> Value {
    let response = client
        .get(format!("{base_url}{path_and_query}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "GET {path_and_query} must be accepted, got {status}: {body}"
    );
    serde_json::from_str(&body).unwrap()
}

async fn deploy(base_url: &str, client: &reqwest::Client) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P140 historic element id deployment",
            "resourceName": "p140-rest.cmmn",
            "resource": P140_REST_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::CREATED,
        "deployment failed: {}",
        response.text().await.unwrap()
    );
}

async fn start_case(client: &reqwest::Client, base_url: &str, case_definition_key: &str) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": case_definition_key }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::CREATED,
        "start case failed: {}",
        response.text().await.unwrap()
    );
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn active_task_id(client: &reqwest::Client, base_url: &str, case_instance_id: &str) -> String {
    let body = get_ok(
        client,
        base_url,
        &format!("/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"),
    )
    .await;
    body["data"][0]["id"].as_str().unwrap().to_string()
}

async fn complete_task(client: &reqwest::Client, base_url: &str, task_id: &str) {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "task completion failed: {}",
        response.text().await.unwrap()
    );
}

fn historic_human_task_row(data: &[Value], element_id: &str) -> Value {
    data.iter()
        .find(|row| {
            row["elementId"].as_str() == Some(element_id)
                && row["planItemDefinitionType"].as_str() == Some("humantask")
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "expected historic humantask with elementId={element_id}, got: {data:?}"
            )
        })
}

/// Pins P131 correction + P140 migration path on a completed plain human task:
/// `elementId` is the plan item XML id, `planItemDefinitionId` is the
/// definitionRef target, and the two must differ. Also pins
/// `planItemDefinitionType == "humantask"`.
#[tokio::test]
async fn historic_plan_item_exposes_element_id_and_definition_type() {
    let (base_url, client) = spawn_server("p140-historic-element-id").await;
    deploy(&base_url, &client).await;

    let case_id = start_case(&client, &base_url, "p140PlainCase").await;
    let task_id = active_task_id(&client, &base_url, &case_id).await;
    complete_task(&client, &base_url, &task_id).await;

    let body = get_ok(
        &client,
        &base_url,
        &format!("/cmmn-history/historic-plan-item-instances?caseInstanceId={case_id}"),
    )
    .await;
    let data = body["data"].as_array().expect("data array");
    let row = historic_human_task_row(data, "planItemReview");

    // One assertion family nails both P131 (definitionRef target) and P140
    // (old plan-item XML id now on elementId) — the values must be distinct.
    assert_eq!(row["elementId"].as_str(), Some("planItemReview"));
    assert_eq!(row["planItemDefinitionId"].as_str(), Some("reviewTask"));
    assert_ne!(
        row["elementId"].as_str(),
        row["planItemDefinitionId"].as_str(),
        "elementId (plan item XML id) must differ from planItemDefinitionId (definitionRef target)"
    );
    assert_eq!(row["planItemDefinitionType"].as_str(), Some("humantask"));
}

/// Stage-nested human task must surface a non-empty `stageInstanceId`
/// (HistoricPlanItemInstanceResponse.java:39).
#[tokio::test]
async fn historic_plan_item_stage_nested_task_has_stage_instance_id() {
    let (base_url, client) = spawn_server("p140-historic-stage-element-id").await;
    deploy(&base_url, &client).await;

    let case_id = start_case(&client, &base_url, "p140StageCase").await;
    let task_id = active_task_id(&client, &base_url, &case_id).await;
    complete_task(&client, &base_url, &task_id).await;

    let body = get_ok(
        &client,
        &base_url,
        &format!("/cmmn-history/historic-plan-item-instances?caseInstanceId={case_id}"),
    )
    .await;
    let data = body["data"].as_array().expect("data array");
    let row = historic_human_task_row(data, "planItemInner");

    assert_eq!(row["elementId"].as_str(), Some("planItemInner"));
    assert_eq!(row["planItemDefinitionId"].as_str(), Some("innerTask"));
    assert_ne!(
        row["elementId"].as_str(),
        row["planItemDefinitionId"].as_str()
    );
    assert_eq!(row["planItemDefinitionType"].as_str(), Some("humantask"));

    let stage_instance_id = row["stageInstanceId"]
        .as_str()
        .expect("stage-nested historic humantask must expose stageInstanceId");
    assert!(
        !stage_instance_id.is_empty(),
        "stageInstanceId must be non-empty for a stage-nested task"
    );
}
