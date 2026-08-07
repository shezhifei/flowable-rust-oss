mod test_support;

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{
    EventPayload, EventRegistryConfiguration, EventRegistryDeploymentRequest,
    EventRegistryDeploymentResource, FlowableEventRegistryService, InboundChannelAdapter,
    OutboundChannelAdapter, OutboundEventTransformer,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct CountingOutboundAdapter {
    sends: AtomicUsize,
}

impl OutboundChannelAdapter for CountingOutboundAdapter {
    fn send(
        &self,
        _destination: Option<&str>,
        _event: EventPayload,
        _channel_config: &Value,
    ) -> Result<(), FlowableError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct TaggedOutboundAdapter {
    tag: String,
    sink: Arc<Mutex<Vec<String>>>,
}

impl OutboundChannelAdapter for TaggedOutboundAdapter {
    fn send(
        &self,
        _destination: Option<&str>,
        _event: EventPayload,
        _channel_config: &Value,
    ) -> Result<(), FlowableError> {
        self.sink.lock().unwrap().push(self.tag.clone());
        Ok(())
    }
}

struct IdentityOutboundTransformer;

impl OutboundEventTransformer for IdentityOutboundTransformer {
    fn transform(
        &self,
        payload: &Value,
        _channel_config: &Value,
        _event_type: &str,
    ) -> Result<Value, FlowableError> {
        Ok(payload.clone())
    }
}

struct MarkerInboundAdapter;

impl InboundChannelAdapter for MarkerInboundAdapter {}

fn channel_resource(key: &str, channel_type: &str, implementation: &str) -> EventRegistryDeploymentResource {
    EventRegistryDeploymentResource {
        resource_name: format!("{key}.channel"),
        resource: json!({
            "key": key,
            "name": key,
            "channelType": channel_type,
            "resourceName": format!("{key}.channel"),
            "type": implementation,
            "destination": format!("dest-{key}")
        })
        .to_string(),
    }
}

fn channel_with_processors(
    key: &str,
    channel_type: &str,
    implementation: &str,
    extra: Value,
) -> EventRegistryDeploymentResource {
    let mut body = json!({
        "key": key,
        "name": key,
        "channelType": channel_type,
        "resourceName": format!("{key}.channel"),
        "type": implementation,
        "destination": format!("dest-{key}")
    });
    if let Some(object) = body.as_object_mut() {
        if let Some(extra_object) = extra.as_object() {
            for (field, value) in extra_object {
                object.insert(field.clone(), value.clone());
            }
        }
    }
    EventRegistryDeploymentResource {
        resource_name: format!("{key}.channel"),
        resource: body.to_string(),
    }
}

#[test]
fn default_configuration_accepts_in_memory_and_rest_adapters() {
    let service = FlowableEventRegistryService::new(Arc::new(ProcessEngine::new(
        "event-registry-default-adapters".to_string(),
    )));

    service
        .deploy(EventRegistryDeploymentRequest {
            name: "default adapters".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                channel_resource("inMemoryOut", "outbound", "in-memory"),
                channel_resource("restOut", "outbound", "rest"),
                channel_resource("inMemoryIn", "inbound", "in-memory"),
            ],
        })
        .unwrap();
}

#[test]
fn deployment_rejects_unknown_adapter_with_channel_key_and_allowed_names() {
    let service = FlowableEventRegistryService::new(Arc::new(ProcessEngine::new(
        "event-registry-unknown-adapter".to_string(),
    )));

    let error = service
        .deploy(EventRegistryDeploymentRequest {
            name: "unknown adapter".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![channel_resource("ordersHttp", "inbound", "http")],
        })
        .unwrap_err();

    let message = match error {
        FlowableError::DeploymentValidationError(message)
        | FlowableError::ExecutionError(message)
        | FlowableError::Generic(message) => message,
        other => panic!("unexpected error: {other:?}"),
    };
    assert!(
        message.contains("ordersHttp"),
        "message should include channel key: {message}"
    );
    assert!(
        message.contains("http"),
        "message should include unknown name: {message}"
    );
    assert!(
        message.contains("in-memory") || message.to_lowercase().contains("allowed"),
        "message should include allowed names: {message}"
    );
}

#[test]
fn deployment_rejects_unknown_processor_names_with_allowed_names() {
    let service = FlowableEventRegistryService::new(Arc::new(ProcessEngine::new(
        "event-registry-unknown-processor".to_string(),
    )));

    let error = service
        .deploy(EventRegistryDeploymentRequest {
            name: "unknown processor".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![channel_with_processors(
                "ordersInbound",
                "inbound",
                "in-memory",
                json!({ "filter": "missing-filter" }),
            )],
        })
        .unwrap_err();

    let message = match error {
        FlowableError::DeploymentValidationError(message)
        | FlowableError::ExecutionError(message)
        | FlowableError::Generic(message) => message,
        other => panic!("unexpected error: {other:?}"),
    };
    assert!(
        message.contains("ordersInbound"),
        "message should include channel key: {message}"
    );
    assert!(
        message.contains("missing-filter"),
        "message should include unknown processor: {message}"
    );
    assert!(
        message.to_lowercase().contains("allowed") || message.contains("filter"),
        "message should describe allowed processors: {message}"
    );
}

#[test]
fn two_services_can_register_different_implementations_under_same_name_without_leakage() {
    let sink_a = Arc::new(Mutex::new(Vec::new()));
    let sink_b = Arc::new(Mutex::new(Vec::new()));

    let mut config_a = EventRegistryConfiguration::default();
    config_a.register_outbound_adapter(
        "shared",
        Arc::new(TaggedOutboundAdapter {
            tag: "service-a".to_string(),
            sink: Arc::clone(&sink_a),
        }),
    );
    config_a.register_outbound_transformer("json", Arc::new(IdentityOutboundTransformer));

    let mut config_b = EventRegistryConfiguration::default();
    config_b.register_outbound_adapter(
        "shared",
        Arc::new(TaggedOutboundAdapter {
            tag: "service-b".to_string(),
            sink: Arc::clone(&sink_b),
        }),
    );
    config_b.register_outbound_transformer("json", Arc::new(IdentityOutboundTransformer));

    let service_a = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("event-registry-registry-a".to_string())),
        config_a,
    );
    let service_b = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("event-registry-registry-b".to_string())),
        config_b,
    );

    for (service, channel_key) in [(&service_a, "channelA"), (&service_b, "channelB")] {
        service
            .deploy(EventRegistryDeploymentRequest {
                name: format!("deploy-{channel_key}"),
                category: None,
                parent_deployment_id: None,
                tenant_id: None,
                resources: vec![
                    EventRegistryDeploymentResource {
                        resource_name: format!("{channel_key}.event"),
                        resource: json!({
                            "key": channel_key,
                            "name": channel_key,
                            "eventType": format!("{channel_key}.evt"),
                            "channelKey": channel_key,
                            "resourceName": format!("{channel_key}.event"),
                            "payload": []
                        })
                        .to_string(),
                    },
                    channel_resource(channel_key, "outbound", "shared"),
                ],
            })
            .unwrap();
    }

    service_a
        .publish_outbound_event(flowable_event_registry_service::OutboundEventRequest {
            event_definition_key: "channelA".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap();
    service_b
        .publish_outbound_event(flowable_event_registry_service::OutboundEventRequest {
            event_definition_key: "channelB".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap();

    assert_eq!(sink_a.lock().unwrap().as_slice(), &["service-a".to_string()]);
    assert_eq!(sink_b.lock().unwrap().as_slice(), &["service-b".to_string()]);
    assert!(sink_a.lock().unwrap().iter().all(|tag| tag != "service-b"));
    assert!(sink_b.lock().unwrap().iter().all(|tag| tag != "service-a"));
}

#[test]
fn configuration_registry_lookups_are_local_to_service_instance() {
    let counter = Arc::new(CountingOutboundAdapter {
        sends: AtomicUsize::new(0),
    });
    let mut config = EventRegistryConfiguration::default();
    config.register_outbound_adapter("in-memory", Arc::clone(&counter) as Arc<dyn OutboundChannelAdapter>);

    let service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("event-registry-local-registry".to_string())),
        config,
    );

    // ensure inbound marker registration does not pollute outbound names
    let mut inbound_only = EventRegistryConfiguration::builder();
    inbound_only = inbound_only.inbound_adapter("custom-in", Arc::new(MarkerInboundAdapter));
    let inbound_service = FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new("event-registry-inbound-only".to_string())),
        inbound_only.build(),
    );

    let error = inbound_service
        .deploy(EventRegistryDeploymentRequest {
            name: "missing outbound".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![channel_resource("out", "outbound", "custom-in")],
        })
        .unwrap_err();
    match error {
        FlowableError::DeploymentValidationError(message)
        | FlowableError::ExecutionError(message) => {
            assert!(message.contains("out"));
            assert!(message.contains("custom-in"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    service
        .deploy(EventRegistryDeploymentRequest {
            name: "local adapter deploy".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "local.event".to_string(),
                    resource: json!({
                        "key": "localEvent",
                        "name": "local",
                        "eventType": "local.evt",
                        "channelKey": "localChannel",
                        "resourceName": "local.event",
                        "payload": []
                    })
                    .to_string(),
                },
                channel_resource("localChannel", "outbound", "in-memory"),
            ],
        })
        .unwrap();

    service
        .publish_outbound_event(flowable_event_registry_service::OutboundEventRequest {
            event_definition_key: "localEvent".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap();

    assert_eq!(counter.sends.load(Ordering::SeqCst), 1);
}

#[test]
fn configuration_exposes_immutable_name_maps_after_build() {
    let config = EventRegistryConfiguration::default();
    let outbound_names: BTreeMap<_, _> = config
        .outbound_adapter_names()
        .into_iter()
        .map(|name| (name, ()))
        .collect();
    assert!(outbound_names.contains_key("in-memory"));
    assert!(outbound_names.contains_key("rest"));
    assert!(config.inbound_adapter_names().iter().any(|name| name == "in-memory"));
    assert!(config.payload_extractor_names().iter().any(|name| name == "json"));
    assert!(config.filter_names().iter().any(|name| name == "default"));
    assert!(config.tenant_detector_names().iter().any(|name| name == "default"));
    assert!(config.inbound_transformer_names().iter().any(|name| name == "default"));
    assert!(config.key_detector_names().iter().any(|name| name == "default"));
    assert!(config.outbound_transformer_names().iter().any(|name| name == "json"));
}
