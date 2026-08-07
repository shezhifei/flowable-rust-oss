mod test_support;

use flowable_engine::engine::outbound_event_dispatch::OutboundEventDispatchRequest;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{
    ChannelDefinitionUpdateRequest, EventInstanceStatus, EventPayload, EventRegistryConfiguration,
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, FlowableEventRegistryService,
    OutboundChannelAdapter, OutboundEventRequest, OutboundEventTransformer,
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct StageLog {
    stages: Mutex<Vec<String>>,
    payloads: Mutex<Vec<Value>>,
}

struct LoggingOutboundTransformer {
    log: Arc<StageLog>,
    marker: String,
}

impl OutboundEventTransformer for LoggingOutboundTransformer {
    fn transform(
        &self,
        payload: &Value,
        _channel_config: &Value,
        _event_type: &str,
    ) -> Result<Value, FlowableError> {
        self.log.stages.lock().unwrap().push("transform".to_string());
        let mut object = payload.as_object().cloned().unwrap_or_default();
        object.insert("transformed".to_string(), Value::String(self.marker.clone()));
        Ok(Value::Object(object))
    }
}

struct LoggingOutboundAdapter {
    log: Arc<StageLog>,
    fail_times: AtomicUsize,
}

impl OutboundChannelAdapter for LoggingOutboundAdapter {
    fn send(
        &self,
        _destination: Option<&str>,
        event: EventPayload,
        _channel_config: &Value,
    ) -> Result<(), FlowableError> {
        self.log.stages.lock().unwrap().push("adapter".to_string());
        self.log.payloads.lock().unwrap().push(event.payload.clone());
        let remaining = self.fail_times.load(Ordering::SeqCst);
        if remaining > 0 {
            self.fail_times.fetch_sub(1, Ordering::SeqCst);
            return Err(FlowableError::ExecutionError(
                "adapter temporary failure".to_string(),
            ));
        }
        Ok(())
    }
}

fn deploy_outbound(
    service: &FlowableEventRegistryService,
    event_key: &str,
    channel_key: &str,
    adapter_type: &str,
    tenant_id: Option<&str>,
) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("deploy-{event_key}"),
            category: None,
            parent_deployment_id: None,
            tenant_id: tenant_id.map(str::to_string),
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
                        "type": adapter_type,
                        "destination": format!("dest-{channel_key}"),
                        "outboundTransformer": "json"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
}

fn service_with_logging(
    name: &str,
    fail_times: usize,
) -> (FlowableEventRegistryService, Arc<StageLog>) {
    let log = Arc::new(StageLog::default());
    let mut config = EventRegistryConfiguration::default();
    config.register_outbound_transformer(
        "json",
        Arc::new(LoggingOutboundTransformer {
            log: Arc::clone(&log),
            marker: "yes".to_string(),
        }),
    );
    config.register_outbound_adapter(
        "in-memory",
        Arc::new(LoggingOutboundAdapter {
            log: Arc::clone(&log),
            fail_times: AtomicUsize::new(fail_times),
        }),
    );
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new(name.to_string())),
        config,
    );
    (service, log)
}

#[test]
fn outbound_pipeline_runs_transform_before_adapter_with_transformed_payload() {
    let (service, log) = service_with_logging("outbound-pipeline-order", 0);
    deploy_outbound(&service, "orderPublished", "ordersOut", "in-memory", None);

    let delivery = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "A-1" }),
            tenant_id: None,
        })
        .unwrap();

    assert_eq!(delivery.status, EventInstanceStatus::Published);
    assert_eq!(log.stages.lock().unwrap().as_slice(), &["transform", "adapter"]);
    let payload = log.payloads.lock().unwrap()[0].clone();
    assert_eq!(payload["orderId"], json!("A-1"));
    assert_eq!(payload["transformed"], json!("yes"));
    assert!(delivery.dispatch_token.is_some());
}

#[test]
fn outbound_pipeline_rejects_unknown_adapter_at_deploy_time() {
    let service = FlowableEventRegistryService::new(Arc::new(ProcessEngine::new(
        "outbound-unknown-adapter".to_string(),
    )));
    let error = service
        .deploy(EventRegistryDeploymentRequest {
            name: "bad".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "c.channel".to_string(),
                resource: json!({
                    "key": "c",
                    "name": "c",
                    "channelType": "outbound",
                    "resourceName": "c.channel",
                    "type": "kafka"
                })
                .to_string(),
            }],
        })
        .unwrap_err();
    match error {
        FlowableError::DeploymentValidationError(message) => {
            assert!(message.contains("c"));
            assert!(message.contains("kafka"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn outbound_pipeline_validation_failure_does_not_call_adapter() {
    let (service, log) = service_with_logging("outbound-pipeline-validation", 0);
    deploy_outbound(&service, "orderPublished", "ordersOut", "in-memory", None);

    let error = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap_err();
    assert!(error.to_string().contains("orderId") || error.to_string().contains("invalid"));
    assert!(log.stages.lock().unwrap().is_empty());
}

#[test]
fn outbound_pipeline_adapter_failure_marks_failed_with_created_history() {
    let (service, log) = service_with_logging("outbound-pipeline-fail", 1);
    deploy_outbound(&service, "orderPublished", "ordersOut", "in-memory", None);

    service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "fail" }),
            tenant_id: None,
        })
        .unwrap_err();

    assert_eq!(log.stages.lock().unwrap().as_slice(), &["transform", "adapter"]);
    let delivery = service
        .create_event_instance_delivery_query()
        .list_page()
        .unwrap()
        .data
        .pop()
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Failed);
    assert_eq!(
        delivery.status_history,
        vec![EventInstanceStatus::Created, EventInstanceStatus::Failed]
    );
    assert!(delivery.dispatch_token.is_some());
}

#[test]
fn outbound_retry_reuses_original_definition_id_and_reruns_pipeline() {
    let (service, log) = service_with_logging("outbound-pipeline-retry", 1);
    deploy_outbound(&service, "orderPublished", "ordersOut", "in-memory", None);

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
    let original_definition_id = failed.event_definition_id.clone();
    let original_token = failed.dispatch_token.clone();

    // Deploy a newer version with a different required field so "latest" would fail.
    deploy_outbound(&service, "orderPublished", "ordersOut", "in-memory", None);
    let store_definition = service
        .create_event_definition_query()
        .key("orderPublished")
        .latest()
        .list()
        .unwrap()
        .pop()
        .unwrap();
    assert_ne!(store_definition.id, original_definition_id);

    let retried = service.retry_event_delivery(&failed.id).unwrap();
    assert_eq!(retried.status, EventInstanceStatus::Published);
    assert_eq!(retried.event_definition_id, original_definition_id);
    assert_eq!(retried.dispatch_token, original_token);
    assert_eq!(retried.retry_count, 1);
    // transform+adapter on first attempt, transform+adapter on retry
    assert_eq!(
        log.stages.lock().unwrap().as_slice(),
        &["transform", "adapter", "transform", "adapter"]
    );
    assert_eq!(log.payloads.lock().unwrap().len(), 2);
    assert_eq!(log.payloads.lock().unwrap()[1]["transformed"], json!("yes"));
}

#[test]
fn outbound_tenant_isolation_uses_tenant_specific_definition() {
    let (service, _log) = service_with_logging("outbound-pipeline-tenant", 0);
    deploy_outbound(
        &service,
        "tenantOrder",
        "tenantOut",
        "in-memory",
        Some("tenant-a"),
    );
    deploy_outbound(&service, "tenantOrder", "tenantOut", "in-memory", None);

    let tenant_delivery = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "tenantOrder".to_string(),
            event_payload: json!({ "orderId": "T" }),
            tenant_id: Some("tenant-a".to_string()),
        })
        .unwrap();
    assert_eq!(tenant_delivery.tenant_id.as_deref(), Some("tenant-a"));

    let global_delivery = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "tenantOrder".to_string(),
            event_payload: json!({ "orderId": "G" }),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(global_delivery.tenant_id, None);
}

#[test]
fn in_memory_adapter_is_invoked_through_registry() {
    let counter = Arc::new(AtomicUsize::new(0));
    struct CountingAdapter(Arc<AtomicUsize>);
    impl OutboundChannelAdapter for CountingAdapter {
        fn send(
            &self,
            _destination: Option<&str>,
            _event: EventPayload,
            _channel_config: &Value,
        ) -> Result<(), FlowableError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let mut config = EventRegistryConfiguration::default();
    config.register_outbound_adapter("in-memory", Arc::new(CountingAdapter(Arc::clone(&counter))));
    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("outbound-in-memory-count".to_string())),
        config,
    );
    deploy_outbound(&service, "orderPublished", "ordersOut", "in-memory", None);
    service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "1" }),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Keep ChannelDefinitionUpdateRequest import live for REST-style updates used in retry tests.
    let channel = service
        .create_channel_definition_query()
        .key("ordersOut")
        .list()
        .unwrap()
        .pop()
        .unwrap();
    service
        .update_channel_definition(
            &channel.id,
            ChannelDefinitionUpdateRequest {
                name: Some("ordersOut".to_string()),
                configuration: None,
            },
        )
        .unwrap();
}

/// P94: service construction installs the engine outbound hook so BPMN send-event
/// (engine crate) can reach transform + adapter without a crate dependency cycle.
#[test]
fn engine_outbound_hook_installed_by_service_runs_transform_and_adapter() {
    let log = Arc::new(StageLog::default());
    let mut config = EventRegistryConfiguration::default();
    config.register_outbound_transformer(
        "json",
        Arc::new(LoggingOutboundTransformer {
            log: Arc::clone(&log),
            marker: "bridge".to_string(),
        }),
    );
    config.register_outbound_adapter(
        "in-memory",
        Arc::new(LoggingOutboundAdapter {
            log: Arc::clone(&log),
            fail_times: AtomicUsize::new(0),
        }),
    );
    let engine = Arc::new(ProcessEngine::new(
        "outbound-engine-hook-bridge".to_string(),
    ));
    let _service = FlowableEventRegistryService::with_configuration(Arc::clone(&engine), config);

    assert!(
        engine.get_config().outbound_event_dispatch.is_installed(),
        "FlowableEventRegistryService must install the engine outbound dispatch hook"
    );

    // Same request shape as execute_send_event_service_task after eventInParameters assembly.
    engine
        .get_config()
        .outbound_event_dispatch
        .dispatch(&OutboundEventDispatchRequest {
            channel_key: "ordersOut".to_string(),
            channel_configuration: json!({
                "type": "in-memory",
                "destination": "dest-ordersOut",
                "outboundTransformer": "json"
            }),
            event_type: "order.published".to_string(),
            payload: json!({ "orderId": "bridge-1" }),
            dispatch_token: Some("dispatch:bridge-test".to_string()),
        })
        .expect("installed hook should run the service outbound pipeline");

    assert_eq!(
        log.stages.lock().unwrap().as_slice(),
        &["transform", "adapter"]
    );
    let payload = log.payloads.lock().unwrap()[0].clone();
    assert_eq!(payload["orderId"], json!("bridge-1"));
    assert_eq!(payload["transformed"], json!("bridge"));
}
