//! P74 contract: remaining `DeleteReason` constants on historic activities.
//!
//! Java evidence (checkout `flowable-engine`):
//! - `BOUNDARY_EVENT_INTERRUPTING` → `"boundary event"`; runtime
//!   `BoundaryEventActivityBehavior#deleteChildExecutions` records
//!   `"boundary event (" + boundaryActivityId + ")"` on the interrupted host.
//! - `EVENT_SUBPROCESS_INTERRUPTING` → `"event subprocess"`; runtime
//!   `EventSubProcess*StartEventActivityBehavior#trigger` records
//!   `"event subprocess(" + startEventId + ")"` on cancelled host activities.
//! - `TRANSACTION_CANCELED` → `"transaction canceled"` (bare); runtime
//!   `CancelEndEventActivityBehavior#deleteChildExecutions` passes the bare
//!   constant into `deleteExecutionAndRelatedData`.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::history::delete_reason::{
    self, BOUNDARY_EVENT_INTERRUPTING, EVENT_SUBPROCESS_INTERRUPTING, TRANSACTION_CANCELED,
};

fn deploy(engine: &ProcessEngine, resource: &str, xml: &str) {
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name(format!("{resource} deployment"))
                .add_string(format!("{resource}.bpmn20.xml"), xml.to_string()),
        )
        .unwrap();
}

fn start_by_key(engine: &ProcessEngine, key: &str) -> String {
    let runtime_service = engine.get_runtime_service();
    runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_key(key.to_string()),
        )
        .unwrap()
        .id
}

// ── BOUNDARY_EVENT_INTERRUPTING ────────────────────────────────────────────

const BOUNDARY_INTERRUPT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p74BoundaryDeleteReason" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="hostTask" />
        <userTask id="hostTask" name="Host Task" />
        <boundaryEvent id="boundaryCancel" attachedToRef="hostTask" cancelActivity="true">
            <messageEventDefinition messageRef="cancelMsg" />
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="hostTask" targetRef="normalEnd" />
        <sequenceFlow id="f3" sourceRef="boundaryCancel" targetRef="afterBoundary" />
        <userTask id="afterBoundary" name="After Boundary" />
        <sequenceFlow id="f4" sourceRef="afterBoundary" targetRef="end" />
        <endEvent id="normalEnd" />
        <endEvent id="end" />
    </process>
</definitions>"#;

/// P74: interrupting boundary records Java
/// `DeleteReason.BOUNDARY_EVENT_INTERRUPTING + " (" + boundaryId + ")"` on the
/// cancelled host userTask historic activity.
#[test]
fn interrupting_boundary_sets_host_historic_activity_delete_reason() {
    let engine = ProcessEngine::new("p74-boundary-delete-reason".to_string());
    deploy(&engine, "p74_boundary_delete_reason", BOUNDARY_INTERRUPT_XML);
    let pi = start_by_key(&engine, "p74BoundaryDeleteReason");

    let history = engine.get_history_service();
    let pre = history
        .create_historic_activity_instance_query()
        .process_instance_id(pi.clone())
        .list()
        .unwrap();
    let pre_host = pre
        .iter()
        .find(|a| a.activity_id == "hostTask" && a.end_time.is_none())
        .expect("open historic activity for hostTask before interrupt");
    assert!(
        pre_host.delete_reason.is_none(),
        "host must not have a deleteReason before boundary fires"
    );

    engine
        .get_runtime_service()
        .trigger_boundary_event("boundaryCancel".to_string(), pi.clone())
        .unwrap();

    let post = history
        .create_historic_activity_instance_query()
        .process_instance_id(pi)
        .list()
        .unwrap();
    let host = post
        .iter()
        .find(|a| a.activity_id == "hostTask")
        .expect("historic activity for interrupted hostTask");
    assert!(
        host.end_time.is_some(),
        "interrupted host historic activity must be ended"
    );
    let expected = delete_reason::boundary_event_interrupting("boundaryCancel");
    assert_eq!(
        host.delete_reason.as_deref(),
        Some(expected.as_str()),
        "host must carry Java boundary interrupt reason (bare constant is {BOUNDARY_EVENT_INTERRUPTING:?})"
    );

    // Boundary path continues normally — afterBoundary has no deleteReason.
    let after = post
        .iter()
        .find(|a| a.activity_id == "afterBoundary" && a.end_time.is_none())
        .expect("open afterBoundary after interrupt path");
    assert!(
        after.delete_reason.is_none(),
        "normally started activity on boundary path must keep deleteReason null"
    );
}

// ── EVENT_SUBPROCESS_INTERRUPTING ──────────────────────────────────────────

const EVENT_SUBPROCESS_INTERRUPT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p74EventSubprocessDeleteReason" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="mainTask" />
        <userTask id="mainTask" name="Main Task" />
        <sequenceFlow id="f2" sourceRef="mainTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />

        <subProcess id="eventSubProcess" triggeredByEvent="true">
            <startEvent id="eventSubStart" isInterrupting="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </startEvent>
            <sequenceFlow id="esF1" sourceRef="eventSubStart" targetRef="esTask" />
            <userTask id="esTask" name="Event Sub Task" />
            <sequenceFlow id="esF2" sourceRef="esTask" targetRef="esEnd" />
            <endEvent id="esEnd" />
        </subProcess>
    </process>
</definitions>"#;

/// P74: interrupting event subprocess records Java
/// `DeleteReason.EVENT_SUBPROCESS_INTERRUPTING + "(" + startEventId + ")"` on
/// the cancelled main-flow host activity.
#[test]
fn interrupting_event_subprocess_sets_host_historic_activity_delete_reason() {
    let engine = ProcessEngine::new("p74-es-delete-reason".to_string());
    deploy(
        &engine,
        "p74_es_delete_reason",
        EVENT_SUBPROCESS_INTERRUPT_XML,
    );
    let pi = start_by_key(&engine, "p74EventSubprocessDeleteReason");

    let history = engine.get_history_service();
    let pre = history
        .create_historic_activity_instance_query()
        .process_instance_id(pi.clone())
        .list()
        .unwrap();
    let pre_main = pre
        .iter()
        .find(|a| a.activity_id == "mainTask" && a.end_time.is_none())
        .expect("open historic activity for mainTask before interrupt");
    assert!(pre_main.delete_reason.is_none());

    let _ = engine
        .get_runtime_service()
        .trigger_event_subprocess_by_message("cancelMessage".to_string(), pi.clone());

    let post = history
        .create_historic_activity_instance_query()
        .process_instance_id(pi)
        .list()
        .unwrap();
    let main = post
        .iter()
        .find(|a| a.activity_id == "mainTask")
        .expect("historic activity for cancelled mainTask");
    assert!(
        main.end_time.is_some(),
        "cancelled host historic activity must be ended"
    );
    let expected = delete_reason::event_subprocess_interrupting("eventSubStart");
    assert_eq!(
        main.delete_reason.as_deref(),
        Some(expected.as_str()),
        "host must carry Java event-subprocess interrupt reason (bare constant is {EVENT_SUBPROCESS_INTERRUPTING:?})"
    );

    // Event-subprocess path host task is a normal complete path so far.
    let es_task = post
        .iter()
        .find(|a| a.activity_id == "esTask" && a.end_time.is_none())
        .expect("open esTask after event subprocess activation");
    assert!(
        es_task.delete_reason.is_none(),
        "event-subprocess own activity must not inherit host cancel reason"
    );
}

// ── TRANSACTION_CANCELED ───────────────────────────────────────────────────

/// Concurrent cancel end: one path hits cancelEnd while hostTask is still open,
/// so the open host receives `TRANSACTION_CANCELED` when the scope is destroyed.
const TRANSACTION_CANCEL_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p74TxCancelDeleteReason" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="tx" />
        <transaction id="tx">
            <startEvent id="txStart" />
            <sequenceFlow id="tf1" sourceRef="txStart" targetRef="txFork" />
            <parallelGateway id="txFork" />
            <sequenceFlow id="tf2" sourceRef="txFork" targetRef="hostTask" />
            <sequenceFlow id="tf3" sourceRef="txFork" targetRef="txCancelEnd" />
            <userTask id="hostTask" name="Host Inside Tx" />
            <sequenceFlow id="tf4" sourceRef="hostTask" targetRef="txJoin" />
            <parallelGateway id="txJoin" />
            <sequenceFlow id="tf5" sourceRef="txJoin" targetRef="txSuccessEnd" />
            <endEvent id="txSuccessEnd" />
            <endEvent id="txCancelEnd">
                <cancelEventDefinition />
            </endEvent>
        </transaction>
        <boundaryEvent id="catchCancel" attachedToRef="tx">
            <cancelEventDefinition />
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="catchCancel" targetRef="afterCancel" />
        <userTask id="afterCancel" name="After Cancel" />
        <sequenceFlow id="f3" sourceRef="tx" targetRef="afterSuccess" />
        <userTask id="afterSuccess" name="After Success" />
        <sequenceFlow id="f4" sourceRef="afterCancel" targetRef="end" />
        <sequenceFlow id="f5" sourceRef="afterSuccess" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

/// P74: cancel end event records bare Java `DeleteReason.TRANSACTION_CANCELED`
/// on historic activities destroyed inside the transaction scope.
#[test]
fn cancel_end_sets_transaction_canceled_delete_reason_on_destroyed_host() {
    let engine = ProcessEngine::new("p74-tx-cancel-delete-reason".to_string());
    deploy(&engine, "p74_tx_cancel_delete_reason", TRANSACTION_CANCEL_XML);
    let pi = start_by_key(&engine, "p74TxCancelDeleteReason");

    // Cancel end is automatic on one fork branch: hostTask is cancelled as
    // soon as the cancel path runs. After cancel, afterCancel should be open.
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.clone())
        .unwrap();
    let keys: Vec<_> = tasks.iter().map(|t| t.task_definition_key.as_str()).collect();
    assert!(
        keys.contains(&"afterCancel"),
        "cancel path should leave afterCancel open; got {keys:?}"
    );
    assert!(
        !keys.contains(&"hostTask"),
        "hostTask inside transaction must be destroyed; got {keys:?}"
    );

    let history = engine.get_history_service();
    let activities = history
        .create_historic_activity_instance_query()
        .process_instance_id(pi)
        .list()
        .unwrap();
    let host = activities
        .iter()
        .find(|a| a.activity_id == "hostTask")
        .expect("historic activity for destroyed hostTask");
    assert!(
        host.end_time.is_some(),
        "destroyed host historic activity must be ended"
    );
    assert_eq!(
        host.delete_reason.as_deref(),
        Some(TRANSACTION_CANCELED),
        "destroyed in-transaction host must carry bare Java TRANSACTION_CANCELED"
    );

    // afterCancel is a normal leave target of the cancel boundary — no reason.
    let after = activities
        .iter()
        .find(|a| a.activity_id == "afterCancel")
        .expect("historic activity for afterCancel");
    assert!(
        after.delete_reason.is_none(),
        "activity after cancel boundary must keep deleteReason null"
    );
}
