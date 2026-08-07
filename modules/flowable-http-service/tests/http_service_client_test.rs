use flowable_http_service::{
    DeterministicHttpRuntime, HttpRequest, HttpRuntime, RealHttpClient, RealHttpClientConfig,
};
use serde_json::json;

// ── DeterministicHttpRuntime tests ─────────────────────────────────

#[test]
fn deterministic_http_runtime_echoes_request_as_runtime_evidence() {
    let runtime = DeterministicHttpRuntime::default();
    let exchange = runtime
        .execute(&HttpRequest {
            method: "POST".to_string(),
            url: "https://example.flowable.local/orders".to_string(),
            headers: [("Accept".to_string(), "application/json".to_string())]
                .into_iter()
                .collect(),
            body: Some(json!({"orderId": 42, "approved": true})),
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        })
        .expect("owned HTTP subset should execute");

    assert_eq!(exchange.request.method, "POST");
    assert_eq!(
        exchange.request.url,
        "https://example.flowable.local/orders"
    );
    assert_eq!(exchange.response.status_code, 200);
    assert_eq!(exchange.response.body["accepted"], true);
    assert_eq!(exchange.response.body["echo"]["orderId"], 42);
}

#[test]
fn deterministic_http_runtime_rejects_unsupported_methods() {
    let runtime = DeterministicHttpRuntime::default();
    let error = runtime
        .execute(&HttpRequest {
            method: "PUT".to_string(),
            url: "https://example.flowable.local/orders/42".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        })
        .expect_err("unsupported methods should fail structurally");

    assert!(error.to_string().contains("PUT"));
}

#[test]
fn deterministic_http_runtime_mode_is_deterministic() {
    let runtime = DeterministicHttpRuntime::default();
    assert_eq!(
        runtime.mode(),
        flowable_http_service::HttpRuntimeMode::Deterministic
    );
}

// ── RealHttpClient tests (using httpbin.org) ───────────────────────

/// Helper to create a default RealHttpClient.
fn real_client() -> RealHttpClient {
    RealHttpClient::new(RealHttpClientConfig {
        default_timeout_ms: 15_000,
        default_connect_timeout_ms: 10_000,
        user_agent: Some("Flowable-Rust-Test/0.1".to_string()),
        ..RealHttpClientConfig::default()
    })
    .expect("should build test client")
}

#[test]
#[ignore = "requires network access to httpbin.org"]
fn real_http_client_get_request() {
    let client = real_client();
    let exchange = client
        .execute(&HttpRequest {
            method: "GET".to_string(),
            url: "https://httpbin.org/get".to_string(),
            headers: [("Accept".to_string(), "application/json".to_string())]
                .into_iter()
                .collect(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        })
        .expect("GET should succeed");

    assert_eq!(exchange.response.status_code, 200);
    // httpbin echoes the URL back
    assert!(
        exchange.response.body["url"]
            .as_str()
            .unwrap_or("")
            .contains("httpbin")
    );
}

#[test]
#[ignore = "requires network access to httpbin.org"]
fn real_http_client_post_json_request() {
    let client = real_client();
    let exchange = client
        .execute(&HttpRequest {
            method: "POST".to_string(),
            url: "https://httpbin.org/post".to_string(),
            headers: Default::default(),
            body: Some(json!({"name": "flowable", "version": 1})),
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: Some("json".to_string()),
        })
        .expect("POST should succeed");

    assert_eq!(exchange.response.status_code, 200);
    assert_eq!(exchange.response.body["json"]["name"], "flowable");
    assert_eq!(exchange.response.body["json"]["version"], 1);
}

#[test]
#[ignore = "requires network access to httpbin.org"]
fn real_http_client_handles_404_error() {
    let client = real_client();
    let error = client
        .execute(&HttpRequest {
            method: "GET".to_string(),
            url: "https://httpbin.org/status/404".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        })
        .expect_err("404 should produce error");

    assert_eq!(error.status_code, Some(404));
    assert!(error.request_url.is_some());
    assert!(error.request_method.is_some());
}

#[test]
#[ignore = "requires network access to httpbin.org"]
fn real_http_client_handles_500_error() {
    let client = real_client();
    let error = client
        .execute(&HttpRequest {
            method: "GET".to_string(),
            url: "https://httpbin.org/status/500".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        })
        .expect_err("500 should produce error");

    assert_eq!(error.status_code, Some(500));
    assert!(error.response_body_excerpt.is_some());
}

#[test]
#[ignore = "requires network access to httpbin.org"]
fn real_http_client_basic_auth() {
    let client = real_client();
    let exchange = client
        .execute(&HttpRequest {
            method: "GET".to_string(),
            url: "https://httpbin.org/basic-auth/testuser/testpass".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: Some(flowable_http_service::BasicAuth {
                username: "testuser".to_string(),
                password: "testpass".to_string(),
            }),
            body_encoding: None,
        })
        .expect("basic auth should succeed");

    assert_eq!(exchange.response.status_code, 200);
    assert_eq!(exchange.response.body["authenticated"], true);
    assert_eq!(exchange.response.body["user"], "testuser");
}

#[test]
#[ignore = "requires network access to httpbin.org"]
fn real_http_client_follows_redirects() {
    let client = real_client();
    let exchange = client
        .execute(&HttpRequest {
            method: "GET".to_string(),
            url: "https://httpbin.org/redirect/1".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: Some(true),
            basic_auth: None,
            body_encoding: None,
        })
        .expect("redirect should be followed");

    assert_eq!(exchange.response.status_code, 200);
}

#[test]
#[ignore = "requires network access to httpbin.org"]
fn real_http_client_does_not_follow_redirects_when_disabled() {
    let client = real_client();
    let exchange = client
        .execute(&HttpRequest {
            method: "GET".to_string(),
            url: "https://httpbin.org/redirect/1".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: Some(false),
            basic_auth: None,
            body_encoding: None,
        })
        .expect("redirect response should be returned as-is");

    // With redirect disabled, httpbin returns 302
    assert!(exchange.response.status_code == 302 || exchange.response.status_code == 301);
}

#[test]
fn real_http_client_timeout_on_unreachable_host() {
    let client = RealHttpClient::new(RealHttpClientConfig {
        default_timeout_ms: 2_000,
        default_connect_timeout_ms: 1_000,
        user_agent: Some("Flowable-Rust-Test/0.1".to_string()),
        ..RealHttpClientConfig::default()
    })
    .expect("should build client");

    // TEST-NET-3 documentation address (not private under SSRF policy) — expect timeout/fail.
    // Private 10.x would be rejected by the SSRF guard before any connect attempt.
    let error = client
        .execute(&HttpRequest {
            method: "GET".to_string(),
            url: "http://203.0.113.1:81/".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: Some(1_000),
            connect_timeout_ms: Some(500),
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        })
        .expect_err("unreachable host should timeout or fail");

    assert!(error.request_url.is_some());
    let safe = error.request_url.as_deref().unwrap_or("");
    assert!(
        !safe.contains("/x") && safe.matches('/').count() <= 2,
        "error request_url must not echo path beyond scheme://host:port: {safe:?}"
    );
    assert!(error.request_method.is_some());
}

#[test]
fn real_http_client_rejects_private_destination_by_default() {
    let client = RealHttpClient::new(RealHttpClientConfig::default()).expect("client");
    let error = client
        .execute(&HttpRequest {
            method: "GET".to_string(),
            url: "http://127.0.0.1:9/secret?token=x".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        })
        .expect_err("loopback must be blocked by SSRF guard");
    assert!(error.message.contains("SSRF guard") || error.message.contains("blocked"));
    assert!(!error.message.contains("secret"));
    assert!(!error.message.contains("token"));
    assert_eq!(error.request_url.as_deref(), Some("http://127.0.0.1:9"));
}
