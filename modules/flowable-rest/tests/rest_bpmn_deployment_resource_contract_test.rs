use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
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

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

#[tokio::test]
async fn bpmn_deployment_resources_are_listed_and_return_stored_bytes() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-deployment-resources").await;
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="resourceContractProcess" name="Resource Contract Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Resource Contract Deployment",
            "resourceName": "models/resource_contract_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());
    let deployment: Value = deploy_response.json().await.unwrap();
    let deployment_id = deployment["id"].as_str().unwrap();

    let get_deployment_response = client
        .get(format!("{base_url}/repository/deployments/{deployment_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_deployment_response.status().is_success());
    let get_deployment_body: Value = get_deployment_response.json().await.unwrap();
    assert_eq!(get_deployment_body["id"], deployment_id);
    assert_eq!(get_deployment_body["name"], "Resource Contract Deployment");

    let resources_response = client
        .get(format!(
            "{base_url}/repository/deployments/{deployment_id}/resources"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(resources_response.status().is_success());
    let resources: Value = resources_response.json().await.unwrap();
    assert_eq!(resources.as_array().unwrap().len(), 1);
    assert_eq!(
        resources[0]["id"],
        "models/resource_contract_process.bpmn20.xml"
    );
    assert_eq!(
        resources[0]["url"],
        format!(
            "/repository/deployments/{deployment_id}/resourcedata/models/resource_contract_process.bpmn20.xml"
        )
    );

    let resource_data_response = client
        .get(format!(
            "{base_url}/repository/deployments/{deployment_id}/resourcedata/models/resource_contract_process.bpmn20.xml"
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
            "{base_url}/repository/deployments/{deployment_id}/resources/models/resource_contract_process.bpmn20.xml"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(resource_entry_response.status().is_success());
    let resource_entry: Value = resource_entry_response.json().await.unwrap();
    assert_eq!(
        resource_entry["id"],
        "models/resource_contract_process.bpmn20.xml"
    );

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let definition_resource_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/resourcedata"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definition_resource_response.status().is_success());
    assert_eq!(definition_resource_response.text().await.unwrap(), xml);

    let definition_model_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/model"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definition_model_response.status().is_success());
    let model: Value = definition_model_response.json().await.unwrap();
    let model_json = model.to_string();
    assert!(model_json.contains("resourceContractProcess"));
    assert!(model_json.contains("start"));
}
