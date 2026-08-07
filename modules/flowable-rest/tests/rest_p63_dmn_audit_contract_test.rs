use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const AUDIT_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="p63-definitions" namespace="http://flowable.org/dmn">
  <decision id="p63RestRouting" name="P63 REST routing">
    <decisionTable id="p63-table" hitPolicy="UNIQUE">
      <input id="tier-input">
        <inputExpression id="tier-expression" typeRef="string"><text>tier</text></inputExpression>
      </input>
      <output id="route-output" name="route" typeRef="string" />
      <rule id="gold-rule">
        <inputEntry id="gold-condition"><text>'gold'</text></inputEntry>
        <outputEntry id="gold-conclusion"><text>'priority'</text></outputEntry>
      </rule>
      <rule id="standard-rule">
        <inputEntry id="standard-condition"><text>'standard'</text></inputEntry>
        <outputEntry id="standard-conclusion"><text>'normal'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("p63-rest".to_string()));
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
    tokio::spawn(async move { run_server(engine, listener).await.unwrap() });
    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn runtime_response_and_history_auditdata_include_inputs_and_rule_audit() {
    let (base_url, client) = spawn_server().await;
    let deployment = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P63 REST audit",
            "resourceName": "p63-audit.dmn",
            "resource": AUDIT_DMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deployment.status(), reqwest::StatusCode::CREATED);

    let response = client
        .post(format!("{base_url}/dmn-runtime/decision-executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "p63RestRouting",
            "variables": {"tier": "gold"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let execution: Value = response.json().await.unwrap();

    assert_eq!(execution["inputVariables"]["tier"], "gold");
    assert_eq!(execution["ruleExecutions"][0]["ruleId"], "gold-rule");
    assert_eq!(
        execution["ruleExecutions"][0]["conditionResults"][0],
        json!({"id": "gold-condition", "result": true})
    );
    assert_eq!(
        execution["ruleExecutions"][0]["conclusionResults"][0],
        json!({"id": "gold-conclusion", "result": "priority"})
    );

    let audit_response = client
        .get(format!(
            "{base_url}/dmn-history/historic-decision-executions/{}/auditdata",
            execution["id"].as_str().unwrap()
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(audit_response.status(), reqwest::StatusCode::OK);
    let audit: Value = audit_response.json().await.unwrap();
    assert_eq!(audit["inputVariables"]["tier"], "gold");
    assert_eq!(audit["ruleExecutions"], execution["ruleExecutions"]);
}
