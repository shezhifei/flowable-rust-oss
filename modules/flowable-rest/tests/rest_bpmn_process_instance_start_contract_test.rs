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
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

fn one_task_process_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
            <process id="{process_id}" name="{process_id}" isExecutable="true">
                <startEvent id="start" />
                <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
                <userTask id="task1" name="Task 1" />
                <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
                <endEvent id="end" />
            </process>
        </definitions>"#
    )
}

async fn deploy_via_rest(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
) -> reqwest::Response {
    client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": one_task_process_xml(process_id)
        }))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn authenticated_start_records_starter_and_supports_involved_user_queries() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-authenticated-starter").await;

    let deploy_response = deploy_via_rest(&client, &base_url, "authenticatedStarterProcess").await;
    assert!(deploy_response.status().is_success());
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let started: Value = start_response.json().await.unwrap();
    let process_instance_id = started["id"].as_str().unwrap();

    let stored = engine
        .get_runtime_store()
        .db_store()
        .find_by_id::<flowable_engine::runtime::process_instance::ProcessInstance>(
            "process_instances",
            process_instance_id,
        )
        .unwrap()
        .expect("runtime process instance should exist");
    assert_eq!(stored.start_user_id.as_deref(), Some("admin"));

    for path in [
        format!("/runtime/process-instances/{process_instance_id}/identitylinks"),
        format!("/history/historic-process-instances/{process_instance_id}/identitylinks"),
    ] {
        let response = client
            .get(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success(), "path was {path}");
        let links: Value = response.json().await.unwrap();
        assert!(links.as_array().unwrap().iter().any(|link| {
            (link["user"] == "admin" || link["userId"] == "admin") && link["type"] == "starter"
        }));
    }

    for path in [
        "/runtime/process-instances?involvedUser=admin",
        "/history/historic-process-instances?involvedUser=admin",
        "/runtime/process-instances?startedBy=admin",
        "/history/historic-process-instances?startedBy=admin",
    ] {
        let response = client
            .get(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success(), "path was {path}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["total"], 1, "body for {path} was {body}");
        assert_eq!(body["data"][0]["id"], process_instance_id);
    }

    for path in [
        "/runtime/process-instances?involvedUser=someone-else",
        "/history/historic-process-instances?involvedUser=someone-else",
    ] {
        let response = client
            .get(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success(), "path was {path}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["total"], 0, "body for {path} was {body}");
    }
}

#[tokio::test]
async fn start_process_instance_by_key_sets_initial_variables_and_returns_them() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-start-by-key").await;

    let deploy_response = deploy_via_rest(&client, &base_url, "startByKeyProcess").await;
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "startByKeyProcess",
            "businessKey": "BK-42",
            "returnVariables": true,
            "variables": [
                { "name": "amount", "type": "integer", "value": 42 },
                { "name": "approved", "type": "boolean", "value": true }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    assert_eq!(start_body["processDefinitionId"], process_definition_id);
    assert_eq!(start_body["businessKey"], "BK-42");
    assert_eq!(start_body["variables"].as_array().unwrap().len(), 2);
    assert_eq!(start_body["variables"][0]["name"], "amount");
    assert_eq!(start_body["variables"][0]["type"], "integer");
    assert_eq!(start_body["variables"][0]["value"], 42);
    assert_eq!(start_body["variables"][0]["scope"], "local");
    assert_eq!(start_body["variables"][1]["name"], "approved");
    assert_eq!(start_body["variables"][1]["type"], "boolean");
    assert_eq!(start_body["variables"][1]["value"], true);

    let process_instance_id = start_body["id"].as_str().unwrap();
    let variables_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(variables_response.status().is_success());
    let variables_body: Value = variables_response.json().await.unwrap();
    assert_eq!(variables_body.as_array().unwrap().len(), 2);
    assert_eq!(variables_body[0]["name"], "amount");
    assert_eq!(variables_body[0]["value"], 42);
    assert_eq!(variables_body[1]["name"], "approved");
    assert_eq!(variables_body[1]["value"], true);
}

#[tokio::test]
async fn start_process_instance_rejects_process_definition_id_and_key_together() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-start-id-key-conflict").await;

    let deploy_response = deploy_via_rest(&client, &base_url, "idKeyConflictProcess").await;
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "processDefinitionKey": "idKeyConflictProcess"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "BAD_REQUEST");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("Only one of processDefinitionId, processDefinitionKey or message")
    );
}

#[tokio::test]
async fn start_process_instance_by_key_honors_tenant_id_when_repository_has_tenant() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-start-key-tenant").await;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Tenant deployment".to_string())
                .tenant_id("tenant-a".to_string())
                .add_string(
                    "tenant_process.bpmn20.xml".to_string(),
                    one_task_process_xml("tenantStartProcess"),
                ),
        )
        .unwrap();

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definitions()
        .unwrap()
        .into_iter()
        .find(|definition| definition.tenant_id.as_deref() == Some("tenant-a"))
        .unwrap()
        .id;

    let missing_tenant_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "tenantStartProcess",
            "tenantId": "tenant-b"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing_tenant_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "tenantStartProcess",
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();

    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    assert_eq!(start_body["processDefinitionId"], process_definition_id);
}
