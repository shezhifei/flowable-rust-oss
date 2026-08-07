//! P1 tenant-isolation contract tests for channel resolution: raw inbound
//! channel addressing and legacy retry deliveries must never resolve a
//! channel that belongs to a foreign tenant. Allowed resolution order when
//! `fallbackToDefaultTenant` is enabled is exact tenant → tenantless default;
//! an any-tenant (foreign tenant) fallback is always forbidden.

mod test_support;

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{
    EventDefinition, EventInstanceDelivery, EventRegistryConfiguration,
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, FlowableEventRegistryService,
    InboundEventConsumer, InboundEventContext, InboundEventKeyDetector, InboundRawEvent,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FixedKeyDetector {
    key: String,
}

impl InboundEventKeyDetector for FixedKeyDetector {
    fn detect_event_key(
        &self,
        _context: &InboundEventContext,
        _channel_config: &Value,
    ) -> Result<String, FlowableError> {
        Ok(self.key.clone())
    }
}

struct CountingConsumer {
    invocations: AtomicUsize,
    fail_times: AtomicUsize,
}

impl InboundEventConsumer for CountingConsumer {
    fn consume(
        &self,
        _delivery: &EventInstanceDelivery,
        _definition: &EventDefinition,
    ) -> Result<(), FlowableError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        if self.fail_times.load(Ordering::SeqCst) > 0 {
            self.fail_times.fetch_sub(1, Ordering::SeqCst);
            return Err(FlowableError::ExecutionError(
                "consumer failed".to_string(),
            ));
        }
        Ok(())
    }
}

fn inbound_fixture(
    name: &str,
    consumer_fail_times: usize,
) -> (
    Arc<ProcessEngine>,
    FlowableEventRegistryService,
    Arc<CountingConsumer>,
) {
    inbound_fixture_with_fallback(name, consumer_fail_times, false)
}

fn inbound_fixture_with_fallback(
    name: &str,
    consumer_fail_times: usize,
    fallback_to_default_tenant: bool,
) -> (
    Arc<ProcessEngine>,
    FlowableEventRegistryService,
    Arc<CountingConsumer>,
) {
    let consumer = Arc::new(CountingConsumer {
        invocations: AtomicUsize::new(0),
        fail_times: AtomicUsize::new(consumer_fail_times),
    });
    let mut config = EventRegistryConfiguration::builder()
        .fallback_to_default_tenant(fallback_to_default_tenant)
        .build();
    config.register_consumer("counting", Arc::clone(&consumer) as Arc<_>);
    config.register_key_detector(
        "fixed",
        Arc::new(FixedKeyDetector {
            key: "tenantScopedOrder".to_string(),
        }),
    );
    let engine = Arc::new(ProcessEngine::new(name.to_string()));
    let service = FlowableEventRegistryService::with_configuration(Arc::clone(&engine), config);
    (engine, service, consumer)
}

fn deploy_inbound_for_tenant(
    service: &FlowableEventRegistryService,
    tenant_id: Option<&str>,
    resource_prefix: &str,
) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("deploy-{resource_prefix}"),
            category: None,
            parent_deployment_id: None,
            tenant_id: tenant_id.map(str::to_string),
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{resource_prefix}-order.event"),
                    resource: json!({
                        "key": "tenantScopedOrder",
                        "name": "Tenant scoped order",
                        "eventType": "tenant.scoped.order",
                        "channelKey": "tenantScopedInbound",
                        "resourceName": format!("{resource_prefix}-order.event"),
                        "payload": []
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{resource_prefix}-inbound.channel"),
                    resource: json!({
                        "key": "tenantScopedInbound",
                        "name": "Tenant scoped inbound",
                        "channelType": "inbound",
                        "resourceName": format!("{resource_prefix}-inbound.channel"),
                        "type": "in-memory",
                        "keyDetector": "fixed",
                        "consumer": "counting"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
}

fn raw_event(tenant_hint: Option<&str>) -> InboundRawEvent {
    InboundRawEvent {
        channel_key: "tenantScopedInbound".to_string(),
        body: json!({ "orderId": "T-1" }),
        headers: BTreeMap::new(),
        tenant_hint: tenant_hint.map(str::to_string),
    }
}

#[test]
fn raw_inbound_addressing_rejects_foreign_tenant_channel() {
    let (_engine, service, consumer) =
        inbound_fixture("channel-tenant-isolation-foreign", 0);
    // The channel exists only under tenant-a; there is no tenantless default.
    deploy_inbound_for_tenant(&service, Some("tenant-a"), "tenant-a");

    // Exact tenant resolves and processes normally.
    let delivery = service
        .process_inbound_channel_event(raw_event(Some("tenant-a")))
        .unwrap();
    assert_eq!(delivery.channel_key, "tenantScopedInbound");
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 1);

    // A tenant-b request must not fall back to the tenant-a-only channel.
    let error = service
        .process_inbound_channel_event(raw_event(Some("tenant-b")))
        .unwrap_err();
    assert!(
        matches!(error, FlowableError::NotFound(ref message)
            if message.contains("tenantScopedInbound") && message.contains("tenant-b")),
        "expected NotFound for foreign tenant, got: {error:?}"
    );

    // A tenantless request may only use tenantless channels.
    let error = service
        .process_inbound_channel_event(raw_event(None))
        .unwrap_err();
    assert!(
        matches!(error, FlowableError::NotFound(_)),
        "expected NotFound for tenantless request, got: {error:?}"
    );

    // Neither rejected request may have reached the consumer.
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 1);
}

#[test]
fn raw_inbound_addressing_falls_back_to_tenantless_default_channel() {
    // Java GetChannelModelCmd.java:84-90 — fallback only when switch is on.
    let (_engine, service, consumer) =
        inbound_fixture_with_fallback("channel-tenant-isolation-default", 0, true);
    // Only the tenantless default exists.
    deploy_inbound_for_tenant(&service, None, "global");

    // A tenant-scoped request falls back to the tenantless default channel.
    let delivery = service
        .process_inbound_channel_event(raw_event(Some("tenant-b")))
        .unwrap();
    assert_eq!(delivery.channel_key, "tenantScopedInbound");
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 1);
}

#[test]
fn legacy_retry_resolves_strictly_by_delivery_tenant() {
    let (engine, service, consumer) =
        inbound_fixture("channel-tenant-isolation-legacy-retry", 2);
    // The channel exists only under tenant-a.
    deploy_inbound_for_tenant(&service, Some("tenant-a"), "tenant-a");

    // Two failed deliveries (consumer fails twice, then succeeds).
    service
        .process_inbound_channel_event(raw_event(Some("tenant-a")))
        .unwrap_err();
    service
        .process_inbound_channel_event(raw_event(Some("tenant-a")))
        .unwrap_err();
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 2);

    let failed = service
        .create_event_instance_delivery_query()
        .list_page()
        .unwrap()
        .data;
    assert_eq!(failed.len(), 2);

    // Strip the recorded channel definition ids to simulate legacy deliveries.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut same_tenant = store
        .find_event_registry_event_instance_delivery(&failed[0].id, &mut session)
        .unwrap()
        .unwrap();
    same_tenant.channel_definition_id = None;
    same_tenant.tenant_id = Some("tenant-a".to_string());
    store
        .update_event_registry_event_instance_delivery(same_tenant, &mut session)
        .unwrap();
    let mut foreign_tenant = store
        .find_event_registry_event_instance_delivery(&failed[1].id, &mut session)
        .unwrap()
        .unwrap();
    foreign_tenant.channel_definition_id = None;
    foreign_tenant.tenant_id = Some("tenant-b".to_string());
    store
        .update_event_registry_event_instance_delivery(foreign_tenant, &mut session)
        .unwrap();
    session.flush_and_commit().unwrap();

    // Legacy delivery in the channel's own tenant retries fine.
    let retried = service.retry_event_delivery(&failed[0].id).unwrap();
    assert_eq!(retried.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 3);

    // Legacy delivery in tenant-b must not borrow the tenant-a channel: the
    // retry fails with an explicit compatibility error before any dispatch.
    let error = service.retry_event_delivery(&failed[1].id).unwrap_err();
    assert!(
        matches!(error, FlowableError::NotFound(ref message)
            if message.contains("cannot be retried") && message.contains("tenant-b")),
        "expected strict-tenant compatibility error, got: {error:?}"
    );
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 3);
}
