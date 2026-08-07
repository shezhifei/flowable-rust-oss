//! Contract tests for the tenant-aware Content creation entry point (P1 fix).
//!
//! Rules under test:
//! - the authenticated user's tenant is persisted on REST-created content;
//! - a tenantless user still creates tenantless content;
//! - the request body cannot forge a tenant (unknown fields are rejected).

use flowable_content_service::FlowableContentService;
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
    engine.get_identity_service().save_user(User {
        id: "alice".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: Some("tenant-a".to_string()),
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
async fn authenticated_user_tenant_is_persisted_on_created_content() {
    let (engine, base_url, client) = spawn_server("rest-content-tenant-auth").await;

    let create = client
        .post(format!("{}/content-service/content-items", base_url))
        .basic_auth("alice", Some("test"))
        .json(&json!({
            "name": "tenant-upload.txt",
            "mimeType": "text/plain",
            "content": "tenant payload"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let body: Value = create.json().await.unwrap();
    let content_item_id = body["id"].as_str().unwrap();

    // The tenant comes from the authenticated identity, never from the body.
    let stored = FlowableContentService::new(Arc::clone(&engine))
        .get_content_item(content_item_id)
        .unwrap();
    assert_eq!(stored.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(stored.created_by.as_deref(), Some("alice"));
}

#[tokio::test]
async fn tenantless_user_creates_tenantless_content() {
    let (engine, base_url, client) = spawn_server("rest-content-tenant-none").await;

    let create = client
        .post(format!("{}/content-service/content-items", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "plain-upload.txt",
            "mimeType": "text/plain",
            "content": "plain payload"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let body: Value = create.json().await.unwrap();
    let content_item_id = body["id"].as_str().unwrap();

    let stored = FlowableContentService::new(Arc::clone(&engine))
        .get_content_item(content_item_id)
        .unwrap();
    assert_eq!(stored.tenant_id, None);
    assert_eq!(stored.created_by.as_deref(), Some("admin"));
}

#[tokio::test]
async fn request_body_cannot_forge_tenant() {
    let (engine, base_url, client) = spawn_server("rest-content-tenant-forge").await;

    // tenantId is not part of the request contract (deny_unknown_fields):
    // a tenant-a user must not be able to write content into tenant-b.
    let create = client
        .post(format!("{}/content-service/content-items", base_url))
        .basic_auth("alice", Some("test"))
        .json(&json!({
            "name": "forged-upload.txt",
            "mimeType": "text/plain",
            "content": "forged payload",
            "tenantId": "tenant-b"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(create.status(), reqwest::StatusCode::BAD_REQUEST);

    // Nothing may have been created for the rejected request.
    let items = FlowableContentService::new(Arc::clone(&engine))
        .create_content_item_query()
        .name("forged-upload.txt")
        .list()
        .unwrap();
    assert!(items.is_empty());
}
