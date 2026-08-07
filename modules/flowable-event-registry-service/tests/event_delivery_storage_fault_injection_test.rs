//! P2-12: storage fault-injection for the Event Registry delivery pipelines.
//! Session/commit failures must surface as `FlowableError` instead of
//! panicking, failure-path persistence failures must combine both errors, and
//! a Published-commit failure after successful external I/O must be reported
//! as explicit at-least-once with the adapter-visible dispatch token.

mod test_support;

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{
    EventDefinition, EventInstanceDelivery, EventInstanceStatus, EventPayload,
    EventRegistryConfiguration, EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
    FlowableEventRegistryService, InboundEventConsumer, InboundEventContext,
    InboundEventKeyDetector, InboundRawEvent, OutboundChannelAdapter, OutboundEventRequest,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn drop_deliveries_table(engine: &ProcessEngine) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    session
        .execute_raw_sql("DROP TABLE event_registry_event_instance_deliveries")
        .unwrap();
    session.flush_and_commit().unwrap();
}

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

/// Consumer that counts invocations and, when armed, drops the delivery table
/// mid-consume (so the subsequent Failed/Processed persist fails) before
/// optionally failing itself.
struct SabotagingConsumer {
    engine: Arc<ProcessEngine>,
    invocations: AtomicUsize,
    drop_table: bool,
    fail: bool,
}

impl InboundEventConsumer for SabotagingConsumer {
    fn consume(
        &self,
        _delivery: &EventInstanceDelivery,
        _definition: &EventDefinition,
    ) -> Result<(), FlowableError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        if self.drop_table {
            drop_deliveries_table(&self.engine);
        }
        if self.fail {
            return Err(FlowableError::ExecutionError(
                "consumer failed".to_string(),
            ));
        }
        Ok(())
    }
}

/// Adapter that records each dispatch's token and, when armed, drops the
/// delivery table (so the Published persist fails) or fails the send itself.
struct SabotagingAdapter {
    engine: Arc<ProcessEngine>,
    tokens: Mutex<Vec<Option<String>>>,
    drop_table: bool,
    fail_times: AtomicUsize,
}

impl OutboundChannelAdapter for SabotagingAdapter {
    fn send(
        &self,
        _destination: Option<&str>,
        event: EventPayload,
        _channel_config: &Value,
    ) -> Result<(), FlowableError> {
        self.tokens.lock().unwrap().push(event.dispatch_token);
        if self.fail_times.load(Ordering::SeqCst) > 0 {
            self.fail_times.fetch_sub(1, Ordering::SeqCst);
            return Err(FlowableError::ExecutionError(
                "adapter temporary failure".to_string(),
            ));
        }
        if self.drop_table {
            drop_deliveries_table(&self.engine);
        }
        Ok(())
    }
}

fn inbound_service(
    name: &str,
    drop_table_in_consumer: bool,
    consumer_fails: bool,
) -> (FlowableEventRegistryService, Arc<SabotagingConsumer>) {
    let engine = Arc::new(ProcessEngine::new(name.to_string()));
    let consumer = Arc::new(SabotagingConsumer {
        engine: Arc::clone(&engine),
        invocations: AtomicUsize::new(0),
        drop_table: drop_table_in_consumer,
        fail: consumer_fails,
    });
    let mut config = EventRegistryConfiguration::default();
    config.register_consumer("sabotaging", Arc::clone(&consumer) as Arc<_>);
    config.register_key_detector(
        "fixed",
        Arc::new(FixedKeyDetector {
            key: "orderReceived".to_string(),
        }),
    );
    let service = FlowableEventRegistryService::with_configuration(engine, config);
    (service, consumer)
}

fn outbound_service(
    name: &str,
    drop_table_in_adapter: bool,
    adapter_fail_times: usize,
) -> (FlowableEventRegistryService, Arc<SabotagingAdapter>) {
    let engine = Arc::new(ProcessEngine::new(name.to_string()));
    let adapter = Arc::new(SabotagingAdapter {
        engine: Arc::clone(&engine),
        tokens: Mutex::new(Vec::new()),
        drop_table: drop_table_in_adapter,
        fail_times: AtomicUsize::new(adapter_fail_times),
    });
    let mut config = EventRegistryConfiguration::default();
    config.register_outbound_adapter("in-memory", Arc::clone(&adapter) as Arc<_>);
    let service = FlowableEventRegistryService::with_configuration(engine, config);
    (service, adapter)
}

fn deploy_inbound(service: &FlowableEventRegistryService, channel_key: &str, event_key: &str) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("deploy-{channel_key}"),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{event_key}.event"),
                    resource: json!({
                        "key": event_key,
                        "name": event_key,
                        "eventType": format!("{event_key}.type"),
                        "channelKey": channel_key,
                        "resourceName": format!("{event_key}.event"),
                        "payload": []
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{channel_key}.channel"),
                    resource: json!({
                        "key": channel_key,
                        "name": channel_key,
                        "channelType": "inbound",
                        "resourceName": format!("{channel_key}.channel"),
                        "type": "in-memory",
                        "keyDetector": "fixed",
                        "consumer": "sabotaging"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
}

fn deploy_outbound(service: &FlowableEventRegistryService, event_key: &str, channel_key: &str) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("deploy-{event_key}"),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{event_key}.event"),
                    resource: json!({
                        "key": event_key,
                        "name": event_key,
                        "eventType": format!("{event_key}.type"),
                        "channelKey": channel_key,
                        "resourceName": format!("{event_key}.event"),
                        "payload": [
                            { "name": "orderId", "type": "string", "required": true }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{channel_key}.channel"),
                    resource: json!({
                        "key": channel_key,
                        "name": channel_key,
                        "channelType": "outbound",
                        "resourceName": format!("{channel_key}.channel"),
                        "type": "in-memory",
                        "destination": "dest-1"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
}

fn inbound_event(channel_key: &str) -> InboundRawEvent {
    InboundRawEvent {
        channel_key: channel_key.to_string(),
        body: json!({ "orderId": "A-1" }),
        headers: BTreeMap::new(),
        tenant_hint: None,
    }
}

#[test]
fn received_persist_failure_returns_error_before_consumer_runs() {
    let (service, consumer) = inbound_service("fault-received-persist", false, false);
    deploy_inbound(&service, "ordersInbound", "orderReceived");
    drop_deliveries_table(&consumer.engine);

    let error = service
        .process_inbound_channel_event(inbound_event("ordersInbound"))
        .unwrap_err();
    assert!(
        matches!(error, FlowableError::Internal(ref message) if message.contains("no such table")),
        "Received persist failure must map to FlowableError::Internal, got: {error:?}"
    );
    assert_eq!(
        consumer.invocations.load(Ordering::SeqCst),
        0,
        "consumer must not run when the Received status cannot be persisted"
    );
}

#[test]
fn consumer_and_failed_persist_failures_are_combined_in_one_error() {
    let (service, consumer) = inbound_service("fault-failed-persist", true, true);
    deploy_inbound(&service, "ordersInbound", "orderReceived");

    let error = service
        .process_inbound_channel_event(inbound_event("ordersInbound"))
        .unwrap_err();
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 1);
    let message = error.to_string();
    assert!(
        message.contains("consumer failed"),
        "combined error must keep the original consumer failure, got: {message}"
    );
    assert!(
        message.contains("persisting the Failed status also failed"),
        "combined error must report the persistence failure, got: {message}"
    );
    assert!(
        message.contains("no such table"),
        "combined error must include the underlying storage error, got: {message}"
    );
}

#[test]
fn published_persist_failure_after_dispatch_reports_at_least_once() {
    let (service, adapter) = outbound_service("fault-published-persist", true, 0);
    deploy_outbound(&service, "orderPublished", "ordersOut");

    let error = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "A-1" }),
            tenant_id: None,
        })
        .unwrap_err();

    // The external system received the event exactly once before the failure.
    let tokens = adapter.tokens.lock().unwrap().clone();
    assert_eq!(tokens.len(), 1);
    let dispatched_token = tokens[0].clone().expect("adapter must see the dispatch token");

    let message = error.to_string();
    assert!(
        message.contains("persisting the Published status failed"),
        "error must name the failed Published transition, got: {message}"
    );
    assert!(
        message.contains("at-least-once"),
        "error must make the at-least-once semantics explicit, got: {message}"
    );
    assert!(
        message.contains(&dispatched_token),
        "error must reference the adapter-visible dispatch token, got: {message}"
    );
}

#[test]
fn dispatch_token_is_adapter_visible_and_stable_across_retry() {
    let (service, adapter) = outbound_service("fault-token-stable", false, 1);
    deploy_outbound(&service, "orderPublished", "ordersOut");

    service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "retry-me" }),
            tenant_id: None,
        })
        .unwrap_err();
    let failed = service
        .create_event_instance_delivery_query()
        .list_page()
        .unwrap()
        .data
        .pop()
        .unwrap();
    assert_eq!(failed.status, EventInstanceStatus::Failed);

    let retried = service.retry_event_delivery(&failed.id).unwrap();
    assert_eq!(retried.status, EventInstanceStatus::Published);

    let tokens = adapter.tokens.lock().unwrap().clone();
    assert_eq!(tokens.len(), 2, "one original dispatch and one retry dispatch");
    assert!(tokens[0].is_some(), "dispatch token must be adapter-visible");
    assert_eq!(
        tokens[0], tokens[1],
        "retry must replay the original idempotency token"
    );
    assert_eq!(
        tokens[0], retried.dispatch_token,
        "adapter-visible token must match the persisted delivery token"
    );
}
