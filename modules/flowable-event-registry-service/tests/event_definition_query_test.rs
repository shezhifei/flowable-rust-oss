mod test_support;

use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
};
use serde_json::json;
use test_support::{deploy_sample_definitions, service};

#[test]
fn event_definition_query_returns_deterministic_results_and_supported_filters() {
    let service = service("event-definition-query");
    let deployment = deploy_sample_definitions(&service);

    let page = service
        .create_event_definition_query()
        .page(0, 10)
        .list_page()
        .unwrap();

    assert_eq!(page.start, 0);
    assert_eq!(page.size, 2);
    assert_eq!(page.total, 2);
    assert_eq!(
        page.data
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["orderPublished", "orderReceived"]
    );
    assert!(
        page.data
            .iter()
            .all(|item| item.deployment_id == deployment.id)
    );

    let by_channel = service
        .create_event_definition_query()
        .channel_key("ordersInbound")
        .list()
        .unwrap();
    assert_eq!(by_channel.len(), 1);
    assert_eq!(by_channel[0].event_type, "order.received");

    let by_type = service
        .create_event_definition_query()
        .event_type("order.published")
        .list()
        .unwrap();
    assert_eq!(by_type.len(), 1);
    assert_eq!(by_type[0].key, "orderPublished");
}

#[test]
fn event_definition_query_versions_duplicate_keys_and_filters_latest_metadata() {
    let service = service("event-definition-query-version-latest");
    deploy_sample_definitions(&service);

    let second_deployment = service
        .deploy(EventRegistryDeploymentRequest {
            name: "Sample deployment v2".to_string(),
            category: Some("orders-v2".to_string()),
            parent_deployment_id: Some("parent-orders".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "order-received-v2.event".to_string(),
                resource: json!({
                    "key": "orderReceived",
                    "name": "Order received v2",
                    "description": "Inbound order event v2",
                    "eventType": "order.received",
                    "channelKey": "ordersInbound",
                    "resourceName": "order-received-v2.event",
                    "payload": [
                        { "name": "orderId", "type": "string" },
                        { "name": "priority", "type": "string" }
                    ]
                })
                .to_string(),
            }],
        })
        .unwrap();

    let tenant_first_version = service
        .create_event_definition_query()
        .key("orderReceived")
        .tenant_id("tenant-a")
        .version(1)
        .list()
        .unwrap();
    assert_eq!(tenant_first_version.len(), 1);
    assert_eq!(tenant_first_version[0].deployment_id, second_deployment.id);
    assert_eq!(
        tenant_first_version[0].category.as_deref(),
        Some("orders-v2")
    );
    assert_eq!(
        tenant_first_version[0].parent_deployment_id.as_deref(),
        Some("parent-orders")
    );

    service
        .deploy(EventRegistryDeploymentRequest {
            name: "Sample deployment v3".to_string(),
            category: Some("orders-v3".to_string()),
            parent_deployment_id: Some("parent-orders".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "order-received-v3.event".to_string(),
                resource: json!({
                    "key": "orderReceived",
                    "name": "Order received v3",
                    "description": "Inbound order event v3",
                    "eventType": "order.received",
                    "channelKey": "ordersInbound",
                    "resourceName": "order-received-v3.event",
                    "payload": [
                        { "name": "orderId", "type": "string" },
                        { "name": "priority", "type": "string" },
                        { "name": "region", "type": "string" }
                    ]
                })
                .to_string(),
            }],
        })
        .unwrap();

    let latest = service
        .create_event_definition_query()
        .key("orderReceived")
        .latest()
        .order_by("version", true)
        .list()
        .unwrap();
    assert_eq!(latest.len(), 2);
    let tenant_latest = latest
        .iter()
        .find(|definition| definition.tenant_id.as_deref() == Some("tenant-a"))
        .unwrap();
    assert_eq!(tenant_latest.version, 2);
    assert_eq!(tenant_latest.name, "Order received v3");
    let no_tenant_latest = latest
        .iter()
        .find(|definition| definition.tenant_id.is_none())
        .unwrap();
    assert_eq!(no_tenant_latest.version, 1);
    assert_eq!(no_tenant_latest.name, "Order received");

    let category_like = service
        .create_event_definition_query()
        .category_like("orders%")
        .parent_deployment_id("parent-orders")
        .tenant_id_like("tenant-%")
        .list()
        .unwrap();
    assert_eq!(category_like.len(), 2);
    assert!(
        category_like
            .iter()
            .all(|definition| definition.key == "orderReceived")
    );
}

#[test]
fn event_definition_query_rejects_unsupported_filters_structurally() {
    let service = service("event-definition-query-errors");
    deploy_sample_definitions(&service);

    let error = service
        .create_event_definition_query()
        .unsupported_filter("unsupportedEventDefinitionFilter", "unexpected")
        .list_page()
        .unwrap_err();

    match error {
        FlowableError::ExecutionError(message) | FlowableError::Generic(message) => {
            assert!(message.contains("unsupportedEventDefinitionFilter"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
