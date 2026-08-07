use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn test_metrics_endpoint() {
    let engine = Arc::new(ProcessEngine::new("test".to_string()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let engine_clone = engine.clone();

    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    // Give the server a moment to accept connections.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let url = format!("http://{}/metrics", addr);

    let res = client.get(&url).send().await.unwrap();
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
    let engine = Arc::new(ProcessEngine::new("test-req-id".to_string()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let engine_clone = engine.clone();
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let res = client
        .get(format!("http://{}/metrics", addr))
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
