mod test_support;

use flowable_event_registry_service::{
    EventInstanceRequest, EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
    FlowableEventRegistryService, InboundEventRequest, OutboundEventRequest,
};
use serde_json::json;
// These tests exercise exact-tenant preference + default-tenant fallback, so the
// service enables `fallbackToDefaultTenant` (Java AbstractEngineConfiguration:324).
use test_support::service_with_tenant_fallback as service;

fn deploy_runtime_definitions(
    service: &FlowableEventRegistryService,
    name: &str,
    tenant_id: Option<&str>,
    resource_prefix: &str,
    required_field: &str,
) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: name.to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: tenant_id.map(str::to_string),
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{resource_prefix}-received.event"),
                    resource: json!({
                        "key": "tenantAwareOrderReceived",
                        "name": name,
                        "eventType": "tenant.aware.order.received",
                        "channelKey": "tenantAwareOrdersInbound",
                        "resourceName": format!("{resource_prefix}-received.event"),
                        "payload": [
                            { "name": required_field, "type": "string", "required": true }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{resource_prefix}-inbound.channel"),
                    resource: json!({
                        "key": "tenantAwareOrdersInbound",
                        "name": format!("{name} inbound"),
                        "channelType": "inbound",
                        "resourceName": format!("{resource_prefix}-inbound.channel"),
                        "type": "in-memory",
                        "destination": format!("{resource_prefix}-inbound"),
                        "deserializerType": "json"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
}

fn deploy_outbound_runtime_definitions(
    service: &FlowableEventRegistryService,
    name: &str,
    tenant_id: Option<&str>,
    resource_prefix: &str,
    required_field: &str,
) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: name.to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: tenant_id.map(str::to_string),
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{resource_prefix}-published.event"),
                    resource: json!({
                        "key": "tenantAwareOrderPublished",
                        "name": name,
                        "eventType": "tenant.aware.order.published",
                        "channelKey": "tenantAwareOrdersOutbound",
                        "resourceName": format!("{resource_prefix}-published.event"),
                        "payload": [
                            { "name": required_field, "type": "string", "required": true }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{resource_prefix}-outbound.channel"),
                    resource: json!({
                        "key": "tenantAwareOrdersOutbound",
                        "name": format!("{name} outbound"),
                        "channelType": "outbound",
                        "resourceName": format!("{resource_prefix}-outbound.channel"),
                        "type": "in-memory",
                        "destination": format!("{resource_prefix}-outbound"),
                        "serializerType": "json"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
}

#[test]
fn event_instance_resolution_uses_tenant_specific_definition_before_global_fallback() {
    let service = service("event-registry-tenant-runtime-resolution");

    deploy_runtime_definitions(&service, "Global v1", None, "global-v1", "globalOnly");
    deploy_runtime_definitions(&service, "Global v2", None, "global-v2", "globalOnly");
    deploy_runtime_definitions(
        &service,
        "Tenant A v1",
        Some("tenant-a"),
        "tenant-a-v1",
        "tenantOnly",
    );

    let tenant_delivery = service
        .receive_event_instance(EventInstanceRequest {
            event_definition_id: None,
            event_definition_key: Some("tenantAwareOrderReceived".to_string()),
            channel_definition_id: None,
            channel_definition_key: Some("tenantAwareOrdersInbound".to_string()),
            event_payload: json!({ "tenantOnly": "T-100" }),
            tenant_id: Some("tenant-a".to_string()),
        })
        .unwrap();

    let tenant_definition = service
        .get_event_definition(&tenant_delivery.event_definition_id)
        .unwrap();
    assert_eq!(tenant_definition.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(tenant_definition.version, 1);

    let fallback_delivery = service
        .receive_event_instance(EventInstanceRequest {
            event_definition_id: None,
            event_definition_key: Some("tenantAwareOrderReceived".to_string()),
            channel_definition_id: None,
            channel_definition_key: Some("tenantAwareOrdersInbound".to_string()),
            event_payload: json!({ "globalOnly": "G-200" }),
            tenant_id: Some("tenant-b".to_string()),
        })
        .unwrap();

    let fallback_definition = service
        .get_event_definition(&fallback_delivery.event_definition_id)
        .unwrap();
    assert_eq!(fallback_definition.tenant_id, None);
    assert_eq!(fallback_definition.version, 2);
}

#[test]
fn inbound_event_type_resolution_uses_request_tenant_before_global_fallback() {
    let service = service("event-registry-tenant-inbound-runtime-resolution");

    deploy_runtime_definitions(
        &service,
        "Global v1",
        None,
        "global-inbound-v1",
        "globalOnly",
    );
    deploy_runtime_definitions(
        &service,
        "Global v2",
        None,
        "global-inbound-v2",
        "globalOnly",
    );
    deploy_runtime_definitions(
        &service,
        "Tenant A inbound",
        Some("tenant-a"),
        "tenant-a-inbound",
        "tenantOnly",
    );

    let tenant_delivery = service
        .receive_inbound_event(InboundEventRequest {
            event_type: "tenant.aware.order.received".to_string(),
            event_payload: json!({ "tenantOnly": "T-300" }),
            tenant_id: Some("tenant-a".to_string()),
        })
        .unwrap();

    let tenant_definition = service
        .get_event_definition(&tenant_delivery.event_definition_id)
        .unwrap();
    assert_eq!(tenant_definition.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(tenant_delivery.channel_key, "tenantAwareOrdersInbound");

    let fallback_delivery = service
        .receive_inbound_event(InboundEventRequest {
            event_type: "tenant.aware.order.received".to_string(),
            event_payload: json!({ "globalOnly": "G-300" }),
            tenant_id: Some("tenant-b".to_string()),
        })
        .unwrap();

    let fallback_definition = service
        .get_event_definition(&fallback_delivery.event_definition_id)
        .unwrap();
    assert_eq!(fallback_definition.tenant_id, None);
    assert_eq!(fallback_definition.version, 2);
}

#[test]
fn outbound_publish_resolution_uses_request_tenant_before_global_fallback() {
    let service = service("event-registry-tenant-outbound-runtime-resolution");

    deploy_outbound_runtime_definitions(
        &service,
        "Global outbound v1",
        None,
        "global-outbound-v1",
        "globalOnly",
    );
    deploy_outbound_runtime_definitions(
        &service,
        "Global outbound v2",
        None,
        "global-outbound-v2",
        "globalOnly",
    );
    deploy_outbound_runtime_definitions(
        &service,
        "Tenant A outbound",
        Some("tenant-a"),
        "tenant-a-outbound",
        "tenantOnly",
    );

    let tenant_delivery = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "tenantAwareOrderPublished".to_string(),
            event_payload: json!({ "tenantOnly": "T-400" }),
            tenant_id: Some("tenant-a".to_string()),
        })
        .unwrap();

    let tenant_definition = service
        .get_event_definition(&tenant_delivery.event_definition_id)
        .unwrap();
    assert_eq!(tenant_definition.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(tenant_delivery.channel_key, "tenantAwareOrdersOutbound");

    let fallback_delivery = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "tenantAwareOrderPublished".to_string(),
            event_payload: json!({ "globalOnly": "G-400" }),
            tenant_id: Some("tenant-b".to_string()),
        })
        .unwrap();

    let fallback_definition = service
        .get_event_definition(&fallback_delivery.event_definition_id)
        .unwrap();
    assert_eq!(fallback_definition.tenant_id, None);
    assert_eq!(fallback_definition.version, 2);
}

#[test]
fn event_delivery_persists_runtime_tenant_id() {
    use flowable_event_registry_service::EventDirection;

    let service = service("event-registry-tenant-delivery-persistence");

    deploy_runtime_definitions(
        &service,
        "Tenant aware order",
        Some("tenant-x"),
        "tenant-x-order",
        "tenantOnly",
    );
    deploy_runtime_definitions(&service, "Global order", None, "global-order", "globalOnly");

    let tenant_delivery = service
        .receive_event_instance(EventInstanceRequest {
            event_definition_id: None,
            event_definition_key: Some("tenantAwareOrderReceived".to_string()),
            channel_definition_id: None,
            channel_definition_key: Some("tenantAwareOrdersInbound".to_string()),
            event_payload: json!({ "tenantOnly": "X-100" }),
            tenant_id: Some("tenant-x".to_string()),
        })
        .unwrap();
    assert_eq!(tenant_delivery.tenant_id.as_deref(), Some("tenant-x"));

    // Fallback resolves the tenantless definition, but the EventInstance keeps
    // the request/runtime tenant (DefaultInboundEventProcessingPipeline.java:148-150).
    let global_delivery = service
        .receive_event_instance(EventInstanceRequest {
            event_definition_id: None,
            event_definition_key: Some("tenantAwareOrderReceived".to_string()),
            channel_definition_id: None,
            channel_definition_key: Some("tenantAwareOrdersInbound".to_string()),
            event_payload: json!({ "globalOnly": "G-100" }),
            tenant_id: Some("tenant-y".to_string()),
        })
        .unwrap();
    assert_eq!(global_delivery.tenant_id.as_deref(), Some("tenant-y"));

    let deliveries = service
        .create_event_instance_delivery_query()
        .list_page()
        .unwrap();
    assert_eq!(deliveries.total, 2);

    let tenant_filtered = service
        .create_event_instance_delivery_query()
        .tenant_id("tenant-x")
        .list_page()
        .unwrap();
    assert_eq!(tenant_filtered.total, 1);
    assert_eq!(
        tenant_filtered.data[0].tenant_id.as_deref(),
        Some("tenant-x")
    );

    let without_tenant = service
        .create_event_instance_delivery_query()
        .without_tenant_id(true)
        .list_page()
        .unwrap();
    assert_eq!(
        without_tenant.total, 0,
        "runtime tenant is preserved even when definition is tenantless"
    );

    let tenant_like = service
        .create_event_instance_delivery_query()
        .tenant_id_like("tenant-%")
        .list_page()
        .unwrap();
    assert_eq!(tenant_like.total, 2);

    let all_tenant_x = service
        .create_event_instance_delivery_query()
        .direction(EventDirection::Inbound)
        .tenant_id("tenant-x")
        .list_page()
        .unwrap();
    assert_eq!(all_tenant_x.total, 1);

    let all_tenant_y = service
        .create_event_instance_delivery_query()
        .tenant_id("tenant-y")
        .list_page()
        .unwrap();
    assert_eq!(all_tenant_y.total, 1);
}
