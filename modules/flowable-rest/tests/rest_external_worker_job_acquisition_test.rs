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
async fn rest_external_worker_job_acquisition_returns_empty_when_no_jobs() {
    let (_engine, base_url, client) = spawn_server("rest-external-worker-acquisition").await;

    let resp = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "test-worker",
            "maxJobs": 10,
            "lockDurationMs": 30000
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert!(body.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn rest_external_worker_job_unlock_returns_success() {
    let (_engine, base_url, client) = spawn_server("rest-external-worker-unlock").await;

    let resp = client
        .post(format!(
            "{}/external-worker/jobs/nonexistent/unlock",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({"workerId": "test-worker"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}
