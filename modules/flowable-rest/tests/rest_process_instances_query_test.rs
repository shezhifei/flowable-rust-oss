use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn rest_process_instances_query_test() {
    let engine = Arc::new(ProcessEngine::new("rest-pi-query".to_string()));

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

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    let client = reqwest::Client::new();

    // 1. Deploy and start an instance
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="queryProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Task 1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let res = client
        .post(format!("{}/repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Query Deployment",
            "resourceName": "query_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();

    assert!(res.status().is_success());

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
            "businessKey": "Query Instance"
        }))
        .send()
        .await
        .unwrap();

    assert!(res.status().is_success());
    let started: Value = res.json().await.unwrap();
    let process_instance_id = started["id"].as_str().unwrap().to_string();

    // 2. Query process instances
    let res = client
        .get(format!("{}/runtime/process-instances", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(res.status().is_success());
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["start"], 0);
    assert_eq!(body["size"], 1);
    assert_eq!(body["total"], 1);
    let array = body["data"].as_array().unwrap();
    assert_eq!(array.len(), 1);
    assert_eq!(array[0]["businessKey"], "Query Instance");
    assert_eq!(array[0]["isEnded"], false);

    let tasks = client
        .get(format!(
            "{}/runtime/tasks?processInstanceId={}",
            base_url, process_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks.status().is_success());
    let tasks_body: Value = tasks.json().await.unwrap();
    let execution_id = tasks_body["data"][0]["executionId"].as_str().unwrap();
    engine
        .get_variable_service()
        .set_variable(
            execution_id.to_string(),
            "route".to_string(),
            json!("Accepted"),
        )
        .unwrap();

    let update = client
        .put(format!(
            "{}/runtime/process-instances/{}",
            base_url, process_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "businessStatus": "Ready For Review",
            "callbackId": "callback-1",
            "callbackType": "rest",
            "referenceId": "reference-1",
            "referenceType": "external"
        }))
        .send()
        .await
        .unwrap();
    assert!(update.status().is_success());

    let res = client
        .post(format!(
            "{}/query/process-instances?includeProcessVariables=true",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "businessKeyLike": "Query%",
            "businessStatusLikeIgnoreCase": "ready%",
            "callbackId": "callback-1",
            "callbackType": "rest",
            "referenceId": "reference-1",
            "referenceType": "external",
            "processInstanceVariables": [{
                "name": "route",
                "operation": "equalsIgnoreCase",
                "value": "accepted"
            }]
        }))
        .send()
        .await
        .unwrap();

    assert!(res.status().is_success());
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["total"], 1, "body was: {body}");
    assert_eq!(body["data"][0]["id"], process_instance_id);
    assert_eq!(body["data"][0]["businessStatus"], "Ready For Review");
    assert_eq!(body["data"][0]["callbackId"], "callback-1");
    assert_eq!(body["data"][0]["referenceId"], "reference-1");
    assert_eq!(
        body["data"][0]["variables"],
        json!([{
            "name": "route",
            "type": "string",
            "value": "Accepted",
            "scope": "global"
        }])
    );
}
