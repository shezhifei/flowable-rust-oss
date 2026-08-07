use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn rest_operational_endpoints_test() {
    let engine = Arc::new(ProcessEngine::new("rest-ops-test".to_string()));

    // Bind to a random port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    // Run server in background
    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    let client = reqwest::Client::new();

    // 1. Test /health (Liveness)
    let res = client
        .get(format!("{}/health", base_url))
        .send()
        .await
        .expect("health request should succeed");

    assert!(
        res.status().is_success(),
        "Failed with status: {}",
        res.status()
    );
    let health_body = res.text().await.unwrap();
    assert!(health_body.contains("UP"), "Body was: {}", health_body);

    // 2. Test /ready (Readiness)
    let res = client
        .get(format!("{}/ready", base_url))
        .send()
        .await
        .expect("ready request should succeed");

    assert!(res.status().is_success());
    let ready_body = res.text().await.unwrap();
    assert!(ready_body.contains("READY"));
}
