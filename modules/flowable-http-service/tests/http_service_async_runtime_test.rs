use flowable_http_service::{
    AsyncHttpRuntime, AsyncHttpRuntimeConfig, HttpRequest, HttpRuntime, HttpRuntimeMode,
    RealHttpClient, RealHttpClientConfig,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

fn slow_get_server(delay_ms: u64, hits: Arc<AtomicUsize>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let port = listener.local_addr().expect("local addr").port();
    let addr = format!("http://127.0.0.1:{port}");

    thread::spawn(move || {
        // Accept a small burst; handle each connection on its own thread so
        // concurrent clients actually overlap on the server side.
        for _ in 0..8 {
            if let Ok((mut stream, _)) = listener.accept() {
                let hits = Arc::clone(&hits);
                thread::spawn(move || {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf);
                    thread::sleep(Duration::from_millis(delay_ms));
                    let body = b"{\"ok\":true}";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        std::str::from_utf8(body).unwrap()
                    );
                    let _ = stream.write_all(response.as_bytes());
                });
            }
        }
    });

    // Give the accept loop a moment to start.
    thread::sleep(Duration::from_millis(20));
    addr
}

fn sample_request(url: &str) -> HttpRequest {
    HttpRequest {
        method: "GET".to_string(),
        url: url.to_string(),
        headers: Default::default(),
        body: None,
        timeout_ms: Some(5_000),
        connect_timeout_ms: Some(1_000),
        follow_redirects: Some(false),
        basic_auth: None,
        body_encoding: None,
    }
}

#[test]
fn async_runtime_mode_is_async() {
    let real =
        RealHttpClient::new(RealHttpClientConfig::default()).expect("real client");
    let runtime = AsyncHttpRuntime::new(
        Arc::new(real),
        AsyncHttpRuntimeConfig {
            pool_size: 2,
            execute_timeout_ms: 5_000,
        },
    );
    assert_eq!(runtime.mode(), HttpRuntimeMode::Async);
}

#[test]
fn concurrent_async_executes_overlap_on_worker_pool() {
    let hits = Arc::new(AtomicUsize::new(0));
    let delay_ms = 200;
    let addr = slow_get_server(delay_ms, Arc::clone(&hits));

    let real = RealHttpClient::new(RealHttpClientConfig {
        default_timeout_ms: 5_000,
        default_connect_timeout_ms: 1_000,
        retry_count: 0,
        allow_private_networks: true,
        ..RealHttpClientConfig::default()
    })
    .expect("real client");

    let runtime = Arc::new(AsyncHttpRuntime::new(
        Arc::new(real),
        AsyncHttpRuntimeConfig {
            pool_size: 4,
            execute_timeout_ms: 10_000,
        },
    ));

    let concurrency = 4usize;
    let barrier = Arc::new(Barrier::new(concurrency + 1));
    let mut handles = Vec::new();
    let started = Instant::now();

    for _ in 0..concurrency {
        let runtime = Arc::clone(&runtime);
        let barrier = Arc::clone(&barrier);
        let request = sample_request(&addr);
        handles.push(thread::spawn(move || {
            barrier.wait();
            runtime.execute(&request)
        }));
    }

    barrier.wait();
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.join().expect("worker join"));
    }
    let elapsed = started.elapsed();

    for result in &results {
        let exchange = result.as_ref().expect("execute should succeed");
        assert_eq!(exchange.response.status_code, 200);
    }

    // Sequential would be ~concurrency * delay; overlapped pool should finish closer to one delay.
    let sequential_floor = Duration::from_millis(delay_ms * concurrency as u64);
    assert!(
        elapsed < sequential_floor,
        "expected concurrent async executes to overlap: elapsed={elapsed:?}, sequential_floor={sequential_floor:?}"
    );
    assert!(
        elapsed < Duration::from_millis(delay_ms + 800),
        "elapsed {elapsed:?} should stay near one delayed round-trip"
    );
    assert!(
        hits.load(Ordering::SeqCst) >= concurrency,
        "mock server should have served concurrent requests"
    );
}
