//! P134 2a: send-event triggerable `triggerEventCorrelationParameter` configuration.
//!
//! Java: `SendEventTaskActivityBehavior.java:140`
//! (`CorrelationUtil.getCorrelationKey(ELEMENT_TRIGGER_EVENT_CORRELATION_PARAMETER, …)`).
//!
//! Behavior flip: models with `triggerEventCorrelationParameter` now match by key
//! instead of broadcasting on eventType alone. Models without the extension keep
//! `configuration = None` (broadcast) — regression.

use flowable_engine::bpmn::event_registry_correlation::generate_correlation_key;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::RuntimeEventWaitKind;
use flowable_event_registry_service::{
    EventInstanceStatus, EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
    FlowableEventRegistryService, InboundRawEvent,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

fn deploy_inbound_order_accepted(service: &FlowableEventRegistryService, channel: &str) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("p134-inbound-{channel}"),
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
                        "channelKey": channel,
                        "resourceName": "orderAccepted.event",
                        "payload": [
                            { "name": "customerId", "type": "string" },
                            { "name": "acceptedBy", "type": "string" }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{channel}.channel"),
                    resource: json!({
                        "key": channel,
                        "name": channel,
                        "channelType": "inbound",
                        "resourceName": format!("{channel}.channel"),
                        "type": "in-memory",
                        "destination": channel,
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
}

fn deploy_outbound_order_published(service: &FlowableEventRegistryService) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "p134-outbound".to_string(),
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
}

fn send_event_xml_with_trigger_correlation(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="{process_id}" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="sendEventTask" />
            <serviceTask id="sendEventTask"
                         flowable:type="send-event"
                         flowable:triggerable="true">
                <extensionElements>
                    <flowable:eventType>orderPublished</flowable:eventType>
                    <flowable:triggerEventType>orderAccepted</flowable:triggerEventType>
                    <flowable:triggerEventCorrelationParameter name="customerId" value="${{customerId}}"/>
                    <flowable:eventInParameter sourceExpression="${{orderId}}" target="orderId" />
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="sendEventTask" targetRef="afterTask" />
            <userTask id="afterTask" />
            <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

fn send_event_xml_without_trigger_correlation(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="{process_id}" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="sendEventTask" />
            <serviceTask id="sendEventTask"
                         flowable:type="send-event"
                         flowable:triggerable="true">
                <extensionElements>
                    <flowable:eventType>orderPublished</flowable:eventType>
                    <flowable:triggerEventType>orderAccepted</flowable:triggerEventType>
                    <flowable:eventInParameter sourceExpression="${{orderId}}" target="orderId" />
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="sendEventTask" targetRef="afterTask" />
            <userTask id="afterTask" />
            <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

fn expected_customer_key(customer_id: &str) -> String {
    let mut params = BTreeMap::new();
    params.insert("customerId".to_string(), Some(customer_id.to_string()));
    generate_correlation_key(&params)
}

/// Flip assertion: same eventType, different correlation key → no delivery.
/// Same key → deliver. Java SendEventTaskActivityBehavior.java:140 +
/// BaseEventRegistryEventConsumer.findEventSubscriptions:163-174.
#[test]
fn send_event_trigger_correlation_key_match_and_miss() {
    let engine = Arc::new(ProcessEngine::new("p134-send-corr".to_string()));
    let service = FlowableEventRegistryService::with_bpmn_consumer(Arc::clone(&engine));
    deploy_outbound_order_published(&service);
    deploy_inbound_order_accepted(&service, "ordersInboundCorr");

    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string(
                "p134_send_corr.bpmn20.xml".to_string(),
                send_event_xml_with_trigger_correlation("p134SendCorr"),
            ),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();

    let kermit = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id.clone())
                .variable("orderId".to_string(), json!("ORD-1"))
                .variable("customerId".to_string(), json!("kermit")),
        )
        .unwrap();
    let gonzo = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("orderId".to_string(), json!("ORD-2"))
                .variable("customerId".to_string(), json!("gonzo")),
        )
        .unwrap();

    // Wait-state configuration must be the evaluated trigger correlation key.
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let kermit_waits =
            store.find_event_wait_states_by_process_instance_id(&kermit.id, &mut session);
        assert_eq!(kermit_waits.len(), 1);
        assert_eq!(kermit_waits[0].wait_kind, RuntimeEventWaitKind::SendEventTask);
        assert_eq!(
            kermit_waits[0].configuration.as_deref(),
            Some(expected_customer_key("kermit").as_str()),
            "triggerable wait must store triggerEventCorrelationParameter key"
        );
    }

    // Wrong key: neither instance advances.
    let miss = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInboundCorr".to_string(),
            body: json!({ "customerId": "fozzie", "acceptedBy": "x" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();
    assert_eq!(miss.status, EventInstanceStatus::Processed);

    let task_service = engine.get_task_service();
    assert!(
        task_service
            .get_tasks_by_process_instance_id(kermit.id.clone())
            .unwrap()
            .is_empty(),
        "wrong correlation key must not deliver to kermit"
    );
    assert!(
        task_service
            .get_tasks_by_process_instance_id(gonzo.id.clone())
            .unwrap()
            .is_empty(),
        "wrong correlation key must not deliver to gonzo"
    );

    // Matching kermit only.
    service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInboundCorr".to_string(),
            body: json!({ "customerId": "kermit", "acceptedBy": "alice" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();
    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(kermit.id.clone())
            .unwrap()
            .len(),
        1,
        "matching key must deliver to kermit"
    );
    assert!(
        task_service
            .get_tasks_by_process_instance_id(gonzo.id.clone())
            .unwrap()
            .is_empty(),
        "kermit event must not deliver to gonzo"
    );

    // Matching gonzo.
    service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInboundCorr".to_string(),
            body: json!({ "customerId": "gonzo", "acceptedBy": "bob" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();
    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(gonzo.id.clone())
            .unwrap()
            .len(),
        1,
        "matching key must deliver to gonzo"
    );
}

/// Regression: no `triggerEventCorrelationParameter` → configuration stays None
/// (broadcast on eventType), same as pre-P134.
#[test]
fn send_event_without_trigger_correlation_still_broadcasts() {
    let engine = Arc::new(ProcessEngine::new("p134-send-no-corr".to_string()));
    let service = FlowableEventRegistryService::with_bpmn_consumer(Arc::clone(&engine));
    deploy_outbound_order_published(&service);
    deploy_inbound_order_accepted(&service, "ordersInboundNoCorr");

    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string(
                "p134_send_no_corr.bpmn20.xml".to_string(),
                send_event_xml_without_trigger_correlation("p134SendNoCorr"),
            ),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("orderId".to_string(), json!("ORD-X")),
        )
        .unwrap();

    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let waits = store.find_event_wait_states_by_process_instance_id(&pi.id, &mut session);
        assert_eq!(waits.len(), 1);
        assert_eq!(
            waits[0].configuration, None,
            "no triggerEventCorrelationParameter → configuration stays None"
        );
    }

    service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "ordersInboundNoCorr".to_string(),
            body: json!({ "customerId": "anyone", "acceptedBy": "alice" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id)
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "null configuration must still broadcast-match any payload"
    );
}
