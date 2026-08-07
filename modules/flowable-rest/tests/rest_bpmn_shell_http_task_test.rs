use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

/// Helper: boot a REST server, return (base_url, engine, client).
async fn setup() -> (String, Arc<ProcessEngine>, reqwest::Client) {
    // Explicit opt-in: shell tasks disabled by default (security deviation from Java).
    let engine = Arc::new(ProcessEngine::new_with_config(
        "rest-bpmn-shell-http-contract".to_string(),
        ProcessEngineConfiguration {
            shell_tasks_enabled: true,
            ..Default::default()
        },
    ));

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
    (base_url, engine, client)
}

/// Deploy a BPMN XML and return the process definition ID.
async fn deploy(client: &reqwest::Client, base_url: &str, name: &str, xml: &str) -> String {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": format!("{name}.bpmn20.xml"),
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "deploy failed: {}",
        response.text().await.unwrap()
    );

    // Fetch process definition ID from the deployment
    let definitions_response = client
        .get(format!("{base_url}/repository/process-definitions"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definitions_response.status().is_success());
    let body: Value = definitions_response.json().await.unwrap();
    let definitions = body["data"].as_array().unwrap();
    definitions.last().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Start a process instance and return the process instance ID.
async fn start_process(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "start failed: {}",
        response.text().await.unwrap()
    );
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn test_rest_shell_task() {
    let (base_url, _engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="shellProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="shellTask" />
            <serviceTask id="shellTask" flowable:type="shell" flowable:resultVariableName="shellResult">
                <extensionElements>
                    <flowable:command>cmd</flowable:command>
                    <flowable:arg>/c</flowable:arg>
                    <flowable:arg>echo Hello World</flowable:arg>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="shellTask" targetRef="userTask" />
            <userTask id="userTask" />
            <sequenceFlow id="flow3" sourceRef="userTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let process_definition_id = deploy(&client, &base_url, "shell-test", xml).await;
    let process_instance_id = start_process(&client, &base_url, &process_definition_id).await;

    // Retrieve variables
    let response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "failed to get variables: {}",
        response.status()
    );
    let body: Value = response.json().await.unwrap();
    let variables = body.as_array().expect("Expected array of variables");

    let shell_result_var = variables
        .iter()
        .find(|v| v["name"] == "shellResult")
        .expect("Expected shellResult variable");

    let value = &shell_result_var["value"];
    assert_eq!(value["service"], "shell");
    assert_eq!(value["command"], "cmd");
    let stdout = value["stdout"].as_str().expect("stdout should be a string");
    assert!(stdout.contains("Hello World"));
}

#[tokio::test]
async fn test_rest_http_task() {
    let (base_url, _engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="httpProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="httpTask" />
            <serviceTask id="httpTask" flowable:type="http" flowable:resultVariableName="httpResult">
                <extensionElements>
                    <flowable:requestMethod>GET</flowable:requestMethod>
                    <flowable:requestUrl>https://api.example.com/data</flowable:requestUrl>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="httpTask" targetRef="userTask" />
            <userTask id="userTask" />
            <sequenceFlow id="flow3" sourceRef="userTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let process_definition_id = deploy(&client, &base_url, "http-test", xml).await;
    let process_instance_id = start_process(&client, &base_url, &process_definition_id).await;

    // Retrieve variables
    let response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "failed to get variables: {}",
        response.status()
    );
    let body: Value = response.json().await.unwrap();
    let variables = body.as_array().expect("Expected array of variables");

    let http_result_var = variables
        .iter()
        .find(|v| v["name"] == "httpResult")
        .expect("Expected httpResult variable");

    let value = &http_result_var["value"];
    assert_eq!(value["service"], "http");
    assert!(value.get("response").is_some());
    assert!(value["response"].get("statusCode").is_some());
}
