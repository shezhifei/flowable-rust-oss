//! Flowable Event Registry service.
//!
//! Layered as models, deployment, query, runtime (including delivery), adapter,
//! pipeline, and change detection. Public API is re-exported from this crate root.

mod adapter;
mod bpmn_consumer;
mod cache;
mod change_detection;
mod cmmn_consumer;
mod deployment;
mod models;
mod outbound_engine_bridge;
mod pipeline;
mod query;
mod resolver;
mod runtime;
mod ssrf_guard;
mod tenant_fallback;

pub use adapter::{
    boxed_outbound_adapter, InMemoryInboundAdapter, InMemoryOutboundAdapter, InboundChannelAdapter,
    OutboundChannelAdapter, RestChannelAdapter, RestOutboundAdapter,
};
pub use ssrf_guard::{
    safe_url_display, safe_url_for_error, validate_outbound_url, OutboundUrlGuardConfig,
    OutboundUrlGuardError,
};
pub use bpmn_consumer::{BpmnEventRegistryConsumer, BPMN_EVENT_CONSUMER_KEY};
pub use cmmn_consumer::{CmmnEventRegistryConsumer, CMMN_EVENT_CONSUMER_KEY};
pub use cache::DefinitionCache;
pub use change_detection::{ChangeDetectionResult, DEFAULT_CHANGE_POLL_LIMIT};
pub use models::{
    ChannelDefinition, ChannelDefinitionUpdateRequest, EventDefinition,
    EventDefinitionUpdateRequest, EventDeliveryRetry, EventDirection, EventInstanceDelivery,
    EventInstanceRequest, EventInstanceStatus, EventPayload, EventRegistryDeployment,
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, EventRegistryEngineInfo,
    EventRegistryError, EventRegistryResourceData, EventRetryPolicy, InboundEventRequest,
    OutboundEventRequest, PagedResult, ValidationError,
};
pub use pipeline::{
    DefaultEventKeyDetector, DefaultInboundFilter, DefaultInboundTransformer,
    DefaultTenantDetector, EventPayloadValidator, EventRegistryConfiguration,
    EventRegistryConfigurationBuilder, InboundEventConsumer, InboundEventContext,
    InboundEventFilter, InboundEventKeyDetector, InboundEventTransformer, InboundPayloadExtractor,
    InboundRawEvent, InboundTenantDetector, JsonOutboundTransformer, JsonPayloadExtractor,
    NoOpInboundConsumer, OutboundEventTransformer,
};
pub use query::{
    ChannelDefinitionQuery, EventDefinitionQuery, EventInstanceDeliveryQuery,
    EventRegistryDeploymentQuery,
};
pub use outbound_engine_bridge::ConfigurationBackedOutboundEventDispatch;
pub use tenant_fallback::{
    dedup_definition_level_subscriptions_by_key, resolve_definition_with_fallback,
    subscription_matches_event_tenant, TenantFallbackPolicy, NO_TENANT_ID,
};

use flowable_cmmn_engine::CmmnEngine;
use flowable_engine::engine::process_engine::ProcessEngine;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct FlowableEventRegistryService {
    pub(crate) engine: Arc<ProcessEngine>,
    pub(crate) configuration: Arc<EventRegistryConfiguration>,
    pub(crate) definition_cache: Arc<Mutex<DefinitionCache>>,
    pub(crate) last_change_revision: Arc<Mutex<u64>>,
}

impl FlowableEventRegistryService {
    /// Default configuration bound to a specific engine: mirrors the engine-level
    /// outbound HTTP SSRF escape hatches (`http_service.real_client.allow_private_networks`
    /// / `allowed_private_hosts`, P142b) into the event-registry REST outbound guard,
    /// so a deployment that explicitly opts into private endpoints for HTTP service
    /// tasks gets the same single outbound policy for event-registry REST channels.
    /// Both guards default to deny; explicit `with_configuration` callers are untouched.
    fn default_configuration_for_engine(engine: &ProcessEngine) -> EventRegistryConfiguration {
        let http_client = &engine.get_config().http_service.real_client;
        let mut builder = EventRegistryConfiguration::builder();
        if http_client.allow_private_networks || !http_client.allowed_private_hosts.is_empty() {
            // Goes through the builder so the built-in `rest` outbound adapter is
            // constructed with the mirrored guard (mutating the pub field after
            // `Default` would leave the already-built adapter at deny).
            builder = builder.outbound_ssrf_guard(OutboundUrlGuardConfig {
                allow_private_networks: http_client.allow_private_networks,
                allowed_private_hosts: http_client.allowed_private_hosts.clone(),
            });
        }
        builder.build()
    }

    /// Default construction keeps `NoOpInboundConsumer` as the `"default"` consumer
    /// so unit tests that only exercise the pipeline stay isolated from BPMN.
    /// Platform bootstrap should use [`Self::with_bpmn_consumer`].
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        let configuration = Self::default_configuration_for_engine(&engine);
        Self::with_configuration(engine, configuration)
    }

    /// Builds a service whose default + `bpmnEventConsumer` consumers are the
    /// BPMN bridge (`BpmnEventRegistryEventConsumer.java:62-64`).
    pub fn with_bpmn_consumer(engine: Arc<ProcessEngine>) -> Self {
        let configuration = Self::default_configuration_for_engine(&engine);
        Self::with_bpmn_consumer_config(engine, configuration)
    }

    /// Like [`Self::with_bpmn_consumer`] but applies the given configuration
    /// (including tenant-fallback policy) to both the service and the consumer.
    pub fn with_bpmn_consumer_config(
        engine: Arc<ProcessEngine>,
        mut configuration: EventRegistryConfiguration,
    ) -> Self {
        let policy = configuration.tenant_fallback_policy();
        let consumer: Arc<dyn InboundEventConsumer> = Arc::new(
            BpmnEventRegistryConsumer::with_tenant_fallback(Arc::clone(&engine), policy),
        );
        // Overlay both Java key and the channel-config default name.
        configuration.register_consumer(BPMN_EVENT_CONSUMER_KEY, Arc::clone(&consumer));
        configuration.register_consumer("default", consumer);
        Self::with_configuration(engine, configuration)
    }

    /// Builds a service whose default + `cmmnEventConsumer` consumers are the
    /// CMMN bridge (`CmmnEventRegistryEventConsumer.java:63-66`, registration
    /// `CmmnEngineConfiguration.java:1358-1365`).
    pub fn with_cmmn_consumer(engine: Arc<ProcessEngine>, cmmn_engine: Arc<CmmnEngine>) -> Self {
        let configuration = Self::default_configuration_for_engine(&engine);
        Self::with_cmmn_consumer_config(engine, cmmn_engine, configuration)
    }

    /// Like [`Self::with_cmmn_consumer`] but applies the given configuration
    /// (including tenant-fallback policy) to both the service and the consumer.
    pub fn with_cmmn_consumer_config(
        engine: Arc<ProcessEngine>,
        cmmn_engine: Arc<CmmnEngine>,
        mut configuration: EventRegistryConfiguration,
    ) -> Self {
        let policy = configuration.tenant_fallback_policy();
        let consumer: Arc<dyn InboundEventConsumer> = Arc::new(
            CmmnEventRegistryConsumer::with_tenant_fallback(cmmn_engine, policy),
        );
        configuration.register_consumer(CMMN_EVENT_CONSUMER_KEY, Arc::clone(&consumer));
        configuration.register_consumer("default", consumer);
        Self::with_configuration(engine, configuration)
    }

    pub fn with_configuration(
        engine: Arc<ProcessEngine>,
        configuration: EventRegistryConfiguration,
    ) -> Self {
        let configuration = Arc::new(configuration);
        // P94: install transform+adapter pipeline for BPMN send-event path.
        // Engine holds only the hook trait; this closes the cycle-safe injection.
        engine
            .get_config()
            .outbound_event_dispatch
            .install(Arc::new(ConfigurationBackedOutboundEventDispatch::new(
                Arc::clone(&configuration),
            )));
        Self {
            engine,
            configuration,
            definition_cache: Arc::new(Mutex::new(DefinitionCache::new())),
            last_change_revision: Arc::new(Mutex::new(0)),
        }
    }

    pub fn configuration(&self) -> &EventRegistryConfiguration {
        &self.configuration
    }

    /// High-water mark of the durable change log observed by this service instance.
    pub fn last_change_revision(&self) -> u64 {
        *self.last_change_revision.lock().unwrap()
    }

    /// Snapshot of cached latest channel definition for tenant+key, if present.
    pub fn cached_latest_channel(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> Option<ChannelDefinition> {
        self.definition_cache
            .lock()
            .unwrap()
            .latest_channel(key, tenant_id)
            .cloned()
    }

    /// Snapshot of cached latest event definition for tenant+key, if present.
    pub fn cached_latest_event(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> Option<EventDefinition> {
        self.definition_cache
            .lock()
            .unwrap()
            .latest_event(key, tenant_id)
            .cloned()
    }

    /// Poll the durable change log after the last observed revision and reconcile the cache.
    pub fn detect_and_reconcile_changes(
        &self,
    ) -> Result<ChangeDetectionResult, flowable_engine::error::FlowableError> {
        self.detect_and_reconcile_changes_with_limit(DEFAULT_CHANGE_POLL_LIMIT)
    }

    pub fn detect_and_reconcile_changes_with_limit(
        &self,
        limit: usize,
    ) -> Result<ChangeDetectionResult, flowable_engine::error::FlowableError> {
        let store = self.engine.get_runtime_store();
        let after = *self.last_change_revision.lock().unwrap();
        let mut cache = self.definition_cache.lock().unwrap();
        let result =
            change_detection::detect_and_reconcile_changes(&store, &mut cache, after, limit)?;
        *self.last_change_revision.lock().unwrap() = result.last_revision;
        Ok(result)
    }

    pub fn create_channel_definition_query(&self) -> ChannelDefinitionQuery {
        ChannelDefinitionQuery::new(Arc::clone(&self.engine))
    }

    pub fn create_event_definition_query(&self) -> EventDefinitionQuery {
        EventDefinitionQuery::new(Arc::clone(&self.engine))
    }

    pub fn create_deployment_query(&self) -> EventRegistryDeploymentQuery {
        EventRegistryDeploymentQuery::new(Arc::clone(&self.engine))
    }

    pub fn get_engine_info(&self) -> EventRegistryEngineInfo {
        EventRegistryEngineInfo {
            name: self.engine.get_name().to_string(),
            version: self.engine.get_version().to_string(),
            resource_url: None,
            exception: None,
        }
    }
}
