use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

mod client;
mod ssrf_guard;

pub use client::{RealHttpClient, RealHttpClientConfig};
pub use ssrf_guard::{
    safe_url_display, safe_url_for_error, validate_outbound_url, OutboundUrlGuardConfig,
    OutboundUrlGuardError,
};

// ── HttpRuntime trait ──────────────────────────────────────────────

/// HTTP 运行时抽象 — 支持 deterministic、real 与 pooled async 模式
pub trait HttpRuntime: Send + Sync {
    fn execute(&self, request: &HttpRequest) -> Result<HttpExchange, HttpServiceError>;
    fn execute_with_status(&self, request: &HttpRequest) -> Result<HttpExchange, HttpServiceError> {
        self.execute(request)
    }
    fn execute_async<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpExchange, HttpServiceError>> + Send + 'a>> {
        Box::pin(async move { self.execute(request) })
    }
    fn execute_async_with_status<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpExchange, HttpServiceError>> + Send + 'a>> {
        self.execute_async(request)
    }
    fn mode(&self) -> HttpRuntimeMode;
}

// ── HttpRuntimeMode ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpRuntimeMode {
    Deterministic,
    Real,
    /// Blocking execute dispatched onto a fixed worker pool (caller parks on channel).
    Async,
}

// ── HttpServiceError ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpServiceError {
    pub message: String,
    pub status_code: Option<u16>,
    pub response_body_excerpt: Option<String>,
    pub request_url: Option<String>,
    pub request_method: Option<String>,
}

impl HttpServiceError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: None,
            response_body_excerpt: None,
            request_url: None,
            request_method: None,
        }
    }
}

impl std::fmt::Display for HttpServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for HttpServiceError {}

// ── BasicAuth ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasicAuth {
    pub username: String,
    pub password: String,
}

// ── HttpRequest ────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_redirects: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basic_auth: Option<BasicAuth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_encoding: Option<String>,
}

// ── HttpResponse ───────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status_code: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

// ── HttpExchange ───────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HttpExchange {
    pub request: HttpRequest,
    pub response: HttpResponse,
}

// ── DeterministicHttpRuntime ───────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DeterministicHttpRuntime {
    allowed_methods: BTreeSet<String>,
}

impl Default for DeterministicHttpRuntime {
    fn default() -> Self {
        Self::new(["GET", "POST"])
    }
}

impl DeterministicHttpRuntime {
    pub fn new<I, S>(allowed_methods: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            allowed_methods: allowed_methods
                .into_iter()
                .map(|method| method.into().to_uppercase())
                .collect(),
        }
    }
}

impl HttpRuntime for DeterministicHttpRuntime {
    fn execute(&self, request: &HttpRequest) -> Result<HttpExchange, HttpServiceError> {
        let normalized_method = request.method.trim().to_uppercase();
        if normalized_method.is_empty() {
            return Err(HttpServiceError::new(
                "HTTP request method is required for the owned M14 subset",
            ));
        }
        if !self.allowed_methods.contains(&normalized_method) {
            return Err(HttpServiceError::new(format!(
                "HTTP method '{}' is not supported by the owned M14 subset",
                normalized_method
            )));
        }

        if request.url.trim().is_empty() {
            return Err(HttpServiceError::new(
                "HTTP request URL is required for the owned M14 subset",
            ));
        }

        let mut response_headers = BTreeMap::new();
        response_headers.insert("Content-Type".to_string(), "application/json".to_string());
        response_headers.insert(
            "X-Flowable-Transport".to_string(),
            "deterministic-runtime".to_string(),
        );

        let response_body = json!({
            "accepted": true,
            "method": normalized_method,
            "url": request.url,
            "echo": request.body.clone().unwrap_or(Value::Null),
        });

        let mut owned_request = request.clone();
        owned_request.method = normalized_method;

        Ok(HttpExchange {
            request: owned_request,
            response: HttpResponse {
                status_code: 200,
                headers: response_headers,
                body: response_body,
            },
        })
    }

    fn mode(&self) -> HttpRuntimeMode {
        HttpRuntimeMode::Deterministic
    }
}

// ── AsyncHttpRuntime ───────────────────────────────────────────────

/// Configuration for [`AsyncHttpRuntime`].
#[derive(Debug, Clone)]
pub struct AsyncHttpRuntimeConfig {
    /// Number of worker threads that execute blocking HTTP calls.
    pub pool_size: usize,
    /// Maximum time the caller will wait for a worker to finish the exchange.
    pub execute_timeout_ms: u64,
}

impl Default for AsyncHttpRuntimeConfig {
    fn default() -> Self {
        Self {
            pool_size: 4,
            execute_timeout_ms: 60_000,
        }
    }
}

/// Async HTTP runtime backed by a native async reqwest client. The synchronous
/// `execute` method remains only as a compatibility adapter; engine async
/// paths use `execute_async` and never park a worker thread on socket I/O.
pub struct AsyncHttpRuntime {
    inner: Arc<dyn HttpRuntime>,
}

impl AsyncHttpRuntime {
    /// Wrap an inner runtime (typically [`RealHttpClient`]) with a fixed worker pool.
    pub fn new(inner: Arc<dyn HttpRuntime>, config: AsyncHttpRuntimeConfig) -> Self {
        let _ = config;
        Self { inner }
    }

    /// Convenience: build a pooled runtime over a freshly constructed real client.
    pub fn from_real_client(
        client_config: RealHttpClientConfig,
        runtime_config: AsyncHttpRuntimeConfig,
    ) -> Result<Self, HttpServiceError> {
        let client = RealHttpClient::new_async(client_config)?;
        let _ = runtime_config;
        Ok(Self::new(
            Arc::new(client),
            AsyncHttpRuntimeConfig::default(),
        ))
    }
}

impl HttpRuntime for AsyncHttpRuntime {
    fn execute(&self, request: &HttpRequest) -> Result<HttpExchange, HttpServiceError> {
        self.inner.execute(request)
    }

    fn execute_async<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpExchange, HttpServiceError>> + Send + 'a>> {
        self.inner.execute_async(request)
    }

    fn execute_with_status(&self, request: &HttpRequest) -> Result<HttpExchange, HttpServiceError> {
        self.inner.execute_with_status(request)
    }

    fn execute_async_with_status<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpExchange, HttpServiceError>> + Send + 'a>> {
        self.inner.execute_async_with_status(request)
    }

    fn mode(&self) -> HttpRuntimeMode {
        HttpRuntimeMode::Async
    }
}
