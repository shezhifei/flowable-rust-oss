//! P2-10 contract tests: the delivery retry state machine only accepts legal
//! (direction, status) combinations, inbound retries re-run the consumer, and
//! outbound retries replay against the original channel definition version.

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
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct RecordingAdapter {
    destinations: Mutex<Vec<String>>,
    fail_times: AtomicUsize,
}

impl OutboundChannelAdapter for RecordingAdapter {
    fn send(
        &self,
        destination: Option<&str>,
        _event: EventPayload,
        _channel_config: &Value,
    ) -> Result<(), FlowableError> {
        self.destinations
            .lock()
            .unwrap()
            .push(destination.unwrap_or("<none>").to_string());
        if self.fail_times.load(Ordering::SeqCst) > 0 {
            self.fail_times.fetch_sub(1, Ordering::SeqCst);
            return Err(FlowableError::ExecutionError(
                "adapter temporary failure".to_string(),
            ));
        }
        Ok(())
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

fn outbound_service(
    name: &str,
    adapter_fail_times: usize,
) -> (FlowableEventRegistryService, Arc<RecordingAdapter>) {
    let adapter = Arc::new(RecordingAdapter {
        destinations: Mutex::new(Vec::new()),
        fail_times: AtomicUsize::new(adapter_fail_times),
    });
    let mut config = EventRegistryConfiguration::default();
    config.register_outbound_adapter("in-memory", Arc::clone(&adapter) as Arc<_>);
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new(name.to_string())),
        config,
    );
    (service, adapter)
}

fn deploy_outbound_with_destination(
    service: &FlowableEventRegistryService,
    event_key: &str,
    channel_key: &str,
    destination: &str,
) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("deploy-{event_key}-{destination}"),
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
                        "destination": destination
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
}

fn inbound_service(
    name: &str,
    consumer_fail_times: usize,
) -> (FlowableEventRegistryService, Arc<CountingConsumer>) {
    let consumer = Arc::new(CountingConsumer {
        invocations: AtomicUsize::new(0),
        fail_times: AtomicUsize::new(consumer_fail_times),
    });
    let mut config = EventRegistryConfiguration::default();
    config.register_consumer("counting", Arc::clone(&consumer) as Arc<_>);
    config.register_key_detector(
        "fixed",
        Arc::new(FixedKeyDetector {
            key: "orderReceived".to_string(),
        }),
    );
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new(name.to_string())),
        config,
    );
    (service, consumer)
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
                        "consumer": "counting"
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
fn retry_published_outbound_is_rejected_without_redispatch() {
    let (service, adapter) = outbound_service("retry-published-outbound", 0);
    deploy_outbound_with_destination(&service, "orderPublished", "ordersOut", "dest-v1");

    let delivery = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "A-1" }),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Published);
    assert_eq!(adapter.destinations.lock().unwrap().len(), 1);

    let error = service.retry_event_delivery(&delivery.id).unwrap_err();
    assert!(
        matches!(error, FlowableError::Conflict(ref message) if message.contains("not retryable")),
        "expected Conflict, got: {error:?}"
    );

    // No re-dispatch and no state mutation happened.
    assert_eq!(adapter.destinations.lock().unwrap().len(), 1);
    let unchanged = service.get_event_instance_delivery(&delivery.id).unwrap();
    assert_eq!(unchanged.status, EventInstanceStatus::Published);
    assert_eq!(unchanged.status_history, delivery.status_history);
}

#[test]
fn retry_processed_inbound_is_rejected_without_state_change() {
    let (service, consumer) = inbound_service("retry-processed-inbound", 0);
    deploy_inbound(&service, "ordersInbound", "orderReceived");

    let delivery = service
        .process_inbound_channel_event(inbound_event("ordersInbound"))
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 1);

    let error = service.retry_event_delivery(&delivery.id).unwrap_err();
    assert!(
        matches!(error, FlowableError::Conflict(ref message) if message.contains("not retryable")),
        "expected Conflict, got: {error:?}"
    );

    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 1);
    let unchanged = service.get_event_instance_delivery(&delivery.id).unwrap();
    assert_eq!(unchanged.status, EventInstanceStatus::Processed);
    assert_eq!(unchanged.status_history, delivery.status_history);
}

#[test]
fn retry_failed_inbound_reruns_consumer_and_marks_processed() {
    let (service, consumer) = inbound_service("retry-failed-inbound", 1);
    deploy_inbound(&service, "ordersInbound", "orderReceived");

    let error = service
        .process_inbound_channel_event(inbound_event("ordersInbound"))
        .unwrap_err();
    assert!(error.to_string().contains("consumer failed"));
    let failed = service
        .create_event_instance_delivery_query()
        .list_page()
        .unwrap()
        .data
        .pop()
        .unwrap();
    assert_eq!(failed.status, EventInstanceStatus::Failed);

    let retried = service.retry_event_delivery(&failed.id).unwrap();
    assert_eq!(retried.status, EventInstanceStatus::Processed);
    assert_eq!(
        retried.status_history,
        vec![
            EventInstanceStatus::Received,
            EventInstanceStatus::Failed,
            EventInstanceStatus::Processed
        ]
    );
    assert_eq!(retried.retry_count, 1);
    assert_eq!(retried.last_error, None);
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 2);
}

#[test]
fn retry_failed_inbound_consumer_failure_keeps_failed_and_records_retry() {
    let (service, consumer) = inbound_service("retry-failed-inbound-again", 2);
    deploy_inbound(&service, "ordersInbound", "orderReceived");

    service
        .process_inbound_channel_event(inbound_event("ordersInbound"))
        .unwrap_err();
    let failed = service
        .create_event_instance_delivery_query()
        .list_page()
        .unwrap()
        .data
        .pop()
        .unwrap();

    let error = service.retry_event_delivery(&failed.id).unwrap_err();
    assert!(error.to_string().contains("consumer failed"));
    assert_eq!(consumer.invocations.load(Ordering::SeqCst), 2);

    let still_failed = service.get_event_instance_delivery(&failed.id).unwrap();
    assert_eq!(still_failed.status, EventInstanceStatus::Failed);
    assert_eq!(still_failed.retry_count, 1);
    assert!(still_failed.last_error.as_ref().unwrap().contains("consumer failed"));
}

#[test]
fn outbound_retry_replays_against_original_channel_version() {
    let (service, adapter) = outbound_service("retry-original-channel", 1);
    deploy_outbound_with_destination(&service, "orderPublished", "ordersOut", "dest-v1");

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
    let original_channel_id = failed.channel_definition_id.clone().unwrap();

    // Redeploy the channel with a different destination; the retry must not
    // pick up the new version.
    deploy_outbound_with_destination(&service, "orderPublished", "ordersOut", "dest-v2");
    let latest_channel = service
        .create_channel_definition_query()
        .key("ordersOut")
        .latest()
        .list()
        .unwrap()
        .pop()
        .unwrap();
    assert_ne!(latest_channel.id, original_channel_id);

    let retried = service.retry_event_delivery(&failed.id).unwrap();
    assert_eq!(retried.status, EventInstanceStatus::Published);
    assert_eq!(
        retried.channel_definition_id.as_deref(),
        Some(original_channel_id.as_str())
    );
    assert_eq!(
        adapter.destinations.lock().unwrap().as_slice(),
        &["dest-v1", "dest-v1"],
        "retry must dispatch through the original channel version"
    );
}
