use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::json;
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
async fn rest_external_worker_job_complete_returns_not_found_for_missing_job() {
    let (_engine, base_url, client) = spawn_server("rest-external-worker-completion").await;

    let resp = client
        .post(format!(
            "{}/external-worker/jobs/nonexistent/complete",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "test-worker"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == reqwest::StatusCode::NOT_FOUND
            || resp.status() == reqwest::StatusCode::BAD_REQUEST,
        "Expected 404 or 400, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn rest_external_worker_job_bpmn_error_returns_not_found_for_missing_job() {
    let (_engine, base_url, client) = spawn_server("rest-external-worker-bpmn-error").await;

    let resp = client
        .post(format!(
            "{}/external-worker/jobs/nonexistent/bpmnError",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "test-worker",
            "errorCode": "TEST_ERROR"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == reqwest::StatusCode::NOT_FOUND
            || resp.status() == reqwest::StatusCode::BAD_REQUEST,
        "Expected 404 or 400, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn rest_external_worker_job_failure_returns_not_found_for_missing_job() {
    let (_engine, base_url, client) = spawn_server("rest-external-worker-failure").await;

    let resp = client
        .post(format!(
            "{}/external-worker/jobs/nonexistent/failure",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "test-worker",
            "errorMessage": "Something went wrong",
            "retries": 0
        }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == reqwest::StatusCode::NOT_FOUND
            || resp.status() == reqwest::StatusCode::BAD_REQUEST,
        "Expected 404 or 400, got {}",
        resp.status()
    );
}
