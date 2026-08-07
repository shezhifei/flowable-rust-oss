mod test_support;

use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{EventDirection, EventInstanceStatus, OutboundEventRequest};
use serde_json::json;
use test_support::{deploy_sample_definitions, service};

#[test]
fn outbound_processing_persists_delivery_and_management_queries_are_deterministic() {
    let service = service("outbound-event-processing");
    deploy_sample_definitions(&service);

    let delivery = service
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "A-100" }),
            tenant_id: None,
        })
        .unwrap();

    assert_eq!(delivery.direction, EventDirection::Outbound);
    assert_eq!(delivery.status, EventInstanceStatus::Published);
    assert_eq!(
        delivery.status_history,
        vec![EventInstanceStatus::Created, EventInstanceStatus::Published]
    );

    let filtered = service
        .create_event_instance_delivery_query()
        .direction(EventDirection::Outbound)
        .status(EventInstanceStatus::Published)
        .page(0, 10)
        .list_page()
        .unwrap();

    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.data[0].id, delivery.id);
}

#[test]
fn outbound_processing_rejects_unsupported_management_filters_structurally() {
    let service = service("outbound-event-processing-errors");
    deploy_sample_definitions(&service);

    let error = service
        .create_event_instance_delivery_query()
        .unsupported_filter("eventDefinitionKey", "orderPublished")
        .list_page()
        .unwrap_err();

    match error {
        FlowableError::ExecutionError(message) | FlowableError::Generic(message) => {
            assert!(message.contains("eventDefinitionKey"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
