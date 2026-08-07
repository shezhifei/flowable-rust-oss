mod test_support;

use flowable_content_service::{CreateContentItemRequest, FlowableContentService};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_event_registry_service::{
    FlowableEventRegistryService, InboundEventRequest, OutboundEventRequest,
};
use flowable_form_service::{FlowableFormService, FormDeploymentRequest, FormDeploymentResource};
use serde_json::json;
use std::sync::Arc;

#[test]
fn event_registry_coexists_with_owned_m14_forms_and_content_slice() {
    let engine = Arc::new(ProcessEngine::new("m14-event-registry".to_string()));
    let event_registry = FlowableEventRegistryService::new(Arc::clone(&engine));
    let form_service = FlowableFormService::new(Arc::clone(&engine));
    let content_service = FlowableContentService::new(Arc::clone(&engine));

    test_support::deploy_sample_definitions(&event_registry);

    let form_deployment = form_service
        .deploy(FormDeploymentRequest {
            name: "M14 Forms".to_string(),
            resources: vec![FormDeploymentResource {
                resource_name: "travel-request.form".to_string(),
                resource: json!({
                    "key": "travelRequest",
                    "name": "Travel request",
                    "resourceName": "travel-request.form",
                    "fields": [
                        { "id": "employee", "type": "text" }
                    ]
                })
                .to_string(),
            }],
        })
        .unwrap();
    assert_eq!(form_deployment.resource_names, vec!["travel-request.form"]);

    let content_item = content_service
        .create_content_item(CreateContentItemRequest {
            name: "travel-request.json".to_string(),
            mime_type: Some("application/json".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("{\"approved\":true}".to_string()),
            task_id: None,
            process_instance_id: Some("process-42".to_string()),
            scope_type: Some("eventRegistry".to_string()),
            scope_id: Some("ordersOutbound".to_string()),
            created_by: Some("m14".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();
    assert_eq!(
        content_item.process_instance_id.as_deref(),
        Some("process-42")
    );

    let outbound = event_registry
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({
                "orderId": "ORD-42",
                "formDefinitionKey": "travelRequest",
                "contentItemId": content_item.id,
            }),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(outbound.channel_key, "ordersOutbound");

    let inbound = event_registry
        .receive_inbound_event(InboundEventRequest {
            event_type: "order.received".to_string(),
            event_payload: json!({
                "orderId": "ORD-42",
                "formDefinitionKey": "travelRequest"
            }),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(inbound.channel_key, "ordersInbound");

    let forms = form_service
        .create_form_definition_query()
        .key("travelRequest")
        .list()
        .unwrap();
    assert_eq!(forms.len(), 1);

    let content_items = content_service
        .create_content_item_query()
        .scope_id("ordersOutbound")
        .list()
        .unwrap();
    assert_eq!(content_items.len(), 1);
}
