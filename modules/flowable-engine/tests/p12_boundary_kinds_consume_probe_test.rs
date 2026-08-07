//! P12: non-interrupting Error / Escalation / Cancel / Compensate boundary
//! consume-vs-repeat exclusion probes.
//!
//! Java references (the Flowable Java reference tree):
//!
//! - **Error**: `ErrorEventDefinitionParseHandler#executeParse` always creates
//!   `BoundaryEventActivityBehavior(boundaryEvent, true)` — interrupting is
//!   forced at parse time. Converter also forces model `cancelActivity=false`
//!   for error boundaries (`BoundaryEventXMLConverter:86-92`), but runtime
//!   behavior ignores that and always interrupts. Non-interrupting error does
//!   not exist → consume branch is structurally unreachable under Java truth.
//!
//! - **Escalation**: `EscalationEventDefinitionParseHandler:40-41` passes
//!   `boundaryEvent.isCancelActivity()`. Non-interrupting is valid
//!   (`BoundaryEscalationEventTest` suspended-parent fixture uses
//!   `cancelActivity="false"`). Trigger path is
//!   `BoundaryEventActivityBehavior#trigger` → `executeNonInterruptingBehavior`
//!   (`BoundaryEventActivityBehavior.java:114-155`) which **never deletes** the
//!   waiting boundary execution. `EscalationPropagation#executeEventHandler`
//!   (`EscalationPropagation.java:206-222`) re-finds that child by activityId
//!   and re-triggers → **repeat**.
//!
//! - **Cancel**: `BoundaryCancelEventActivityBehavior#trigger` always deletes
//!   the transaction subprocess (one-shot by structure). Cancel end path
//!   (`CancelEndEventActivityBehavior`) likewise destroys the transaction.
//!   Re-fire is impossible after host is gone.
//!
//! - **Compensate**: `BoundaryCompensateEventActivityBehavior` registers a
//!   compensate subscription on execute; trigger consumes matching
//!   subscriptions when `cancelActivity` and then calls super. Compensation
//!   subscriptions are one-shot (see TransactionSubProcessTest / CompensateEventTest).

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;

// ─── Escalation (expected GAP under current keep-set) ───────────────────────

/// Java: non-interrupting escalation keeps the waiting boundary execution
/// (`executeNonInterruptingBehavior`); same scope can throw again and re-hit
/// the boundary (`EscalationPropagation` finds child by activityId).
///
/// Probe: direct `trigger_boundary_event` twice (same entry as message/conditional
/// repeat probes) while host stays open. Asserts subscription survival + second
/// fire — red until Escalation is added to the non-interrupting keep-set.
#[test]
fn p12_non_interrupting_escalation_boundary_state_survives_and_fires_twice() {
    let engine = ProcessEngine::new("p12-escalation-repeat".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <escalation id="esc1" escalationCode="ESC_CODE" />
        <process id="p12EscRepeat" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="hostTask" />
            <userTask id="hostTask" name="Host" />
            <boundaryEvent id="catchEsc" attachedToRef="hostTask" cancelActivity="false">
                <escalationEventDefinition escalationRef="esc1" />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="hostTask" targetRef="hostEnd" />
            <sequenceFlow id="f3" sourceRef="catchEsc" targetRef="escTask" />
            <userTask id="escTask" name="Escalated" />
            <sequenceFlow id="f4" sourceRef="escTask" targetRef="escEnd" />
            <endEvent id="hostEnd" />
            <endEvent id="escEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p12-esc-repeat".to_string())
                .add_string("p12_esc_repeat.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states_before =
        runtime_store.find_boundary_event_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(states_before.len(), 1);
    assert!(!states_before[0].cancel_activity);
    assert_eq!(
        states_before[0].event_subscription.kind,
        EventSubscriptionKind::Escalation
    );
    drop(session);

    // First fire via event-ref (mirrors EscalationPropagation → planTrigger).
    runtime_service.trigger_boundary_event_by_event_ref(
        EventSubscriptionKind::Escalation,
        "ESC_CODE".to_string(),
        pi.id.clone(),
    );

    let tasks_1 = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert!(
        tasks_1.iter().any(|t| t.task_definition_key == "hostTask"),
        "host must remain after non-interrupting escalation"
    );
    let esc_count_1 = tasks_1
        .iter()
        .filter(|t| t.task_definition_key == "escTask")
        .count();
    assert_eq!(
        esc_count_1, 1,
        "first escalation fire must take boundary path"
    );

    let mut session = runtime_store.create_session().unwrap();
    let states_after_1 =
        runtime_store.find_boundary_event_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(
        states_after_1.len(),
        1,
        "Java non-interrupting escalation keeps waiting boundary execution; \
         Rust must not consume boundary state (repeat)"
    );
    drop(session);

    // Second fire: same escalation again while host is still open.
    runtime_service.trigger_boundary_event_by_event_ref(
        EventSubscriptionKind::Escalation,
        "ESC_CODE".to_string(),
        pi.id.clone(),
    );

    let tasks_2 = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    let esc_count_2 = tasks_2
        .iter()
        .filter(|t| t.task_definition_key == "escTask")
        .count();
    assert_eq!(
        esc_count_2, 2,
        "second throw/trigger of non-interrupting escalation must fire again"
    );
    assert!(
        tasks_2.iter().any(|t| t.task_definition_key == "hostTask"),
        "host must still remain after second non-interrupting escalation"
    );

    let mut session = runtime_store.create_session().unwrap();
    let states_after_2 =
        runtime_store.find_boundary_event_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(
        states_after_2.len(),
        1,
        "subscription remains after repeated escalation fires until host ends"
    );
}

// ─── Error (结构性不可达 under Java; one-shot in practice) ──────────────────

/// Java never builds a non-interrupting error boundary behavior
/// (`ErrorEventDefinitionParseHandler:34` hardcodes `interrupting=true`).
/// Converter forces model `cancelActivity=false` (Java + Rust), but the Rust
/// runtime now mirrors the parse handler (`runtime_cancel_activity`): the
/// registered boundary state is always interrupting for Error. After a single
/// error catch the host scope is gone / state is consumed, matching one-shot
/// error semantics. Probe documents: not a repeat kind.
#[test]
fn p12_error_boundary_is_one_shot_after_catch() {
    let engine = ProcessEngine::new("p12-error-oneshot".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    // Explicit cancelActivity="false" in XML — converter still forces false for
    // ErrorEventDefinition (parity with Java BoundaryEventXMLConverter:90-91).
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <error id="err1" errorCode="E1" />
        <process id="p12ErrorOneShot" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="sub" />
            <subProcess id="sub">
                <startEvent id="subStart" />
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="subTask" />
                <userTask id="subTask" />
                <sequenceFlow id="sf2" sourceRef="subTask" targetRef="throwErr" />
                <endEvent id="throwErr">
                    <errorEventDefinition errorRef="err1" />
                </endEvent>
            </subProcess>
            <boundaryEvent id="catchErr" attachedToRef="sub" cancelActivity="false">
                <errorEventDefinition errorRef="err1" />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="catchErr" targetRef="errTask" />
            <userTask id="errTask" />
            <sequenceFlow id="f3" sourceRef="errTask" targetRef="end" />
            <sequenceFlow id="f4" sourceRef="sub" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p12-error-oneshot".to_string())
                .add_string("p12_error_oneshot.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states =
        runtime_store.find_boundary_event_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(states.len(), 1);
    assert_eq!(
        states[0].event_subscription.kind,
        EventSubscriptionKind::Error
    );
    // Model cancelActivity is forced false for error (converter parity), but
    // the runtime registration mirrors Java's parse handler and hardcodes
    // interrupting=true (`ErrorEventDefinitionParseHandler.java:34`).
    assert!(
        states[0].cancel_activity,
        "error boundary runtime state must be interrupting regardless of the \
         model flag (Java runtime hardcodes interrupting=true)"
    );
    drop(session);

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks[0].task_definition_key, "subTask");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 1);
    assert_eq!(tasks_after[0].task_definition_key, "errTask");

    let mut session = runtime_store.create_session().unwrap();
    let states_after =
        runtime_store.find_boundary_event_states_by_process_instance_id(&pi.id, &mut session);
    assert!(
        states_after.is_empty(),
        "error boundary is one-shot: state gone after catch (no repeat)"
    );

    // Second artificial trigger must not spawn another errTask (no state).
    drop(session);
    runtime_service.trigger_boundary_event_by_event_ref(
        EventSubscriptionKind::Error,
        "E1".to_string(),
        pi.id.clone(),
    );
    let tasks_again = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_again
            .iter()
            .filter(|t| t.task_definition_key == "errTask")
            .count(),
        1,
        "second error trigger without state must not re-fire"
    );
}

// ─── Cancel (one-shot by structure) ─────────────────────────────────────────

/// Cancel boundary only lives on transaction; cancel end / cancel boundary
/// always destroy the transaction (`BoundaryCancelEventActivityBehavior#trigger`,
/// `CancelEndEventActivityBehavior`). Re-fire is structurally impossible.
#[test]
fn p12_cancel_boundary_is_one_shot_after_transaction_cancelled() {
    let engine = ProcessEngine::new("p12-cancel-oneshot".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p12CancelOneShot" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="tx" />
            <transaction id="tx">
                <startEvent id="txStart" />
                <sequenceFlow id="txF1" sourceRef="txStart" targetRef="txTask" />
                <userTask id="txTask" />
                <sequenceFlow id="txF2" sourceRef="txTask" targetRef="throwCancel" />
                <endEvent id="throwCancel">
                    <cancelEventDefinition />
                </endEvent>
            </transaction>
            <boundaryEvent id="catchCancel" attachedToRef="tx">
                <cancelEventDefinition />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="catchCancel" targetRef="cancelTask" />
            <userTask id="cancelTask" />
            <sequenceFlow id="f3" sourceRef="cancelTask" targetRef="end" />
            <sequenceFlow id="f4" sourceRef="tx" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p12-cancel-oneshot".to_string())
                .add_string("p12_cancel_oneshot.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states =
        runtime_store.find_boundary_event_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(states.len(), 1);
    assert_eq!(
        states[0].event_subscription.kind,
        EventSubscriptionKind::Cancel
    );
    drop(session);

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks[0].task_definition_key, "txTask");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 1);
    assert_eq!(tasks_after[0].task_definition_key, "cancelTask");

    // Host transaction is gone — cannot re-cancel the same transaction.
    // Note: Rust CancelEndEventActivityBehavior rewrites the tx execution and
    // takes the boundary outgoing path without going through
    // execute_boundary_trigger, so residual boundary_event_state may linger
    // (cleanup gap, out of P12 scope). One-shot is structural (host destroyed).
    let mut session = runtime_store.create_session().unwrap();
    let tx_still_present = runtime_store
        .snapshot_executions(&mut session)
        .values()
        .any(|e| {
            e.process_instance_id.as_deref() == Some(pi.id.as_str())
                && e.activity_id.as_deref() == Some("tx")
        });
    assert!(
        !tx_still_present,
        "cancel destroys the transaction host — structural one-shot"
    );
    drop(session);

    // Even if residual Cancel boundary state remains, host is gone so a second
    // trigger must not create another cancelTask (host lookup fails / no-op).
    runtime_service.trigger_boundary_event_by_event_ref(
        EventSubscriptionKind::Cancel,
        String::new(),
        pi.id.clone(),
    );
    let tasks_again = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_again
            .iter()
            .filter(|t| t.task_definition_key == "cancelTask")
            .count(),
        1,
        "second cancel trigger after host destroyed must not re-fire"
    );
}

// ─── Compensate (one-shot subscription model) ───────────────────────────────

/// Compensate boundary registers a compensation subscription (Java
/// `BoundaryCompensateEventActivityBehavior#execute`). Firing compensation
/// consumes subscriptions; boundary path is not a repeatable external
/// subscription like message/signal. Probe: non-interrupting compensate
/// boundary state is consumed on direct trigger (Rust current path) / host
/// remains for cancelActivity=false, and second trigger does not re-fire.
///
/// Java: when cancelActivity is false, `trigger` skips subscription delete
/// then `executeNonInterruptingBehavior` keeps wait execution — however
/// compensation is driven by `CompensateEventSubscriptionEntity` which is
/// one-shot when compensation is thrown. For the Rust boundary_event_state
/// consume branch, one-shot matches practical compensation semantics.
#[test]
fn p12_non_interrupting_compensate_boundary_consume_on_trigger() {
    let engine = ProcessEngine::new("p12-compensate-consume".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p12CompConsume" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="hostTask" />
            <userTask id="hostTask" name="Host" />
            <boundaryEvent id="catchComp" attachedToRef="hostTask" cancelActivity="false">
                <compensateEventDefinition />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="hostTask" targetRef="hostEnd" />
            <sequenceFlow id="f3" sourceRef="catchComp" targetRef="compTask" />
            <userTask id="compTask" name="Comp Path" />
            <sequenceFlow id="f4" sourceRef="compTask" targetRef="compEnd" />
            <endEvent id="hostEnd" />
            <endEvent id="compEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p12-comp-consume".to_string())
                .add_string("p12_comp_consume.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states =
        runtime_store.find_boundary_event_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(states.len(), 1, "compensate boundary should register state");
    assert!(!states[0].cancel_activity);
    assert_eq!(
        states[0].event_subscription.kind,
        EventSubscriptionKind::Compensate
    );
    drop(session);

    runtime_service
        .trigger_boundary_event("catchComp".to_string(), pi.id.clone())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert!(
        tasks.iter().any(|t| t.task_definition_key == "hostTask"),
        "non-interrupting compensate keeps host"
    );
    let comp_count = tasks
        .iter()
        .filter(|t| t.task_definition_key == "compTask")
        .count();
    assert_eq!(
        comp_count, 1,
        "first compensate boundary trigger takes path"
    );

    let mut session = runtime_store.create_session().unwrap();
    let states_after =
        runtime_store.find_boundary_event_states_by_process_instance_id(&pi.id, &mut session);
    // Document current Rust consume for non-interrupting compensate.
    // Java pure boundary trigger would keep wait (executeNonInterruptingBehavior),
    // but compensation is one-shot via subscription model — consume is acceptable
    // parity for practical compensate boundary_event_state.
    assert!(
        states_after.is_empty(),
        "non-interrupting compensate boundary_event_state is consumed after trigger \
         (one-shot compensation model; not a message-like repeat subscription)"
    );
    drop(session);

    runtime_service
        .trigger_boundary_event("catchComp".to_string(), pi.id.clone())
        .unwrap();
    let tasks_2 = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_2
            .iter()
            .filter(|t| t.task_definition_key == "compTask")
            .count(),
        1,
        "second compensate trigger after consume must not re-fire"
    );
}
