use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const STAGED_CASE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="stageOverviewCase" name="Stage Overview Case">
    <casePlanModel id="stageOverviewPlan" name="Stage Overview Plan" autoComplete="false">
      <planItem id="planItemStageA" name="Stage A" definitionRef="stageA" />
      <stage id="stageA" name="Stage A">
        <planItem id="planItemReview" name="Review Task" definitionRef="reviewTask" />
        <humanTask id="reviewTask" name="Review Task" isBlocking="true" />
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

async fn deploy_staged_case(base_url: &str, client: &reqwest::Client) {
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Stage overview deployment",
            "resourceName": "stage-overview-case.cmmn",
            "resource": STAGED_CASE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());
}

async fn start_staged_case(base_url: &str, client: &reqwest::Client, business_key: &str) -> String {
    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "stageOverviewCase",
            "businessKey": business_key
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let started_case: Value = start_response.json().await.unwrap();
    started_case["id"].as_str().unwrap().to_string()
}

async fn assert_historic_stage_overview_keeps_stage(
    base_url: &str,
    client: &reqwest::Client,
    case_instance_id: &str,
) {
    let historic_overview = client
        .get(format!(
            "{base_url}/cmmn-history/historic-case-instances/{case_instance_id}/stage-overview"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_overview.status(), reqwest::StatusCode::OK);
    let historic_body: Value = historic_overview.json().await.unwrap();
    assert_eq!(historic_body.as_array().unwrap().len(), 1);
    assert_eq!(historic_body[0]["id"], "stageA");
    assert_eq!(historic_body[0]["name"], "Stage A");
    assert_eq!(historic_body[0]["current"], false);
    assert_eq!(historic_body[0]["ended"], true);
    assert!(historic_body[0]["endTime"].as_str().is_some());
}

#[tokio::test]
async fn cmmn_stage_overview_paths_return_runtime_and_historic_stage_state() {
    let (base_url, client) = spawn_server("rest-cmmn-stage-overview").await;

    deploy_staged_case(&base_url, &client).await;
    let case_instance_id = start_staged_case(&base_url, &client, "stage-overview-1").await;

    let runtime_overview = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/stage-overview"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_overview.status(), reqwest::StatusCode::OK);
    let runtime_body: Value = runtime_overview.json().await.unwrap();
    assert_eq!(runtime_body.as_array().unwrap().len(), 1);
    assert_eq!(runtime_body[0]["id"], "stageA");
    assert_eq!(runtime_body[0]["name"], "Stage A");
    assert_eq!(runtime_body[0]["current"], true);
    assert_eq!(runtime_body[0]["ended"], false);
    assert!(runtime_body[0]["endTime"].is_null());

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(tasks_response.status(), reqwest::StatusCode::OK);
    let tasks: Value = tasks_response.json().await.unwrap();
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let complete_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_id}/complete"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(complete_response.status(), reqwest::StatusCode::OK);

    let historic_overview = client
        .get(format!(
            "{base_url}/cmmn-history/historic-case-instances/{case_instance_id}/stage-overview"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_overview.status(), reqwest::StatusCode::OK);
    let historic_body: Value = historic_overview.json().await.unwrap();
    assert_eq!(historic_body.as_array().unwrap().len(), 1);
    assert_eq!(historic_body[0]["id"], "stageA");
    assert_eq!(historic_body[0]["name"], "Stage A");
    assert_eq!(historic_body[0]["current"], false);
    assert_eq!(historic_body[0]["ended"], true);
    assert!(historic_body[0]["endTime"].as_str().is_some());
}

#[tokio::test]
async fn cmmn_historic_stage_overview_survives_runtime_terminate_with_active_stage() {
    let (base_url, client) = spawn_server("rest-cmmn-stage-overview-terminate").await;
    deploy_staged_case(&base_url, &client).await;
    let case_instance_id = start_staged_case(&base_url, &client, "stage-overview-terminate").await;

    let terminate_response = client
        .delete(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(terminate_response.status(), reqwest::StatusCode::NO_CONTENT);

    assert_historic_stage_overview_keeps_stage(&base_url, &client, &case_instance_id).await;
}

#[tokio::test]
async fn cmmn_historic_stage_overview_survives_runtime_delete_with_active_stage() {
    let (base_url, client) = spawn_server("rest-cmmn-stage-overview-delete").await;
    deploy_staged_case(&base_url, &client).await;
    let case_instance_id = start_staged_case(&base_url, &client, "stage-overview-delete").await;

    let delete_response = client
        .delete(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/delete"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    assert_historic_stage_overview_keeps_stage(&base_url, &client, &case_instance_id).await;
}
