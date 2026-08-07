use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_authenticated_metrics_server(name: &str) -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(name.to_string()));
    // run_server treats user id "admin" as REST admin; seed credentials for basic auth.
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
    let addr = listener.local_addr().unwrap();
    let engine_clone = engine.clone();
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (format!("http://{}", addr), reqwest::Client::new())
}

#[tokio::test]
async fn test_metrics_endpoint() {
    let (base_url, client) = spawn_authenticated_metrics_server("test").await;
    let url = format!("{}/metrics", base_url);

    let res = client
        .get(&url)
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);

    // Request-id propagation from SetRequestIdLayer + PropagateRequestIdLayer.
    let request_id = res
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .expect("x-request-id response header from Trace/request-id layers");
    assert!(!request_id.is_empty());

    let body = res.text().await.unwrap();
    assert!(body.contains("flowable_process_instances_total"));
    assert!(body.contains("flowable_tasks_total"));
    // Job lifecycle / timer coordination Prometheus surface (C3).
    assert!(body.contains("flowable_job_acquire_attempts_total"));
    assert!(body.contains("flowable_job_acquire_conflicts_total"));
    assert!(body.contains("flowable_job_acquired_total"));
    assert!(body.contains("flowable_job_acquire_batch_size"));
    assert!(body.contains("flowable_timer_lease_renew_successes_total"));
    assert!(body.contains("flowable_job_execute_total"));
    assert!(body.contains("flowable_job_execute_failures_total"));
}

#[tokio::test]
async fn metrics_propagates_client_request_id() {
    let (base_url, client) = spawn_authenticated_metrics_server("test-req-id").await;
    let res = client
        .get(format!("{}/metrics", base_url))
        .basic_auth("admin", Some("test"))
        .header("x-request-id", "client-trace-abc")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    assert_eq!(
        res.headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
        Some("client-trace-abc")
    );
}
