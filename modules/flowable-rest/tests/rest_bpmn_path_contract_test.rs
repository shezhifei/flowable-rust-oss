use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn bpmn_runtime_repository_query_and_history_paths_are_available() {
    let engine = Arc::new(ProcessEngine::new("rest-bpmn-paths".to_string()));

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

    let client = reqwest::Client::new();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="pathProcess" name="Path Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Task 1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Path Deployment",
            "resourceName": "path_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let definition_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definition_response.status().is_success());
    let definition_body: Value = definition_response.json().await.unwrap();
    assert_eq!(definition_body["id"], process_definition_id);
    assert_eq!(definition_body["key"], "pathProcess");

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "Path Instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap();

    let instance_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(instance_response.status().is_success());
    let instance_body: Value = instance_response.json().await.unwrap();
    assert_eq!(instance_body["id"], process_instance_id);
    assert_eq!(instance_body["businessKey"], "Path Instance");

    let query_response = client
        .post(format!("{base_url}/query/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(query_response.status().is_success());
    let query_body: Value = query_response.json().await.unwrap();
    assert_eq!(query_body["total"], 1);
    assert_eq!(query_body["data"][0]["id"], process_instance_id);

    let historic_response = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_response.status().is_success());
    let historic_body: Value = historic_response.json().await.unwrap();
    assert_eq!(historic_body["id"], process_instance_id);
    assert_eq!(historic_body["processDefinitionId"], process_definition_id);

    let historic_query_response = client
        .post(format!("{base_url}/query/historic-process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(historic_query_response.status().is_success());
    let historic_query_body: Value = historic_query_response.json().await.unwrap();
    assert_eq!(historic_query_body["total"], 1);
    assert_eq!(historic_query_body["data"][0]["id"], process_instance_id);

    let create_identity_link_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "user": "kermit",
            "type": "starter"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_identity_link_response.status(), 201);
    let created_identity_link: Value = create_identity_link_response.json().await.unwrap();
    assert_eq!(created_identity_link["user"], "kermit");
    assert_eq!(created_identity_link["type"], "starter");
    assert!(created_identity_link["group"].is_null());

    let identity_links_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(identity_links_response.status().is_success());
    let identity_links: Value = identity_links_response.json().await.unwrap();
    let identity_links = identity_links.as_array().unwrap();
    assert_eq!(identity_links.len(), 2);
    assert!(
        identity_links
            .iter()
            .any(|link| link["user"] == "kermit" && link["type"] == "starter")
    );
    assert!(
        identity_links
            .iter()
            .any(|link| link["user"] == "admin" && link["type"] == "starter")
    );
}
