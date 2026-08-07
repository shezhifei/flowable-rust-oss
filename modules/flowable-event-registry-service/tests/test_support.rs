use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_event_registry_service::{
    EventRegistryConfiguration, EventRegistryDeployment, EventRegistryDeploymentRequest,
    EventRegistryDeploymentResource, FlowableEventRegistryService, OutboundUrlGuardConfig,
};
use serde_json::json;
use std::sync::Arc;

#[allow(dead_code)]
pub fn service(name: &str) -> FlowableEventRegistryService {
    // Local integration tests bind mock REST receivers on 127.0.0.1; opt into private
    // destinations (production default remains deny).
    let configuration = EventRegistryConfiguration::builder()
        .outbound_ssrf_guard(OutboundUrlGuardConfig {
            allow_private_networks: true,
            ..Default::default()
        })
        .build();
    FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new(name.to_string())),
        configuration,
    )
}

/// Service with Java multi-tenant fallback enabled
/// (`AbstractEngineConfiguration.setFallbackToDefaultTenant(true)` + empty default tenant).
#[allow(dead_code)]
pub fn service_with_tenant_fallback(name: &str) -> FlowableEventRegistryService {
    let configuration = EventRegistryConfiguration::builder()
        .fallback_to_default_tenant(true)
        .build();
    FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new(name.to_string())),
        configuration,
    )
}

#[allow(dead_code)]
pub fn deploy_sample_definitions(
    service: &FlowableEventRegistryService,
) -> EventRegistryDeployment {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "Sample deployment".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "order-published.event".to_string(),
                    resource: json!({
                        "key": "orderPublished",
                        "name": "Order published",
                        "description": "Outbound order event",
                        "eventType": "order.published",
                        "channelKey": "ordersOutbound",
                        "resourceName": "order-published.event",
                        "payload": [
                            { "name": "orderId", "type": "string" }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "orders-outbound.channel".to_string(),
                    resource: json!({
                        "key": "ordersOutbound",
                        "name": "Orders outbound",
                        "description": "Outbound orders channel",
                        "channelType": "outbound",
                        "resourceName": "orders-outbound.channel",
                        "type": "in-memory",
                        "destination": "orders-outbound",
                        "serializerType": "json"
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "order-received.event".to_string(),
                    resource: json!({
                        "key": "orderReceived",
                        "name": "Order received",
                        "description": "Inbound order event",
                        "eventType": "order.received",
                        "channelKey": "ordersInbound",
                        "resourceName": "order-received.event",
                        "payload": [
                            { "name": "orderId", "type": "string" }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "orders-inbound.channel".to_string(),
                    resource: json!({
                        "key": "ordersInbound",
                        "name": "Orders inbound",
                        "description": "Inbound orders channel",
                        "channelType": "inbound",
                        "resourceName": "orders-inbound.channel",
                        "type": "in-memory",
                        "destination": "orders-inbound",
                        "deserializerType": "json"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap()
}
