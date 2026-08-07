mod test_support;

use flowable_event_registry_service::{
    EventDirection, EventInstanceRequest, EventInstanceStatus, EventPayloadValidator,
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, FlowableEventRegistryService,
    OutboundEventRequest,
};
use serde_json::{Value, json};
use test_support::service;

fn deploy_strict_payload_definitions(service: &FlowableEventRegistryService) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "Strict payload deployment".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "strict-order-published.event".to_string(),
                    resource: json!({
                        "key": "strictOrderPublished",
                        "name": "Strict order published",
                        "eventType": "strict.order.published",
                        "channelKey": "strictOrdersOutbound",
                        "resourceName": "strict-order-published.event",
                        "payload": [
                            { "name": "orderId", "type": "string", "required": true },
                            { "name": "amount", "type": "double", "required": true }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "strict-orders-outbound.channel".to_string(),
                    resource: json!({
                        "key": "strictOrdersOutbound",
                        "name": "Strict orders outbound",
                        "channelType": "outbound",
                        "resourceName": "strict-orders-outbound.channel",
                        "type": "in-memory",
                        "destination": "strict-orders-outbound",
                        "serializerType": "json"
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "strict-order-received.event".to_string(),
                    resource: json!({
                        "key": "strictOrderReceived",
                        "name": "Strict order received",
                        "eventType": "strict.order.received",
                        "channelKey": "strictOrdersInbound",
                        "resourceName": "strict-order-received.event",
                        "payload": [
                            { "name": "orderId", "type": "string", "required": true },
                            { "name": "amount", "type": "double", "required": true }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "strict-orders-inbound.channel".to_string(),
                    resource: json!({
                        "key": "strictOrdersInbound",
                        "name": "Strict orders inbound",
                        "channelType": "inbound",
                        "resourceName": "strict-orders-inbound.channel",
                        "type": "in-memory",
                        "destination": "strict-orders-inbound",
                        "deserializerType": "json"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
}

fn assert_validator_rejects(
    service: &FlowableEventRegistryService,
    event_definition_key: &str,
    payload: &Value,
) {
    let definition = service
        .create_event_definition_query()
        .key(event_definition_key)
        .list()
        .unwrap()
        .pop()
        .unwrap();

    assert!(
        EventPayloadValidator
            .validate(&definition, payload)
            .is_err(),
        "test setup must use a payload rejected by EventPayloadValidator"
    );
}

#[test]
fn outbound_publish_rejects_payloads_that_do_not_match_event_schema() {
    let service = service("outbound-event-payload-validation");
    deploy_strict_payload_definitions(&service);
    let invalid_payload = json!({
        "orderId": 42,
        "amount": "19.99"
    });

    assert_validator_rejects(&service, "strictOrderPublished", &invalid_payload);

    let result = service.publish_outbound_event(OutboundEventRequest {
        event_definition_key: "strictOrderPublished".to_string(),
        event_payload: invalid_payload,
        tenant_id: None,
    });

    assert!(
        result.is_err(),
        "invalid outbound payload should be rejected before a successful delivery is persisted; got {result:?}"
    );

    let deliveries = service
        .create_event_instance_delivery_query()
        .direction(EventDirection::Outbound)
        .status(EventInstanceStatus::Published)
        .event_type("strict.order.published")
        .list_page()
        .unwrap();
    assert_eq!(deliveries.total, 0);
}

#[test]
fn receive_event_instance_rejects_payloads_that_do_not_match_event_schema() {
    let service = service("receive-event-instance-payload-validation");
    deploy_strict_payload_definitions(&service);
    let invalid_payload = json!({
        "orderId": "A-100"
    });

    assert_validator_rejects(&service, "strictOrderReceived", &invalid_payload);

    let result = service.receive_event_instance(EventInstanceRequest {
        event_definition_id: None,
        event_definition_key: Some("strictOrderReceived".to_string()),
        channel_definition_id: None,
        channel_definition_key: Some("strictOrdersInbound".to_string()),
        event_payload: invalid_payload,
        tenant_id: None,
    });

    assert!(
        result.is_err(),
        "invalid inbound event instance payload should be rejected before a successful delivery is persisted; got {result:?}"
    );

    let deliveries = service
        .create_event_instance_delivery_query()
        .direction(EventDirection::Inbound)
        .status(EventInstanceStatus::Processed)
        .event_type("strict.order.received")
        .list_page()
        .unwrap();
    assert_eq!(deliveries.total, 0);
}
