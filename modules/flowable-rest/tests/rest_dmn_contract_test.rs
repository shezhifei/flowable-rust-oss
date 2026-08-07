use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const DISH_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="dish-defs"
             namespace="http://flowable.org/dmn">
  <decision id="dishDecision" name="Dish decision">
    <decisionTable id="dishTable" hitPolicy="FIRST">
      <input id="input1">
        <inputExpression id="inputExpr1" typeRef="string">
          <text>dishType</text>
        </inputExpression>
      </input>
      <output id="output1" name="dish" typeRef="string" />
      <rule id="rule1">
        <inputEntry><text>'salad'</text></inputEntry>
        <outputEntry><text>'light'</text></outputEntry>
      </rule>
      <rule id="rule2">
        <inputEntry><text>-</text></inputEntry>
        <outputEntry><text>'default'</text></outputEntry>
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

async fn deploy_dmn(client: &reqwest::Client, base_url: &str) -> Value {
    let response = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "dish deployment",
            "resourceName": "dish.dmn",
            "resource": DISH_DMN
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
    response.json().await.unwrap()
}

async fn execute_decision(
    client: &reqwest::Client,
    base_url: &str,
    dish_type: &str,
    business_key: &str,
) -> Value {
    let response = client
        .post(format!("{base_url}/dmn-runtime/decision-executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "dishDecision",
            "businessKey": business_key,
            "variables": {
                "dishType": dish_type
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
async fn historic_decision_execution_query_delete_and_post_match_canonical_contract() {
    let (base_url, client) = spawn_server("rest-dmn-historic-query-contract").await;
    let deployment = deploy_dmn(&client, &base_url).await;
    let deployment_id = deployment["id"].as_str().unwrap();

    let first = execute_decision(&client, &base_url, "salad", "order-1").await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let second = execute_decision(&client, &base_url, "soup", "order-2").await;
    let first_id = first["id"].as_str().unwrap();
    let second_id = second["id"].as_str().unwrap();

    let get_query = client
        .get(format!(
            "{base_url}/dmn-history/historic-decision-executions?decisionKey=dishDecision&deploymentId={deployment_id}&sort=startTime&order=desc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_query.status(), reqwest::StatusCode::OK);
    let get_body: Value = get_query.json().await.unwrap();
    assert_eq!(get_body["start"], 0);
    assert_eq!(get_body["size"], 1);
    assert_eq!(get_body["total"], 2);
    assert_eq!(get_body["data"][0]["id"], second_id);
    // Historic responses keep the raw map shape (Java serves audit data from
    // the stored execution JSON, not via DmnRestResponseFactory).
    assert_eq!(get_body["data"][0]["resultVariables"][0]["dish"], "default");

    let post_query = client
        .post(format!(
            "{base_url}/dmn-query/historic-decision-executions?start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionDefinitionId": first["decisionTableId"],
            "businessKey": "order-1",
            "sort": "decisionKey",
            "order": "asc"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(post_query.status(), reqwest::StatusCode::OK);
    let post_body: Value = post_query.json().await.unwrap();
    assert_eq!(post_body["total"], 1);
    assert_eq!(post_body["data"][0]["id"], first_id);
    assert_eq!(post_body["data"][0]["businessKey"], "order-1");
    assert_eq!(post_body["data"][0]["inputVariables"]["dishType"], "salad");

    let bad_sort = client
        .get(format!(
            "{base_url}/dmn-history/historic-decision-executions?sort=unsupportedHistoricDecisionSort"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_sort.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_sort_body: Value = bad_sort.json().await.unwrap();
    assert_eq!(bad_sort_body["code"], "BAD_REQUEST");
    assert!(bad_sort_body["details"].as_str().unwrap().contains(
        "Unsupported historic decision execution sort field 'unsupportedHistoricDecisionSort'"
    ));

    let delete = client
        .delete(format!(
            "{base_url}/dmn-history/historic-decision-executions/{first_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let after_delete = client
        .get(format!(
            "{base_url}/dmn-history/historic-decision-executions/{first_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(after_delete.status(), reqwest::StatusCode::NOT_FOUND);

    let bulk_delete = client
        .post(format!(
            "{base_url}/dmn-history/historic-decision-executions/delete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionExecutionIds": [second_id]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bulk_delete.status(), reqwest::StatusCode::NO_CONTENT);

    let empty = client
        .get(format!(
            "{base_url}/dmn-history/historic-decision-executions?decisionKey=dishDecision"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), reqwest::StatusCode::OK);
    let empty_body: Value = empty.json().await.unwrap();
    assert_eq!(empty_body["total"], 0);
}

#[tokio::test]
async fn runtime_execution_response_includes_runtime_fields_and_can_disable_history() {
    let (base_url, client) = spawn_server("rest-dmn-runtime-disable-history-contract").await;
    let deployment = deploy_dmn(&client, &base_url).await;
    let deployment_id = deployment["id"].as_str().unwrap();

    let execute = client
        .post(format!("{base_url}/dmn-runtime/decision-executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "dishDecision",
            "businessKey": "order-no-history",
            "disableHistory": true,
            "variables": {
                "dishType": "salad"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(execute.status(), reqwest::StatusCode::CREATED);
    let execute_body: Value = execute.json().await.unwrap();
    assert_eq!(execute_body["deploymentId"], deployment_id);
    assert_eq!(execute_body["businessKey"], "order-no-history");
    assert_eq!(execute_body["hitPolicy"], "FIRST");
    assert_eq!(execute_body["ruleHitCount"], 1);
    assert_eq!(
        execute_body["resultVariables"][0][0],
        json!({"name": "dish", "type": "string", "value": "light"})
    );

    let history = client
        .get(format!(
            "{base_url}/dmn-history/historic-decision-executions?businessKey=order-no-history"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(history.status(), reqwest::StatusCode::OK);
    let history_body: Value = history.json().await.unwrap();
    assert_eq!(history_body["total"], 0);
}
