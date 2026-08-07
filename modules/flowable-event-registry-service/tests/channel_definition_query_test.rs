mod test_support;

use flowable_engine::error::FlowableError;
use test_support::{deploy_sample_definitions, service};

#[test]
fn channel_definition_query_returns_deterministic_results_and_supported_filters() {
    let service = service("channel-definition-query");
    let deployment = deploy_sample_definitions(&service);

    let page = service
        .create_channel_definition_query()
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
        vec!["ordersInbound", "ordersOutbound"]
    );
    assert!(
        page.data
            .iter()
            .all(|item| item.deployment_id == deployment.id)
    );

    let inbound_only = service
        .create_channel_definition_query()
        .channel_type("inbound")
        .list()
        .unwrap();
    assert_eq!(inbound_only.len(), 1);
    assert_eq!(inbound_only[0].key, "ordersInbound");

    let by_name = service
        .create_channel_definition_query()
        .name("Orders outbound")
        .list()
        .unwrap();
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].resource_name, "orders-outbound.channel");
}

#[test]
fn channel_definition_query_rejects_unsupported_filters_structurally() {
    let service = service("channel-definition-query-errors");
    deploy_sample_definitions(&service);

    let error = service
        .create_channel_definition_query()
        .unsupported_filter("unsupportedField", "value")
        .list_page()
        .unwrap_err();

    match error {
        FlowableError::ExecutionError(message) | FlowableError::Generic(message) => {
            assert!(message.contains("unsupportedField"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn channel_definition_query_supports_metadata_version_and_latest() {
    let service = service("channel-definition-query-metadata");

    service
        .deploy(
            flowable_event_registry_service::EventRegistryDeploymentRequest {
                name: "Tenant deployment v1".to_string(),
                category: Some("orders-v1".to_string()),
                parent_deployment_id: Some("parent-orders".to_string()),
                tenant_id: Some("tenant-a".to_string()),
                resources: vec![
                    flowable_event_registry_service::EventRegistryDeploymentResource {
                        resource_name: "orders-inbound-v1.channel".to_string(),
                        resource: serde_json::json!({
                            "key": "ordersInbound",
                            "name": "Orders inbound v1",
                            "channelType": "inbound",
                            "resourceName": "orders-inbound-v1.channel",
                            "type": "in-memory"
                        })
                        .to_string(),
                    },
                ],
            },
        )
        .unwrap();
    service
        .deploy(
            flowable_event_registry_service::EventRegistryDeploymentRequest {
                name: "Tenant deployment v2".to_string(),
                category: Some("orders-v2".to_string()),
                parent_deployment_id: Some("parent-orders".to_string()),
                tenant_id: Some("tenant-a".to_string()),
                resources: vec![
                    flowable_event_registry_service::EventRegistryDeploymentResource {
                        resource_name: "orders-inbound-v2.channel".to_string(),
                        resource: serde_json::json!({
                            "key": "ordersInbound",
                            "name": "Orders inbound v2",
                            "channelType": "inbound",
                            "resourceName": "orders-inbound-v2.channel",
                            "type": "in-memory"
                        })
                        .to_string(),
                    },
                ],
            },
        )
        .unwrap();
    service
        .deploy(
            flowable_event_registry_service::EventRegistryDeploymentRequest {
                name: "Global deployment".to_string(),
                category: Some("global".to_string()),
                parent_deployment_id: None,
                tenant_id: None,
                resources: vec![
                    flowable_event_registry_service::EventRegistryDeploymentResource {
                        resource_name: "orders-inbound-global.channel".to_string(),
                        resource: serde_json::json!({
                            "key": "ordersInbound",
                            "name": "Orders inbound global",
                            "channelType": "inbound",
                            "resourceName": "orders-inbound-global.channel",
                            "type": "in-memory"
                        })
                        .to_string(),
                    },
                ],
            },
        )
        .unwrap();

    let version_one = service
        .create_channel_definition_query()
        .key("ordersInbound")
        .tenant_id("tenant-a")
        .version(1)
        .list()
        .unwrap();
    assert_eq!(version_one.len(), 1);
    assert_eq!(version_one[0].category.as_deref(), Some("orders-v1"));

    let latest = service
        .create_channel_definition_query()
        .key("ordersInbound")
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
    assert_eq!(tenant_latest.name, "Orders inbound v2");
    let global_latest = latest
        .iter()
        .find(|definition| definition.tenant_id.is_none())
        .unwrap();
    assert_eq!(global_latest.version, 1);

    let metadata = service
        .create_channel_definition_query()
        .category_like("orders-%")
        .category_not_equals("orders-v1")
        .parent_deployment_id("parent-orders")
        .tenant_id_like("tenant-%")
        .order_by("category", false)
        .list()
        .unwrap();
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata[0].category.as_deref(), Some("orders-v2"));
}
