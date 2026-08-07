use crate::adapter::{
    InMemoryInboundAdapter, InMemoryOutboundAdapter, InboundChannelAdapter, OutboundChannelAdapter,
    RestOutboundAdapter,
};
use crate::models::{EventDefinition, EventInstanceDelivery, EventPayload, ValidationError};
use crate::ssrf_guard::OutboundUrlGuardConfig;
use crate::tenant_fallback::{TenantFallbackPolicy, NO_TENANT_ID};
use flowable_engine::error::FlowableError;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Raw inbound request addressed by channel key (Task 16 will drive the full pipeline).
#[derive(Clone, Debug)]
pub struct InboundRawEvent {
    pub channel_key: String,
    pub body: Value,
    pub headers: BTreeMap<String, String>,
    pub tenant_hint: Option<String>,
}

/// Mutable pipeline context passed between inbound stages.
#[derive(Clone, Debug)]
pub struct InboundEventContext {
    pub channel_key: String,
    pub raw_body: Value,
    pub headers: BTreeMap<String, String>,
    pub payload: Value,
    pub tenant_id: Option<String>,
    pub event_key: Option<String>,
}

impl InboundEventContext {
    pub fn from_raw(raw: &InboundRawEvent) -> Self {
        Self {
            channel_key: raw.channel_key.clone(),
            raw_body: raw.body.clone(),
            headers: raw.headers.clone(),
            payload: raw.body.clone(),
            tenant_id: raw.tenant_hint.clone(),
            event_key: None,
        }
    }
}

pub trait InboundPayloadExtractor: Send + Sync {
    fn extract(
        &self,
        raw: &InboundRawEvent,
        channel_config: &Value,
    ) -> Result<Value, FlowableError>;
}

pub trait InboundEventFilter: Send + Sync {
    fn retain(
        &self,
        context: &InboundEventContext,
        channel_config: &Value,
    ) -> Result<bool, FlowableError>;
}

pub trait InboundTenantDetector: Send + Sync {
    fn detect_tenant(
        &self,
        context: &InboundEventContext,
        channel_config: &Value,
    ) -> Result<Option<String>, FlowableError>;
}

pub trait InboundEventTransformer: Send + Sync {
    fn transform(
        &self,
        context: &InboundEventContext,
        channel_config: &Value,
    ) -> Result<Value, FlowableError>;
}

pub trait InboundEventKeyDetector: Send + Sync {
    fn detect_event_key(
        &self,
        context: &InboundEventContext,
        channel_config: &Value,
    ) -> Result<String, FlowableError>;
}

pub trait InboundEventConsumer: Send + Sync {
    fn consume(
        &self,
        delivery: &EventInstanceDelivery,
        definition: &EventDefinition,
    ) -> Result<(), FlowableError>;
}

pub trait OutboundEventTransformer: Send + Sync {
    fn transform(
        &self,
        payload: &Value,
        channel_config: &Value,
        event_type: &str,
    ) -> Result<Value, FlowableError>;
}

#[derive(Default)]
pub struct JsonPayloadExtractor;

impl InboundPayloadExtractor for JsonPayloadExtractor {
    fn extract(
        &self,
        raw: &InboundRawEvent,
        _channel_config: &Value,
    ) -> Result<Value, FlowableError> {
        Ok(raw.body.clone())
    }
}

#[derive(Default)]
pub struct DefaultInboundFilter;

impl InboundEventFilter for DefaultInboundFilter {
    fn retain(
        &self,
        context: &InboundEventContext,
        channel_config: &Value,
    ) -> Result<bool, FlowableError> {
        if channel_config
            .get("rejectAll")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(false);
        }
        if let Some(field) = channel_config
            .get("filterRequiredField")
            .and_then(Value::as_str)
            && context.payload.get(field).is_none()
        {
            return Ok(false);
        }
        Ok(true)
    }
}

#[derive(Default)]
pub struct DefaultTenantDetector;

impl InboundTenantDetector for DefaultTenantDetector {
    fn detect_tenant(
        &self,
        context: &InboundEventContext,
        channel_config: &Value,
    ) -> Result<Option<String>, FlowableError> {
        if let Some(fixed) = channel_config
            .get("tenantId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return Ok(Some(fixed.to_string()));
        }
        if let Some(header_name) = channel_config
            .get("tenantIdHeader")
            .and_then(Value::as_str)
            .or_else(|| {
                channel_config
                    .get("channelEventTenantIdDetection")
                    .and_then(|value| value.get("jsonField"))
                    .and_then(Value::as_str)
            })
        {
            if let Some(value) = context.headers.get(header_name) {
                return Ok(Some(value.clone()));
            }
            if let Some(value) = context
                .payload
                .get(header_name)
                .and_then(Value::as_str)
            {
                return Ok(Some(value.to_string()));
            }
        }
        Ok(context.tenant_id.clone())
    }
}

#[derive(Default)]
pub struct DefaultInboundTransformer;

impl InboundEventTransformer for DefaultInboundTransformer {
    fn transform(
        &self,
        context: &InboundEventContext,
        channel_config: &Value,
    ) -> Result<Value, FlowableError> {
        if channel_config
            .get("failTransform")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(FlowableError::ExecutionError(format!(
                "Inbound transform failed for channel '{}'",
                context.channel_key
            )));
        }
        Ok(context.payload.clone())
    }
}

#[derive(Default)]
pub struct DefaultEventKeyDetector;

impl InboundEventKeyDetector for DefaultEventKeyDetector {
    fn detect_event_key(
        &self,
        context: &InboundEventContext,
        channel_config: &Value,
    ) -> Result<String, FlowableError> {
        if let Some(fixed) = channel_config
            .get("fixedEventKey")
            .and_then(Value::as_str)
            .or_else(|| {
                channel_config
                    .get("channelEventKeyDetection")
                    .and_then(|value| value.get("fixedValue"))
                    .and_then(Value::as_str)
            })
            .filter(|value| !value.is_empty())
        {
            return Ok(fixed.to_string());
        }

        let field = channel_config
            .get("eventKeyJsonField")
            .and_then(Value::as_str)
            .or_else(|| {
                channel_config
                    .get("channelEventKeyDetection")
                    .and_then(|value| value.get("jsonField"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("eventKey");

        if let Some(value) = context.payload.get(field).and_then(Value::as_str) {
            return Ok(value.to_string());
        }
        if let Some(value) = context.headers.get(field) {
            return Ok(value.clone());
        }
        if let Some(value) = context.event_key.clone() {
            return Ok(value);
        }

        Err(FlowableError::ExecutionError(format!(
            "Unable to detect event key for channel '{}'",
            context.channel_key
        )))
    }
}

#[derive(Default)]
pub struct NoOpInboundConsumer;

impl InboundEventConsumer for NoOpInboundConsumer {
    fn consume(
        &self,
        _delivery: &EventInstanceDelivery,
        _definition: &EventDefinition,
    ) -> Result<(), FlowableError> {
        Ok(())
    }
}

#[derive(Default)]
pub struct JsonOutboundTransformer;

impl OutboundEventTransformer for JsonOutboundTransformer {
    fn transform(
        &self,
        payload: &Value,
        _channel_config: &Value,
        _event_type: &str,
    ) -> Result<Value, FlowableError> {
        Ok(payload.clone())
    }
}

/// Engine-local Event Registry registries. Maps are populated before construction
/// and are never mutated after the service is created.
#[derive(Clone)]
pub struct EventRegistryConfiguration {
    payload_extractors: BTreeMap<String, Arc<dyn InboundPayloadExtractor>>,
    filters: BTreeMap<String, Arc<dyn InboundEventFilter>>,
    tenant_detectors: BTreeMap<String, Arc<dyn InboundTenantDetector>>,
    inbound_transformers: BTreeMap<String, Arc<dyn InboundEventTransformer>>,
    key_detectors: BTreeMap<String, Arc<dyn InboundEventKeyDetector>>,
    consumers: BTreeMap<String, Arc<dyn InboundEventConsumer>>,
    outbound_transformers: BTreeMap<String, Arc<dyn OutboundEventTransformer>>,
    inbound_adapters: BTreeMap<String, Arc<dyn InboundChannelAdapter>>,
    outbound_adapters: BTreeMap<String, Arc<dyn OutboundChannelAdapter>>,
    /// Java `AbstractEngineConfiguration.fallbackToDefaultTenant` (:324), default false.
    pub fallback_to_default_tenant: bool,
    /// Fixed default-tenant value used when fallback is enabled.
    /// Empty string = Java `NO_TENANT_ID` (`AbstractEngineConfiguration.java:329`).
    pub default_tenant: String,
    /// SSRF guard applied to the built-in `rest` outbound adapter.
    pub outbound_ssrf_guard: OutboundUrlGuardConfig,
}

impl Default for EventRegistryConfiguration {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl EventRegistryConfiguration {
    pub fn builder() -> EventRegistryConfigurationBuilder {
        EventRegistryConfigurationBuilder::default().with_defaults()
    }

    pub fn register_payload_extractor(
        &mut self,
        name: impl Into<String>,
        extractor: Arc<dyn InboundPayloadExtractor>,
    ) {
        self.payload_extractors.insert(name.into(), extractor);
    }

    pub fn register_filter(&mut self, name: impl Into<String>, filter: Arc<dyn InboundEventFilter>) {
        self.filters.insert(name.into(), filter);
    }

    pub fn register_tenant_detector(
        &mut self,
        name: impl Into<String>,
        detector: Arc<dyn InboundTenantDetector>,
    ) {
        self.tenant_detectors.insert(name.into(), detector);
    }

    pub fn register_inbound_transformer(
        &mut self,
        name: impl Into<String>,
        transformer: Arc<dyn InboundEventTransformer>,
    ) {
        self.inbound_transformers.insert(name.into(), transformer);
    }

    pub fn register_key_detector(
        &mut self,
        name: impl Into<String>,
        detector: Arc<dyn InboundEventKeyDetector>,
    ) {
        self.key_detectors.insert(name.into(), detector);
    }

    pub fn register_consumer(
        &mut self,
        name: impl Into<String>,
        consumer: Arc<dyn InboundEventConsumer>,
    ) {
        self.consumers.insert(name.into(), consumer);
    }

    pub fn register_outbound_transformer(
        &mut self,
        name: impl Into<String>,
        transformer: Arc<dyn OutboundEventTransformer>,
    ) {
        self.outbound_transformers.insert(name.into(), transformer);
    }

    pub fn register_inbound_adapter(
        &mut self,
        name: impl Into<String>,
        adapter: Arc<dyn InboundChannelAdapter>,
    ) {
        self.inbound_adapters.insert(name.into(), adapter);
    }

    pub fn register_outbound_adapter(
        &mut self,
        name: impl Into<String>,
        adapter: Arc<dyn OutboundChannelAdapter>,
    ) {
        self.outbound_adapters.insert(name.into(), adapter);
    }

    /// Enable/disable tenant fallback (Java `setFallbackToDefaultTenant`).
    pub fn set_fallback_to_default_tenant(&mut self, enabled: bool) {
        self.fallback_to_default_tenant = enabled;
    }

    /// Set the fixed default-tenant id used when fallback is enabled
    /// (Java `setDefaultTenantValue`). Empty = `NO_TENANT_ID`.
    pub fn set_default_tenant(&mut self, default_tenant: impl Into<String>) {
        self.default_tenant = default_tenant.into();
    }

    /// Snapshot policy for definition resolve + consumer matching.
    pub fn tenant_fallback_policy(&self) -> TenantFallbackPolicy {
        TenantFallbackPolicy {
            fallback_to_default_tenant: self.fallback_to_default_tenant,
            default_tenant: self.default_tenant.clone(),
        }
    }

    pub fn payload_extractor_names(&self) -> Vec<String> {
        self.payload_extractors.keys().cloned().collect()
    }

    pub fn filter_names(&self) -> Vec<String> {
        self.filters.keys().cloned().collect()
    }

    pub fn tenant_detector_names(&self) -> Vec<String> {
        self.tenant_detectors.keys().cloned().collect()
    }

    pub fn inbound_transformer_names(&self) -> Vec<String> {
        self.inbound_transformers.keys().cloned().collect()
    }

    pub fn key_detector_names(&self) -> Vec<String> {
        self.key_detectors.keys().cloned().collect()
    }

    pub fn consumer_names(&self) -> Vec<String> {
        self.consumers.keys().cloned().collect()
    }

    pub fn outbound_transformer_names(&self) -> Vec<String> {
        self.outbound_transformers.keys().cloned().collect()
    }

    pub fn inbound_adapter_names(&self) -> Vec<String> {
        self.inbound_adapters.keys().cloned().collect()
    }

    pub fn outbound_adapter_names(&self) -> Vec<String> {
        self.outbound_adapters.keys().cloned().collect()
    }

    pub fn payload_extractor(&self, name: &str) -> Option<Arc<dyn InboundPayloadExtractor>> {
        self.payload_extractors.get(name).cloned()
    }

    pub fn filter(&self, name: &str) -> Option<Arc<dyn InboundEventFilter>> {
        self.filters.get(name).cloned()
    }

    pub fn tenant_detector(&self, name: &str) -> Option<Arc<dyn InboundTenantDetector>> {
        self.tenant_detectors.get(name).cloned()
    }

    pub fn inbound_transformer(&self, name: &str) -> Option<Arc<dyn InboundEventTransformer>> {
        self.inbound_transformers.get(name).cloned()
    }

    pub fn key_detector(&self, name: &str) -> Option<Arc<dyn InboundEventKeyDetector>> {
        self.key_detectors.get(name).cloned()
    }

    pub fn consumer(&self, name: &str) -> Option<Arc<dyn InboundEventConsumer>> {
        self.consumers.get(name).cloned()
    }

    pub fn outbound_transformer(&self, name: &str) -> Option<Arc<dyn OutboundEventTransformer>> {
        self.outbound_transformers.get(name).cloned()
    }

    pub fn inbound_adapter(&self, name: &str) -> Option<Arc<dyn InboundChannelAdapter>> {
        self.inbound_adapters.get(name).cloned()
    }

    pub fn outbound_adapter(&self, name: &str) -> Option<Arc<dyn OutboundChannelAdapter>> {
        self.outbound_adapters.get(name).cloned()
    }

    pub(crate) fn validate_channel_configuration(
        &self,
        channel_key: &str,
        channel_type: &str,
        configuration: &Value,
    ) -> Result<(), FlowableError> {
        let adapter_type = configuration
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                FlowableError::DeploymentValidationError(format!(
                    "Channel '{}' is missing adapter type",
                    channel_key
                ))
            })?;

        match channel_type {
            "inbound" => {
                require_registered(
                    channel_key,
                    "inbound adapter",
                    adapter_type,
                    &self.inbound_adapter_names(),
                )?;
                validate_optional_named(
                    channel_key,
                    "payload extractor",
                    first_string(configuration, &["payloadExtractor", "deserializerType"]),
                    &self.payload_extractor_names(),
                )?;
                validate_optional_named(
                    channel_key,
                    "filter",
                    first_string(configuration, &["filter", "eventFilter"]),
                    &self.filter_names(),
                )?;
                validate_optional_named(
                    channel_key,
                    "tenant detector",
                    first_string(configuration, &["tenantDetector", "tenantDetection"]),
                    &self.tenant_detector_names(),
                )?;
                validate_optional_named(
                    channel_key,
                    "inbound transformer",
                    first_string(configuration, &["transformer", "eventTransformer"]),
                    &self.inbound_transformer_names(),
                )?;
                validate_optional_named(
                    channel_key,
                    "key detector",
                    first_string(configuration, &["keyDetector", "eventKeyDetector"]),
                    &self.key_detector_names(),
                )?;
                validate_optional_named(
                    channel_key,
                    "consumer",
                    first_string(configuration, &["consumer"]),
                    &self.consumer_names(),
                )?;
            }
            "outbound" => {
                require_registered(
                    channel_key,
                    "outbound adapter",
                    adapter_type,
                    &self.outbound_adapter_names(),
                )?;
                validate_optional_named(
                    channel_key,
                    "outbound transformer",
                    first_string(
                        configuration,
                        &["outboundTransformer", "serializerType", "transformer"],
                    ),
                    &self.outbound_transformer_names(),
                )?;
            }
            other => {
                return Err(FlowableError::DeploymentValidationError(format!(
                    "Unsupported channelType '{}' for channel '{}'",
                    other, channel_key
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn resolve_payload_extractor(
        &self,
        configuration: &Value,
    ) -> Result<Arc<dyn InboundPayloadExtractor>, FlowableError> {
        let name = first_string(configuration, &["payloadExtractor", "deserializerType"])
            .unwrap_or_else(|| "json".to_string());
        self.payload_extractor(&name).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Unknown payload extractor '{}'. Allowed: {}",
                name,
                join_names(&self.payload_extractor_names())
            ))
        })
    }

    pub(crate) fn resolve_filter(
        &self,
        configuration: &Value,
    ) -> Result<Arc<dyn InboundEventFilter>, FlowableError> {
        let name = first_string(configuration, &["filter", "eventFilter"])
            .unwrap_or_else(|| "default".to_string());
        self.filter(&name).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Unknown filter '{}'. Allowed: {}",
                name,
                join_names(&self.filter_names())
            ))
        })
    }

    pub(crate) fn resolve_tenant_detector(
        &self,
        configuration: &Value,
    ) -> Result<Arc<dyn InboundTenantDetector>, FlowableError> {
        let name = first_string(configuration, &["tenantDetector", "tenantDetection"])
            .unwrap_or_else(|| "default".to_string());
        self.tenant_detector(&name).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Unknown tenant detector '{}'. Allowed: {}",
                name,
                join_names(&self.tenant_detector_names())
            ))
        })
    }

    pub(crate) fn resolve_inbound_transformer(
        &self,
        configuration: &Value,
    ) -> Result<Arc<dyn InboundEventTransformer>, FlowableError> {
        let name = first_string(configuration, &["transformer", "eventTransformer"])
            .unwrap_or_else(|| "default".to_string());
        self.inbound_transformer(&name).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Unknown inbound transformer '{}'. Allowed: {}",
                name,
                join_names(&self.inbound_transformer_names())
            ))
        })
    }

    pub(crate) fn resolve_key_detector(
        &self,
        configuration: &Value,
    ) -> Result<Arc<dyn InboundEventKeyDetector>, FlowableError> {
        let name = first_string(configuration, &["keyDetector", "eventKeyDetector"])
            .unwrap_or_else(|| "default".to_string());
        self.key_detector(&name).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Unknown key detector '{}'. Allowed: {}",
                name,
                join_names(&self.key_detector_names())
            ))
        })
    }

    pub(crate) fn resolve_consumer(
        &self,
        configuration: &Value,
    ) -> Result<Arc<dyn InboundEventConsumer>, FlowableError> {
        let name =
            first_string(configuration, &["consumer"]).unwrap_or_else(|| "default".to_string());
        self.consumer(&name).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Unknown consumer '{}'. Allowed: {}",
                name,
                join_names(&self.consumer_names())
            ))
        })
    }

    pub(crate) fn resolve_outbound_adapter(
        &self,
        channel_key: &str,
        configuration: &Value,
    ) -> Result<Arc<dyn OutboundChannelAdapter>, FlowableError> {
        let adapter_type = configuration
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("in-memory");
        self.outbound_adapter(adapter_type).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Unsupported outbound channel implementation '{}' for channel '{}'. Allowed: {}",
                adapter_type,
                channel_key,
                join_names(&self.outbound_adapter_names())
            ))
        })
    }

    pub(crate) fn resolve_outbound_transformer(
        &self,
        configuration: &Value,
    ) -> Result<Arc<dyn OutboundEventTransformer>, FlowableError> {
        let name = first_string(
            configuration,
            &["outboundTransformer", "serializerType", "transformer"],
        )
        .unwrap_or_else(|| "json".to_string());
        self.outbound_transformer(&name).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Unknown outbound transformer '{}'. Allowed: {}",
                name,
                join_names(&self.outbound_transformer_names())
            ))
        })
    }
}

#[derive(Default)]
pub struct EventRegistryConfigurationBuilder {
    payload_extractors: BTreeMap<String, Arc<dyn InboundPayloadExtractor>>,
    filters: BTreeMap<String, Arc<dyn InboundEventFilter>>,
    tenant_detectors: BTreeMap<String, Arc<dyn InboundTenantDetector>>,
    inbound_transformers: BTreeMap<String, Arc<dyn InboundEventTransformer>>,
    key_detectors: BTreeMap<String, Arc<dyn InboundEventKeyDetector>>,
    consumers: BTreeMap<String, Arc<dyn InboundEventConsumer>>,
    outbound_transformers: BTreeMap<String, Arc<dyn OutboundEventTransformer>>,
    inbound_adapters: BTreeMap<String, Arc<dyn InboundChannelAdapter>>,
    outbound_adapters: BTreeMap<String, Arc<dyn OutboundChannelAdapter>>,
    fallback_to_default_tenant: bool,
    default_tenant: String,
    outbound_ssrf_guard: OutboundUrlGuardConfig,
}

impl EventRegistryConfigurationBuilder {
    pub fn with_defaults(mut self) -> Self {
        self.payload_extractors
            .insert("json".to_string(), Arc::new(JsonPayloadExtractor));
        self.filters
            .insert("default".to_string(), Arc::new(DefaultInboundFilter));
        self.tenant_detectors
            .insert("default".to_string(), Arc::new(DefaultTenantDetector));
        self.inbound_transformers
            .insert("default".to_string(), Arc::new(DefaultInboundTransformer));
        self.key_detectors
            .insert("default".to_string(), Arc::new(DefaultEventKeyDetector));
        self.consumers
            .insert("default".to_string(), Arc::new(NoOpInboundConsumer));
        self.outbound_transformers
            .insert("json".to_string(), Arc::new(JsonOutboundTransformer));
        self.inbound_adapters
            .insert("in-memory".to_string(), Arc::new(InMemoryInboundAdapter));
        self.outbound_adapters
            .insert("in-memory".to_string(), Arc::new(InMemoryOutboundAdapter));
        // `rest` adapter is installed in `build()` so it picks up `outbound_ssrf_guard`.
        // AbstractEngineConfiguration.java:324/329 defaults.
        self.fallback_to_default_tenant = false;
        self.default_tenant = NO_TENANT_ID.to_string();
        self.outbound_ssrf_guard = OutboundUrlGuardConfig::default();
        self
    }

    /// Java `setFallbackToDefaultTenant`.
    pub fn fallback_to_default_tenant(mut self, enabled: bool) -> Self {
        self.fallback_to_default_tenant = enabled;
        self
    }

    /// Java `setDefaultTenantValue` — empty string is `NO_TENANT_ID`.
    pub fn default_tenant(mut self, default_tenant: impl Into<String>) -> Self {
        self.default_tenant = default_tenant.into();
        self
    }

    /// SSRF guard for the built-in REST outbound adapter (security deviation from Java).
    /// Default denies private/loopback/link-local destinations; set
    /// `allow_private_networks` or `allowed_private_hosts` for internal deployments.
    pub fn outbound_ssrf_guard(mut self, config: OutboundUrlGuardConfig) -> Self {
        self.outbound_ssrf_guard = config.clone();
        self.outbound_adapters.insert(
            "rest".to_string(),
            Arc::new(RestOutboundAdapter::with_ssrf_guard(config)),
        );
        self
    }

    pub fn payload_extractor(
        mut self,
        name: impl Into<String>,
        extractor: Arc<dyn InboundPayloadExtractor>,
    ) -> Self {
        self.payload_extractors.insert(name.into(), extractor);
        self
    }

    pub fn filter(
        mut self,
        name: impl Into<String>,
        filter: Arc<dyn InboundEventFilter>,
    ) -> Self {
        self.filters.insert(name.into(), filter);
        self
    }

    pub fn tenant_detector(
        mut self,
        name: impl Into<String>,
        detector: Arc<dyn InboundTenantDetector>,
    ) -> Self {
        self.tenant_detectors.insert(name.into(), detector);
        self
    }

    pub fn inbound_transformer(
        mut self,
        name: impl Into<String>,
        transformer: Arc<dyn InboundEventTransformer>,
    ) -> Self {
        self.inbound_transformers.insert(name.into(), transformer);
        self
    }

    pub fn key_detector(
        mut self,
        name: impl Into<String>,
        detector: Arc<dyn InboundEventKeyDetector>,
    ) -> Self {
        self.key_detectors.insert(name.into(), detector);
        self
    }

    pub fn consumer(
        mut self,
        name: impl Into<String>,
        consumer: Arc<dyn InboundEventConsumer>,
    ) -> Self {
        self.consumers.insert(name.into(), consumer);
        self
    }

    pub fn outbound_transformer(
        mut self,
        name: impl Into<String>,
        transformer: Arc<dyn OutboundEventTransformer>,
    ) -> Self {
        self.outbound_transformers.insert(name.into(), transformer);
        self
    }

    pub fn inbound_adapter(
        mut self,
        name: impl Into<String>,
        adapter: Arc<dyn InboundChannelAdapter>,
    ) -> Self {
        self.inbound_adapters.insert(name.into(), adapter);
        self
    }

    pub fn outbound_adapter(
        mut self,
        name: impl Into<String>,
        adapter: Arc<dyn OutboundChannelAdapter>,
    ) -> Self {
        self.outbound_adapters.insert(name.into(), adapter);
        self
    }

    pub fn build(self) -> EventRegistryConfiguration {
        let mut outbound_adapters = self.outbound_adapters;
        // Install / refresh the built-in REST adapter with the configured SSRF policy
        // unless a custom adapter already replaced the `rest` key after with_defaults.
        // Callers that pass `.outbound_adapter("rest", ...)` after with_defaults keep
        // their adapter; we only insert when the key is absent.
        outbound_adapters
            .entry("rest".to_string())
            .or_insert_with(|| {
                Arc::new(RestOutboundAdapter::with_ssrf_guard(
                    self.outbound_ssrf_guard.clone(),
                ))
            });
        EventRegistryConfiguration {
            payload_extractors: self.payload_extractors,
            filters: self.filters,
            tenant_detectors: self.tenant_detectors,
            inbound_transformers: self.inbound_transformers,
            key_detectors: self.key_detectors,
            consumers: self.consumers,
            outbound_transformers: self.outbound_transformers,
            inbound_adapters: self.inbound_adapters,
            outbound_adapters,
            fallback_to_default_tenant: self.fallback_to_default_tenant,
            default_tenant: self.default_tenant,
            outbound_ssrf_guard: self.outbound_ssrf_guard,
        }
    }
}

fn first_string(configuration: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        configuration
            .get(*field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn validate_optional_named(
    channel_key: &str,
    kind: &str,
    name: Option<String>,
    allowed: &[String],
) -> Result<(), FlowableError> {
    if let Some(name) = name {
        require_registered(channel_key, kind, &name, allowed)?;
    }
    Ok(())
}

fn require_registered(
    channel_key: &str,
    kind: &str,
    name: &str,
    allowed: &[String],
) -> Result<(), FlowableError> {
    if allowed.iter().any(|candidate| candidate == name) {
        return Ok(());
    }
    Err(FlowableError::DeploymentValidationError(format!(
        "Unknown {} '{}' for channel '{}'. Allowed: {}",
        kind,
        name,
        channel_key,
        join_names(allowed)
    )))
}

fn join_names(names: &[String]) -> String {
    if names.is_empty() {
        "<none>".to_string()
    } else {
        names.join(", ")
    }
}

pub struct EventPayloadValidator;

impl EventPayloadValidator {
    pub fn validate(
        &self,
        event_definition: &EventDefinition,
        payload: &Value,
    ) -> Result<(), ValidationError> {
        let payload_fields = event_definition
            .payload
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let name = item.get("name")?.as_str()?;
                        let field_type = item.get("type")?.as_str()?;
                        let required = item
                            .get("required")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        Some((name.to_string(), field_type.to_string(), required))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let payload_obj = payload.as_object().ok_or_else(|| ValidationError {
            field: None,
            message: "Event payload must be a JSON object".to_string(),
        })?;

        for (name, field_type, required) in &payload_fields {
            if *required && !payload_obj.contains_key(name.as_str()) {
                return Err(ValidationError {
                    field: Some(name.clone()),
                    message: format!("Required field '{}' is missing", name),
                });
            }

            if let Some(value) = payload_obj.get(name.as_str())
                && !validate_value_type(value, field_type)
            {
                return Err(ValidationError {
                    field: Some(name.clone()),
                    message: format!(
                        "Field '{}' has incorrect type, expected '{}'",
                        name, field_type
                    ),
                });
            }
        }

        Ok(())
    }
}

fn validate_value_type(value: &Value, expected_type: &str) -> bool {
    match expected_type {
        "string" => value.is_string(),
        "double" | "float" => value.is_f64() || value.is_i64() || value.is_u64(),
        "boolean" | "bool" => value.is_boolean(),
        "integer" | "int" => value.is_i64() || value.is_u64(),
        "long" => value.is_i64() || value.is_u64(),
        "date" => value.is_string(),
        _ => true,
    }
}

pub(crate) fn validate_event_payload(
    event_definition: &EventDefinition,
    payload: &Value,
) -> Result<(), FlowableError> {
    EventPayloadValidator
        .validate(event_definition, payload)
        .map_err(|error| {
            FlowableError::ExecutionError(format!(
                "Event payload for definition '{}' is invalid: {}",
                event_definition.key, error
            ))
        })
}

pub(crate) fn dispatch_outbound_event(
    configuration: &EventRegistryConfiguration,
    channel: &crate::models::ChannelDefinition,
    event: EventPayload,
) -> Result<(), FlowableError> {
    use crate::adapter::rest_destination_from_config;

    let adapter = configuration.resolve_outbound_adapter(&channel.key, &channel.configuration)?;
    let transformed = configuration
        .resolve_outbound_transformer(&channel.configuration)?
        .transform(&event.payload, &channel.configuration, &event.event_type)?;
    let destination = rest_destination_from_config(&channel.configuration);
    adapter.send(
        destination,
        EventPayload {
            event_type: event.event_type,
            payload: transformed,
            dispatch_token: event.dispatch_token,
        },
        &channel.configuration,
    )
}
