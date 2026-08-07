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
        id: "kermit".to_string(),
        first_name: Some("Kermit".to_string()),
        last_name: None,
        email: Some("kermit@muppets.test".to_string()),
        password: Some("thegreen".to_string()),
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
async fn token_rest_endpoints_cover_create_query_and_delete() {
    let (_engine, base_url, client) = spawn_server("rest-token-contract").await;

    let create = client
        .post(format!("{}/identity/tokens", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "token-1",
            "token_value": "alpha-token",
            "user_id": "kermit"
        }))
        .send()
        .await
        .unwrap();

    assert!(create.status().is_success());
    let created: Value = create.json().await.unwrap();
    assert_eq!(created["id"], "token-1");
    assert_eq!(created["token_value"], "alpha-token");
    assert_eq!(created["user_id"], "kermit");

    let list_by_user = client
        .get(format!("{}/identity/tokens?user_id=kermit", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_by_user.status().is_success());
    let list_by_user_body: Value = list_by_user.json().await.unwrap();
    assert_eq!(list_by_user_body.as_array().unwrap().len(), 1);
    assert_eq!(list_by_user_body[0]["id"], "token-1");

    let list_by_value = client
        .get(format!(
            "{}/identity/tokens?token_value=alpha-token",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_by_value.status().is_success());
    let list_by_value_body: Value = list_by_value.json().await.unwrap();
    assert_eq!(list_by_value_body.as_array().unwrap().len(), 1);
    assert_eq!(list_by_value_body[0]["user_id"], "kermit");

    let delete = client
        .delete(format!("{}/identity/tokens/token-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let list_after_delete = client
        .get(format!("{}/identity/tokens?user_id=kermit", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list_after_delete.status().is_success());
    let list_after_delete_body: Value = list_after_delete.json().await.unwrap();
    assert!(list_after_delete_body.as_array().unwrap().is_empty());
}
