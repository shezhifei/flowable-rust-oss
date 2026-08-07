mod test_support;

use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
};
use serde_json::json;
use test_support::service;

#[test]
fn deployment_registers_channel_and_event_definitions_with_deterministic_resource_order() {
    let service = service("event-registry-deployment");
    let deployment = test_support::deploy_sample_definitions(&service);

    assert_eq!(
        deployment.resource_names,
        vec![
            "order-published.event",
            "order-received.event",
            "orders-inbound.channel",
            "orders-outbound.channel"
        ]
    );
}

#[test]
fn deployment_query_filters_canonical_metadata() {
    let service = service("event-registry-deployment-metadata");

    service
        .deploy(EventRegistryDeploymentRequest {
            name: "Tenant deployment".to_string(),
            category: Some("orders".to_string()),
            parent_deployment_id: Some("parent-orders".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "orders-inbound.channel".to_string(),
                resource: json!({
                    "key": "ordersInbound",
                    "name": "Orders inbound",
                    "channelType": "inbound",
                    "resourceName": "orders-inbound.channel",
                    "type": "in-memory"
                })
                .to_string(),
            }],
        })
        .unwrap();
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "Global deployment".to_string(),
            category: Some("global".to_string()),
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "orders-outbound.channel".to_string(),
                resource: json!({
                    "key": "ordersOutbound",
                    "name": "Orders outbound",
                    "channelType": "outbound",
                    "resourceName": "orders-outbound.channel",
                    "type": "in-memory"
                })
                .to_string(),
            }],
        })
        .unwrap();

    let tenant_page = service
        .create_deployment_query()
        .category("orders")
        .parent_deployment_id_like("parent-%")
        .tenant_id_like("tenant-%")
        .list_page()
        .unwrap();
    assert_eq!(tenant_page.total, 1);
    assert_eq!(tenant_page.data[0].name, "Tenant deployment");

    let without_tenant = service
        .create_deployment_query()
        .category_not_equals("orders")
        .without_tenant_id()
        .list_page()
        .unwrap();
    assert_eq!(without_tenant.total, 1);
    assert_eq!(without_tenant.data[0].name, "Global deployment");
}

#[test]
fn deployment_rejects_unsupported_channel_implementations() {
    let service = service("event-registry-deployment-errors");

    let error = service
        .deploy(EventRegistryDeploymentRequest {
            name: "Unsupported deployment".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "orders-http.channel".to_string(),
                resource: json!({
                    "key": "ordersHttp",
                    "name": "Orders http",
                    "channelType": "inbound",
                    "resourceName": "orders-http.channel",
                    "type": "http"
                })
                .to_string(),
            }],
        })
        .unwrap_err();

    match error {
        FlowableError::DeploymentValidationError(message)
        | FlowableError::ExecutionError(message)
        | FlowableError::Generic(message) => assert!(message.contains("http")),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn deployment_versions_redeployed_channel_keys_per_tenant() {
    let service = service("event-registry-channel-versioning");

    service
        .deploy(EventRegistryDeploymentRequest {
            name: "Tenant channel v1".to_string(),
            category: Some("orders-v1".to_string()),
            parent_deployment_id: Some("parent-orders".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "orders-inbound-v1.channel".to_string(),
                resource: json!({
                    "key": "ordersInbound",
                    "name": "Orders inbound v1",
                    "channelType": "inbound",
                    "resourceName": "orders-inbound-v1.channel",
                    "type": "in-memory"
                })
                .to_string(),
            }],
        })
        .unwrap();
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "Tenant channel v2".to_string(),
            category: Some("orders-v2".to_string()),
            parent_deployment_id: Some("parent-orders".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "orders-inbound-v2.channel".to_string(),
                resource: json!({
                    "key": "ordersInbound",
                    "name": "Orders inbound v2",
                    "channelType": "inbound",
                    "resourceName": "orders-inbound-v2.channel",
                    "type": "in-memory"
                })
                .to_string(),
            }],
        })
        .unwrap();
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "Global channel".to_string(),
            category: Some("global".to_string()),
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "orders-inbound-global.channel".to_string(),
                resource: json!({
                    "key": "ordersInbound",
                    "name": "Orders inbound global",
                    "channelType": "inbound",
                    "resourceName": "orders-inbound-global.channel",
                    "type": "in-memory"
                })
                .to_string(),
            }],
        })
        .unwrap();

    let channels = service
        .create_channel_definition_query()
        .key("ordersInbound")
        .order_by("version", false)
        .list()
        .unwrap();
    assert_eq!(channels.len(), 3);
    assert_eq!(
        channels
            .iter()
            .filter(|definition| definition.tenant_id.as_deref() == Some("tenant-a"))
            .map(|definition| definition.version)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let global = channels
        .iter()
        .find(|definition| definition.tenant_id.is_none())
        .unwrap();
    assert_eq!(global.version, 1);
}

#[test]
fn deployment_still_rejects_duplicate_channel_keys_in_one_deployment() {
    let service = service("event-registry-duplicate-channel-keys");

    let error = service
        .deploy(EventRegistryDeploymentRequest {
            name: "Duplicate channels".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "orders-inbound-a.channel".to_string(),
                    resource: json!({
                        "key": "ordersInbound",
                        "name": "Orders inbound A",
                        "channelType": "inbound",
                        "resourceName": "orders-inbound-a.channel",
                        "type": "in-memory"
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "orders-inbound-b.channel".to_string(),
                    resource: json!({
                        "key": "ordersInbound",
                        "name": "Orders inbound B",
                        "channelType": "inbound",
                        "resourceName": "orders-inbound-b.channel",
                        "type": "in-memory"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap_err();

    match error {
        FlowableError::DeploymentValidationError(message) => {
            assert!(message.contains("more than once in deployment"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
