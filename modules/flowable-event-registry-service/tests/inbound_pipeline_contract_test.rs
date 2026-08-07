mod test_support;

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{
    EventDefinition, EventInstanceDelivery, EventInstanceStatus, EventRegistryConfiguration,
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, FlowableEventRegistryService,
    InboundEventConsumer, InboundEventContext, InboundEventFilter, InboundEventKeyDetector,
    InboundEventTransformer, InboundPayloadExtractor, InboundRawEvent, InboundTenantDetector,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct StageLog {
    stages: Mutex<Vec<String>>,
}

impl StageLog {
    fn push(&self, stage: &str) {
        self.stages.lock().unwrap().push(stage.to_string());
    }

    fn snapshot(&self) -> Vec<String> {
        self.stages.lock().unwrap().clone()
    }
}

struct LoggingExtractor {
    log: Arc<StageLog>,
}

impl InboundPayloadExtractor for LoggingExtractor {
    fn extract(
        &self,
        raw: &InboundRawEvent,
        _channel_config: &Value,
    ) -> Result<Value, FlowableError> {
        self.log.push("extraction");
        Ok(raw.body.clone())
    }
}

struct LoggingFilter {
    log: Arc<StageLog>,
    retain: bool,
}

impl InboundEventFilter for LoggingFilter {
    fn retain(
        &self,
        _context: &InboundEventContext,
        _channel_config: &Value,
    ) -> Result<bool, FlowableError> {
        self.log.push("filter");
        Ok(self.retain)
    }
}

struct LoggingTenantDetector {
    log: Arc<StageLog>,
    tenant: Option<String>,
}

impl InboundTenantDetector for LoggingTenantDetector {
    fn detect_tenant(
        &self,
        _context: &InboundEventContext,
        _channel_config: &Value,
    ) -> Result<Option<String>, FlowableError> {
        self.log.push("tenant");
        Ok(self.tenant.clone())
    }
}

struct LoggingTransformer {
    log: Arc<StageLog>,
    fail: bool,
}

impl InboundEventTransformer for LoggingTransformer {
    fn transform(
        &self,
        context: &InboundEventContext,
        _channel_config: &Value,
    ) -> Result<Value, FlowableError> {
        self.log.push("transform");
        if self.fail {
            return Err(FlowableError::ExecutionError("transform failed".to_string()));
        }
        Ok(context.payload.clone())
    }
}

struct LoggingKeyDetector {
    log: Arc<StageLog>,
    key: Option<String>,
}

impl InboundEventKeyDetector for LoggingKeyDetector {
    fn detect_event_key(
        &self,
        _context: &InboundEventContext,
        _channel_config: &Value,
    ) -> Result<String, FlowableError> {
        self.log.push("key_detection");
        self.key
            .clone()
            .ok_or_else(|| FlowableError::ExecutionError("missing event key".to_string()))
    }
}

struct LoggingConsumer {
    log: Arc<StageLog>,
    fail: bool,
    seen: Mutex<Vec<String>>,
}

impl InboundEventConsumer for LoggingConsumer {
    fn consume(
        &self,
        delivery: &EventInstanceDelivery,
        _definition: &EventDefinition,
    ) -> Result<(), FlowableError> {
        self.log.push("consumer");
        self.seen.lock().unwrap().push(delivery.id.clone());
        if self.fail {
            return Err(FlowableError::ExecutionError("consumer failed".to_string()));
        }
        Ok(())
    }
}

fn deploy_inbound_channel(
    service: &FlowableEventRegistryService,
    channel_key: &str,
    event_key: &str,
    event_type: &str,
    processors: Value,
    payload: Value,
) {
    let mut channel = json!({
        "key": channel_key,
        "name": channel_key,
        "channelType": "inbound",
        "resourceName": format!("{channel_key}.channel"),
        "type": "in-memory",
        "destination": channel_key,
        "payloadExtractor": "json",
        "filter": "default",
        "tenantDetector": "default",
        "transformer": "default",
        "keyDetector": "default",
        "consumer": "default",
    });
    if let (Some(channel_obj), Some(extra)) = (channel.as_object_mut(), processors.as_object()) {
        for (k, v) in extra {
            channel_obj.insert(k.clone(), v.clone());
        }
    }

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
                        "eventType": event_type,
                        "channelKey": channel_key,
                        "resourceName": format!("{event_key}.event"),
                        "payload": payload
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{channel_key}.channel"),
                    resource: channel.to_string(),
                },
            ],
        })
        .unwrap();
}

fn config_with_log(
    log: Arc<StageLog>,
    filter_retain: bool,
    transform_fail: bool,
    consumer_fail: bool,
    event_key: &str,
    tenant: Option<&str>,
) -> EventRegistryConfiguration {
    let mut config = EventRegistryConfiguration::default();
    config.register_payload_extractor(
        "json",
        Arc::new(LoggingExtractor {
            log: Arc::clone(&log),
        }),
    );
    config.register_filter(
        "default",
        Arc::new(LoggingFilter {
            log: Arc::clone(&log),
            retain: filter_retain,
        }),
    );
    config.register_tenant_detector(
        "default",
        Arc::new(LoggingTenantDetector {
            log: Arc::clone(&log),
            tenant: tenant.map(str::to_string),
        }),
    );
    config.register_inbound_transformer(
        "default",
        Arc::new(LoggingTransformer {
            log: Arc::clone(&log),
            fail: transform_fail,
        }),
    );
    config.register_key_detector(
        "default",
        Arc::new(LoggingKeyDetector {
            log: Arc::clone(&log),
            key: Some(event_key.to_string()),
        }),
    );
    config.register_consumer(
        "default",
        Arc::new(LoggingConsumer {
            log: Arc::clone(&log),
            fail: consumer_fail,
            seen: Mutex::new(Vec::new()),
        }),
    );
    config
}

#[test]
fn inbound_pipeline_runs_stages_in_adr6_order() {
    let log = Arc::new(StageLog::default());
    let config = config_with_log(Arc::clone(&log), true, false, false, "orderReceived", None);
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("inbound-pipeline-order".to_string())),
        config,
    );
    deploy_inbound_channel(
        &service,
        "ordersInbound",
        "orderReceived",
        "order.received",
        json!({}),
        json!([{ "name": "orderId", "type": "string" }]),
    );

    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInbound".to_string(),
            body: json!({ "orderId": "A-1" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();

    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(
        delivery.status_history,
        vec![
            EventInstanceStatus::Received,
            EventInstanceStatus::Processed
        ]
    );
    assert_eq!(
        log.snapshot(),
        vec![
            "extraction",
            "filter",
            "tenant",
            "transform",
            "key_detection",
            "consumer"
        ]
    );
}

#[test]
fn inbound_pipeline_filter_rejection_short_circuits_without_delivery() {
    let log = Arc::new(StageLog::default());
    let config = config_with_log(Arc::clone(&log), false, false, false, "orderReceived", None);
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("inbound-pipeline-filter".to_string())),
        config,
    );
    deploy_inbound_channel(
        &service,
        "ordersInbound",
        "orderReceived",
        "order.received",
        json!({}),
        json!([]),
    );

    let error = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInbound".to_string(),
            body: json!({ "orderId": "A-1" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap_err();

    match error {
        FlowableError::BadRequest(message) | FlowableError::ExecutionError(message) => {
            assert!(message.to_lowercase().contains("filter"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(
        log.snapshot(),
        vec!["extraction", "filter"]
    );
    assert!(
        service
            .create_event_instance_delivery_query()
            .list_page()
            .unwrap()
            .data
            .is_empty()
    );
}

#[test]
fn inbound_pipeline_missing_key_short_circuits_before_consumer() {
    let log = Arc::new(StageLog::default());
    let mut config = config_with_log(Arc::clone(&log), true, false, false, "orderReceived", None);
    config.register_key_detector(
        "default",
        Arc::new(LoggingKeyDetector {
            log: Arc::clone(&log),
            key: None,
        }),
    );
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("inbound-pipeline-missing-key".to_string())),
        config,
    );
    deploy_inbound_channel(
        &service,
        "ordersInbound",
        "orderReceived",
        "order.received",
        json!({}),
        json!([]),
    );

    let error = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInbound".to_string(),
            body: json!({}),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap_err();
    assert!(error.to_string().contains("missing event key") || error.to_string().contains("key"));
    assert!(!log.snapshot().contains(&"consumer".to_string()));
    assert!(
        service
            .create_event_instance_delivery_query()
            .list_page()
            .unwrap()
            .data
            .is_empty()
    );
}

#[test]
fn inbound_pipeline_transform_failure_does_not_persist_received() {
    let log = Arc::new(StageLog::default());
    let config = config_with_log(Arc::clone(&log), true, true, false, "orderReceived", None);
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("inbound-pipeline-transform-fail".to_string())),
        config,
    );
    deploy_inbound_channel(
        &service,
        "ordersInbound",
        "orderReceived",
        "order.received",
        json!({}),
        json!([]),
    );

    let error = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInbound".to_string(),
            body: json!({}),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap_err();
    assert!(error.to_string().contains("transform failed"));
    assert_eq!(
        log.snapshot(),
        vec!["extraction", "filter", "tenant", "transform"]
    );
    assert!(
        service
            .create_event_instance_delivery_query()
            .list_page()
            .unwrap()
            .data
            .is_empty()
    );
}

#[test]
fn inbound_pipeline_payload_validation_failure_marks_no_processed_delivery() {
    let log = Arc::new(StageLog::default());
    let config = config_with_log(Arc::clone(&log), true, false, false, "orderReceived", None);
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("inbound-pipeline-payload".to_string())),
        config,
    );
    deploy_inbound_channel(
        &service,
        "ordersInbound",
        "orderReceived",
        "order.received",
        json!({}),
        json!([{ "name": "orderId", "type": "string", "required": true }]),
    );

    let error = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInbound".to_string(),
            body: json!({}),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap_err();
    assert!(error.to_string().to_lowercase().contains("invalid") || error.to_string().contains("orderId"));
    assert!(!log.snapshot().contains(&"consumer".to_string()));
}

#[test]
fn inbound_pipeline_consumer_failure_persists_failed_after_received() {
    let log = Arc::new(StageLog::default());
    let config = config_with_log(Arc::clone(&log), true, false, true, "orderReceived", None);
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("inbound-pipeline-consumer-fail".to_string())),
        config,
    );
    deploy_inbound_channel(
        &service,
        "ordersInbound",
        "orderReceived",
        "order.received",
        json!({}),
        json!([]),
    );

    let error = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInbound".to_string(),
            body: json!({}),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap_err();
    assert!(error.to_string().contains("consumer failed"));

    let deliveries = service
        .create_event_instance_delivery_query()
        .list_page()
        .unwrap()
        .data;
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, EventInstanceStatus::Failed);
    assert_eq!(
        deliveries[0].status_history,
        vec![
            EventInstanceStatus::Received,
            EventInstanceStatus::Failed
        ]
    );
    assert!(deliveries[0].last_error.as_ref().unwrap().contains("consumer failed"));
}

#[test]
fn inbound_pipeline_tenant_detection_precedes_definition_resolution() {
    let log = Arc::new(StageLog::default());
    let config = config_with_log(
        Arc::clone(&log),
        true,
        false,
        false,
        "tenantAwareOrder",
        Some("tenant-a"),
    );
    // key detector returns shared key; tenant detector returns tenant-a
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("inbound-pipeline-tenant".to_string())),
        config,
    );

    // global definition requires globalOnly
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "global".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "tenantAwareOrder.event".to_string(),
                    resource: json!({
                        "key": "tenantAwareOrder",
                        "name": "global",
                        "eventType": "tenant.aware",
                        "channelKey": "tenantChannel",
                        "resourceName": "tenantAwareOrder.event",
                        "payload": [{ "name": "globalOnly", "type": "string", "required": true }]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "tenantChannel.channel".to_string(),
                    resource: json!({
                        "key": "tenantChannel",
                        "name": "tenantChannel",
                        "channelType": "inbound",
                        "resourceName": "tenantChannel.channel",
                        "type": "in-memory",
                        "payloadExtractor": "json",
                        "filter": "default",
                        "tenantDetector": "default",
                        "transformer": "default",
                        "keyDetector": "default",
                        "consumer": "default"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "tenant".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: Some("tenant-a".to_string()),
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "tenantAwareOrder.event".to_string(),
                    resource: json!({
                        "key": "tenantAwareOrder",
                        "name": "tenant",
                        "eventType": "tenant.aware",
                        "channelKey": "tenantChannel",
                        "resourceName": "tenantAwareOrder.event",
                        "payload": [{ "name": "tenantOnly", "type": "string", "required": true }]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "tenantChannel.channel".to_string(),
                    resource: json!({
                        "key": "tenantChannel",
                        "name": "tenantChannel",
                        "channelType": "inbound",
                        "resourceName": "tenantChannel.channel",
                        "type": "in-memory",
                        "payloadExtractor": "json",
                        "filter": "default",
                        "tenantDetector": "default",
                        "transformer": "default",
                        "keyDetector": "default",
                        "consumer": "default"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();

    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "tenantChannel".to_string(),
            body: json!({ "tenantOnly": "ok" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();

    assert_eq!(delivery.tenant_id.as_deref(), Some("tenant-a"));
    assert!(log.snapshot().iter().position(|s| s == "tenant").unwrap()
        < log.snapshot().iter().position(|s| s == "key_detection").unwrap());
}

#[test]
fn legacy_event_type_receive_routes_through_channel_pipeline() {
    let log = Arc::new(StageLog::default());
    let config = config_with_log(Arc::clone(&log), true, false, false, "orderReceived", None);
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("inbound-pipeline-compat".to_string())),
        config,
    );
    deploy_inbound_channel(
        &service,
        "ordersInbound",
        "orderReceived",
        "order.received",
        json!({}),
        json!([{ "name": "orderId", "type": "string" }]),
    );

    let delivery = service
        .receive_inbound_event(flowable_event_registry_service::InboundEventRequest {
            event_type: "order.received".to_string(),
            event_payload: json!({ "orderId": "compat" }),
            tenant_id: None,
        })
        .unwrap();

    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert!(log.snapshot().contains(&"extraction".to_string()));
    assert!(log.snapshot().contains(&"consumer".to_string()));
}
