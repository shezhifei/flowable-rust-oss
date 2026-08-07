use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const NUMERIC_UNARY_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="rest-score-defs"
             namespace="http://flowable.org/dmn">
  <decision id="restScoreDecision" name="REST Score Decision">
    <decisionTable id="scoreTable" hitPolicy="FIRST">
      <input id="input1" label="Score">
        <inputExpression id="inputExpression1" typeRef="number">
          <text>score</text>
        </inputExpression>
      </input>
      <output id="output1" name="band" typeRef="string" />
      <rule id="ruleExcellent">
        <inputEntry><text>&gt;= 90</text></inputEntry>
        <outputEntry><text>'excellent'</text></outputEntry>
      </rule>
      <rule id="rulePassing">
        <inputEntry><text>&gt; 70</text></inputEntry>
        <outputEntry><text>'passing'</text></outputEntry>
      </rule>
      <rule id="ruleExact">
        <inputEntry><text>= 50</text></inputEntry>
        <outputEntry><text>'exact'</text></outputEntry>
      </rule>
      <rule id="ruleNegative">
        <inputEntry><text>&lt; 0</text></inputEntry>
        <outputEntry><text>'negative'</text></outputEntry>
      </rule>
      <rule id="ruleLow">
        <inputEntry><text>&lt;= 10</text></inputEntry>
        <outputEntry><text>'low'</text></outputEntry>
      </rule>
      <rule id="ruleDefault">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'default'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

const UNSUPPORTED_COMPLEX_UNARY_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="rest-complex-unary-defs"
             namespace="http://flowable.org/dmn">
  <decision id="restComplexUnaryDecision" name="REST Complex Unary Decision">
    <decisionTable id="complexUnaryTable" hitPolicy="FIRST">
      <input id="input1" label="Score">
        <inputExpression id="inputExpression1" typeRef="number">
          <text>score</text>
        </inputExpression>
      </input>
      <output id="output1" name="band" typeRef="string" />
      <rule id="ruleRange">
        <inputEntry><text>&gt; 'high'</text></inputEntry>
        <outputEntry><text>'range'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

const UNSUPPORTED_HIT_POLICY_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="rest-hit-policy-defs"
             namespace="http://flowable.org/dmn">
  <decision id="restUnsupportedHitPolicyDecision" name="REST Unsupported Hit Policy Decision">
    <decisionTable id="unsupportedHitPolicyTable" hitPolicy="UNSUPPORTED">
      <input id="input1" label="Score">
        <inputExpression id="inputExpression1" typeRef="number">
          <text>score</text>
        </inputExpression>
      </input>
      <output id="output1" name="band" typeRef="string" />
      <rule id="ruleAny">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'any'</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

const EXTENDED_IF_PART_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="restExtendedIfPartCase" name="REST Extended ifPart Case">
    <casePlanModel id="planModel" name="Plan Model" autoComplete="false">
      <planItem id="planItemSource" name="Source Task" definitionRef="sourceTask" />
      <planItem id="planItemStatus" name="Status Task" definitionRef="statusTask">
        <entryCriterion id="entryCriterionStatus" sentryRef="sentryStatus" />
      </planItem>
      <planItem id="planItemAmount" name="Amount Task" definitionRef="amountTask">
        <entryCriterion id="entryCriterionAmount" sentryRef="sentryAmount" />
      </planItem>
      <planItem id="planItemDecision" name="Decision Task" definitionRef="decisionTask">
        <entryCriterion id="entryCriterionDecision" sentryRef="sentryDecision" />
      </planItem>
      <humanTask id="sourceTask" name="Source Task" isBlocking="true" />
      <humanTask id="statusTask" name="Status Task" isBlocking="true" />
      <humanTask id="amountTask" name="Amount Task" isBlocking="true" />
      <humanTask id="decisionTask" name="Decision Task" isBlocking="true" />
      <sentry id="sentryStatus">
        <planItemOnPart id="onSourceCompleteForStatus" sourceRef="planItemSource">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>${status == "approved"}</condition>
        </ifPart>
      </sentry>
      <sentry id="sentryAmount">
        <planItemOnPart id="onSourceCompleteForAmount" sourceRef="planItemSource">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>amount == 42.5</condition>
        </ifPart>
      </sentry>
      <sentry id="sentryDecision">
        <planItemOnPart id="onSourceCompleteForDecision" sourceRef="planItemSource">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>decision != 'denied'</condition>
        </ifPart>
      </sentry>
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

async fn deploy_dmn(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "REST Numeric Unary DMN",
            "resourceName": "rest-numeric-unary.dmn",
            "resource": NUMERIC_UNARY_DMN
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "deployment response: {}",
        response.text().await.unwrap()
    );
}

async fn deploy_dmn_resource(
    client: &reqwest::Client,
    base_url: &str,
    resource_name: &str,
    resource: &str,
) -> reqwest::Response {
    client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": resource_name,
            "resourceName": resource_name,
            "resource": resource
        }))
        .send()
        .await
        .unwrap()
}

async fn execute_score(client: &reqwest::Client, base_url: &str, score: Value) -> Value {
    let response = client
        .post(format!("{base_url}/dmn-runtime/decision-executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "restScoreDecision",
            "variables": {
                "score": score
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "execution response: {}",
        response.text().await.unwrap()
    );
    response.json().await.unwrap()
}

async fn deploy_cmmn(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "REST Extended ifPart CMMN",
            "resourceName": "rest-extended-if-part.cmmn",
            "resource": EXTENDED_IF_PART_CMMN
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "deployment response: {}",
        response.text().await.unwrap()
    );
}

async fn start_if_part_case(client: &reqwest::Client, base_url: &str) -> String {
    start_if_part_case_with_variables(
        client,
        base_url,
        json!({
            "status": "approved",
            "amount": 42.5,
            "decision": "needs-review"
        }),
    )
    .await
}

async fn start_if_part_case_with_variables(
    client: &reqwest::Client,
    base_url: &str,
    variables: Value,
) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "restExtendedIfPartCase",
            "businessKey": "rest-if-part-bk",
            "variables": variables
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "start response: {}",
        response.text().await.unwrap()
    );
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn active_task_definition_ids(
    client: &reqwest::Client,
    base_url: &str,
    case_instance_id: &str,
) -> Vec<String> {
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}&state=ACTIVE&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    let mut ids = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["planItemDefinitionId"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

async fn active_plan_item_definition_ids(
    client: &reqwest::Client,
    base_url: &str,
    case_instance_id: &str,
) -> Vec<String> {
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_instance_id}&state=ACTIVE&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    let mut ids = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["planItemDefinitionId"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

async fn complete_only_active_task(
    client: &reqwest::Client,
    base_url: &str,
    case_instance_id: &str,
) {
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}&state=ACTIVE&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1);
    // Java PlanItemInstanceEntityManagerImpl.java:92-95: definitionRef target,
    // not the plan item XML id exposed separately as elementId.
    assert_eq!(body["data"][0]["planItemDefinitionId"], "sourceTask");
    let source_task_id = body["data"][0]["id"].as_str().unwrap();

    let complete_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{source_task_id}/complete"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        complete_response.status(),
        StatusCode::OK,
        "complete response: {}",
        complete_response.text().await.unwrap()
    );
}

#[tokio::test]
async fn dmn_rest_runtime_executes_numeric_unary_tests_and_reports_rule_hit_count() {
    let (base_url, client) = spawn_server("rest-dmn-numeric-unary-contract").await;
    deploy_dmn(&client, &base_url).await;

    for (score, expected_band) in [
        (json!(90), "excellent"),
        (json!(71), "passing"),
        (json!(50), "exact"),
        (json!(-1), "negative"),
        (json!(10), "low"),
        (json!(42), "default"),
    ] {
        let execution = execute_score(&client, &base_url, score).await;
        assert_eq!(execution["decisionKey"], "restScoreDecision");
        assert_eq!(
            execution["resultVariables"][0][0],
            json!({"name": "band", "type": "string", "value": expected_band})
        );
        assert_eq!(execution["ruleHitCount"], 1);
    }
}

#[tokio::test]
async fn dmn_rest_deploy_rejects_unsupported_unary_and_hit_policy_with_structured_errors() {
    let (base_url, client) = spawn_server("rest-dmn-unsupported-contract").await;

    let complex_unary_response = deploy_dmn_resource(
        &client,
        &base_url,
        "rest-unsupported-complex-unary.dmn",
        UNSUPPORTED_COMPLEX_UNARY_DMN,
    )
    .await;

    assert_eq!(complex_unary_response.status(), StatusCode::BAD_REQUEST);
    let complex_unary_body: Value = complex_unary_response.json().await.unwrap();
    assert_eq!(complex_unary_body["code"], "BAD_REQUEST");
    assert_eq!(complex_unary_body["message"], "Bad Request");
    assert!(
        complex_unary_body["details"]
            .as_str()
            .unwrap()
            .contains("unsupported unary test '> 'high''"),
        "{complex_unary_body:#?}"
    );

    let hit_policy_response = deploy_dmn_resource(
        &client,
        &base_url,
        "rest-unsupported-hit-policy.dmn",
        UNSUPPORTED_HIT_POLICY_DMN,
    )
    .await;

    assert_eq!(hit_policy_response.status(), StatusCode::BAD_REQUEST);
    let hit_policy_body: Value = hit_policy_response.json().await.unwrap();
    assert_eq!(hit_policy_body["code"], "BAD_REQUEST");
    assert_eq!(hit_policy_body["message"], "Bad Request");
    assert!(
        hit_policy_body["details"]
            .as_str()
            .unwrap()
            .contains("unsupported decisionTable hitPolicy `UNSUPPORTED`"),
        "{hit_policy_body:#?}"
    );
}

#[tokio::test]
async fn cmmn_rest_runtime_activates_if_part_gated_tasks_for_string_number_and_not_equal() {
    let (base_url, client) = spawn_server("rest-cmmn-if-part-contract").await;
    deploy_cmmn(&client, &base_url).await;
    let case_instance_id = start_if_part_case(&client, &base_url).await;

    assert_eq!(
        active_task_definition_ids(&client, &base_url, &case_instance_id).await,
        // Java PlanItemInstanceEntityManagerImpl.java:92-95.
        vec!["sourceTask".to_string()]
    );

    complete_only_active_task(&client, &base_url, &case_instance_id).await;

    assert_eq!(
        active_task_definition_ids(&client, &base_url, &case_instance_id).await,
        vec![
            "amountTask".to_string(),
            "decisionTask".to_string(),
            "statusTask".to_string(),
        ]
    );
}

#[tokio::test]
async fn cmmn_rest_runtime_keeps_if_part_gated_tasks_inactive_when_conditions_are_false() {
    let (base_url, client) = spawn_server("rest-cmmn-if-part-negative-contract").await;
    deploy_cmmn(&client, &base_url).await;
    let case_instance_id = start_if_part_case_with_variables(
        &client,
        &base_url,
        json!({
            "status": "pending",
            "amount": 43,
            "decision": "denied"
        }),
    )
    .await;

    assert_eq!(
        active_task_definition_ids(&client, &base_url, &case_instance_id).await,
        // Java PlanItemInstanceEntityManagerImpl.java:92-95.
        vec!["sourceTask".to_string()]
    );
    assert_eq!(
        active_plan_item_definition_ids(&client, &base_url, &case_instance_id).await,
        // Java PlanItemInstanceEntityManagerImpl.java:92-95.
        vec!["sourceTask".to_string()]
    );

    complete_only_active_task(&client, &base_url, &case_instance_id).await;

    assert!(
        active_task_definition_ids(&client, &base_url, &case_instance_id)
            .await
            .is_empty()
    );
    assert!(
        active_plan_item_definition_ids(&client, &base_url, &case_instance_id)
            .await
            .is_empty()
    );
}
