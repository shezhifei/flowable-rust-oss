use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::fs;
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

#[tokio::test]
async fn content_storage_endpoints_cover_metadata_data_status_and_delete() {
    let (_engine, base_url, client) = spawn_server("rest-content-storage-m41").await;

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

    let metadata = client
        .get(format!(
            "{}/content-service/content-items/{}/object",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = metadata.json().await.unwrap();
    assert_eq!(metadata_body["storageBackend"], "local-fs");
    assert_eq!(metadata_body["size"], 16);
    assert!(metadata_body["storageId"].is_string());
    assert!(metadata_body["checksum"].is_string());

    let object_data = client
        .get(format!(
            "{}/content-service/content-items/{}/object/data",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(object_data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        object_data
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "text/plain"
    );
    assert_eq!(
        object_data.bytes().await.unwrap().as_ref(),
        b"approved payload"
    );

    let storage_status = client
        .get(format!("{}/content-service/storage/status", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(storage_status.status(), reqwest::StatusCode::OK);
    let status_body: Value = storage_status.json().await.unwrap();
    assert_eq!(status_body["backend"], "local-fs");
    assert_eq!(status_body["status"], "ok");

    let delete = client
        .delete(format!(
            "{}/content-service/content-items/{}",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn content_storage_endpoints_return_structured_not_found_when_object_is_missing() {
    let (_engine, base_url, client) = spawn_server("rest-content-storage-object-missing").await;

    let create = client
        .post(format!("{}/content-service/content-items", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "missing-object.txt",
            "mimeType": "text/plain",
            "content": "payload stored then removed"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body: Value = create.json().await.unwrap();
    let content_item_id = create_body["id"].as_str().unwrap();

    let metadata = client
        .get(format!(
            "{}/content-service/content-items/{}/object",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = metadata.json().await.unwrap();
    let storage_id = metadata_body["storageId"].as_str().unwrap();
    let storage_path = std::env::current_dir()
        .unwrap()
        .join("flowable-content-storage")
        .join(&storage_id[..2])
        .join(storage_id);
    fs::remove_file(storage_path).unwrap();

    let missing_metadata = client
        .get(format!(
            "{}/content-service/content-items/{}/object",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_metadata.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_metadata_body: Value = missing_metadata.json().await.unwrap();
    assert_eq!(missing_metadata_body["code"], "NOT_FOUND");
    assert!(
        missing_metadata_body["details"]
            .as_str()
            .unwrap()
            .contains(storage_id)
    );

    let missing_data = client
        .get(format!(
            "{}/content-service/content-items/{}/object/data",
            base_url, content_item_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_data.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_data_body: Value = missing_data.json().await.unwrap();
    assert_eq!(missing_data_body["code"], "NOT_FOUND");
    assert!(
        missing_data_body["details"]
            .as_str()
            .unwrap()
            .contains(content_item_id)
    );
}
