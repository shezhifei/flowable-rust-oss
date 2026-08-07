use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

// --- COMPLETE hit policy: returns all matching rules (like RULE_ORDER) ---
const COMPLETE_HIT_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="complete-defs"
             namespace="http://flowable.org/dmn">
  <decision id="completeDecision" name="Complete Decision">
    <decisionTable id="completeTable" hitPolicy="COMPLETE">
      <input id="input1">
        <inputExpression id="inputExpr1" typeRef="string">
          <text>status</text>
        </inputExpression>
      </input>
      <output id="output1" name="action" typeRef="string" />
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'log'</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry><text>'active'</text></inputEntry>
        <outputEntry><text>'notify'</text></outputEntry>
      </rule>
      <rule id="rule3">
        <inputEntry><text>'active'</text></inputEntry>
        <outputEntry><text>'audit'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

// --- COLLECT with SUM aggregation, single number output (P82c) ---
const COLLECT_SUM_SINGLE_OUTPUT_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="collect-sum-defs"
             namespace="http://flowable.org/dmn">
  <decision id="collectSumDecision" name="Collect Sum Decision">
    <decisionTable id="collectSumTable" hitPolicy="COLLECT" aggregation="SUM">
      <input id="input1">
        <inputExpression id="inputExpr1" typeRef="number">
          <text>score</text>
        </inputExpression>
      </input>
      <output id="output1" name="bonus" typeRef="number" />
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>10</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry><text>> 50</text></inputEntry>
        <outputEntry><text>20</text></outputEntry>
      </rule>
      <rule id="rule3">
        <inputEntry><text>> 80</text></inputEntry>
        <outputEntry><text>30</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

// --- COLLECT+SUM multi-output (invalid under P82c / Java RuleEngineExecutorImpl) ---
const COLLECT_MULTI_OUTPUT_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="collect-multi-defs"
             namespace="http://flowable.org/dmn">
  <decision id="collectMultiDecision" name="Collect Multi Decision">
    <decisionTable id="collectMultiTable" hitPolicy="COLLECT" aggregation="SUM">
      <input id="input1">
        <inputExpression id="inputExpr1" typeRef="number">
          <text>score</text>
        </inputExpression>
      </input>
      <output id="output1" name="bonus" typeRef="number" />
      <output id="output2" name="penalty" typeRef="number" />
      <rule id="rule1">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>10</text></outputEntry>
        <outputEntry><text>1</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

// --- Substring and Replace string function unary tests ---
const STRING_FUNC_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="string-func-defs"
             namespace="http://flowable.org/dmn">
  <decision id="stringFuncDecision" name="String Function Decision">
    <decisionTable id="stringFuncTable" hitPolicy="FIRST">
      <input id="input1">
        <inputExpression id="inputExpr1">
          <text>code</text>
        </inputExpression>
      </input>
      <output id="output1" name="result" typeRef="string" />
      <rule id="rule1">
        <inputEntry><text>substring(?, 1, 3) = "ABC"</text></inputEntry>
        <outputEntry><text>'starts-abc'</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry><text>replace(?, "[0-9]+", "X") = "ABC-X"</text></inputEntry>
        <outputEntry><text>'replaced'</text></outputEntry>
      </rule>
      <rule id="rule3">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'default'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

// --- Nested not() and comma-separated not() arguments ---
const NESTED_NOT_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="nested-not-defs"
             namespace="http://flowable.org/dmn">
  <decision id="nestedNotDecision" name="Nested Not Decision">
    <decisionTable id="nestedNotTable" hitPolicy="FIRST">
      <input id="input1">
        <inputExpression id="inputExpr1">
          <text>role</text>
        </inputExpression>
      </input>
      <output id="output1" name="access" typeRef="string" />
      <rule id="rule1">
        <inputEntry><text>not(not("admin"))</text></inputEntry>
        <outputEntry><text>'admin-access'</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry><text>not("blocked", "suspended")</text></inputEntry>
        <outputEntry><text>'allowed'</text></outputEntry>
      </rule>
      <rule id="rule3">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'denied'</text></outputEntry>
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
    variables: Value,
) -> Value {
    let response = client
        .post(format!("{base_url}/dmn-runtime/decision-executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": decision_key,
            "variables": variables
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        reqwest::StatusCode::CREATED,
        "execution failed: {}",
        response.text().await.unwrap()
    );
    response.json().await.unwrap()
}

#[tokio::test]
async fn complete_hit_policy_returns_all_matching_rules() {
    let (base_url, client) = spawn_server("rest-dmn-complete-hit").await;
    deploy_dmn(&client, &base_url, "complete-decision", COMPLETE_HIT_DMN).await;

    // "active" matches rule1 (wildcard), rule2 ('active'), rule3 ('active') → 3 hits
    let result = execute_decision(
        &client,
        &base_url,
        "completeDecision",
        json!({"status": "active"}),
    )
    .await;

    assert_eq!(result["decisionKey"], "completeDecision");
    assert_eq!(result["ruleHitCount"], 3);
    // P79 row shape + P85 EngineRestVariable wrapper
    assert_eq!(result["resultVariables"].as_array().unwrap().len(), 3);
    assert_eq!(
        result["resultVariables"][0][0],
        json!({"name": "action", "type": "string", "value": "log"})
    );
    assert_eq!(
        result["resultVariables"][1][0],
        json!({"name": "action", "type": "string", "value": "notify"})
    );
    assert_eq!(
        result["resultVariables"][2][0],
        json!({"name": "action", "type": "string", "value": "audit"})
    );
    assert_eq!(result["multipleResults"], true);

    // "inactive" matches only rule1 (wildcard) → 1 hit (still multipleResults for Complete)
    let result2 = execute_decision(
        &client,
        &base_url,
        "completeDecision",
        json!({"status": "inactive"}),
    )
    .await;

    assert_eq!(result2["ruleHitCount"], 1);
    assert_eq!(
        result2["resultVariables"][0][0],
        json!({"name": "action", "type": "string", "value": "log"})
    );
    assert_eq!(result2["multipleResults"], true);
}

/// P82c: COLLECT+SUM with a single number output deploys and aggregates.
#[tokio::test]
async fn collect_aggregation_sums_single_number_output() {
    let (base_url, client) = spawn_server("rest-dmn-collect-sum").await;
    deploy_dmn(
        &client,
        &base_url,
        "collect-sum-decision",
        COLLECT_SUM_SINGLE_OUTPUT_DMN,
    )
    .await;

    // score=90: rules 1+2+3 → SUM bonus=60
    let result = execute_decision(
        &client,
        &base_url,
        "collectSumDecision",
        json!({"score": 90}),
    )
    .await;

    assert_eq!(result["decisionKey"], "collectSumDecision");
    // SUM aggregation yields a float → "double" (P85 type inference)
    assert_eq!(
        result["resultVariables"][0][0],
        json!({"name": "bonus", "type": "double", "value": 60.0})
    );

    // score=30: only rule1 → SUM bonus=10
    let result2 = execute_decision(
        &client,
        &base_url,
        "collectSumDecision",
        json!({"score": 30}),
    )
    .await;

    assert_eq!(
        result2["resultVariables"][0][0],
        json!({"name": "bonus", "type": "double", "value": 10.0})
    );
}

/// P82c: COLLECT+aggregation with multiple outputs is rejected at deploy (400).
#[tokio::test]
async fn collect_aggregation_multi_output_rejected_at_deploy() {
    let (base_url, client) = spawn_server("rest-dmn-collect-multi-reject").await;
    let response = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "collect-multi-decision",
            "resourceName": "collect-multi-decision.dmn",
            "resource": COLLECT_MULTI_OUTPUT_DMN
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    let details = body["details"].as_str().unwrap_or("");
    assert!(
        details.contains("multiple outputs") && details.contains("not supported"),
        "unexpected details: {details}"
    );
}

#[tokio::test]
async fn substring_and_replace_unary_tests_match_at_runtime() {
    let (base_url, client) = spawn_server("rest-dmn-string-func").await;
    deploy_dmn(&client, &base_url, "string-func-decision", STRING_FUNC_DMN).await;

    // "ABCDEF" → substring(?, 1, 3) = "ABC" → match rule1
    let result = execute_decision(
        &client,
        &base_url,
        "stringFuncDecision",
        json!({"code": "ABCDEF"}),
    )
    .await;

    assert_eq!(result["resultVariables"][0][0]["value"], "starts-abc");

    // "DEF-123" → substring fails, replace(?, "[0-9]+", "X") = "DEF-X" ≠ "ABC-X" → default
    let result2 = execute_decision(
        &client,
        &base_url,
        "stringFuncDecision",
        json!({"code": "DEF-123"}),
    )
    .await;

    assert_eq!(result2["resultVariables"][0][0]["value"], "default");

    // "ABC-456" → substring(?, 1, 3) = "ABC" → match rule1
    let result3 = execute_decision(
        &client,
        &base_url,
        "stringFuncDecision",
        json!({"code": "ABC-456"}),
    )
    .await;

    assert_eq!(result3["resultVariables"][0][0]["value"], "starts-abc");

    // "XYZ-999" → no substring match, replace yields "XYZ-X" ≠ "ABC-X" → default
    let result4 = execute_decision(
        &client,
        &base_url,
        "stringFuncDecision",
        json!({"code": "XYZ-999"}),
    )
    .await;

    assert_eq!(result4["resultVariables"][0][0]["value"], "default");
}

#[tokio::test]
async fn nested_not_and_comma_separated_not_unary_tests_evaluate_correctly() {
    let (base_url, client) = spawn_server("rest-dmn-nested-not").await;
    deploy_dmn(&client, &base_url, "nested-not-decision", NESTED_NOT_DMN).await;

    // "admin" → not(not("admin")) = true → admin-access
    let result = execute_decision(
        &client,
        &base_url,
        "nestedNotDecision",
        json!({"role": "admin"}),
    )
    .await;

    assert_eq!(result["resultVariables"][0][0]["value"], "admin-access");

    // "user" → not(not("admin")) = false (not admin), not("blocked","suspended") = true → allowed
    let result2 = execute_decision(
        &client,
        &base_url,
        "nestedNotDecision",
        json!({"role": "user"}),
    )
    .await;

    assert_eq!(result2["resultVariables"][0][0]["value"], "allowed");

    // "blocked" → not(not("admin")) = false, not("blocked","suspended") = false → denied
    let result3 = execute_decision(
        &client,
        &base_url,
        "nestedNotDecision",
        json!({"role": "blocked"}),
    )
    .await;

    assert_eq!(result3["resultVariables"][0][0]["value"], "denied");

    // "suspended" → not(not("admin")) = false, not("blocked","suspended") = false → denied
    let result4 = execute_decision(
        &client,
        &base_url,
        "nestedNotDecision",
        json!({"role": "suspended"}),
    )
    .await;

    assert_eq!(result4["resultVariables"][0][0]["value"], "denied");
}
