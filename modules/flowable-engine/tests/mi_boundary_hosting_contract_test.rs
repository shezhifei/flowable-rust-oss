//! Contract tests for MI boundary hosting on receive / transaction / sub_process
//! (P9-1). Extends P8-A (`mi_boundary_attachment_contract_test.rs`) which
//! covered userTask only.
//!
//! Java evidence (flowable-engine, only-read reference):
//!   - `ContinueProcessOperation#executeMultiInstanceSynchronous` (221–233):
//!     boundary events are created once on the MI root for **every** Activity
//!     type (ReceiveTask, Transaction, SubProcess all `instanceof Activity`).
//!   - `ContinueMultiInstanceOperation`: instance children never create
//!     boundary events.
//!   - Sequential SubProcess MI (`SequentialMultiInstanceBehavior` 106–124):
//!     each round creates a new scope child; the boundary timer stays on the
//!     MI root for the whole MI lifetime — due time is fixed at MI entry.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::runtime::execution::Execution;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// BPMN fixtures
// ---------------------------------------------------------------------------

const PARALLEL_MI_RECEIVE_INTERRUPTING_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miReceiveInterruptingTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miReceive" />
        <receiveTask id="miReceive" name="MI Receive">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </receiveTask>
        <boundaryEvent id="miTimer" attachedToRef="miReceive" cancelActivity="true">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miReceive" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miTimer" targetRef="timeoutTask" />
        <userTask id="timeoutTask" name="Timeout" />
        <sequenceFlow id="f5" sourceRef="timeoutTask" targetRef="timeoutEnd" />
        <endEvent id="timeoutEnd" />
    </process>
</definitions>"#;

const PARALLEL_MI_RECEIVE_NON_INTERRUPTING_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miReceiveNonInterruptingTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miReceive" />
        <receiveTask id="miReceive" name="MI Receive">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </receiveTask>
        <boundaryEvent id="miTimer" attachedToRef="miReceive" cancelActivity="false">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miReceive" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miTimer" targetRef="timeoutTask" />
        <userTask id="timeoutTask" name="Timeout" />
        <sequenceFlow id="f5" sourceRef="timeoutTask" targetRef="timeoutEnd" />
        <endEvent id="timeoutEnd" />
    </process>
</definitions>"#;

const PARALLEL_MI_RECEIVE_INTERRUPTING_MESSAGE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <message id="cancelMsg" name="cancelMsg" />
    <process id="miReceiveInterruptingMessage" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miReceive" />
        <receiveTask id="miReceive" name="MI Receive">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </receiveTask>
        <boundaryEvent id="miMsg" attachedToRef="miReceive" cancelActivity="true">
            <messageEventDefinition messageRef="cancelMsg" />
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miReceive" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miMsg" targetRef="msgTask" />
        <userTask id="msgTask" name="Message Path" />
        <sequenceFlow id="f5" sourceRef="msgTask" targetRef="msgEnd" />
        <endEvent id="msgEnd" />
    </process>
</definitions>"#;

const PARALLEL_MI_TRANSACTION_INTERRUPTING_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miTxInterruptingTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTx" />
        <transaction id="miTx">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
            <startEvent id="txStart" />
            <sequenceFlow id="txf1" sourceRef="txStart" targetRef="txTask" />
            <userTask id="txTask" name="Tx Task" />
            <sequenceFlow id="txf2" sourceRef="txTask" targetRef="txEnd" />
            <endEvent id="txEnd" />
        </transaction>
        <boundaryEvent id="miTimer" attachedToRef="miTx" cancelActivity="true">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miTx" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miTimer" targetRef="timeoutTask" />
        <userTask id="timeoutTask" name="Timeout" />
        <sequenceFlow id="f5" sourceRef="timeoutTask" targetRef="timeoutEnd" />
        <endEvent id="timeoutEnd" />
    </process>
</definitions>"#;

const PARALLEL_MI_TRANSACTION_NON_INTERRUPTING_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miTxNonInterruptingTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTx" />
        <transaction id="miTx">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
            <startEvent id="txStart" />
            <sequenceFlow id="txf1" sourceRef="txStart" targetRef="txTask" />
            <userTask id="txTask" name="Tx Task" />
            <sequenceFlow id="txf2" sourceRef="txTask" targetRef="txEnd" />
            <endEvent id="txEnd" />
        </transaction>
        <boundaryEvent id="miTimer" attachedToRef="miTx" cancelActivity="false">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miTx" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miTimer" targetRef="timeoutTask" />
        <userTask id="timeoutTask" name="Timeout" />
        <sequenceFlow id="f5" sourceRef="timeoutTask" targetRef="timeoutEnd" />
        <endEvent id="timeoutEnd" />
    </process>
</definitions>"#;

const PARALLEL_MI_SUBPROCESS_INTERRUPTING_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miSubInterruptingTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miSub" />
        <subProcess id="miSub" name="MI SubProcess">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
            <startEvent id="subStart" />
            <sequenceFlow id="sf1" sourceRef="subStart" targetRef="innerTask" />
            <userTask id="innerTask" name="Inner Task" />
            <sequenceFlow id="sf2" sourceRef="innerTask" targetRef="subEnd" />
            <endEvent id="subEnd" />
        </subProcess>
        <boundaryEvent id="miTimer" attachedToRef="miSub" cancelActivity="true">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miSub" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miTimer" targetRef="timeoutTask" />
        <userTask id="timeoutTask" name="Timeout" />
        <sequenceFlow id="f5" sourceRef="timeoutTask" targetRef="timeoutEnd" />
        <endEvent id="timeoutEnd" />
    </process>
</definitions>"#;

const PARALLEL_MI_SUBPROCESS_NON_INTERRUPTING_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miSubNonInterruptingTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miSub" />
        <subProcess id="miSub" name="MI SubProcess">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
            <startEvent id="subStart" />
            <sequenceFlow id="sf1" sourceRef="subStart" targetRef="innerTask" />
            <userTask id="innerTask" name="Inner Task" />
            <sequenceFlow id="sf2" sourceRef="innerTask" targetRef="subEnd" />
            <endEvent id="subEnd" />
        </subProcess>
        <boundaryEvent id="miTimer" attachedToRef="miSub" cancelActivity="false">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miSub" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miTimer" targetRef="timeoutTask" />
        <userTask id="timeoutTask" name="Timeout" />
        <sequenceFlow id="f5" sourceRef="timeoutTask" targetRef="timeoutEnd" />
        <endEvent id="timeoutEnd" />
    </process>
</definitions>"#;

const SEQUENTIAL_MI_SUBPROCESS_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miSubSequentialTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miSub" />
        <subProcess id="miSub" name="MI SubProcess">
            <multiInstanceLoopCharacteristics isSequential="true">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
            <startEvent id="subStart" />
            <sequenceFlow id="sf1" sourceRef="subStart" targetRef="innerTask" />
            <userTask id="innerTask" name="Inner Task" />
            <sequenceFlow id="sf2" sourceRef="innerTask" targetRef="subEnd" />
            <endEvent id="subEnd" />
        </subProcess>
        <boundaryEvent id="miTimer" attachedToRef="miSub" cancelActivity="true">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miSub" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miTimer" targetRef="timeoutTask" />
        <userTask id="timeoutTask" name="Timeout" />
        <sequenceFlow id="f5" sourceRef="timeoutTask" targetRef="timeoutEnd" />
        <endEvent id="timeoutEnd" />
    </process>
</definitions>"#;

// ---------------------------------------------------------------------------
// Harness (mirrors mi_boundary_attachment_contract_test)
// ---------------------------------------------------------------------------

fn engine_with_time(name: &str) -> (ProcessEngine, Arc<TestTimeSource>) {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 26, 8, 0, 0).unwrap(),
    ));
    (
        ProcessEngine::with_time_source(name.to_string(), time_source.clone()),
        time_source,
    )
}

fn deploy(engine: &ProcessEngine, resource: &str, xml: &str) -> String {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string(resource.to_string(), xml.to_string()),
    )
    .unwrap();
    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone()
}

fn start(
    engine: &ProcessEngine,
    definition_id: String,
) -> flowable_engine::runtime::process_instance::ProcessInstance {
    let runtime = engine.get_runtime_service();
    runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .unwrap()
}

fn all_executions(engine: &ProcessEngine, process_instance_id: &str) -> Vec<Execution> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|e| e.process_instance_id.as_deref() == Some(process_instance_id))
        .collect()
}

fn mi_root_id(engine: &ProcessEngine, process_instance_id: &str) -> String {
    all_executions(engine, process_instance_id)
        .into_iter()
        .find(|e| e.is_multi_instance_root)
        .expect("MI root execution must exist")
        .id
}

fn task_keys(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    let mut keys: Vec<String> = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
        .into_iter()
        .map(|t| t.task_definition_key)
        .collect();
    keys.sort();
    keys
}

fn boundary_timers(
    engine: &ProcessEngine,
    process_instance_id: &str,
    boundary_event_id: &str,
) -> Vec<flowable_engine::persistence::runtime_store::RuntimeTimerJobState> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .find_timer_job_states_by_process_instance_id(process_instance_id, &mut session)
        .into_iter()
        .filter(|j| j.is_boundary && j.activity_id == boundary_event_id)
        .collect()
}

fn boundary_states(
    engine: &ProcessEngine,
    process_instance_id: &str,
    boundary_event_id: &str,
) -> Vec<flowable_engine::persistence::runtime_store::RuntimeBoundaryEventState> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .find_boundary_event_states_by_process_instance_id(process_instance_id, &mut session)
        .into_iter()
        .filter(|s| s.boundary_event_id == boundary_event_id)
        .collect()
}

// ===========================================================================
// ReceiveTask — parallel MI
// ===========================================================================

/// Java: one boundary timer per definition for the whole MI receive activity,
/// hosted on the MI root — never one per instance child.
#[test]
fn parallel_mi_receive_boundary_timer_registers_once_on_mi_root() {
    let (engine, _time) = engine_with_time("mi-receive-timer-register");
    let definition_id = deploy(
        &engine,
        "mi_receive_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_RECEIVE_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["miReceive"; 3]
    );

    let mi_root_id = mi_root_id(&engine, &process_instance.id);
    let timers = boundary_timers(&engine, &process_instance.id, "miTimer");
    assert_eq!(
        timers.len(),
        1,
        "Java schedules exactly one boundary timer for the whole MI receive activity; \
         got {} (per-instance registration bug)",
        timers.len()
    );
    assert_eq!(
        timers[0].execution_id, mi_root_id,
        "the boundary timer must be hosted on the MI root execution"
    );
}

/// Java `executeInterruptingBehavior`: fire interrupting timer → cancel entire MI.
#[test]
fn interrupting_mi_receive_boundary_timer_cancels_whole_multi_instance() {
    let (engine, time) = engine_with_time("mi-receive-interrupting-cancel");
    let definition_id = deploy(
        &engine,
        "mi_receive_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_RECEIVE_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["miReceive"; 3]
    );

    time.advance_time(60 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(
        executed.len(),
        1,
        "exactly one boundary timer job exists (got {})",
        executed.len()
    );

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["timeoutTask"],
        "interrupting boundary on MI receive cancels ALL instances"
    );
    assert!(
        all_executions(&engine, &process_instance.id)
            .iter()
            .all(|e| !e.is_multi_instance_root),
        "MI root must be deleted by the interrupting trigger"
    );
    assert!(
        all_executions(&engine, &process_instance.id)
            .iter()
            .all(|e| e.activity_id.as_deref() != Some("miReceive")),
        "no instance child execution may survive"
    );
}

/// Java non-interrupting: keep all receive instances + take boundary path once.
#[test]
fn non_interrupting_mi_receive_boundary_timer_keeps_all_instances_alive() {
    let (engine, time) = engine_with_time("mi-receive-non-interrupting");
    let definition_id = deploy(
        &engine,
        "mi_receive_non_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_RECEIVE_NON_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["miReceive"; 3]
    );

    time.advance_time(60 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1);

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["miReceive", "miReceive", "miReceive", "timeoutTask"],
        "non-interrupting boundary keeps all 3 receive instances and takes outgoing once"
    );
    mi_root_id(&engine, &process_instance.id);
}

/// Message boundary state is a single row hosted on the MI root; interrupting
/// cancels the whole MI.
#[test]
fn parallel_mi_receive_message_boundary_registers_once_and_interrupts_whole_mi() {
    let engine = ProcessEngine::new("mi-receive-message-cancel".to_string());
    let definition_id = deploy(
        &engine,
        "mi_receive_interrupting_message.bpmn20.xml",
        PARALLEL_MI_RECEIVE_INTERRUPTING_MESSAGE_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["miReceive"; 3]
    );

    let mi_root_id = mi_root_id(&engine, &process_instance.id);
    let states = boundary_states(&engine, &process_instance.id, "miMsg");
    assert_eq!(
        states.len(),
        1,
        "exactly one message boundary state for the whole MI receive activity"
    );
    assert_eq!(
        states[0].host_execution_id, mi_root_id,
        "message boundary host must be the MI root, not an instance child"
    );

    engine
        .get_runtime_service()
        .trigger_boundary_event_by_message_ref(
            "cancelMsg".to_string(),
            process_instance.id.clone(),
        );

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["msgTask"],
        "interrupting message boundary on MI receive cancels ALL instances"
    );
    assert!(
        all_executions(&engine, &process_instance.id)
            .iter()
            .all(|e| !e.is_multi_instance_root),
        "MI root must be deleted by the interrupting trigger"
    );
}

// ===========================================================================
// Transaction — parallel MI
// ===========================================================================

#[test]
fn parallel_mi_transaction_boundary_timer_registers_once_on_mi_root() {
    let (engine, _time) = engine_with_time("mi-tx-timer-register");
    let definition_id = deploy(
        &engine,
        "mi_tx_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_TRANSACTION_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["txTask"; 3]);

    let mi_root_id = mi_root_id(&engine, &process_instance.id);
    let timers = boundary_timers(&engine, &process_instance.id, "miTimer");
    assert_eq!(
        timers.len(),
        1,
        "Java schedules exactly one boundary timer for the whole MI transaction; \
         got {} (per-instance registration bug)",
        timers.len()
    );
    assert_eq!(timers[0].execution_id, mi_root_id);
}

#[test]
fn interrupting_mi_transaction_boundary_timer_cancels_whole_multi_instance() {
    let (engine, time) = engine_with_time("mi-tx-interrupting-cancel");
    let definition_id = deploy(
        &engine,
        "mi_tx_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_TRANSACTION_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["txTask"; 3]);

    time.advance_time(60 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1, "exactly one boundary timer job exists");

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["timeoutTask"],
        "interrupting boundary on MI transaction cancels ALL instances"
    );
    assert!(
        all_executions(&engine, &process_instance.id)
            .iter()
            .all(|e| !e.is_multi_instance_root),
        "MI root must be deleted"
    );
    assert!(
        all_executions(&engine, &process_instance.id)
            .iter()
            .all(|e| e.activity_id.as_deref() != Some("miTx")
                && e.activity_id.as_deref() != Some("txTask")),
        "no transaction / inner-task instance may survive"
    );
}

#[test]
fn non_interrupting_mi_transaction_boundary_timer_keeps_all_instances_alive() {
    let (engine, time) = engine_with_time("mi-tx-non-interrupting");
    let definition_id = deploy(
        &engine,
        "mi_tx_non_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_TRANSACTION_NON_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["txTask"; 3]);

    time.advance_time(60 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1);

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["timeoutTask", "txTask", "txTask", "txTask"],
        "non-interrupting boundary keeps all 3 tx instances and takes outgoing once"
    );
    mi_root_id(&engine, &process_instance.id);
}

// ===========================================================================
// SubProcess — parallel MI
// ===========================================================================

#[test]
fn parallel_mi_subprocess_boundary_timer_registers_once_on_mi_root() {
    let (engine, _time) = engine_with_time("mi-sub-timer-register");
    let definition_id = deploy(
        &engine,
        "mi_sub_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_SUBPROCESS_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["innerTask"; 3]
    );

    let mi_root_id = mi_root_id(&engine, &process_instance.id);
    let timers = boundary_timers(&engine, &process_instance.id, "miTimer");
    assert_eq!(
        timers.len(),
        1,
        "Java schedules exactly one boundary timer for the whole MI SubProcess; \
         got {} (per-instance registration bug)",
        timers.len()
    );
    assert_eq!(timers[0].execution_id, mi_root_id);
}

#[test]
fn interrupting_mi_subprocess_boundary_timer_cancels_whole_multi_instance() {
    let (engine, time) = engine_with_time("mi-sub-interrupting-cancel");
    let definition_id = deploy(
        &engine,
        "mi_sub_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_SUBPROCESS_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["innerTask"; 3]
    );

    time.advance_time(60 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1, "exactly one boundary timer job exists");

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["timeoutTask"],
        "interrupting boundary on MI SubProcess cancels ALL instances"
    );
    assert!(
        all_executions(&engine, &process_instance.id)
            .iter()
            .all(|e| !e.is_multi_instance_root),
        "MI root must be deleted"
    );
    assert!(
        all_executions(&engine, &process_instance.id)
            .iter()
            .all(|e| e.activity_id.as_deref() != Some("miSub")
                && e.activity_id.as_deref() != Some("innerTask")),
        "no SubProcess / inner-task instance may survive"
    );
}

#[test]
fn non_interrupting_mi_subprocess_boundary_timer_keeps_all_instances_alive() {
    let (engine, time) = engine_with_time("mi-sub-non-interrupting");
    let definition_id = deploy(
        &engine,
        "mi_sub_non_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_SUBPROCESS_NON_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["innerTask"; 3]
    );

    time.advance_time(60 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1);

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["innerTask", "innerTask", "innerTask", "timeoutTask"],
        "non-interrupting boundary keeps all 3 inner tasks and takes outgoing once"
    );
    mi_root_id(&engine, &process_instance.id);
}

// ===========================================================================
// SubProcess — sequential MI: due time must not reset across rounds
// ===========================================================================

/// Java: entire sequential SubProcess MI has one timer on the MI root with due
/// fixed at MI entry. Completing a round (DestroyScope of old scope child) must
/// not delete/recreate the timer and must not shift due_time.
#[test]
fn sequential_mi_subprocess_boundary_timer_due_stable_across_rounds() {
    let (engine, time) = engine_with_time("mi-sub-seq-due-stable");
    let definition_id = deploy(
        &engine,
        "mi_sub_sequential_timer.bpmn20.xml",
        SEQUENTIAL_MI_SUBPROCESS_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["innerTask"]);

    let mi_root_id = mi_root_id(&engine, &process_instance.id);
    let timers_r0 = boundary_timers(&engine, &process_instance.id, "miTimer");
    assert_eq!(
        timers_r0.len(),
        1,
        "round 0: exactly one boundary timer on sequential MI SubProcess"
    );
    assert_eq!(timers_r0[0].execution_id, mi_root_id);
    let due_r0 = timers_r0[0].due_time;
    let job_id_r0 = timers_r0[0].timer_job_id.clone();

    // Advance wall-clock a little so a re-created timer would get a later due.
    time.advance_time(5 * 60 * 1000);

    // Complete round 0 → sequential MI advances to round 1 (new scope child).
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    engine
        .get_task_service()
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["innerTask"],
        "round 1 must still expose the inner task"
    );

    let timers_r1 = boundary_timers(&engine, &process_instance.id, "miTimer");
    assert_eq!(
        timers_r1.len(),
        1,
        "round 1: still exactly one boundary timer (not re-created per scope child)"
    );
    assert_eq!(
        timers_r1[0].execution_id, mi_root_id,
        "round 1: timer remains hosted on the MI root (not the new scope child)"
    );
    assert_eq!(
        timers_r1[0].timer_job_id, job_id_r0,
        "round 1: same timer job must survive DestroyScope of the previous scope child"
    );
    assert_eq!(
        timers_r1[0].due_time, due_r0,
        "round 1: due_time must not reset when a sequential SubProcess round advances"
    );

    // One more round to lock stability across two advances.
    time.advance_time(5 * 60 * 1000);
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    engine
        .get_task_service()
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["innerTask"]);

    let timers_r2 = boundary_timers(&engine, &process_instance.id, "miTimer");
    assert_eq!(timers_r2.len(), 1);
    assert_eq!(timers_r2[0].timer_job_id, job_id_r0);
    assert_eq!(timers_r2[0].due_time, due_r0);
    assert_eq!(timers_r2[0].execution_id, mi_root_id);
}
