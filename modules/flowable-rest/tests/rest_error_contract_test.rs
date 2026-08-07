use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn rest_error_contract_test() {
    let engine = Arc::new(ProcessEngine::new("rest-error-contract".to_string()));

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

    // 1. missing auth returns structured 401
    let res = client
        .post(format!("{}/repository/deployments", base_url))
        .json(&json!({"name": "Test"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["code"], "UNAUTHORIZED");
    assert!(body["message"].is_string());

    // 2. invalid payload returns structured 4xx JSON error
    let res = client
        .post(format!("{}/runtime/process-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({"invalid_field": "data"})) // missing processDefinitionId
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["code"], "BAD_REQUEST");

    // 3. unknown process definition returns structured 404
    let res = client
        .post(format!("{}/runtime/process-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": "unknown_id"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");

    // 4. complete request with empty body returns structured 400
    let res = client
        .post(format!("{}/runtime/tasks/unknown_task/complete", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["code"], "BAD_REQUEST");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("canonical 'action: \"complete\"' shape")
    );

    // 5. complete request for unknown task with canonical body returns structured 404
    let res = client
        .post(format!("{}/runtime/tasks/unknown_task/complete", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");

    // 5. history query endpoint returns stable JSON shape
    let res = client
        .get(format!("{}/history/historic-process-instances", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["start"], 0);
    assert_eq!(body["size"], 0);
    assert_eq!(body["total"], 0);
    assert!(body["data"].is_array());
}
