use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const ADHOC_MANUAL_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="adhocRestProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="adhocSubProcess" />
        <adHocSubProcess id="adhocSubProcess">
            <userTask id="innerUserTask" name="Inner User Task" />
        </adHocSubProcess>
        <sequenceFlow id="flow2" sourceRef="adhocSubProcess" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

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
    let engine_for_server = Arc::clone(&engine);

    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_process(client: &reqwest::Client, base_url: &str) -> String {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Adhoc REST deployment",
            "resourceName": "adhoc-rest.bpmn20.xml",
            "resource": ADHOC_MANUAL_XML
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "deployment failed: {}",
        response.text().await.unwrap()
    );

    let definitions_response = client
        .get(format!("{base_url}/repository/process-definitions"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definitions_response.status().is_success());
    let definitions_body: Value = definitions_response.json().await.unwrap();
    definitions_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|definition| definition["key"].as_str() == Some("adhocRestProcess"))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn start_process_instance(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "variables": []
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "start failed: {}",
        response.text().await.unwrap()
    );

    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn runtime_tasks(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> Vec<Value> {
    let response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    body["data"].as_array().unwrap().clone()
}

#[tokio::test]
async fn process_instance_adhoc_task_endpoints_activate_and_complete_waiting_task() {
    let (engine, base_url, client) = spawn_server("rest-adhoc-task-endpoints").await;
    let process_definition_id = deploy_process(&client, &base_url).await;
    let process_instance_id =
        start_process_instance(&client, &base_url, &process_definition_id).await;

    assert!(
        runtime_tasks(&client, &base_url, &process_instance_id)
            .await
            .is_empty(),
        "ad-hoc subprocess should wait until a task is manually activated"
    );

    let activate_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/adhoc-tasks/activate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskId": "innerUserTask"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        activate_response.status().is_success(),
        "activate failed: {}",
        activate_response.text().await.unwrap()
    );

    let tasks = runtime_tasks(&client, &base_url, &process_instance_id).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["taskDefinitionKey"], "innerUserTask");
    assert_eq!(tasks[0]["name"], "Inner User Task");

    let complete_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/adhoc-tasks/innerUserTask/complete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(
        complete_response.status().is_success(),
        "complete failed: {}",
        complete_response.text().await.unwrap()
    );

    assert!(
        runtime_tasks(&client, &base_url, &process_instance_id)
            .await
            .is_empty(),
        "completed ad-hoc user task should no longer be active"
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let process_instance = store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should remain queryable");
    assert!(process_instance.is_ended);
    let _ = session.rollback();
}

#[tokio::test]
async fn process_instance_adhoc_task_endpoint_returns_structured_error_for_missing_process() {
    let (_engine, base_url, client) = spawn_server("rest-adhoc-task-missing-process").await;

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/missing-process/adhoc-tasks/activate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskId": "innerUserTask"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("missing-process")
    );
}

#[tokio::test]
async fn process_instance_adhoc_task_endpoint_returns_structured_error_for_invalid_task() {
    let (_engine, base_url, client) = spawn_server("rest-adhoc-task-invalid-task").await;
    let process_definition_id = deploy_process(&client, &base_url).await;
    let process_instance_id =
        start_process_instance(&client, &base_url, &process_definition_id).await;

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/adhoc-tasks/activate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskId": "missingTask"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");
    assert!(body["details"].as_str().unwrap().contains("missingTask"));
}
