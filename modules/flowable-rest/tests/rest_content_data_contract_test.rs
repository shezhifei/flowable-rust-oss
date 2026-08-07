use flowable_content_service::{CreateContentItemRequest, FlowableContentService};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
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

#[tokio::test]
async fn content_item_data_endpoint_returns_durable_payload_and_mime_type() {
    let (_engine, base_url, client) = spawn_server("rest-content-data").await;

    let create = client
        .post(format!("{}/content-service/content-items", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "policy.txt",
            "mimeType": "text/plain",
            "taskId": "task-data",
            "processInstanceId": "process-data",
            "scopeType": "bpmn",
            "scopeId": "scope-data",
            "content": "approved payload"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body: Value = create.json().await.unwrap();
    let content_item_id = create_body["id"].as_str().unwrap();

    let get_data = client
        .get(format!(
            "{}/content-service/content-items/{}/data",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        get_data
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "text/plain"
    );
    assert_eq!(
        get_data.bytes().await.unwrap().as_ref(),
        b"approved payload"
    );
}

#[tokio::test]
async fn content_item_data_endpoint_returns_structured_not_found_for_missing_payload() {
    let (engine, base_url, client) = spawn_server("rest-content-data-missing").await;

    let metadata_only = FlowableContentService::new(Arc::clone(&engine))
        .create_content_item(CreateContentItemRequest {
            name: "metadata-only.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: None,
            task_id: Some("task-metadata".to_string()),
            process_instance_id: Some("process-metadata".to_string()),
            scope_type: Some("bpmn".to_string()),
            scope_id: Some("scope-metadata".to_string()),
            created_by: None,
            expires_in_seconds: None,
        })
        .unwrap();

    let response = client
        .get(format!(
            "{}/content-service/content-items/{}/data",
            base_url, metadata_only.id
        ))
        .basic_auth("admin", Some("test"))
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
            .contains(metadata_only.id.as_str())
    );
}

#[tokio::test]
async fn content_item_list_accepts_created_sort_and_applies_paging_after_sorting() {
    let (_engine, base_url, client) = spawn_server("rest-content-created-sort").await;

    let old = client
        .post(format!("{}/content-service/content-items", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "old.txt",
            "mimeType": "text/plain",
            "content": "old"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), reqwest::StatusCode::CREATED);

    tokio::time::sleep(Duration::from_millis(25)).await;

    let new = client
        .post(format!("{}/content-service/content-items", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "new.txt",
            "mimeType": "text/plain",
            "content": "new"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(new.status(), reqwest::StatusCode::CREATED);

    let sorted = client
        .get(format!(
            "{}/content-service/content-items?sort=created&order=desc&start=0&size=1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(sorted.status(), reqwest::StatusCode::OK);
    let body: Value = sorted.json().await.unwrap();
    assert_eq!(body["start"], 0);
    assert_eq!(body["size"], 1);
    assert_eq!(body["total"], 2);
    assert_eq!(body["data"][0]["name"], "new.txt");
}
