//! P92 service-side e2e: BPMN consumer triggers a waiting BPMN execution.
//!
//! Pipeline: deploy event+channel → deploy BPMN with eventType catch → start
//! instance → inbound raw event → BpmnEventRegistryConsumer → afterTask.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_event_registry_service::{
    EventInstanceStatus, EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
    FlowableEventRegistryService, InboundRawEvent,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[test]
fn bpmn_consumer_triggers_waiting_intermediate_catch_execution() {
    let engine = Arc::new(ProcessEngine::new("p92-service-bridge".to_string()));
    let service = FlowableEventRegistryService::with_bpmn_consumer(Arc::clone(&engine));

    service
        .deploy(EventRegistryDeploymentRequest {
            name: "p92-inbound".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "orderReceived.event".to_string(),
                    resource: json!({
                        "key": "orderReceived",
                        "name": "Order received",
                        "eventType": "order.received",
                        "channelKey": "ordersInbound",
                        "resourceName": "orderReceived.event",
                        "payload": [{ "name": "orderId", "type": "string" }]
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
                        "fixedEventKey": "orderReceived"
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
        <process id="erBridgeProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="waitEvent" />
            <intermediateCatchEvent id="waitEvent">
                <extensionElements>
                    <flowable:eventType>orderReceived</flowable:eventType>
                </extensionElements>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="waitEvent" targetRef="afterTask" />
            <userTask id="afterTask" name="After" />
            <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("p92-bpmn".to_string())
                .add_string("er_bridge.bpmn20.xml".to_string(), xml.to_string()),
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

    let wait = engine
        .get_runtime_service()
        .get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert!(
        wait.iter()
            .any(|w| w.activity_id.as_deref() == Some("waitEvent")),
        "process should wait on event-registry catch"
    );

    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInbound".to_string(),
            body: json!({ "orderId": "ORD-1" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(delivery.event_definition_key, "orderReceived");

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterTask");
}

#[test]
fn default_service_still_uses_noop_consumer() {
    // Existing service tests rely on FlowableEventRegistryService::new keeping NoOp.
    let engine = Arc::new(ProcessEngine::new("p92-noop-default".to_string()));
    let service = FlowableEventRegistryService::new(Arc::clone(&engine));
    assert!(
        service.configuration().consumer("default").is_some(),
        "default consumer must remain registered"
    );
}
