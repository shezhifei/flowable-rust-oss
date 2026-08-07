//! P130 e2e: send-event triggerable wait → inbound consumer → out parameters.
//!
//! Java: `BpmnEventRegistryEventConsumer` → `runtimeService.trigger` →
//! `SendEventTaskActivityBehavior#trigger` (`SendEventTaskActivityBehavior.java:230-265`)
//! + `EventInstanceBpmnUtil.handleEventInstanceOutParameters` (:122-134).

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::{
    EventSubscriptionKind, RuntimeEventWaitKind,
};
use flowable_event_registry_service::{
    EventInstanceStatus, EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
    FlowableEventRegistryService, InboundRawEvent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

#[test]
fn bpmn_consumer_routes_send_event_task_and_maps_out_parameters() {
    let engine = Arc::new(ProcessEngine::new("p130-send-event-e2e".to_string()));
    let service = FlowableEventRegistryService::with_bpmn_consumer(Arc::clone(&engine));

    // Outbound event/channel for send-event execute path.
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "p130-outbound".to_string(),
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

    // Inbound event/channel for trigger path.
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "p130-inbound".to_string(),
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
                        "channelKey": "ordersInbound",
                        "resourceName": "orderAccepted.event",
                        "payload": [
                            { "name": "acceptedBy", "type": "string" },
                            { "name": "orderId", "type": "string" }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "ordersInbound.channel".to_string(),
                    resource: json!({
                        "key": "ordersInbound",
                        "name": "ordersInbound",
                        "channelType": "inbound",
                        "resourceName": "ordersInbound.channel",
                        "type": "in-memory",
                        "destination": "ordersInbound",
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
        <process id="p130SendEventProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="sendEventTask" />
            <serviceTask id="sendEventTask"
                         name="Publish And Await"
                         flowable:type="send-event"
                         flowable:triggerable="true"
                         flowable:resultVariableName="sendEventResult">
                <extensionElements>
                    <flowable:eventType>orderPublished</flowable:eventType>
                    <flowable:triggerEventType>orderAccepted</flowable:triggerEventType>
                    <flowable:eventInParameter sourceExpression="${orderId}" target="orderId" />
                    <flowable:eventOutParameter source="acceptedBy" target="acceptedBy" />
                    <flowable:out source="payload.acceptedBy" target="acceptedByFromResult" />
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="sendEventTask" targetRef="afterTask" />
            <userTask id="afterTask" name="After Send And Receive" />
            <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("p130-bpmn".to_string())
                .add_string("p130_send_event.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let process_definition_id = repository.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("orderId".to_string(), json!("ORD-P130")),
        )
        .unwrap();

    let wait = engine
        .get_runtime_service()
        .get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(wait.len(), 1, "send-event triggerable must register a wait state");
    assert_eq!(wait[0].wait_kind, flowable_engine::engine::task_service::EventWaitKind::SendEventTask);
    assert_eq!(wait[0].event_ref.as_deref(), Some("orderAccepted"));
    assert_eq!(wait[0].activity_id.as_deref(), Some("sendEventTask"));

    // Confirm underlying subscription is EventRegistry (consumer filter key).
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let ws = store
            .find_event_wait_state_by_execution_id(&wait[0].execution_id, &mut session)
            .expect("wait state row");
        assert_eq!(ws.wait_kind, RuntimeEventWaitKind::SendEventTask);
        assert_eq!(
            ws.event_subscription.as_ref().map(|s| s.kind.clone()),
            Some(EventSubscriptionKind::EventRegistry)
        );
    }

    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInbound".to_string(),
            body: json!({ "acceptedBy": "Nina", "orderId": "ORD-P130" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(delivery.event_definition_key, "orderAccepted");

    let wait_after = engine
        .get_runtime_service()
        .get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert!(
        wait_after.is_empty(),
        "inbound consumer must consume the send-event wait state"
    );

    let accepted_by = engine
        .get_runtime_service()
        .get_variable(process_instance.id.clone(), "acceptedBy".to_string())
        .unwrap();
    assert_eq!(accepted_by, Some(json!("Nina")));

    let from_result = engine
        .get_runtime_service()
        .get_variable(
            process_instance.id.clone(),
            "acceptedByFromResult".to_string(),
        )
        .unwrap();
    assert_eq!(from_result, Some(json!("Nina")));

    let result = engine
        .get_runtime_service()
        .get_variable(process_instance.id.clone(), "sendEventResult".to_string())
        .unwrap()
        .expect("resultVariableName must be set on trigger");
    assert_eq!(result["service"], json!("send-event"));
    assert_eq!(result["payload"]["acceptedBy"], json!("Nina"));

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterTask");

    // Inbound delivery recorded (pipeline Processed + optional trigger-path record).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let deliveries = store
        .list_event_registry_event_instance_deliveries(&mut session)
        .unwrap();
    assert!(
        deliveries.iter().any(|d| {
            d.event_definition_key == "orderAccepted"
                && d.status == EventInstanceStatus::Processed
                && matches!(d.payload, Value::Object(ref m) if m.get("acceptedBy") == Some(&json!("Nina")))
        }),
        "inbound delivery for orderAccepted must be recorded as Processed"
    );
    // Outbound from send-event execute also recorded.
    assert!(
        deliveries
            .iter()
            .any(|d| d.event_definition_key == "orderPublished"),
        "outbound orderPublished delivery should exist from send-event execute"
    );
}

/// Missing payload field → process variable null (EventInstanceBpmnUtil.java:127).
#[test]
fn bpmn_consumer_send_event_missing_out_payload_field_writes_null() {
    let engine = Arc::new(ProcessEngine::new("p130-send-event-null".to_string()));
    let service = FlowableEventRegistryService::with_bpmn_consumer(Arc::clone(&engine));

    service
        .deploy(EventRegistryDeploymentRequest {
            name: "p130-null-outbound".to_string(),
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
                        "payload": []
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
            name: "p130-null-inbound".to_string(),
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
                        "channelKey": "ordersInbound",
                        "resourceName": "orderAccepted.event",
                        "payload": [{ "name": "acceptedBy", "type": "string" }]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "ordersInbound.channel".to_string(),
                    resource: json!({
                        "key": "ordersInbound",
                        "name": "ordersInbound",
                        "channelType": "inbound",
                        "resourceName": "ordersInbound.channel",
                        "type": "in-memory",
                        "destination": "ordersInbound",
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
        <process id="p130NullOutProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="sendEventTask" />
            <serviceTask id="sendEventTask" flowable:type="send-event" flowable:triggerable="true">
                <extensionElements>
                    <flowable:eventType>orderPublished</flowable:eventType>
                    <flowable:triggerEventType>orderAccepted</flowable:triggerEventType>
                    <flowable:eventOutParameter source="acceptedBy" target="acceptedBy" />
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="sendEventTask" targetRef="afterTask" />
            <userTask id="afterTask" name="After" />
            <endEvent id="end" />
            <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
        </process>
    </definitions>"#;

    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("p130-null-bpmn".to_string())
                .add_string("p130_null.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let process_definition_id = repository.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInbound".to_string(),
            // acceptedBy intentionally absent
            body: json!({ "note": "no acceptedBy" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();

    let accepted_by = engine
        .get_runtime_service()
        .get_variable(process_instance.id.clone(), "acceptedBy".to_string())
        .unwrap();
    assert_eq!(
        accepted_by,
        Some(Value::Null),
        "missing payload field must map to null (EventInstanceBpmnUtil.java:127)"
    );

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterTask");
}
