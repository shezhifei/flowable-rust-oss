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
async fn rest_token_crud_and_query_parity() {
    let (engine, base_url, client) = spawn_server("rest-token-parity").await;

    engine.get_identity_service().save_user(User {
        id: "kermit".to_string(),
        first_name: Some("Kermit".to_string()),
        last_name: None,
        email: None,
        password: Some("pass".to_string()),
        tenant_id: None,
    });
    engine.get_identity_service().save_user(User {
        id: "fozzie".to_string(),
        first_name: Some("Fozzie".to_string()),
        last_name: None,
        email: None,
        password: Some("pass".to_string()),
        tenant_id: None,
    });

    let resp = client
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
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let resp2 = client
        .post(format!("{}/identity/tokens", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "token-2",
            "token_value": "beta-token",
            "user_id": "fozzie"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), reqwest::StatusCode::OK);

    let list = client
        .get(format!("{}/identity/tokens", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let body: Value = list.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);

    let by_user = client
        .get(format!("{}/identity/tokens?user_id=kermit", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_user.status(), reqwest::StatusCode::OK);
    let by_user_body: Value = by_user.json().await.unwrap();
    assert_eq!(by_user_body.as_array().unwrap().len(), 1);
    assert_eq!(by_user_body[0]["token_value"], "alpha-token");
    assert_eq!(by_user_body[0]["user_id"], "kermit");

    let delete = client
        .delete(format!("{}/identity/tokens/token-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let after_delete = client
        .get(format!("{}/identity/tokens?user_id=kermit", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let after_body: Value = after_delete.json().await.unwrap();
    assert!(after_body.as_array().unwrap().is_empty());
}
