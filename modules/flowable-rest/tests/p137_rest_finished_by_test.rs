//! P137 — REST historic CMMN `finishedBy` query plumbing without auth context.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="p137FinishedByRestCase" name="P137 finished-by REST case">
    <casePlanModel id="p137Plan" name="P137 plan">
      <planItem id="reviewPlanItem" name="Review" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn setup() -> (String, reqwest::Client) {
    let process_engine = Arc::new(ProcessEngine::new("rest-p137-finished-by".to_string()));
    process_engine
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
    let server_engine = Arc::clone(&process_engine);
    tokio::spawn(async move {
        run_server(server_engine, listener).await.unwrap();
    });
    let client = reqwest::Client::new();

    let deployment = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P137 finishedBy",
            "resourceName": "p137-finished-by.cmmn",
            "resource": MODEL
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deployment.status(), reqwest::StatusCode::CREATED);

    let started = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": "p137FinishedByRestCase" }))
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), reqwest::StatusCode::CREATED);

    (base_url, client)
}

async fn body(response: reqwest::Response) -> (reqwest::StatusCode, Value) {
    let status = response.status();
    let body = response.json().await.unwrap();
    (status, body)
}

#[tokio::test]
async fn finished_by_get_and_post_are_valid_filters_with_no_rest_actor_data() {
    let (base_url, client) = setup().await;

    let (status, response) = body(
        client
            .get(format!(
                "{base_url}/cmmn-history/historic-case-instances?finishedBy=admin"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{response}");
    assert_eq!(response["total"], 0);

    let (status, response) = body(
        client
            .post(format!(
                "{base_url}/cmmn-query/historic-case-instances"
            ))
            .basic_auth("admin", Some("test"))
            .json(&json!({ "finishedBy": "admin" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{response}");
    assert_eq!(response["total"], 0);
}

#[tokio::test]
async fn unknown_finishing_actor_parameter_is_still_rejected() {
    let (base_url, client) = setup().await;
    let (status, response) = body(
        client
            .get(format!(
                "{base_url}/cmmn-history/historic-case-instances?finishUser=admin"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert!(response["details"].as_str().unwrap().contains("finishUser"));
}
