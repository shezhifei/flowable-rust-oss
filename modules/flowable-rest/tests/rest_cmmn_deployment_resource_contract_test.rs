use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

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
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn cmmn_deployment_resources_are_listed_and_return_stored_bytes() {
    let (base_url, client) = spawn_server("rest-cmmn-deployment-resources").await;
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="resourceContractCase" name="Resource Contract Case">
    <casePlanModel id="resourceContractPlan" name="Resource Contract Plan" autoComplete="false">
      <planItem id="planItemRootTask" name="Root Task" definitionRef="rootTask" />
      <humanTask id="rootTask" name="Root Task" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN Resource Contract Deployment",
            "resourceName": "models/resource_contract_case.cmmn",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());
    let deployment: Value = deploy_response.json().await.unwrap();
    let deployment_id = deployment["id"].as_str().unwrap();

    let get_deployment_response = client
        .get(format!(
            "{base_url}/cmmn-repository/deployments/{deployment_id}"
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
        "CMMN Resource Contract Deployment"
    );

    let resources_response = client
        .get(format!(
            "{base_url}/cmmn-repository/deployments/{deployment_id}/resources"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(resources_response.status().is_success());
    let resources: Value = resources_response.json().await.unwrap();
    assert_eq!(resources.as_array().unwrap().len(), 1);
    assert_eq!(resources[0]["id"], "models/resource_contract_case.cmmn");
    assert_eq!(
        resources[0]["url"],
        format!(
            "/cmmn-repository/deployments/{deployment_id}/resourcedata/models/resource_contract_case.cmmn"
        )
    );
    assert_eq!(resources[0]["mediaType"], "application/xml");

    let resource_data_response = client
        .get(format!(
            "{base_url}/cmmn-repository/deployments/{deployment_id}/resourcedata/models/resource_contract_case.cmmn"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(resource_data_response.status().is_success());
    assert_eq!(
        resource_data_response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/xml"
    );
    assert_eq!(resource_data_response.text().await.unwrap(), xml);

    let resource_entry_response = client
        .get(format!(
            "{base_url}/cmmn-repository/deployments/{deployment_id}/resources/models/resource_contract_case.cmmn"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(resource_entry_response.status().is_success());
    let resource_entry: Value = resource_entry_response.json().await.unwrap();
    assert_eq!(resource_entry["id"], "models/resource_contract_case.cmmn");

    let definitions_response = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions?key=resourceContractCase"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definitions_response.status().is_success());
    let definitions: Value = definitions_response.json().await.unwrap();
    let case_definition_id = definitions["data"][0]["id"].as_str().unwrap();

    let definition_resource_response = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_definition_id}/resourcedata"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definition_resource_response.status().is_success());
    assert_eq!(definition_resource_response.text().await.unwrap(), xml);

    let definition_model_response = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_definition_id}/model"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definition_model_response.status().is_success());
    let model: Value = definition_model_response.json().await.unwrap();
    let model_json = model.to_string();
    assert!(model_json.contains("resourceContractCase"));
    assert!(model_json.contains("rootTask"));
}

#[tokio::test]
async fn cmmn_xml_deployment_preserves_stage_planning_table_in_definition_model() {
    let (base_url, client) = spawn_server("rest-cmmn-planning-table-model").await;
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="planningTableCase" name="Planning Table Case">
    <casePlanModel id="planningPlan" name="Planning Plan" autoComplete="false">
      <planItem id="planItemReviewStage" definitionRef="reviewStage" />
      <stage id="reviewStage" name="Review Stage">
        <planItem id="planItemAnchor" definitionRef="anchorTask" />
        <humanTask id="anchorTask" name="Anchor Task" />
        <humanTask id="peerReviewTask" name="Peer Review" />
        <planningTable id="reviewPlanningTable" name="Review Planning">
          <discretionaryItem id="discretionaryPeerReview" name="Peer Review" definitionRef="peerReviewTask" />
        </planningTable>
      </stage>
    </casePlanModel>
  </case>
</definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN Planning Table Deployment",
            "resourceName": "models/planning_table_case.cmmn",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let definitions_response = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions?key=planningTableCase"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definitions_response.status().is_success());
    let definitions: Value = definitions_response.json().await.unwrap();
    let case_definition_id = definitions["data"][0]["id"].as_str().unwrap();

    let model_response = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_definition_id}/model"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(model_response.status().is_success());
    let model: Value = model_response.json().await.unwrap();
    let review_stage = &model["model"]["case_plan_model"]["stages"][0];
    assert_eq!(
        review_stage["planning_tables"][0]["id"],
        "reviewPlanningTable"
    );
    assert_eq!(
        review_stage["planning_tables"][0]["discretionary_items"][0]["definition_ref"],
        "peerReviewTask"
    );
}
