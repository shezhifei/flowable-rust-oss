use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const FIRST_HIT_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="loan-defs"
             name="Loan Decisions"
             namespace="http://flowable.org/dmn">
  <decision id="loanEligibility" name="Loan Eligibility">
    <decisionTable id="loanDecisionTable" hitPolicy="FIRST">
      <input id="input1" label="Credit score">
        <inputExpression id="inputExpression1" typeRef="number">
          <text>creditScore</text>
        </inputExpression>
      </input>
      <output id="output1" label="Approved" name="approved" typeRef="boolean" />
      <rule id="rule1">
        <inputEntry id="inputEntry1"><text>730</text></inputEntry>
        <outputEntry id="outputEntry1"><text>true</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

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
async fn dmn_deployment_and_decision_resource_data_return_stored_bytes() {
    let (base_url, client) = spawn_server("rest-dmn-deployment-resource-contract").await;

    let deploy_response = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "DMN Resource Contract Deployment",
            "resourceName": "models/loan-resource-contract.dmn",
            "resource": FIRST_HIT_DMN
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());
    let deployment: Value = deploy_response.json().await.unwrap();
    let deployment_id = deployment["id"].as_str().unwrap();

    let get_deployment_response = client
        .get(format!(
            "{base_url}/dmn-repository/deployments/{deployment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_deployment_response.status().is_success());
    let get_deployment_body: Value = get_deployment_response.json().await.unwrap();
    assert_eq!(get_deployment_body["id"], deployment_id);
    assert_eq!(
        get_deployment_body["name"],
        "DMN Resource Contract Deployment"
    );
    assert_eq!(
        get_deployment_body["resourceNames"][0],
        "models/loan-resource-contract.dmn"
    );

    let deployment_resource_data_response = client
        .get(format!(
            "{base_url}/dmn-repository/deployments/{deployment_id}/resourcedata/models/loan-resource-contract.dmn"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(deployment_resource_data_response.status().is_success());
    assert_eq!(
        deployment_resource_data_response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/xml"
    );
    assert_eq!(
        deployment_resource_data_response.text().await.unwrap(),
        FIRST_HIT_DMN
    );

    let decisions_response = client
        .get(format!(
            "{base_url}/dmn-repository/decision-tables?deploymentId={deployment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(decisions_response.status().is_success());
    let decisions: Value = decisions_response.json().await.unwrap();
    let decision_table_id = decisions["data"][0]["id"].as_str().unwrap();

    let decision_table_resource_data_response = client
        .get(format!(
            "{base_url}/dmn-repository/decision-tables/{decision_table_id}/resourcedata"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(decision_table_resource_data_response.status().is_success());
    assert_eq!(
        decision_table_resource_data_response.text().await.unwrap(),
        FIRST_HIT_DMN
    );

    let decision_resource_data_response = client
        .get(format!(
            "{base_url}/dmn-repository/decisions/{decision_table_id}/resourcedata"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(decision_resource_data_response.status().is_success());
    assert_eq!(
        decision_resource_data_response.text().await.unwrap(),
        FIRST_HIT_DMN
    );
}
