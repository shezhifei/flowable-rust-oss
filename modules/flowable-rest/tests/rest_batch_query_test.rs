use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    engine.get_identity_service().save_user(User {
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
async fn rest_batch_create_query_delete_parity() {
    let (_engine, base_url, client) = spawn_server("rest-batch-query-parity").await;

    let create = client
        .post(format!("{}/management/batches", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "batch-1",
            "batchType": "processMigration",
            "status": "completed",
            "totalItems": 10,
            "itemsProcessed": 10,
            "createTime": 0
        }))
        .send()
        .await
        .unwrap();
    assert!(create.status().is_success());
    let created: Value = create.json().await.unwrap();
    assert_eq!(created["id"], "batch-1");

    let create2 = client
        .post(format!("{}/management/batches", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "batch-2",
            "batchType": "decisionTable",
            "status": "running",
            "totalItems": 5,
            "itemsProcessed": 2,
            "createTime": 0
        }))
        .send()
        .await
        .unwrap();
    assert!(create2.status().is_success());

    let list = client
        .get(format!("{}/management/batches", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let body: Value = list.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 2);

    let by_type = client
        .get(format!(
            "{}/management/batches?batchType=processMigration",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let by_type_body: Value = by_type.json().await.unwrap();
    assert_eq!(by_type_body["data"].as_array().unwrap().len(), 1);
    assert_eq!(by_type_body["data"][0]["id"], "batch-1");

    let by_status = client
        .get(format!("{}/management/batches?status=running", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let by_status_body: Value = by_status.json().await.unwrap();
    assert_eq!(by_status_body["data"].as_array().unwrap().len(), 1);
    assert_eq!(by_status_body["data"][0]["id"], "batch-2");

    let get = client
        .get(format!("{}/management/batches/batch-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), reqwest::StatusCode::OK);
    let batch: Value = get.json().await.unwrap();
    assert_eq!(batch["batchType"], "processMigration");
    assert_eq!(batch["status"], "completed");

    let delete = client
        .delete(format!("{}/management/batches/batch-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let after = client
        .get(format!("{}/management/batches", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let after_body: Value = after.json().await.unwrap();
    assert_eq!(after_body["data"].as_array().unwrap().len(), 1);
    assert_eq!(after_body["data"][0]["id"], "batch-2");
}

#[tokio::test]
async fn rest_batch_not_found_returns_structured_error() {
    let (_engine, base_url, client) = spawn_server("rest-batch-not-found").await;

    let resp = client
        .get(format!("{}/management/batches/nonexistent", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");
}
