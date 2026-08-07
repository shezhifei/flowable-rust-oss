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
    let engine_for_server = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

fn user_task_xml(process_id: &str, process_name: &str, task_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="{process_id}" name="{process_name}" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="{task_name}" />
        <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#
    )
}

async fn deploy_and_start(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    process_name: &str,
    task_name: &str,
) -> Value {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": user_task_xml(process_id, process_name, task_name)
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{response:?}");

    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": process_id
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{response:?}");
    response.json().await.unwrap()
}

#[tokio::test]
async fn process_instance_query_supports_name_sort() {
    let (_engine, base_url, client) = spawn_server("m46_pi_name_sort").await;

    let pi_c = deploy_and_start(&client, &base_url, "charlie_proc", "Charlie", "TaskC").await;
    let pi_a = deploy_and_start(&client, &base_url, "alpha_proc", "Alpha", "TaskA").await;
    let pi_b = deploy_and_start(&client, &base_url, "bravo_proc", "Bravo", "TaskB").await;

    let response = client
        .get(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .query(&[("sort", "name"), ("order", "asc")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200, "{:?}", response.text().await);
    let body: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    let data = body["data"].as_array().unwrap();
    assert!(data.len() >= 3);

    let names: Vec<&str> = data
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    let mut sorted_names = names.clone();
    sorted_names.sort();
    assert_eq!(
        names, sorted_names,
        "process instances should be sorted by name asc"
    );

    let _ = (pi_a, pi_b, pi_c);
}

#[tokio::test]
async fn process_instance_query_supports_name_sort_desc() {
    let (_engine, base_url, client) = spawn_server("m46_pi_name_sort_desc").await;

    deploy_and_start(&client, &base_url, "charlie_proc", "Charlie", "TaskC").await;
    deploy_and_start(&client, &base_url, "alpha_proc", "Alpha", "TaskA").await;
    deploy_and_start(&client, &base_url, "bravo_proc", "Bravo", "TaskB").await;

    let response = client
        .get(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .query(&[("sort", "name"), ("order", "desc")])
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    let data = body["data"].as_array().unwrap();
    assert!(data.len() >= 3);

    let names: Vec<&str> = data
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .collect();
    let mut sorted_names = names.clone();
    sorted_names.sort();
    sorted_names.reverse();
    assert_eq!(
        names, sorted_names,
        "process instances should be sorted by name desc"
    );
}

#[tokio::test]
async fn historic_task_query_supports_description_sort() {
    let (_engine, base_url, client) = spawn_server("m46_ht_desc_sort").await;

    deploy_and_start(
        &client,
        &base_url,
        "desc_sort_proc",
        "DescSort",
        "ReviewTask",
    )
    .await;

    let response = client
        .get(format!("{base_url}/history/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .query(&[("sort", "description"), ("order", "asc")])
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        200,
        "description sort should be accepted, got: {:?}",
        response.text().await
    );
    let body: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert!(body["data"].as_array().is_some());
}
