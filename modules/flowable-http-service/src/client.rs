use crate::ssrf_guard::{
    safe_url_display, validate_outbound_url, OutboundUrlGuardConfig, OutboundUrlGuardError,
};
use crate::{
    HttpExchange, HttpRequest, HttpResponse, HttpRuntime, HttpRuntimeMode, HttpServiceError,
};
use reqwest::Client as AsyncClient;
use reqwest::blocking::Client as BlockingClient;
use reqwest::redirect::Policy;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ── CircuitBreakerState ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CircuitBreakerState {
    Closed { failure_count: u32 },
    Open { opened_at: Instant },
    HalfOpen,
}

// ── RealHttpClientConfig ───────────────────────────────────────────

/// Configuration for [`RealHttpClient`].
#[derive(Debug, Clone)]
pub struct RealHttpClientConfig {
    /// Default request timeout in milliseconds.
    pub default_timeout_ms: u64,
    /// Default connection timeout in milliseconds.
    pub default_connect_timeout_ms: u64,
    /// Optional `User-Agent` header value.
    pub user_agent: Option<String>,

    // M42: Advanced features configuration
    pub retry_count: u32,
    pub retry_backoff_ms: u64,
    pub cache_enabled: bool,
    pub cache_ttl_ms: u64,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_cooldown_ms: u64,

    // OAuth2 configuration
    pub oauth2_client_id: Option<String>,
    pub oauth2_client_secret: Option<String>,
    pub oauth2_token_url: Option<String>,

    // mTLS configuration
    pub client_cert_pem: Option<String>,
    pub client_key_pem: Option<String>,

    /// SSRF guard: when `true`, allow private/loopback/link-local destinations.
    /// Default `false` (security deviation from Java — Java has no outbound guard).
    pub allow_private_networks: bool,
    /// Explicit hosts/IPs permitted even when private networks are otherwise denied.
    pub allowed_private_hosts: Vec<String>,
}

impl Default for RealHttpClientConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: 30_000,
            default_connect_timeout_ms: 10_000,
            user_agent: Some("Flowable-Rust-HTTP/0.1".to_string()),
            retry_count: 3,
            retry_backoff_ms: 100,
            cache_enabled: false,
            cache_ttl_ms: 60_000,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_ms: 10_000,
            oauth2_client_id: None,
            oauth2_client_secret: None,
            oauth2_token_url: None,
            client_cert_pem: None,
            client_key_pem: None,
            allow_private_networks: false,
            allowed_private_hosts: Vec::new(),
        }
    }
}

impl RealHttpClientConfig {
    fn ssrf_guard(&self) -> OutboundUrlGuardConfig {
        OutboundUrlGuardConfig {
            allow_private_networks: self.allow_private_networks,
            allowed_private_hosts: self.allowed_private_hosts.clone(),
        }
    }
}

// ── RealHttpClient ─────────────────────────────────────────────────

/// A real HTTP client with advanced transport behaviors.
pub struct RealHttpClient {
    client_follow: Option<BlockingClient>,
    client_no_follow: Option<BlockingClient>,
    async_client_follow: AsyncClient,
    async_client_no_follow: AsyncClient,
    config: RealHttpClientConfig,

    // M42 state tracking
    cache: Mutex<HashMap<String, (HttpExchange, Instant)>>,
    circuit_breakers: Mutex<HashMap<String, CircuitBreakerState>>,
    oauth_token: Mutex<Option<(String, Instant)>>,
}

impl RealHttpClient {
    /// Create a new [`RealHttpClient`] with the given configuration.
    pub fn new(config: RealHttpClientConfig) -> Result<Self, HttpServiceError> {
        let base_builder = || {
            let mut b = BlockingClient::builder()
                .timeout(Duration::from_millis(config.default_timeout_ms))
                .connect_timeout(Duration::from_millis(config.default_connect_timeout_ms));

            if let Some(ref ua) = config.user_agent {
                b = b.user_agent(ua.clone());
            }

            // M42: mTLS client certificates config
            if let (Some(cert), Some(key)) = (&config.client_cert_pem, &config.client_key_pem)
                && let Ok(identity) =
                    reqwest::Identity::from_pkcs8_pem(cert.as_bytes(), key.as_bytes())
            {
                b = b.identity(identity);
            }

            b
        };

        let client_follow = base_builder()
            .redirect(Policy::limited(10))
            .build()
            .map_err(|e| HttpServiceError::new(format!("Failed to build HTTP client: {e}")))?;

        let client_no_follow = base_builder()
            .redirect(Policy::none())
            .build()
            .map_err(|e| HttpServiceError::new(format!("Failed to build HTTP client: {e}")))?;

        let async_base_builder = || {
            let mut builder = AsyncClient::builder()
                .timeout(Duration::from_millis(config.default_timeout_ms))
                .connect_timeout(Duration::from_millis(config.default_connect_timeout_ms));
            if let Some(ref user_agent) = config.user_agent {
                builder = builder.user_agent(user_agent.clone());
            }
            if let (Some(cert), Some(key)) = (&config.client_cert_pem, &config.client_key_pem)
                && let Ok(identity) =
                    reqwest::Identity::from_pkcs8_pem(cert.as_bytes(), key.as_bytes())
            {
                builder = builder.identity(identity);
            }
            builder
        };
        let async_client_follow = async_base_builder()
            .redirect(Policy::limited(10))
            .build()
            .map_err(|e| {
                HttpServiceError::new(format!("Failed to build async HTTP client: {e}"))
            })?;
        let async_client_no_follow = async_base_builder()
            .redirect(Policy::none())
            .build()
            .map_err(|e| {
                HttpServiceError::new(format!("Failed to build async HTTP client: {e}"))
            })?;

        Ok(Self {
            client_follow: Some(client_follow),
            client_no_follow: Some(client_no_follow),
            async_client_follow,
            async_client_no_follow,
            config,
            cache: Mutex::new(HashMap::new()),
            circuit_breakers: Mutex::new(HashMap::new()),
            oauth_token: Mutex::new(None),
        })
    }

    /// Construct the production async client without allocating reqwest's
    /// internal blocking runtime. This variant is safe to create and drop from
    /// a Tokio task.
    pub fn new_async(config: RealHttpClientConfig) -> Result<Self, HttpServiceError> {
        let async_base_builder = || {
            let mut builder = AsyncClient::builder()
                .timeout(Duration::from_millis(config.default_timeout_ms))
                .connect_timeout(Duration::from_millis(config.default_connect_timeout_ms));
            if let Some(ref user_agent) = config.user_agent {
                builder = builder.user_agent(user_agent.clone());
            }
            if let (Some(cert), Some(key)) = (&config.client_cert_pem, &config.client_key_pem)
                && let Ok(identity) =
                    reqwest::Identity::from_pkcs8_pem(cert.as_bytes(), key.as_bytes())
            {
                builder = builder.identity(identity);
            }
            builder
        };
        let async_client_follow = async_base_builder()
            .redirect(Policy::limited(10))
            .build()
            .map_err(|error| {
                HttpServiceError::new(format!("Failed to build async HTTP client: {error}"))
            })?;
        let async_client_no_follow = async_base_builder()
            .redirect(Policy::none())
            .build()
            .map_err(|error| {
                HttpServiceError::new(format!("Failed to build async HTTP client: {error}"))
            })?;
        Ok(Self {
            client_follow: None,
            client_no_follow: None,
            async_client_follow,
            async_client_no_follow,
            config,
            cache: Mutex::new(HashMap::new()),
            circuit_breakers: Mutex::new(HashMap::new()),
            oauth_token: Mutex::new(None),
        })
    }

    /// Select the appropriate internal client based on redirect preference.
    fn select_client(
        &self,
        follow_redirects: Option<bool>,
    ) -> Result<&BlockingClient, HttpServiceError> {
        match follow_redirects {
            Some(false) => self.client_no_follow.as_ref(),
            _ => self.client_follow.as_ref(),
        }
        .ok_or_else(|| {
            HttpServiceError::new(
                "blocking HTTP compatibility API is unavailable on an async-only client",
            )
        })
    }

    fn build_dynamic_client(
        &self,
        request: &HttpRequest,
    ) -> Result<Option<BlockingClient>, HttpServiceError> {
        let Some(connect_timeout_ms) = request.connect_timeout_ms else {
            return Ok(None);
        };

        let mut builder = BlockingClient::builder()
            .timeout(Duration::from_millis(
                request.timeout_ms.unwrap_or(self.config.default_timeout_ms),
            ))
            .connect_timeout(Duration::from_millis(connect_timeout_ms))
            .redirect(match request.follow_redirects {
                Some(false) => Policy::none(),
                _ => Policy::limited(10),
            });

        if let Some(ref ua) = self.config.user_agent {
            builder = builder.user_agent(ua.clone());
        }

        if let (Some(cert), Some(key)) = (&self.config.client_cert_pem, &self.config.client_key_pem)
            && let Ok(identity) = reqwest::Identity::from_pkcs8_pem(cert.as_bytes(), key.as_bytes())
        {
            builder = builder.identity(identity);
        }

        builder
            .build()
            .map(Some)
            .map_err(|e| HttpServiceError::new(format!("Failed to build HTTP client: {e}")))
    }

    fn guard_url(&self, url: &str) -> Result<(), HttpServiceError> {
        validate_outbound_url(url, &self.config.ssrf_guard()).map_err(ssrf_to_http_error)
    }

    /// Build a `reqwest::RequestBuilder` from an [`HttpRequest`].
    fn build_request(
        &self,
        client: &BlockingClient,
        request: &HttpRequest,
    ) -> Result<reqwest::blocking::RequestBuilder, HttpServiceError> {
        let method = request.method.trim().to_uppercase();
        let url = request.url.trim();

        if url.is_empty() {
            return Err(HttpServiceError::new("HTTP request URL is required"));
        }
        self.guard_url(url)?;
        let safe_url = safe_url_display(url);

        let mut req_builder = match method.as_str() {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "DELETE" => client.delete(url),
            "PATCH" => client.patch(url),
            other => {
                return Err(HttpServiceError {
                    message: format!("Unsupported HTTP method: {other}"),
                    status_code: None,
                    response_body_excerpt: None,
                    request_url: Some(safe_url),
                    request_method: Some(other.to_string()),
                });
            }
        };

        // ── headers ────────────────────────────────────────────
        for (key, value) in &request.headers {
            req_builder = req_builder.header(key.as_str(), value.as_str());
        }

        // ── body ───────────────────────────────────────────────
        if let Some(ref body) = request.body {
            let encoding = request.body_encoding.as_deref().unwrap_or("json");
            match encoding {
                "json" => {
                    req_builder = req_builder.header("Content-Type", "application/json");
                    req_builder = req_builder.json(body);
                }
                "form" => {
                    req_builder =
                        req_builder.header("Content-Type", "application/x-www-form-urlencoded");
                    if let Some(obj) = body.as_object() {
                        let params: Vec<(String, String)> = obj
                            .iter()
                            .map(|(k, v)| {
                                let val = match v {
                                    Value::String(s) => s.clone(),
                                    other => other.to_string(),
                                };
                                (k.clone(), val)
                            })
                            .collect();
                        req_builder = req_builder.form(&params);
                    } else {
                        req_builder = req_builder.body(body.to_string());
                    }
                }
                "text" => {
                    req_builder = req_builder.header("Content-Type", "text/plain");
                    req_builder = req_builder.body(body.to_string());
                }
                _ => {
                    req_builder = req_builder.header("Content-Type", "application/json");
                    req_builder = req_builder.json(body);
                }
            }
        }

        // ── timeout ────────────────────────────────────────────
        let timeout_ms = request.timeout_ms.unwrap_or(self.config.default_timeout_ms);
        req_builder = req_builder.timeout(Duration::from_millis(timeout_ms));

        // ── basic auth ─────────────────────────────────────────
        if let Some(ref auth) = request.basic_auth {
            req_builder = req_builder.basic_auth(&auth.username, Some(&auth.password));
        }

        // M42: OAuth2 Client Credentials authentication flow
        if let Some(token) = self.get_oauth2_token() {
            req_builder = req_builder.header("Authorization", format!("Bearer {token}"));
        }

        Ok(req_builder)
    }

    /// Extract host for circuit breaker tracking
    fn extract_host(&self, url: &str) -> String {
        if let Some(pos) = url.find("://") {
            let remaining = &url[pos + 3..];
            let end = remaining.find('/').unwrap_or(remaining.len());
            let end_colon = remaining.find(':').unwrap_or(end);
            remaining[..end.min(end_colon)].to_string()
        } else {
            url.to_string()
        }
    }

    /// Retrieve OAuth2 Token in-memory cache
    fn get_oauth2_token(&self) -> Option<String> {
        let (client_id, client_secret, token_url) = (
            &self.config.oauth2_client_id,
            &self.config.oauth2_client_secret,
            &self.config.oauth2_token_url,
        );

        let (Some(id), Some(secret), Some(url)) = (client_id, client_secret, token_url) else {
            return None;
        };

        {
            let token_lock = self.oauth_token.lock().unwrap();
            if let Some((ref token, expiry)) = *token_lock
                && Instant::now() < expiry
            {
                return Some(token.clone());
            }
        }

        // Get new token (synchronous OAuth2 client credentials flow)
        let form_params = [
            ("grant_type", "client_credentials"),
            ("client_id", id.as_str()),
            ("client_secret", secret.as_str()),
        ];

        let client = self.client_follow.as_ref()?;
        let res = client.post(url).form(&form_params).send().ok()?;
        if res.status().is_success()
            && let Ok(body) = res.json::<serde_json::Value>()
            && let Some(token) = body.get("access_token").and_then(|t| t.as_str())
        {
            let expires_in = body
                .get("expires_in")
                .and_then(|e| e.as_u64())
                .unwrap_or(3600);
            let expiry_instant =
                Instant::now() + Duration::from_secs(expires_in.saturating_sub(10));

            let mut token_lock = self.oauth_token.lock().unwrap();
            *token_lock = Some((token.to_string(), expiry_instant));
            return Some(token.to_string());
        }
        None
    }

    /// Check circuit breaker status
    fn check_circuit_breaker(&self, host: &str) -> Result<(), HttpServiceError> {
        let mut cb_lock = self.circuit_breakers.lock().unwrap();
        let state = cb_lock
            .entry(host.to_string())
            .or_insert(CircuitBreakerState::Closed { failure_count: 0 });

        match *state {
            CircuitBreakerState::Open { opened_at } => {
                if opened_at.elapsed()
                    >= Duration::from_millis(self.config.circuit_breaker_cooldown_ms)
                {
                    *state = CircuitBreakerState::HalfOpen;
                    Ok(())
                } else {
                    Err(HttpServiceError::new(format!(
                        "Circuit breaker is OPEN for host: {host}"
                    )))
                }
            }
            _ => Ok(()),
        }
    }

    /// Record request outcome to circuit breaker
    fn record_circuit_breaker(&self, host: &str, success: bool) {
        let mut cb_lock = self.circuit_breakers.lock().unwrap();
        let state = cb_lock
            .entry(host.to_string())
            .or_insert(CircuitBreakerState::Closed { failure_count: 0 });

        if success {
            *state = CircuitBreakerState::Closed { failure_count: 0 };
        } else {
            match *state {
                CircuitBreakerState::Closed {
                    ref mut failure_count,
                } => {
                    *failure_count += 1;
                    if *failure_count >= self.config.circuit_breaker_threshold {
                        *state = CircuitBreakerState::Open {
                            opened_at: Instant::now(),
                        };
                    }
                }
                CircuitBreakerState::HalfOpen | CircuitBreakerState::Open { .. } => {
                    *state = CircuitBreakerState::Open {
                        opened_at: Instant::now(),
                    };
                }
            }
        }
    }

    /// Perform HTTP execute with retries
    fn execute_with_retries(
        &self,
        request: &HttpRequest,
        client: &BlockingClient,
    ) -> Result<HttpExchange, HttpServiceError> {
        let url = request.url.trim().to_string();
        let safe_url = safe_url_display(&url);
        let method = request.method.trim().to_uppercase();

        let mut last_error = None;
        let retry_count = self.config.retry_count;

        for attempt in 0..=retry_count {
            if attempt > 0 {
                let delay = self.config.retry_backoff_ms * 2u64.pow(attempt - 1);
                std::thread::sleep(Duration::from_millis(delay));
            }

            let req_builder = match self.build_request(client, request) {
                Ok(b) => b,
                Err(e) => {
                    // SSRF / validation failures are permanent — do not retry.
                    if e.message.contains("SSRF guard") || e.message.contains("Outbound URL") {
                        return Err(e);
                    }
                    last_error = Some(e);
                    continue;
                }
            };

            let res_result = req_builder.send().map_err(|e| {
                if e.is_timeout() {
                    HttpServiceError {
                        message: format!("HTTP request timed out for {safe_url}: {e}"),
                        status_code: None,
                        response_body_excerpt: None,
                        request_url: Some(safe_url.clone()),
                        request_method: Some(method.clone()),
                    }
                } else {
                    HttpServiceError {
                        message: format!("HTTP request failed for {safe_url}: {e}"),
                        status_code: None,
                        response_body_excerpt: None,
                        request_url: Some(safe_url.clone()),
                        request_method: Some(method.clone()),
                    }
                }
            });

            let response = match res_result {
                Ok(r) => r,
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            };

            let status_code = response.status().as_u16();
            let response_headers: BTreeMap<String, String> = response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();

            let response_body_text = match response.text().map_err(|e| HttpServiceError {
                message: format!("Failed to read response body from {safe_url}: {e}"),
                status_code: Some(status_code),
                response_body_excerpt: None,
                request_url: Some(safe_url.clone()),
                request_method: Some(method.clone()),
            }) {
                Ok(t) => t,
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            };

            let response_body: Value = serde_json::from_str(&response_body_text)
                .unwrap_or(Value::String(response_body_text.clone()));

            if status_code >= 500 {
                // 5xx is transient; retry
                last_error = Some(HttpServiceError {
                    message: format!("HTTP {status_code} for {safe_url}"),
                    status_code: Some(status_code),
                    response_body_excerpt: Some(if response_body_text.len() > 500 {
                        format!("{}...", &response_body_text[..500])
                    } else {
                        response_body_text
                    }),
                    request_url: Some(safe_url.clone()),
                    request_method: Some(method.clone()),
                });
                continue;
            } else if status_code >= 400 {
                // 4xx is client error; do not retry, return immediately
                return Err(HttpServiceError {
                    message: format!("HTTP {status_code} for {safe_url}"),
                    status_code: Some(status_code),
                    response_body_excerpt: Some(if response_body_text.len() > 500 {
                        format!("{}...", &response_body_text[..500])
                    } else {
                        response_body_text
                    }),
                    request_url: Some(safe_url),
                    request_method: Some(method),
                });
            }

            return Ok(HttpExchange {
                request: request.clone(),
                response: HttpResponse {
                    status_code,
                    headers: response_headers,
                    body: response_body,
                },
            });
        }

        Err(last_error
            .unwrap_or_else(|| HttpServiceError::new("HTTP execution failed after retries")))
    }
}

fn ssrf_to_http_error(error: OutboundUrlGuardError) -> HttpServiceError {
    HttpServiceError {
        message: error.message,
        status_code: None,
        response_body_excerpt: None,
        request_url: error.safe_target,
        request_method: None,
    }
}

impl HttpRuntime for RealHttpClient {
    fn execute(&self, request: &HttpRequest) -> Result<HttpExchange, HttpServiceError> {
        let method = request.method.trim().to_uppercase();
        let url = request.url.trim().to_string();
        if url.is_empty() {
            return Err(HttpServiceError::new("HTTP request URL is required"));
        }
        // SSRF guard before any network I/O (including async fire-and-forget).
        self.guard_url(&url)?;

        // M42: Async request handling (returns 202 Accepted immediately)
        if request.headers.get("X-Flowable-Async").map(|v| v.as_str()) == Some("true") {
            let req_clone = request.clone();
            // Clone configuration for the spawned thread
            let default_timeout = self.config.default_timeout_ms;
            let default_connect = self.config.default_connect_timeout_ms;
            let ua = self.config.user_agent.clone();
            let _follow_client = self.client_follow.clone();

            std::thread::spawn(move || {
                let mut b = BlockingClient::builder()
                    .timeout(Duration::from_millis(
                        req_clone.timeout_ms.unwrap_or(default_timeout),
                    ))
                    .connect_timeout(Duration::from_millis(
                        req_clone.connect_timeout_ms.unwrap_or(default_connect),
                    ));
                if let Some(ref user_agent) = ua {
                    b = b.user_agent(user_agent.clone());
                }
                if let Ok(client) = b.build() {
                    let method = req_clone.method.trim().to_uppercase();
                    let mut req_builder = match method.as_str() {
                        "GET" => client.get(&req_clone.url),
                        "POST" => client.post(&req_clone.url),
                        "PUT" => client.put(&req_clone.url),
                        "DELETE" => client.delete(&req_clone.url),
                        "PATCH" => client.patch(&req_clone.url),
                        _ => return,
                    };
                    for (k, v) in &req_clone.headers {
                        if k != "X-Flowable-Async" {
                            req_builder = req_builder.header(k, v);
                        }
                    }
                    if let Some(ref body) = req_clone.body {
                        req_builder = req_builder.json(body);
                    }
                    let _ = req_builder.send();
                }
            });

            let mut response_headers = BTreeMap::new();
            response_headers.insert("Content-Type".to_string(), "application/json".to_string());
            return Ok(HttpExchange {
                request: request.clone(),
                response: HttpResponse {
                    status_code: 202,
                    headers: response_headers,
                    body: json!({
                        "status": "accepted",
                        "message": "asynchronous request spawned"
                    }),
                },
            });
        }

        // M42: Cache lookup for GET requests
        if self.config.cache_enabled && method == "GET" {
            let cache_lock = self.cache.lock().unwrap();
            if let Some((cached_exchange, cached_at)) = cache_lock.get(&url)
                && cached_at.elapsed() < Duration::from_millis(self.config.cache_ttl_ms)
            {
                return Ok(cached_exchange.clone());
            }
        }

        let host = self.extract_host(&url);

        // M42: Circuit breaker check
        self.check_circuit_breaker(&host)?;

        let dynamic_client = self.build_dynamic_client(request)?;
        let client = match dynamic_client.as_ref() {
            Some(client) => client,
            None => self.select_client(request.follow_redirects)?,
        };

        // Perform execution
        let outcome = self.execute_with_retries(request, client);

        // M42: Circuit breaker state recording
        self.record_circuit_breaker(&host, outcome.is_ok());

        let exchange = outcome?;

        // M42: Cache insertion
        if self.config.cache_enabled && method == "GET" {
            let mut cache_lock = self.cache.lock().unwrap();
            cache_lock.insert(url, (exchange.clone(), Instant::now()));
        }

        Ok(exchange)
    }

    fn execute_with_status(&self, request: &HttpRequest) -> Result<HttpExchange, HttpServiceError> {
        match self.execute(request) {
            Ok(exchange) => Ok(exchange),
            Err(error) if error.status_code.is_some() => Ok(HttpExchange {
                request: request.clone(),
                response: HttpResponse {
                    status_code: error.status_code.unwrap_or_default(),
                    headers: BTreeMap::new(),
                    body: error
                        .response_body_excerpt
                        .as_deref()
                        .and_then(|body| serde_json::from_str(body).ok())
                        .unwrap_or_else(|| {
                            error
                                .response_body_excerpt
                                .clone()
                                .map(Value::String)
                                .unwrap_or(Value::Null)
                        }),
                },
            }),
            Err(error) => Err(error),
        }
    }

    fn execute_async<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpExchange, HttpServiceError>> + Send + 'a>> {
        Box::pin(async move {
            let method = request.method.trim().to_uppercase();
            let url = request.url.trim().to_string();
            if url.is_empty() {
                return Err(HttpServiceError::new("HTTP request URL is required"));
            }
            self.guard_url(&url)?;
            let safe_url = safe_url_display(&url);
            let dynamic_client = if let Some(connect_timeout_ms) = request.connect_timeout_ms {
                let mut builder = AsyncClient::builder()
                    .timeout(Duration::from_millis(
                        request
                            .timeout_ms
                            .unwrap_or(self.config.default_timeout_ms)
                            .max(1),
                    ))
                    .connect_timeout(Duration::from_millis(connect_timeout_ms.max(1)))
                    .redirect(match request.follow_redirects {
                        Some(false) => Policy::none(),
                        _ => Policy::limited(10),
                    });
                if let Some(user_agent) = &self.config.user_agent {
                    builder = builder.user_agent(user_agent.clone());
                }
                Some(builder.build().map_err(|error| {
                    HttpServiceError::new(format!("Failed to build async request client: {error}"))
                })?)
            } else {
                None
            };
            let client = dynamic_client
                .as_ref()
                .unwrap_or(match request.follow_redirects {
                    Some(false) => &self.async_client_no_follow,
                    _ => &self.async_client_follow,
                });
            let mut last_error = None;
            for attempt in 0..=self.config.retry_count {
                if attempt > 0 {
                    let delay = self
                        .config
                        .retry_backoff_ms
                        .saturating_mul(2u64.saturating_pow(attempt - 1));
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                let mut builder = match method.as_str() {
                    "GET" => client.get(&url),
                    "POST" => client.post(&url),
                    "PUT" => client.put(&url),
                    "DELETE" => client.delete(&url),
                    "PATCH" => client.patch(&url),
                    other => {
                        return Err(HttpServiceError {
                            message: format!("Unsupported HTTP method: {other}"),
                            status_code: None,
                            response_body_excerpt: None,
                            request_url: Some(safe_url.clone()),
                            request_method: Some(other.to_string()),
                        });
                    }
                };
                for (key, value) in &request.headers {
                    builder = builder.header(key, value);
                }
                if let Some(body) = &request.body {
                    match request.body_encoding.as_deref().unwrap_or("json") {
                        "form" => {
                            let fields = body
                                .as_object()
                                .map(|object| {
                                    object
                                        .iter()
                                        .map(|(k, v)| {
                                            (
                                                k.clone(),
                                                v.as_str()
                                                    .map(ToOwned::to_owned)
                                                    .unwrap_or_else(|| v.to_string()),
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            builder = builder.form(&fields);
                        }
                        "text" => {
                            builder = builder
                                .header("Content-Type", "text/plain")
                                .body(body.to_string())
                        }
                        _ => builder = builder.json(body),
                    }
                }
                if let Some(timeout_ms) = request.timeout_ms {
                    builder = builder.timeout(Duration::from_millis(timeout_ms.max(1)));
                }
                if let Some(timeout_ms) = request.connect_timeout_ms {
                    let _ = timeout_ms;
                }
                if let Some(auth) = &request.basic_auth {
                    builder = builder.basic_auth(&auth.username, Some(&auth.password));
                }
                match builder.send().await {
                    Ok(response) => {
                        let status = response.status().as_u16();
                        let headers = response
                            .headers()
                            .iter()
                            .map(|(key, value)| {
                                (
                                    key.to_string(),
                                    value.to_str().unwrap_or_default().to_string(),
                                )
                            })
                            .collect();
                        let text = response.text().await.map_err(|error| HttpServiceError {
                            message: format!("Failed to read response body from {safe_url}: {error}"),
                            status_code: Some(status),
                            response_body_excerpt: None,
                            request_url: Some(safe_url.clone()),
                            request_method: Some(method.clone()),
                        })?;
                        let body =
                            serde_json::from_str(&text).unwrap_or(Value::String(text.clone()));
                        if status >= 500 {
                            last_error = Some(HttpServiceError {
                                message: format!("HTTP {status} for {safe_url}"),
                                status_code: Some(status),
                                response_body_excerpt: Some(text.chars().take(500).collect()),
                                request_url: Some(safe_url.clone()),
                                request_method: Some(method.clone()),
                            });
                            continue;
                        }
                        if status >= 400 {
                            return Err(HttpServiceError {
                                message: format!("HTTP {status} for {safe_url}"),
                                status_code: Some(status),
                                response_body_excerpt: Some(text.chars().take(500).collect()),
                                request_url: Some(safe_url.clone()),
                                request_method: Some(method.clone()),
                            });
                        }
                        return Ok(HttpExchange {
                            request: request.clone(),
                            response: HttpResponse {
                                status_code: status,
                                headers,
                                body,
                            },
                        });
                    }
                    Err(error) => {
                        last_error = Some(HttpServiceError {
                            message: format!("HTTP request failed for {safe_url}: {error}"),
                            status_code: None,
                            response_body_excerpt: None,
                            request_url: Some(safe_url.clone()),
                            request_method: Some(method.clone()),
                        });
                    }
                }
            }
            Err(last_error
                .unwrap_or_else(|| HttpServiceError::new("HTTP execution failed after retries")))
        })
    }

    fn execute_async_with_status<'a>(
        &'a self,
        request: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpExchange, HttpServiceError>> + Send + 'a>> {
        Box::pin(async move {
            match self.execute_async(request).await {
                Ok(exchange) => Ok(exchange),
                Err(error) if error.status_code.is_some() => Ok(HttpExchange {
                    request: request.clone(),
                    response: HttpResponse {
                        status_code: error.status_code.unwrap_or_default(),
                        headers: BTreeMap::new(),
                        body: error
                            .response_body_excerpt
                            .as_deref()
                            .and_then(|body| serde_json::from_str(body).ok())
                            .unwrap_or_else(|| {
                                error
                                    .response_body_excerpt
                                    .clone()
                                    .map(Value::String)
                                    .unwrap_or(Value::Null)
                            }),
                    },
                }),
                Err(error) => Err(error),
            }
        })
    }

    fn mode(&self) -> HttpRuntimeMode {
        HttpRuntimeMode::Real
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_http_client_config_defaults() {
        let config = RealHttpClientConfig::default();
        assert_eq!(config.default_timeout_ms, 30_000);
        assert_eq!(config.default_connect_timeout_ms, 10_000);
        assert!(config.user_agent.is_some());
        assert_eq!(config.retry_count, 3);
        assert!(!config.cache_enabled);
    }

    #[test]
    fn real_http_client_mode_is_real() {
        let client =
            RealHttpClient::new(RealHttpClientConfig::default()).expect("should build client");
        assert_eq!(client.mode(), HttpRuntimeMode::Real);
    }

    #[test]
    fn test_extract_host() {
        let client = RealHttpClient::new(RealHttpClientConfig::default()).unwrap();
        assert_eq!(
            client.extract_host("https://example.com/api/v1"),
            "example.com"
        );
        assert_eq!(
            client.extract_host("http://localhost:8080/foo"),
            "localhost"
        );
        assert_eq!(client.extract_host("http://127.0.0.1/"), "127.0.0.1");
    }
}
