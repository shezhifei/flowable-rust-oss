use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn rest_http_smoke_test() {
    // 1. Setup Engine and Server
    let engine = Arc::new(ProcessEngine::new("rest-http-smoke".to_string()));

    // Add admin user
    let user = flowable_engine::identity::entities::User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    };
    engine.get_identity_service().save_user(user);

    // Bind to a random port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    // Run server in background
    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    let client = reqwest::Client::new();

    // 1.5. Test Unauthorized Request
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="httpProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="HTTP Task" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let res_unauth = client
        .post(format!("{}/repository/deployments", base_url))
        .json(&json!({
            "name": "HTTP Deployment",
            "resourceName": "http_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .expect("request should be sent");
    assert_eq!(res_unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Test Deployment (BPMN XML)
    let res = client
        .post(format!("{}/repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "HTTP Deployment",
            "resourceName": "http_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .expect("deployment request should succeed");

    assert!(res.status().is_success());
    let deployment: Value = res.json().await.unwrap();
    assert_eq!(deployment["name"], "HTTP Deployment");

    // 3. Test Process Instance Start
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let res = client
        .post(format!("{}/runtime/process-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "HTTP Instance"
        }))
        .send()
        .await
        .expect("start process request should succeed");

    assert!(res.status().is_success());
    let process_instance: Value = res.json().await.unwrap();
    let process_instance_id = process_instance["id"].as_str().unwrap().to_string();

    // 4. Test Task Query
    let res = client
        .get(format!(
            "{}/runtime/tasks?processInstanceId={}",
            base_url, process_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .expect("task query request should succeed");

    assert!(res.status().is_success());
    let tasks: Value = res.json().await.unwrap();
    let task_array = tasks["data"].as_array().expect("tasks should be an array");
    assert_eq!(task_array.len(), 1);
    let task_id = task_array[0]["id"].as_str().unwrap().to_string();

    // 5. Test Task Completion
    let res = client
        .post(format!("{}/runtime/tasks/{}/complete", base_url, task_id))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .expect("task complete request should succeed");

    assert!(res.status().is_success());

    // 6. Verify Engine State
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored_pi = store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should remain persisted");
    assert!(stored_pi.is_ended);
    let _ = session.rollback();
}
