//! P134 2b: inbound send-event trigger must not create a second delivery row.
//!
//! Pre-fix: pipeline inserts Received→Processed, and send-event trigger
//! inserted another event-instance:Uuid row (Received→Processed). Java has one
//! delivery per logical inbound event. Fix: consumer passes delivery id;
//! trigger updates that row; pipeline still advances status to Processed.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::EventRegistryEventDirection;
use flowable_event_registry_service::{
    EventInstanceStatus, EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
    FlowableEventRegistryService, InboundRawEvent,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[test]
fn inbound_send_event_keeps_single_delivery_record() {
    let engine = Arc::new(ProcessEngine::new("p134-single-delivery".to_string()));
    let service = FlowableEventRegistryService::with_bpmn_consumer(Arc::clone(&engine));

    service
        .deploy(EventRegistryDeploymentRequest {
            name: "p134-out".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "orderPublished.event".to_string(),
                    resource: json!({
                        "key": "orderPublished",
                        "name": "Order published",
                        "eventType": "order.published",
                        "channelKey": "ordersOutbound",
                        "resourceName": "orderPublished.event",
                        "payload": [{ "name": "orderId", "type": "string" }]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "ordersOutbound.channel".to_string(),
                    resource: json!({
                        "key": "ordersOutbound",
                        "name": "ordersOutbound",
                        "channelType": "outbound",
                        "resourceName": "ordersOutbound.channel",
                        "type": "in-memory",
                        "destination": "ordersOutbound"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();

    service
        .deploy(EventRegistryDeploymentRequest {
            name: "p134-in".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "orderAccepted.event".to_string(),
                    resource: json!({
                        "key": "orderAccepted",
                        "name": "Order accepted",
                        "eventType": "order.accepted",
                        "channelKey": "ordersInboundSingle",
                        "resourceName": "orderAccepted.event",
                        "payload": [
                            { "name": "acceptedBy", "type": "string" }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "ordersInboundSingle.channel".to_string(),
                    resource: json!({
                        "key": "ordersInboundSingle",
                        "name": "ordersInboundSingle",
                        "channelType": "inbound",
                        "resourceName": "ordersInboundSingle.channel",
                        "type": "in-memory",
                        "destination": "ordersInboundSingle",
                        "payloadExtractor": "json",
                        "filter": "default",
                        "tenantDetector": "default",
                        "transformer": "default",
                        "keyDetector": "default",
                        "consumer": "default",
                        "fixedEventKey": "orderAccepted"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="p134SingleDelivery" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="sendEventTask" />
            <serviceTask id="sendEventTask" flowable:type="send-event" flowable:triggerable="true">
                <extensionElements>
                    <flowable:eventType>orderPublished</flowable:eventType>
                    <flowable:triggerEventType>orderAccepted</flowable:triggerEventType>
                    <flowable:eventInParameter sourceExpression="${orderId}" target="orderId" />
                    <flowable:eventOutParameter source="acceptedBy" target="acceptedBy" />
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="f2" sourceRef="sendEventTask" targetRef="afterTask" />
            <userTask id="afterTask" />
            <sequenceFlow id="f3" sourceRef="afterTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p134_single_delivery.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("orderId".to_string(), json!("ORD-SINGLE")),
        )
        .unwrap();

    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInboundSingle".to_string(),
            body: json!({ "acceptedBy": "Nina" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(delivery.event_definition_key, "orderAccepted");

    // Exactly one inbound delivery for this event key (no dual-record).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let inbound: Vec<_> = store
        .list_event_registry_event_instance_deliveries(&mut session)
        .unwrap()
        .into_iter()
        .filter(|d| {
            d.direction == EventRegistryEventDirection::Inbound
                && d.event_definition_key == "orderAccepted"
        })
        .collect();
    assert_eq!(
        inbound.len(),
        1,
        "pipeline + send-event trigger must share one delivery, got {:?}",
        inbound
            .iter()
            .map(|d| (&d.id, &d.status))
            .collect::<Vec<_>>()
    );
    assert_eq!(inbound[0].id, delivery.id);
    assert_eq!(
        inbound[0].status,
        flowable_engine::persistence::runtime_store::EventRegistryEventInstanceStatus::Processed
    );
    assert_eq!(
        inbound[0].status_history,
        vec![
            flowable_engine::persistence::runtime_store::EventRegistryEventInstanceStatus::Received,
            flowable_engine::persistence::runtime_store::EventRegistryEventInstanceStatus::Processed,
        ]
    );

    // Trigger consumed the wait and mapped out params.
    let waits = engine
        .get_runtime_service()
        .get_event_wait_states_by_process_instance_id(pi.id.clone());
    assert!(waits.is_empty());
    let accepted = engine
        .get_runtime_service()
        .get_variable(pi.id, "acceptedBy".to_string())
        .unwrap();
    assert_eq!(accepted, Some(json!("Nina")));
}
