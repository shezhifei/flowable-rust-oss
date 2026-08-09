//! HTTP proxy core aligned with Java `FlowableClientService`.

use super::server_config::ServerConfig;
use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use bytes::Bytes;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("Unable to connect to the Flowable server.")]
    Connect,
    #[error("Connection to the Flowable server timed out.")]
    Timeout,
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    Other(String),
}

impl ProxyError {
    pub fn from_reqwest(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout
        } else if err.is_connect() {
            Self::Connect
        } else {
            Self::Other(format!("{}: {err}", std::any::type_name_of_val(&err)))
        }
    }
}

/// Build engine URL: `{address}:{port}/{contextRoot}/{restRoot}/{uri}`
/// matching Java `FlowableClientService.getServerUrl`.
pub fn build_server_url(config: &ServerConfig, uri: &str) -> String {
    let context = strip_slashes(&config.context_root);
    let rest = strip_slashes(&config.rest_root);

    let mut base = format!("{}:{}", config.server_address.trim_end_matches('/'), config.port);
    if !context.is_empty() {
        base.push('/');
        base.push_str(&context);
    }
    if !rest.is_empty() {
        base.push('/');
        base.push_str(&rest);
    }

    let uri = if uri.is_empty() {
        String::new()
    } else if uri.starts_with('/') {
        uri.to_string()
    } else {
        format!("/{uri}")
    };
    format!("{base}{uri}")
}

fn strip_slashes(s: &str) -> String {
    s.trim().trim_matches('/').to_string()
}

pub struct ProxyClient {
    client: reqwest::Client,
    /// Always send preemptive Basic (Java flag defaults false but engine REST
    /// requires auth; Rust UI admin defaults to preemptive for reliability).
    preemptive_basic: bool,
}

impl ProxyClient {
    pub fn new() -> Self {
        let preemptive = std::env::var("FLOWABLE_ADMIN_PREEMPTIVE_BASIC")
            .map(|v| !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .danger_accept_invalid_certs(true)
            .build()
            .expect("reqwest client");
        Self {
            client,
            preemptive_basic: preemptive,
        }
    }

    pub async fn execute_json(
        &self,
        config: &ServerConfig,
        password: &str,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Bytes>,
        content_type: Option<&str>,
        expected: StatusCode,
    ) -> Result<Response, ProxyError> {
        let mut url = build_server_url(config, path);
        if !query.is_empty() {
            let mut ser = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in query {
                ser.append_pair(k, v);
            }
            let qs = ser.finish();
            if url.contains('?') {
                url.push('&');
                url.push_str(&qs);
            } else {
                url.push('?');
                url.push_str(&qs);
            }
        }

        let mut builder = self
            .client
            .request(
                reqwest::Method::from_bytes(method.as_str().as_bytes())
                    .unwrap_or(reqwest::Method::GET),
                &url,
            )
            .header(header::ACCEPT.as_str(), "application/json");

        if self.preemptive_basic {
            let token = B64.encode(format!("{}:{}", config.user_name, password));
            builder = builder.header(header::AUTHORIZATION.as_str(), format!("Basic {token}"));
        } else {
            builder = builder.basic_auth(&config.user_name, Some(password));
        }

        if let Some(b) = body {
            if let Some(ct) = content_type {
                builder = builder.header(header::CONTENT_TYPE.as_str(), ct);
            }
            builder = builder.body(b);
        }

        let response = builder.send().await.map_err(ProxyError::from_reqwest)?;
        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let resp_headers = response.headers().clone();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| ProxyError::Other(e.to_string()))?;

        if status == expected
            || (expected == StatusCode::OK
                && (status == StatusCode::CREATED || status == StatusCode::NO_CONTENT))
        {
            return Ok(build_axum_response(status, &resp_headers, bytes));
        }

        // Align with Java extractError: prefer JSON "exception" field.
        let message = extract_error(&bytes, &format!("An error occurred while calling Flowable: {status}"));
        Err(ProxyError::Message(message))
    }

    /// Passthrough status + body + content-type (binary/resource downloads).
    pub async fn execute_passthrough(
        &self,
        config: &ServerConfig,
        password: &str,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Bytes>,
        content_type: Option<&str>,
    ) -> Result<Response, ProxyError> {
        let mut url = build_server_url(config, path);
        if !query.is_empty() {
            let mut ser = url::form_urlencoded::Serializer::new(String::new());
            for (k, v) in query {
                ser.append_pair(k, v);
            }
            let qs = ser.finish();
            if url.contains('?') {
                url.push('&');
            } else {
                url.push('?');
            }
            url.push_str(&qs);
        }

        let mut builder = self.client.request(
            reqwest::Method::from_bytes(method.as_str().as_bytes())
                .unwrap_or(reqwest::Method::GET),
            &url,
        );

        let token = B64.encode(format!("{}:{}", config.user_name, password));
        builder = builder.header(header::AUTHORIZATION.as_str(), format!("Basic {token}"));

        if let Some(b) = body {
            if let Some(ct) = content_type {
                builder = builder.header(header::CONTENT_TYPE.as_str(), ct);
            }
            builder = builder.body(b);
        }

        let response = builder.send().await.map_err(ProxyError::from_reqwest)?;
        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        if status == StatusCode::UNAUTHORIZED {
            let bytes = response
                .bytes()
                .await
                .unwrap_or_default();
            let message = extract_error(
                &bytes,
                "An error occurred while calling Flowable: 401 Unauthorized",
            );
            return Err(ProxyError::Message(message));
        }
        let headers = response.headers().clone();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| ProxyError::Other(e.to_string()))?;
        Ok(build_axum_response(status, &headers, bytes))
    }
}

impl Default for ProxyClient {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_error(body: &Bytes, default: &str) -> String {
    if body.is_empty() {
        return default.to_string();
    }
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(exc) = v.get("exception").and_then(|x| x.as_str()) {
            return exc.to_string();
        }
        if let Some(msg) = v.get("message").and_then(|x| x.as_str()) {
            return msg.to_string();
        }
    }
    default.to_string()
}

fn build_axum_response(status: StatusCode, headers: &HeaderMap, body: Bytes) -> Response {
    let mut response = Response::builder().status(status);
    if let Some(ct) = headers.get(header::CONTENT_TYPE) {
        if let Ok(v) = HeaderValue::from_bytes(ct.as_bytes()) {
            response = response.header(header::CONTENT_TYPE, v);
        }
    } else if !body.is_empty() {
        response = response.header(header::CONTENT_TYPE, "application/json");
    }
    if let Some(cd) = headers.get(header::CONTENT_DISPOSITION) {
        if let Ok(v) = HeaderValue::from_bytes(cd.as_bytes()) {
            response = response.header(header::CONTENT_DISPOSITION, v);
        }
    }
    response
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::server_config::ServerConfig;

    fn sample_config() -> ServerConfig {
        ServerConfig {
            id: "1".into(),
            name: "p".into(),
            description: "d".into(),
            server_address: "http://localhost".into(),
            port: 8080,
            context_root: String::new(),
            rest_root: String::new(),
            user_name: "admin".into(),
            password: "x".into(),
            endpoint_type: 1,
            tenant_id: None,
        }
    }

    #[test]
    fn url_without_roots() {
        let cfg = sample_config();
        assert_eq!(
            build_server_url(&cfg, "repository/deployments"),
            "http://localhost:8080/repository/deployments"
        );
    }

    #[test]
    fn url_with_roots() {
        let mut cfg = sample_config();
        cfg.context_root = "flowable-ui".into();
        cfg.rest_root = "process-api".into();
        assert_eq!(
            build_server_url(&cfg, "/runtime/tasks"),
            "http://localhost:8080/flowable-ui/process-api/runtime/tasks"
        );
    }
}
