mod test_support;

use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{EventDirection, EventInstanceStatus, InboundEventRequest};
use serde_json::json;
use test_support::{deploy_sample_definitions, service};

#[test]
fn inbound_processing_persists_runtime_delivery_with_stable_status_transition() {
    let service = service("inbound-event-processing");
    deploy_sample_definitions(&service);

    let delivery = service
        .receive_inbound_event(InboundEventRequest {
            event_type: "order.received".to_string(),
            event_payload: json!({ "orderId": "A-100" }),
            tenant_id: None,
        })
        .unwrap();

    assert_eq!(delivery.direction, EventDirection::Inbound);
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(
        delivery.status_history,
        vec![
            EventInstanceStatus::Received,
            EventInstanceStatus::Processed
        ]
    );

    let persisted = service.get_event_instance_delivery(&delivery.id).unwrap();
    assert_eq!(persisted, delivery);
}

#[test]
fn inbound_processing_rejects_events_bound_to_non_inbound_channels() {
    let service = service("inbound-event-processing-errors");
    deploy_sample_definitions(&service);

    let error = service
        .receive_inbound_event(InboundEventRequest {
            event_type: "order.published".to_string(),
            event_payload: json!({ "orderId": "wrong-channel" }),
            tenant_id: None,
        })
        .unwrap_err();

    match error {
        FlowableError::BadRequest(message) => assert!(message.contains("inbound")),
        other => panic!("unexpected error: {other:?}"),
    }
}
