// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! P1-B contract: a process-start failure for an inbound event-registry event must
//! propagate out of the BPMN consumer and fail the delivery (no ack), so the channel
//! can retry/reject. Java: `BpmnEventRegistryEventConsumer.startProcessInstance`
//! (BpmnEventRegistryEventConsumer.java:228-269) calls `start()` with no try/catch
//! and the caller returns immediately after (:197-198/:205).
//!
//! Before the fix `trigger_process_start` did `let _ = executor.execute(&cmd)`, so a
//! broken start subscription was silently ignored and the delivery was marked
//! Processed (acked) — the inbound message was lost with no retry possible.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::ProcessEventStartSubscription;
use flowable_event_registry_service::{
    EventInstanceStatus, EventRegistryConfiguration, EventRegistryDeploymentRequest,
    EventRegistryDeploymentResource, FlowableEventRegistryService, InboundRawEvent,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

const EVENT_KEY: &str = "p1bStartFailEvt";
const CHANNEL_KEY: &str = "chP1bStartFail";
const MULTI_EVENT_KEY: &str = "p1bMultiStartEvt";
const MULTI_CHANNEL_KEY: &str = "chP1bMulti";
const OK_EVENT_KEY: &str = "p1bStartOkEvt";
const OK_CHANNEL_KEY: &str = "chP1bOk";

fn deploy_event(service: &FlowableEventRegistryService, event_key: &str, channel_key: &str) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("p1b-{event_key}"),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{event_key}.event"),
                    resource: json!({
                        "key": event_key,
                        "name": event_key,
                        "eventType": event_key,
                        "channelKey": channel_key,
                        "resourceName": format!("{event_key}.event"),
                        "payload": [{ "name": "orderId", "type": "string" }],
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{channel_key}.channel"),
                    resource: json!({
                        "key": channel_key,
                        "name": channel_key,
                        "channelType": "inbound",
                        "resourceName": format!("{channel_key}.channel"),
                        "type": "in-memory",
                        "destination": channel_key,
                        "payloadExtractor": "json",
                        "filter": "default",
                        "tenantDetector": "default",
                        "transformer": "default",
                        "keyDetector": "default",
                        "consumer": "default",
                        "fixedEventKey": event_key,
                    })
                    .to_string(),
                },
            ],
        })
        .expect("event deploy");
}

/// Process that fails synchronously on start: the event start event flows straight
/// into a service task whose delegateExpression cannot resolve. Mirrors the Java
/// case where `processInstanceBuilder.start()` throws.
fn failing_process_xml(process_id: &str, event_key: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="{process_id}" isExecutable="true">
            <startEvent id="start">
                <extensionElements>
                    <flowable:eventType>{event_key}</flowable:eventType>
                </extensionElements>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="start" targetRef="brokenSvc" />
            <serviceTask id="brokenSvc" flowable:delegateExpression="${{p1bDelegateThatDoesNotExist}}" />
            <sequenceFlow id="f2" sourceRef="brokenSvc" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

/// Healthy process: starts and parks on a user task.
fn ok_process_xml(process_id: &str, event_key: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="{process_id}" isExecutable="true">
            <startEvent id="start">
                <extensionElements>
                    <flowable:eventType>{event_key}</flowable:eventType>
                </extensionElements>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="start" targetRef="task" />
            <userTask id="task" name="After start" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

fn deploy_process(engine: &ProcessEngine, name: &str, xml: &str) {
    let repository = engine.get_repository_service();
    let builder = repository
        .create_deployment()
        .name(name.to_string())
        .add_string(format!("{name}.bpmn20.xml"), xml.to_string());
    repository.deploy(builder).expect("bpmn deploy");
}

fn service(engine: Arc<ProcessEngine>) -> FlowableEventRegistryService {
    FlowableEventRegistryService::with_bpmn_consumer_config(
        engine,
        EventRegistryConfiguration::builder().build(),
    )
}

fn active_process_instance_keys(engine: &ProcessEngine) -> Vec<String> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut keys: Vec<String> = store
        .snapshot_process_instances(&mut session)
        .into_values()
        .filter(|pi| !pi.is_ended)
        .map(|pi| pi.process_definition_key)
        .collect();
    keys.sort();
    keys
}

fn start_subscriptions_for_event(
    engine: &ProcessEngine,
    event_key: &str,
) -> Vec<ProcessEventStartSubscription> {
    let mut subs: Vec<_> = engine
        .get_event_start_subscriptions()
        .unwrap()
        .into_iter()
        .filter(|sub| sub.event_ref == event_key)
        .collect();
    // Mirror `trigger_process_start` ordering in bpmn_consumer.rs.
    subs.sort_by(|a, b| {
        a.process_definition_id
            .cmp(&b.process_definition_id)
            .then(a.start_event_id.cmp(&b.start_event_id))
    });
    subs
}

fn only_delivery(
    service: &FlowableEventRegistryService,
) -> flowable_event_registry_service::EventInstanceDelivery {
    let mut data = service
        .create_event_instance_delivery_query()
        .list_page()
        .unwrap()
        .data;
    assert_eq!(data.len(), 1, "expected exactly one persisted delivery");
    data.pop().unwrap()
}

/// One start subscription whose process start fails: the delivery returns Err,
/// is persisted as Failed (NOT Processed/acked), the error carries event +
/// subscription identity and the underlying engine error, and no process instance
/// survives the rolled-back start.
#[test]
fn failing_start_subscription_fails_delivery_with_context() {
    let engine = Arc::new(ProcessEngine::new("p1b-start-fail-single".into()).unwrap());
    let service = service(Arc::clone(&engine));
    deploy_event(&service, EVENT_KEY, CHANNEL_KEY);
    deploy_process(
        &engine,
        "p1bFailStart",
        &failing_process_xml("p1bFailStartProc", EVENT_KEY),
    );

    let sub = start_subscriptions_for_event(&engine, EVENT_KEY)
        .pop()
        .expect("one event start subscription");

    let error = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: CHANNEL_KEY.to_string(),
            body: json!({ "orderId": "1" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .expect_err("a failed process start must fail the inbound delivery");

    let message = error.to_string();
    assert!(
        message.contains(EVENT_KEY),
        "error must name the event definition key, got: {message}"
    );
    assert!(
        message.contains(&sub.process_definition_id),
        "error must name the process definition id of the failing subscription, got: {message}"
    );
    assert!(
        message.contains("startEventId='start'"),
        "error must name the failing start event id, got: {message}"
    );
    assert!(
        message.contains("Unknown property"),
        "error must carry the underlying engine cause (JuelExpression-aligned strict EL: Unknown property used in expression), got: {message}"
    );

    // Not acked: the channel boundary sees Err and the delivery row is Failed with
    // the same diagnostic context recorded for retry/observability.
    let delivery = only_delivery(&service);
    assert_eq!(
        delivery.status,
        EventInstanceStatus::Failed,
        "delivery must not be acked as Processed when process start fails"
    );
    let last_error = delivery.last_error.expect("failed delivery records error");
    assert!(last_error.contains(EVENT_KEY));
    assert!(last_error.contains(&sub.process_definition_id));
    assert!(
        last_error.contains("Unknown property"),
        "recorded last_error must carry the engine cause, got: {last_error}"
    );

    // The failed start is rolled back: no orphan process instance.
    assert!(
        active_process_instance_keys(&engine).is_empty(),
        "a failed start must not leave a committed process instance"
    );
}

/// Two matching start subscriptions, one healthy and one broken: the first error
/// aborts the delivery (Java returns immediately after the failing start), the
/// delivery is Failed, and subscriptions ordered after the failure are never
/// started.
#[test]
fn multiple_start_subscriptions_abort_on_first_error() {
    let engine = Arc::new(ProcessEngine::new("p1b-start-fail-multi".into()).unwrap());
    let service = service(Arc::clone(&engine));
    deploy_event(&service, MULTI_EVENT_KEY, MULTI_CHANNEL_KEY);
    deploy_process(
        &engine,
        "p1bMultiOk",
        &ok_process_xml("p1bMultiOkProc", MULTI_EVENT_KEY),
    );
    deploy_process(
        &engine,
        "p1bMultiFail",
        &failing_process_xml("p1bMultiFailProc", MULTI_EVENT_KEY),
    );

    let subs = start_subscriptions_for_event(&engine, MULTI_EVENT_KEY);
    assert_eq!(subs.len(), 2, "both process subscriptions match the event");
    // The consumer sorts by (processDefinitionId, startEventId); definition ids
    // end in random UUIDs, so derive the expected ordering instead of assuming it.
    let failing_is_first = subs[0].process_definition_key == "p1bMultiFailProc";

    let error = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: MULTI_CHANNEL_KEY.to_string(),
            body: json!({ "orderId": "1" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .expect_err("a failing subscription must fail the whole delivery");
    assert!(
        error.to_string().contains(MULTI_EVENT_KEY),
        "error must identify the event, got: {}",
        error
    );

    let delivery = only_delivery(&service);
    assert_eq!(delivery.status, EventInstanceStatus::Failed);

    let active = active_process_instance_keys(&engine);
    if failing_is_first {
        // Fail-fast: the healthy subscription ordered after the failure never runs.
        assert!(
            active.is_empty(),
            "expected no started instances when the broken subscription sorts first, got {active:?}"
        );
    } else {
        // Healthy subscription ran first, then the broken one aborted the delivery:
        // still an overall failure (no ack), and never two instances.
        assert_eq!(
            active,
            vec!["p1bMultiOkProc".to_string()],
            "the subscription after the failing one must not start"
        );
    }
}

/// Success path unchanged: a healthy start subscription delivers, starts one
/// instance and marks the delivery Processed.
#[test]
fn successful_start_subscription_still_processed() {
    let engine = Arc::new(ProcessEngine::new("p1b-start-ok".into()).unwrap());
    let service = service(Arc::clone(&engine));
    deploy_event(&service, OK_EVENT_KEY, OK_CHANNEL_KEY);
    deploy_process(
        &engine,
        "p1bOkStart",
        &ok_process_xml("p1bOkStartProc", OK_EVENT_KEY),
    );

    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: OK_CHANNEL_KEY.to_string(),
            body: json!({ "orderId": "1" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .expect("healthy start subscription delivers");
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(
        active_process_instance_keys(&engine),
        vec!["p1bOkStartProc".to_string()]
    );
}
