use crate::models::{EventPayload, EventRegistryError};
use crate::ssrf_guard::{
    safe_url_display, validate_outbound_url, OutboundUrlGuardConfig, OutboundUrlGuardError,
};
use flowable_engine::error::FlowableError;
use reqwest::{Certificate, Client};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// Header carrying the delivery dispatch token so receivers can deduplicate
/// at-least-once redeliveries without inspecting the event body.
pub const DISPATCH_TOKEN_HEADER: &str = "X-Flowable-Dispatch-Token";

/// Outbound channel adapter registered by name in [`crate::pipeline::EventRegistryConfiguration`].
pub trait OutboundChannelAdapter: Send + Sync {
    fn send(
        &self,
        destination: Option<&str>,
        event: EventPayload,
        channel_config: &Value,
    ) -> Result<(), FlowableError>;
}

/// Inbound channel adapter registration handle (pipeline stages live in [`crate::pipeline`]).
pub trait InboundChannelAdapter: Send + Sync {}

#[derive(Default)]
pub struct InMemoryInboundAdapter;

impl InboundChannelAdapter for InMemoryInboundAdapter {}

#[derive(Default)]
pub struct InMemoryOutboundAdapter;

impl OutboundChannelAdapter for InMemoryOutboundAdapter {
    fn send(
        &self,
        _destination: Option<&str>,
        _event: EventPayload,
        _channel_config: &Value,
    ) -> Result<(), FlowableError> {
        Ok(())
    }
}

pub struct RestOutboundAdapter {
    adapter: RestChannelAdapter,
}

impl RestOutboundAdapter {
    pub fn new() -> Self {
        Self::with_ssrf_guard(OutboundUrlGuardConfig::default())
    }

    pub fn with_ssrf_guard(ssrf_guard: OutboundUrlGuardConfig) -> Self {
        Self {
            adapter: RestChannelAdapter::with_ssrf_guard(ssrf_guard),
        }
    }
}

impl Default for RestOutboundAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutboundChannelAdapter for RestOutboundAdapter {
    fn send(
        &self,
        destination: Option<&str>,
        event: EventPayload,
        channel_config: &Value,
    ) -> Result<(), FlowableError> {
        let destination = destination
            .or_else(|| rest_destination_from_config(channel_config))
            .ok_or_else(|| {
                FlowableError::ExecutionError(
                    "REST outbound channel requires a destination URL".to_string(),
                )
            })?;
        self.adapter
            .send_outbound_blocking_with_configuration(destination, event, channel_config)
            .map_err(|error| FlowableError::ExecutionError(error.to_string()))
    }
}

pub struct RestChannelAdapter {
    pub client: Client,
    ssrf_guard: OutboundUrlGuardConfig,
}

impl RestChannelAdapter {
    pub fn new() -> Self {
        Self::with_ssrf_guard(OutboundUrlGuardConfig::default())
    }

    pub fn with_ssrf_guard(ssrf_guard: OutboundUrlGuardConfig) -> Self {
        Self {
            client: Client::new(),
            ssrf_guard,
        }
    }

    pub async fn handle_inbound(&self, event: EventPayload) -> Result<(), EventRegistryError> {
        serde_json::to_value(&event)
            .map(|_| ())
            .map_err(|e| EventRegistryError::InboundError(e.to_string()))
    }

    pub async fn send_outbound(
        &self,
        url: &str,
        event: EventPayload,
    ) -> Result<(), EventRegistryError> {
        validate_outbound_url(url, &self.ssrf_guard).map_err(ssrf_to_outbound_error)?;
        let safe_url = safe_url_display(url);
        let dispatch_token = event.dispatch_token.clone();
        let mut request = self.client.post(url);
        if let Some(token) = dispatch_token.as_deref() {
            request = request.header(DISPATCH_TOKEN_HEADER, token);
        }
        let response = request
            .json(&event)
            .send()
            .await
            .map_err(|e| EventRegistryError::OutboundError(format!(
                "REST outbound dispatch to '{safe_url}' failed: {e}"
            )))?;
        let status = response.status();
        if !status.is_success() {
            return Err(EventRegistryError::OutboundError(format!(
                "REST outbound dispatch to '{}' failed with status {}",
                safe_url, status
            )));
        }
        Ok(())
    }

    pub fn send_outbound_blocking(
        &self,
        url: &str,
        event: EventPayload,
    ) -> Result<(), EventRegistryError> {
        self.send_outbound_blocking_with_configuration(url, event, &Value::Null)
    }

    pub fn send_outbound_blocking_with_configuration(
        &self,
        url: &str,
        event: EventPayload,
        configuration: &Value,
    ) -> Result<(), EventRegistryError> {
        validate_outbound_url(url, &self.ssrf_guard).map_err(ssrf_to_outbound_error)?;
        let url = url.to_string();
        let configuration = configuration.clone();
        let ssrf_guard = self.ssrf_guard.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| EventRegistryError::OutboundError(e.to_string()))?;
            runtime.block_on(send_outbound_with_configuration(
                &url,
                event,
                &configuration,
                &ssrf_guard,
            ))
        })
        .join()
        .map_err(|_| {
            EventRegistryError::OutboundError("REST outbound dispatch thread panicked".to_string())
        })?
    }
}

async fn send_outbound_with_configuration(
    url: &str,
    event: EventPayload,
    configuration: &Value,
    ssrf_guard: &OutboundUrlGuardConfig,
) -> Result<(), EventRegistryError> {
    // Re-check inside the worker thread (config is already checked by callers).
    validate_outbound_url(url, ssrf_guard).map_err(ssrf_to_outbound_error)?;
    let safe_url = safe_url_display(url);
    let client = rest_client(configuration)?;
    let dispatch_token = event.dispatch_token.clone();
    let mut request = client.post(url);
    if let Some(token) = dispatch_token.as_deref() {
        request = request.header(DISPATCH_TOKEN_HEADER, token);
    }
    let response = request
        .json(&event)
        .send()
        .await
        .map_err(|e| {
            EventRegistryError::OutboundError(format!(
                "REST outbound dispatch to '{safe_url}' failed: {e}"
            ))
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(EventRegistryError::OutboundError(format!(
            "REST outbound dispatch to '{}' failed with status {}",
            safe_url, status
        )));
    }
    Ok(())
}

fn ssrf_to_outbound_error(error: OutboundUrlGuardError) -> EventRegistryError {
    EventRegistryError::OutboundError(error.to_string())
}

fn rest_client(configuration: &Value) -> Result<Client, EventRegistryError> {
    let mut builder = Client::builder().timeout(Duration::from_secs(5));
    if let Some(certificate_pem) = rest_tls_root_certificate_pem(configuration) {
        let certificate = Certificate::from_pem(certificate_pem.trim().as_bytes())
            .map_err(|e| EventRegistryError::OutboundError(e.to_string()))?;
        builder = builder.add_root_certificate(certificate);
    }
    builder
        .build()
        .map_err(|e| EventRegistryError::OutboundError(e.to_string()))
}

fn rest_tls_root_certificate_pem(configuration: &Value) -> Option<&str> {
    configuration
        .get("tlsRootCertificatePem")
        .and_then(Value::as_str)
        .or_else(|| {
            configuration
                .get("tls")
                .and_then(|tls| tls.get("rootCertificatePem"))
                .and_then(Value::as_str)
        })
}

impl Default for RestChannelAdapter {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn rest_destination_from_config(configuration: &Value) -> Option<&str> {
    ["destination", "endpoint", "url"].iter().find_map(|field| {
        configuration
            .get(*field)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
    })
}

/// Convenience: wrap a custom outbound adapter for tests/extensions.
pub fn boxed_outbound_adapter(
    adapter: impl OutboundChannelAdapter + 'static,
) -> Arc<dyn OutboundChannelAdapter> {
    Arc::new(adapter)
}
