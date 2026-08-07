use flowable_http_service::{
    BasicAuth, DeterministicHttpRuntime, HttpRequest, HttpRuntime, HttpRuntimeMode, RealHttpClient,
    RealHttpClientConfig,
};
use serde_json::json;

// ── HttpRuntime trait unified interface ────────────────────────────

/// A helper that accepts any `HttpRuntime` and executes a request.
fn execute_via(runtime: &dyn HttpRuntime, request: &HttpRequest) -> String {
    match runtime.execute(request) {
        Ok(exchange) => format!(
            "OK {} {}",
            exchange.response.status_code, exchange.response.body
        ),
        Err(e) => format!("ERR {} {:?}", e.message, e.status_code),
    }
}

#[test]
fn deterministic_and_real_are_interchangeable_via_trait() {
    let det = DeterministicHttpRuntime::default();
    let real =
        RealHttpClient::new(RealHttpClientConfig::default()).expect("should build real client");

    // Both implement HttpRuntime
    let _runtimes: [&dyn HttpRuntime; 2] = [&det, &real];

    // Deterministic should work
    let result = execute_via(
        &det,
        &HttpRequest {
            method: "GET".to_string(),
            url: "https://example.com/test".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        },
    );
    assert!(result.starts_with("OK 200"));
}

#[test]
fn http_runtime_mode_switching() {
    let det = DeterministicHttpRuntime::default();
    assert_eq!(det.mode(), HttpRuntimeMode::Deterministic);

    let real =
        RealHttpClient::new(RealHttpClientConfig::default()).expect("should build real client");
    assert_eq!(real.mode(), HttpRuntimeMode::Real);

    // Verify they are different
    assert_ne!(det.mode(), real.mode());
}

#[test]
fn deterministic_runtime_returns_structured_error() {
    let runtime = DeterministicHttpRuntime::default();
    let error = runtime
        .execute(&HttpRequest {
            method: "DELETE".to_string(),
            url: "https://example.com/resource".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        })
        .expect_err("DELETE should be rejected");

    // Error should contain the method name
    assert!(error.to_string().contains("DELETE"));
    // Structured fields should be None for deterministic errors
    assert_eq!(error.status_code, None);
    assert_eq!(error.response_body_excerpt, None);
}

#[test]
fn http_request_supports_all_new_fields() {
    let request = HttpRequest {
        method: "POST".to_string(),
        url: "https://example.com/api".to_string(),
        headers: Default::default(),
        body: Some(json!({"key": "value"})),
        timeout_ms: Some(5000),
        connect_timeout_ms: Some(3000),
        follow_redirects: Some(false),
        basic_auth: Some(BasicAuth {
            username: "admin".to_string(),
            password: "secret".to_string(),
        }),
        body_encoding: Some("json".to_string()),
    };

    assert_eq!(request.timeout_ms, Some(5000));
    assert_eq!(request.connect_timeout_ms, Some(3000));
    assert_eq!(request.follow_redirects, Some(false));
    assert_eq!(
        request.basic_auth,
        Some(BasicAuth {
            username: "admin".to_string(),
            password: "secret".to_string(),
        })
    );
    assert_eq!(request.body_encoding, Some("json".to_string()));
}

#[test]
fn http_request_serialization_roundtrip() {
    let request = HttpRequest {
        method: "POST".to_string(),
        url: "https://example.com/api".to_string(),
        headers: [("Authorization".to_string(), "Bearer token123".to_string())]
            .into_iter()
            .collect(),
        body: Some(json!({"data": "test"})),
        timeout_ms: Some(10_000),
        connect_timeout_ms: Some(5_000),
        follow_redirects: Some(true),
        basic_auth: Some(BasicAuth {
            username: "user".to_string(),
            password: "pass".to_string(),
        }),
        body_encoding: Some("json".to_string()),
    };

    let json_str = serde_json::to_string(&request).expect("serialization should work");
    let deserialized: HttpRequest =
        serde_json::from_str(&json_str).expect("deserialization should work");

    assert_eq!(deserialized.method, "POST");
    assert_eq!(deserialized.url, "https://example.com/api");
    assert_eq!(deserialized.timeout_ms, Some(10_000));
    assert_eq!(deserialized.connect_timeout_ms, Some(5_000));
    assert_eq!(deserialized.follow_redirects, Some(true));
    assert_eq!(
        deserialized.basic_auth,
        Some(BasicAuth {
            username: "user".to_string(),
            password: "pass".to_string(),
        })
    );
    assert_eq!(deserialized.body_encoding, Some("json".to_string()));
}

#[test]
fn http_service_error_display_format() {
    let error = flowable_http_service::HttpServiceError {
        message: "Something went wrong".to_string(),
        status_code: Some(502),
        response_body_excerpt: Some("<html>Bad Gateway</html>".to_string()),
        request_url: Some("https://example.com/api".to_string()),
        request_method: Some("GET".to_string()),
    };

    let display = error.to_string();
    assert_eq!(display, "Something went wrong");

    // Structured fields are accessible
    assert_eq!(error.status_code, Some(502));
    assert_eq!(
        error.response_body_excerpt,
        Some("<html>Bad Gateway</html>".to_string())
    );
    assert_eq!(
        error.request_url,
        Some("https://example.com/api".to_string())
    );
    assert_eq!(error.request_method, Some("GET".to_string()));
}

#[test]
fn http_service_error_implements_std_error_trait() {
    let error = flowable_http_service::HttpServiceError::new("test error");
    let _: &dyn std::error::Error = &error;
}

#[test]
#[ignore = "requires network access to httpbin.org"]
fn real_client_structured_error_contains_status_and_body_excerpt() {
    let client = RealHttpClient::new(RealHttpClientConfig {
        default_timeout_ms: 15_000,
        default_connect_timeout_ms: 10_000,
        user_agent: Some("Flowable-Rust-Test/0.1".to_string()),
        ..RealHttpClientConfig::default()
    })
    .expect("should build client");

    let error = client
        .execute(&HttpRequest {
            method: "GET".to_string(),
            url: "https://httpbin.org/status/418".to_string(),
            headers: Default::default(),
            body: None,
            timeout_ms: None,
            connect_timeout_ms: None,
            follow_redirects: None,
            basic_auth: None,
            body_encoding: None,
        })
        .expect_err("418 should produce error");

    assert_eq!(error.status_code, Some(418));
    assert!(error.response_body_excerpt.is_some());
    // Path/query stripped to avoid blind probing via error echoes (P142b SSRF hardening).
    assert_eq!(
        error.request_url,
        Some("https://httpbin.org".to_string())
    );
    assert_eq!(error.request_method, Some("GET".to_string()));
}

#[test]
fn deterministic_runtime_accepts_extended_request_fields() {
    let runtime = DeterministicHttpRuntime::new(["GET", "POST", "PUT", "DELETE", "PATCH"]);
    let exchange = runtime
        .execute(&HttpRequest {
            method: "PUT".to_string(),
            url: "https://example.com/resource/1".to_string(),
            headers: Default::default(),
            body: Some(json!({"updated": true})),
            timeout_ms: Some(5000),
            connect_timeout_ms: Some(2000),
            follow_redirects: Some(false),
            basic_auth: Some(BasicAuth {
                username: "user".to_string(),
                password: "pass".to_string(),
            }),
            body_encoding: Some("json".to_string()),
        })
        .expect("PUT should be allowed with extended methods");

    assert_eq!(exchange.response.status_code, 200);
    assert_eq!(exchange.response.body["method"], "PUT");
}
