//! P88 — REST e2e: real engine typeRef=number integer output → type "double".
//!
//! Java: ExecutionVariableFactory.java:60-69 (Double) +
//! DmnRestResponseFactory.java:257-292 + DoubleRestVariableConverter.java:24-31.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;

const NUMBER_OUTPUT_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="p88-number-defs"
             namespace="http://flowable.org/dmn">
  <decision id="p88NumberOut" name="P88 Number Output">
    <decisionTable id="p88NumberTable" hitPolicy="FIRST">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="priority" typeRef="number" />
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>10</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
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
async fn dmn_rest_number_type_ref_integer_output_is_double() {
    let (base_url, client) = spawn_server("rest-dmn-p88-output-double").await;

    let deploy = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "p88-number-out",
            "resourceName": "p88-number.dmn",
            "resource": NUMBER_OUTPUT_DMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        deploy.status(),
        reqwest::StatusCode::CREATED,
        "deployment: {}",
        deploy.text().await.unwrap()
    );

    let execute = client
        .post(format!("{base_url}/dmn-runtime/decision-executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "p88NumberOut",
            "variables": { "channel": "email" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        execute.status(),
        reqwest::StatusCode::CREATED,
        "execution: {}",
        execute.text().await.unwrap()
    );

    let body: Value = execute.json().await.unwrap();
    assert_eq!(
        body["resultVariables"][0][0],
        json!({"name": "priority", "type": "double", "value": 10.0})
    );
}
