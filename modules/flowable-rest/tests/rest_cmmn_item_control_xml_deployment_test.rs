use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const ITEM_CONTROL_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="itemControlCase" name="Item Control Case">
    <casePlanModel id="itemControlPlan" name="Item Control Plan" autoComplete="false">
      <planItem id="planItemReview" name="Review" definitionRef="reviewTask">
        <itemControl>
          <manualActivationRule>
            <condition>manualActivation == true</condition>
          </manualActivationRule>
          <repetitionRule>
            <condition>repeatReview == true</condition>
          </repetitionRule>
        </itemControl>
      </planItem>
      <humanTask id="reviewTask" name="Review" isBlocking="true" />
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

#[tokio::test]
async fn cmmn_xml_deployment_preserves_item_control_rules_into_runtime_model() {
    let (base_url, client) = spawn_server("rest-cmmn-item-control-xml-deployment").await;

    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Item Control Deployment",
            "resourceName": "item-control.cmmn",
            "resource": ITEM_CONTROL_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_response.status(), StatusCode::CREATED);

    let definitions_response = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions?key=itemControlCase"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definitions_response.status(), StatusCode::OK);
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
    assert_eq!(model_response.status(), StatusCode::OK);
    let model: Value = model_response.json().await.unwrap();
    let plan_item = &model["model"]["case_plan_model"]["plan_items"][0];
    assert_eq!(
        plan_item["manual_activation_rule"]["Comparison"]["variable_name"],
        "manualActivation"
    );
    assert_eq!(
        plan_item["repetition_rule"]["Comparison"]["variable_name"],
        "repeatReview"
    );

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "itemControlCase",
            "variables": {
                "manualActivation": true,
                "repeatReview": true
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), StatusCode::CREATED);
    let case_instance_id = start_response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let enabled_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}&state=ENABLED&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(enabled_response.status(), StatusCode::OK);
    let enabled_body: Value = enabled_response.json().await.unwrap();
    assert_eq!(enabled_body["total"], 1);
    assert_eq!(
        enabled_body["data"][0]["planItemDefinitionId"],
        // Java PlanItemInstanceEntityManagerImpl.java:92-95.
        "reviewTask"
    );
    let first_task_id = enabled_body["data"][0]["id"].as_str().unwrap();

    let activate_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "activatePlanItemDefinitionIds": ["reviewTask"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(activate_response.status(), StatusCode::OK);

    let complete_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{first_task_id}/complete"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(complete_response.status(), StatusCode::OK);

    let repeated_enabled_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}&state=ENABLED&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(repeated_enabled_response.status(), StatusCode::OK);
    let repeated_enabled_body: Value = repeated_enabled_response.json().await.unwrap();
    assert_eq!(repeated_enabled_body["total"], 1);
    assert_ne!(
        repeated_enabled_body["data"][0]["id"].as_str().unwrap(),
        first_task_id
    );
    assert_eq!(
        repeated_enabled_body["data"][0]["planItemDefinitionId"],
        // Java PlanItemInstanceEntityManagerImpl.java:92-95.
        "reviewTask"
    );
}
