//! P137 — REST GET/POST plumbing for CMMN case reference metadata.

use flowable_cmmn_engine::{CmmnCaseInstanceStartRequest, CmmnEngine};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="p137RestCase" name="P137 REST case">
    <casePlanModel id="p137Plan" name="P137 plan">
      <planItem id="reviewPlanItem" name="Review" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review" />
    </casePlanModel>
  </case>
</definitions>"#;

struct Fixture {
    base_url: String,
    client: reqwest::Client,
    matching_case_id: String,
}

async fn setup() -> Fixture {
    let process_engine = Arc::new(ProcessEngine::new("rest-p137-case-reference".to_string()));
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
    let cmmn_engine = process_engine
        .get_config()
        .cmmn_engine
        .as_ref()
        .expect("CMMN engine")
        .clone();

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
            "name": "P137 case reference",
            "resourceName": "p137-reference.cmmn",
            "resource": MODEL
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deployment.status(), reqwest::StatusCode::CREATED);

    let matching_case_id = start_case(
        &cmmn_engine,
        "matching",
        "order-137",
        "event-to-cmmn-1.1-case",
    );
    start_case(&cmmn_engine, "other", "order-other", "external");

    Fixture {
        base_url,
        client,
        matching_case_id,
    }
}

fn start_case(engine: &CmmnEngine, name: &str, reference_id: &str, reference_type: &str) -> String {
    engine
        .start_case_instance_by_key(
            "p137RestCase",
            CmmnCaseInstanceStartRequest::new()
                .with_name(name)
                .with_reference_id(reference_id)
                .with_reference_type(reference_type),
        )
        .expect("start case")
        .id
}

async fn get(fixture: &Fixture, path: &str, query: &str) -> (reqwest::StatusCode, Value) {
    let response = fixture
        .client
        .get(format!("{}{path}?{query}", fixture.base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.json().await.unwrap();
    (status, body)
}

async fn post(fixture: &Fixture, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
    let response = fixture
        .client
        .post(format!("{}{path}", fixture.base_url))
        .basic_auth("admin", Some("test"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.json().await.unwrap();
    (status, body)
}

fn assert_match(body: &Value, expected_id: &str) {
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], expected_id);
    assert_eq!(body["data"][0]["referenceId"], "order-137");
    assert_eq!(body["data"][0]["referenceType"], "event-to-cmmn-1.1-case");
}

#[tokio::test]
async fn runtime_and_historic_reference_filters_support_get_and_java_post_names() {
    let fixture = setup().await;

    for path in [
        "/cmmn-runtime/case-instances",
        "/cmmn-history/historic-case-instances",
    ] {
        let (status, body) = get(
            &fixture,
            path,
            "referenceId=order-137&referenceType=event-to-cmmn-1.1-case",
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::OK, "{path}: {body}");
        assert_match(&body, &fixture.matching_case_id);

        let (status, body) = get(&fixture, path, "referenceId=missing").await;
        assert_eq!(status, reqwest::StatusCode::OK, "{path}: {body}");
        assert_eq!(body["total"], 0);
    }

    for path in [
        "/cmmn-query/case-instances",
        "/cmmn-query/historic-case-instances",
    ] {
        let (status, body) = post(
            &fixture,
            path,
            json!({
                "caseInstanceReferenceId": "order-137",
                "caseInstanceReferenceType": "event-to-cmmn-1.1-case"
            }),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::OK, "{path}: {body}");
        assert_match(&body, &fixture.matching_case_id);
    }
}

#[tokio::test]
async fn unknown_reference_parameter_still_returns_bad_request() {
    let fixture = setup().await;
    let (status, body) = get(
        &fixture,
        "/cmmn-runtime/case-instances",
        "referenceIdentifier=order-137",
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("referenceIdentifier")
    );
}
