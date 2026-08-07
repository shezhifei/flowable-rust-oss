use flowable_http_service::{
    AsyncHttpRuntime, AsyncHttpRuntimeConfig, HttpRequest, HttpRuntime, RealHttpClientConfig,
};
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{Duration, Instant, sleep};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_runtime_uses_non_blocking_reqwest_io() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut request = vec![0_u8; 1024];
                let read = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]);
                if request.contains("GET /slow ") {
                    sleep(Duration::from_millis(250)).await;
                }
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}")
                    .await
                    .unwrap();
            });
        }
    });

    let runtime = AsyncHttpRuntime::from_real_client(
        RealHttpClientConfig {
            retry_count: 0,
            allow_private_networks: true,
            ..Default::default()
        },
        AsyncHttpRuntimeConfig::default(),
    )
    .unwrap();
    let slow = HttpRequest {
        method: "GET".to_string(),
        url: format!("http://{address}/slow"),
        headers: BTreeMap::new(),
        body: None,
        timeout_ms: Some(2_000),
        connect_timeout_ms: None,
        follow_redirects: None,
        basic_auth: None,
        body_encoding: None,
    };
    let fast = HttpRequest {
        url: format!("http://{address}/fast"),
        ..slow.clone()
    };

    let started = Instant::now();
    let slow_future = runtime.execute_async(&slow);
    let fast_future = runtime.execute_async(&fast);
    let (slow_result, fast_result) = tokio::join!(slow_future, fast_future);

    assert_eq!(slow_result.unwrap().response.status_code, 200);
    assert_eq!(fast_result.unwrap().response.status_code, 200);
    assert!(started.elapsed() < Duration::from_millis(450));
}
