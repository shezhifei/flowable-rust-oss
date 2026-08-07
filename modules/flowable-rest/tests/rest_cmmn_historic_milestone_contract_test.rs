use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const MILESTONE_CASE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="milestoneCase" name="Milestone Case">
    <casePlanModel id="milestonePlan" name="Milestone Plan" autoComplete="true">
      <planItem id="planItemReached" name="Reached Milestone" definitionRef="reachedMilestone" />
      <milestone id="reachedMilestone" name="Reached Milestone" />
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

#[tokio::test]
async fn cmmn_historic_milestone_paths_return_reached_milestones() {
    let (base_url, client) = spawn_server("rest-cmmn-historic-milestone").await;

    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Milestone deployment",
            "resourceName": "milestone-case.cmmn",
            "resource": MILESTONE_CASE_CMMN
        }))
        .send()
        .await
        .unwrap();
    let deploy_status = deploy_response.status();
    let deploy_body = deploy_response.text().await.unwrap();
    assert!(
        deploy_status.is_success(),
        "deployment failed: {deploy_body}"
    );

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "milestoneCase",
            "businessKey": "milestone-1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), reqwest::StatusCode::CREATED);
    let started_case: Value = start_response.json().await.unwrap();
    let case_instance_id = started_case["id"].as_str().unwrap();

    let list_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-milestone-instances?caseInstanceId={case_instance_id}&milestoneId=reachedMilestone&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["total"], 1);
    assert_eq!(list_body["data"][0]["caseInstanceId"], case_instance_id);
    assert_eq!(list_body["data"][0]["milestoneId"], "reachedMilestone");
    assert_eq!(list_body["data"][0]["name"], "Reached Milestone");
    assert_eq!(list_body["data"][0]["caseDefinitionKey"], "milestoneCase");
    assert!(list_body["data"][0]["time"].as_str().is_some());
    let milestone_instance_id = list_body["data"][0]["id"].as_str().unwrap();
    let reached_time = list_body["data"][0]["time"].as_str().unwrap();

    let name_filtered = client
        .get(format!(
            "{base_url}/cmmn-history/historic-milestone-instances?caseInstanceId={case_instance_id}&milestoneName=Reached%20Milestone&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(name_filtered.status(), reqwest::StatusCode::OK);
    let name_filtered_body: Value = name_filtered.json().await.unwrap();
    assert_eq!(name_filtered_body["total"], 1);
    assert_eq!(name_filtered_body["data"][0]["id"], milestone_instance_id);

    let before_filtered = client
        .get(format!(
            "{base_url}/cmmn-history/historic-milestone-instances?caseInstanceId={case_instance_id}&reachedBefore=2999-01-01T00:00:00Z&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(before_filtered.status(), reqwest::StatusCode::OK);
    let before_filtered_body: Value = before_filtered.json().await.unwrap();
    assert_eq!(before_filtered_body["total"], 1);

    let after_filtered = client
        .get(format!(
            "{base_url}/cmmn-history/historic-milestone-instances?caseInstanceId={case_instance_id}&reachedAfter=1970-01-01&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(after_filtered.status(), reqwest::StatusCode::OK);
    let after_filtered_body: Value = after_filtered.json().await.unwrap();
    assert_eq!(after_filtered_body["total"], 1);

    let excluded = client
        .get(format!(
            "{base_url}/cmmn-history/historic-milestone-instances?caseInstanceId={case_instance_id}&reachedBefore=1970-01-01&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(excluded.status(), reqwest::StatusCode::OK);
    let excluded_body: Value = excluded.json().await.unwrap();
    assert_eq!(excluded_body["total"], 0);

    let get_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-milestone-instances/{milestone_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["id"], milestone_instance_id);
    assert_eq!(get_body["caseInstanceId"], case_instance_id);
    assert_eq!(get_body["milestoneId"], "reachedMilestone");

    let query_response = client
        .post(format!(
            "{base_url}/cmmn-query/historic-milestone-instances"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseInstanceId": case_instance_id,
            "milestoneId": "reachedMilestone",
            "milestoneName": "Reached Milestone",
            "reachedAfter": "1970-01-01",
            "reachedBefore": reached_time,
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(query_response.status(), reqwest::StatusCode::OK);
    let query_body: Value = query_response.json().await.unwrap();
    assert_eq!(query_body["total"], 0);

    let invalid_date = client
        .get(format!(
            "{base_url}/cmmn-history/historic-milestone-instances?reachedBefore=not-a-date"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_date.status(), reqwest::StatusCode::BAD_REQUEST);
    let invalid_date_body: Value = invalid_date.json().await.unwrap();
    assert_eq!(invalid_date_body["code"], "BAD_REQUEST");
}
