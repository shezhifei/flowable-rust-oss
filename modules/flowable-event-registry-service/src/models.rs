use chrono::{DateTime, Utc};
use flowable_engine::persistence::runtime_store::{
    EventRegistryChannelDefinition, EventRegistryDeployment as StoredEventRegistryDeployment,
    EventRegistryEventDefinition, EventRegistryEventDirection, EventRegistryEventInstanceDelivery,
    EventRegistryEventInstanceStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventRegistryDeploymentRequest {
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub parent_deployment_id: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    pub resources: Vec<EventRegistryDeploymentResource>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventRegistryDeploymentResource {
    pub resource_name: String,
    pub resource: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRegistryResourceData {
    pub deployment_id: String,
    pub resource_name: String,
    pub resource_type: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub created_at: i64,
}

impl EventRegistryResourceData {
    pub(crate) fn new(
        deployment_id: String,
        resource: EventRegistryDeploymentResource,
        created_at: i64,
    ) -> Self {
        Self {
            deployment_id,
            resource_type: "resource".to_string(),
            content_type: content_type_for_name(&resource.resource_name).to_string(),
            bytes: resource.resource.into_bytes(),
            resource_name: resource.resource_name,
            created_at,
        }
    }
}

pub type EventRegistryDeployment = StoredEventRegistryDeployment;
pub type ChannelDefinition = EventRegistryChannelDefinition;
pub type EventDefinition = EventRegistryEventDefinition;
pub type EventDirection = EventRegistryEventDirection;
pub type EventInstanceStatus = EventRegistryEventInstanceStatus;
pub type EventInstanceDelivery = EventRegistryEventInstanceDelivery;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboundEventRequest {
    pub event_type: String,
    pub event_payload: Value,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutboundEventRequest {
    pub event_definition_key: String,
    pub event_payload: Value,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventInstanceRequest {
    pub event_definition_id: Option<String>,
    pub event_definition_key: Option<String>,
    pub channel_definition_id: Option<String>,
    pub channel_definition_key: Option<String>,
    pub event_payload: Value,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PagedResult<T> {
    pub start: usize,
    pub size: usize,
    pub total: usize,
    pub data: Vec<T>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRegistryEngineInfo {
    pub name: String,
    pub version: String,
    pub resource_url: Option<String>,
    pub exception: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPayload {
    pub event_type: String,
    pub payload: Value,
    /// Idempotency key persisted on the delivery before external dispatch;
    /// receivers can deduplicate at-least-once redeliveries on this token.
    /// Not part of the serialized event body: REST dispatch transmits it via
    /// the `X-Flowable-Dispatch-Token` header.
    #[serde(skip_serializing, default)]
    pub dispatch_token: Option<String>,
}

#[derive(Debug)]
pub enum EventRegistryError {
    InboundError(String),
    OutboundError(String),
    ValidationError(String),
    DeliveryError(String),
}

impl std::fmt::Display for EventRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InboundError(msg) => write!(f, "Inbound channel error: {}", msg),
            Self::OutboundError(msg) => write!(f, "Outbound channel error: {}", msg),
            Self::ValidationError(msg) => write!(f, "Validation error: {}", msg),
            Self::DeliveryError(msg) => write!(f, "Delivery error: {}", msg),
        }
    }
}

impl std::error::Error for EventRegistryError {}

#[derive(Clone, Debug)]
pub struct EventRetryPolicy {
    pub max_retries: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub backoff_multiplier: f64,
}

impl Default for EventRetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 1000,
            max_backoff_ms: 30000,
            backoff_multiplier: 2.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDeliveryRetry {
    pub delivery_id: String,
    pub retry_count: u32,
    pub next_retry_at: DateTime<Utc>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelDefinitionUpdateRequest {
    pub name: Option<String>,
    pub configuration: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventDefinitionUpdateRequest {
    pub name: Option<String>,
    pub payload: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationError {
    pub field: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(field) = &self.field {
            write!(
                f,
                "Validation error for field '{}': {}",
                field, self.message
            )
        } else {
            write!(f, "Validation error: {}", self.message)
        }
    }
}

impl std::error::Error for ValidationError {}

pub(crate) fn content_type_for_name(resource_name: &str) -> &'static str {
    let lower_name = resource_name.to_ascii_lowercase();
    if lower_name.ends_with(".channel")
        || lower_name.ends_with(".event")
        || lower_name.ends_with(".json")
    {
        "application/json"
    } else if lower_name.ends_with(".xml") {
        "application/xml"
    } else if lower_name.ends_with(".txt") {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

pub(crate) fn page_items<T>(items: Vec<T>, start: usize, size: Option<usize>) -> PagedResult<T> {
    let total = items.len();
    let page_size = size.unwrap_or(total.saturating_sub(start));
    let data = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    PagedResult {
        start,
        size: data.len(),
        total,
        data,
    }
}
