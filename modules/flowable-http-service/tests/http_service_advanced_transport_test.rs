use flowable_http_service::{HttpRequest, HttpRuntime, RealHttpClient, RealHttpClientConfig};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

#[test]
fn test_http_retry_mechanism() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let addr = format!("http://127.0.0.1:{}", port);

    let fail_count = Arc::new(AtomicUsize::new(0));
    let fail_count_clone = Arc::clone(&fail_count);

    thread::spawn(move || {
        for _ in 0..2 {
            if let Ok((mut stream, _)) = listener.accept() {
                fail_count_clone.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf);
                let response = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(response.as_bytes());
            }
        }
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0; 1024];
            let _ = stream.read(&mut buf);
            let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\n\r\n{\"success\":true}";
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let config = RealHttpClientConfig {
        default_timeout_ms: 1000,
        default_connect_timeout_ms: 500,
        retry_count: 3,
        retry_backoff_ms: 10,
        allow_private_networks: true,
        ..RealHttpClientConfig::default()
    };
    let client = RealHttpClient::new(config).unwrap();

    let request = HttpRequest {
        method: "GET".to_string(),
        url: addr,
        headers: Default::default(),
        body: None,
        timeout_ms: None,
        connect_timeout_ms: None,
        follow_redirects: None,
        basic_auth: None,
        body_encoding: None,
    };

    let exchange = client.execute(&request).unwrap();
    assert_eq!(exchange.response.status_code, 200);
    assert_eq!(fail_count.load(Ordering::SeqCst), 2);
}

#[test]
fn test_http_get_caching() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let addr = format!("http://127.0.0.1:{}", port);

    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_clone = Arc::clone(&request_count);

    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            request_count_clone.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0; 1024];
            let _ = stream.read(&mut buf);
            let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 17\r\n\r\n{\"data\":\"cached\"}";
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let config = RealHttpClientConfig {
        cache_enabled: true,
        cache_ttl_ms: 5000,
        allow_private_networks: true,
        ..RealHttpClientConfig::default()
    };
    let client = RealHttpClient::new(config).unwrap();

    let request = HttpRequest {
        method: "GET".to_string(),
        url: addr,
        headers: Default::default(),
        body: None,
        timeout_ms: None,
        connect_timeout_ms: None,
        follow_redirects: None,
        basic_auth: None,
        body_encoding: None,
    };

    let ex1 = client.execute(&request).unwrap();
    assert_eq!(ex1.response.status_code, 200);

    let ex2 = client.execute(&request).unwrap();
    assert_eq!(ex2.response.status_code, 200);

    assert_eq!(request_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_http_circuit_breaker() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let addr = format!("http://127.0.0.1:{}", port);

    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0; 1024];
            let _ = stream.read(&mut buf);
            let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let config = RealHttpClientConfig {
        circuit_breaker_threshold: 2,
        circuit_breaker_cooldown_ms: 1000,
        retry_count: 0,
        allow_private_networks: true,
        ..RealHttpClientConfig::default()
    };
    let client = RealHttpClient::new(config).unwrap();

    let request = HttpRequest {
        method: "GET".to_string(),
        url: addr,
        headers: Default::default(),
        body: None,
        timeout_ms: None,
        connect_timeout_ms: None,
        follow_redirects: None,
        basic_auth: None,
        body_encoding: None,
    };

    let _ = client.execute(&request);
    let _ = client.execute(&request);

    let err = client.execute(&request).unwrap_err();
    assert!(
        err.message.to_lowercase().contains("circuit breaker")
            || err.message.to_lowercase().contains("open")
    );
}

#[test]
fn test_advanced_config_initialization() {
    let config = RealHttpClientConfig {
        oauth2_client_id: Some("id123".to_string()),
        oauth2_client_secret: Some("secret123".to_string()),
        oauth2_token_url: Some("http://localhost/token".to_string()),
        client_cert_pem: Some("cert".to_string()),
        client_key_pem: Some("key".to_string()),
        ..RealHttpClientConfig::default()
    };
    let client_res = RealHttpClient::new(config);
    assert!(client_res.is_ok());
}
