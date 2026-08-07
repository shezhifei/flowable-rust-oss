use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const UNIQUE_HIT_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="rest-unique-defs"
             namespace="http://flowable.org/dmn">
  <decision id="restUniqueRouting" name="REST Unique Routing">
    <decisionTable id="uniqueRoutingTable" hitPolicy="UNIQUE">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="route" typeRef="string" />
      <rule id="ruleEmail">
        <inputEntry><text>'email'</text></inputEntry>
        <outputEntry><text>'email-queue'</text></outputEntry>
      </rule>
      <rule id="ruleSms">
        <inputEntry><text>'sms'</text></inputEntry>
        <outputEntry><text>'sms-queue'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

const ANY_HIT_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="rest-any-defs"
             namespace="http://flowable.org/dmn">
  <decision id="restAnyRouting" name="REST Any Routing">
    <decisionTable id="anyRoutingTable" hitPolicy="ANY">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="route" typeRef="string" />
      <rule id="ruleFallback">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'manual'</text></outputEntry>
      </rule>
      <rule id="ruleEmail">
        <inputEntry><text>'email'</text></inputEntry>
        <outputEntry><text>'manual'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

const COLLECT_HIT_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="rest-collect-defs"
             namespace="http://flowable.org/dmn">
  <decision id="restCollectRouting" name="REST Collect Routing">
    <decisionTable id="collectRoutingTable" hitPolicy="COLLECT">
      <input id="input1">
        <inputExpression id="inputExpression1" typeRef="string">
          <text>channel</text>
        </inputExpression>
      </input>
      <output id="output1" name="route" typeRef="string" />
      <output id="output2" name="priority" typeRef="number" />
      <rule id="ruleFallback">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'manual'</text></outputEntry>
        <outputEntry><text>10</text></outputEntry>
      </rule>
      <rule id="ruleEmail">
        <inputEntry><text>'email'</text></inputEntry>
        <outputEntry><text>'email-queue'</text></outputEntry>
        <outputEntry><text>20</text></outputEntry>
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

async fn deploy_dmn(client: &reqwest::Client, base_url: &str, name: &str, resource: &str) {
    let response = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": format!("{name}.dmn"),
            "resource": resource
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        reqwest::StatusCode::CREATED,
        "deployment response: {}",
        response.text().await.unwrap()
    );
}

async fn execute_decision(
    client: &reqwest::Client,
    base_url: &str,
    decision_key: &str,
    channel: &str,
) -> Value {
    let response = client
        .post(format!("{base_url}/dmn-runtime/decision-executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": decision_key,
            "variables": {
                "channel": channel
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        reqwest::StatusCode::CREATED,
        "execution response: {}",
        response.text().await.unwrap()
    );
    response.json().await.unwrap()
}

#[tokio::test]
async fn dmn_rest_runtime_executes_unique_and_any_hit_policies() {
    let (base_url, client) = spawn_server("rest-dmn-hit-policy-contract").await;
    deploy_dmn(&client, &base_url, "rest-unique-routing", UNIQUE_HIT_DMN).await;
    deploy_dmn(&client, &base_url, "rest-any-routing", ANY_HIT_DMN).await;
    deploy_dmn(&client, &base_url, "rest-collect-routing", COLLECT_HIT_DMN).await;

    let unique = execute_decision(&client, &base_url, "restUniqueRouting", "email").await;
    assert_eq!(unique["decisionKey"], "restUniqueRouting");
    assert_eq!(
        unique["resultVariables"][0][0],
        json!({"name": "route", "type": "string", "value": "email-queue"})
    );

    let any = execute_decision(&client, &base_url, "restAnyRouting", "email").await;
    assert_eq!(any["decisionKey"], "restAnyRouting");
    assert_eq!(
        any["resultVariables"][0][0],
        json!({"name": "route", "type": "string", "value": "manual"})
    );
    assert_eq!(any["ruleHitCount"], 2);

    let collect = execute_decision(&client, &base_url, "restCollectRouting", "email").await;
    assert_eq!(collect["decisionKey"], "restCollectRouting");
    assert_eq!(collect["ruleHitCount"], 2);
    // P79 row shape + P85 EngineRestVariable wrapper; variables are name-ordered
    // within a row, so priority precedes route.
    assert_eq!(collect["resultVariables"].as_array().unwrap().len(), 2);
    // P88: typeRef="number" → engine f64 → REST "double" (Java
    // ExecutionVariableFactory.java:60-69 + DoubleRestVariableConverter).
    assert_eq!(
        collect["resultVariables"][0],
        json!([
            {"name": "priority", "type": "double", "value": 10.0},
            {"name": "route", "type": "string", "value": "manual"},
        ])
    );
    assert_eq!(
        collect["resultVariables"][1],
        json!([
            {"name": "priority", "type": "double", "value": 20.0},
            {"name": "route", "type": "string", "value": "email-queue"},
        ])
    );
    assert_eq!(collect["multipleResults"], true);
}
