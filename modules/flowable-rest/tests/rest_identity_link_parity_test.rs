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
async fn rest_identity_link_create_query_delete_parity() {
    let (_engine, base_url, client) = spawn_server("rest-identity-link-parity").await;

    let create = client
        .post(format!("{}/runtime/identity-links", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "il-1",
            "linkType": "candidate",
            "userId": "kermit",
            "taskId": "task-1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::OK);

    let create2 = client
        .post(format!("{}/runtime/identity-links", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "il-2",
            "linkType": "assignee",
            "groupId": "admins",
            "processInstanceId": "proc-1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create2.status(), reqwest::StatusCode::OK);

    let list = client
        .get(format!("{}/runtime/identity-links", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let body: Value = list.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);

    let by_task = client
        .get(format!("{}/runtime/identity-links?taskId=task-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_task.status(), reqwest::StatusCode::OK);
    let by_task_body: Value = by_task.json().await.unwrap();
    assert_eq!(by_task_body.as_array().unwrap().len(), 1);
    assert_eq!(by_task_body[0]["userId"], "kermit");

    let by_proc = client
        .get(format!(
            "{}/runtime/identity-links?processInstanceId=proc-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_proc.status(), reqwest::StatusCode::OK);
    let by_proc_body: Value = by_proc.json().await.unwrap();
    assert_eq!(by_proc_body.as_array().unwrap().len(), 1);
    assert_eq!(by_proc_body[0]["groupId"], "admins");

    let delete = client
        .delete(format!("{}/runtime/identity-links/il-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let after_delete = client
        .get(format!("{}/runtime/identity-links?taskId=task-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let after_body: Value = after_delete.json().await.unwrap();
    assert!(after_body.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn rest_task_identity_links_parity() {
    let (_engine, base_url, client) = spawn_server("rest-task-identity-link-parity").await;

    client
        .post(format!("{}/runtime/identity-links", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "il-task-1",
            "linkType": "candidate",
            "userId": "kermit",
            "taskId": "task-1"
        }))
        .send()
        .await
        .unwrap();

    let list = client
        .get(format!("{}/runtime/identity-links?taskId=task-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let body: Value = list.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["linkType"], "candidate");
    assert_eq!(body[0]["taskId"], "task-1");
}
