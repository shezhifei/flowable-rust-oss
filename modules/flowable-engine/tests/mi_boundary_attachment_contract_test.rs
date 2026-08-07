//! Contract tests for Java boundary-event attachment on multi-instance
//! activities (P8-A).
//!
//! Java evidence (flowable-engine, only-read reference):
//!   - `ContinueProcessOperation#continueThroughFlowNode` (120–122): MI
//!     activities go through `executeMultiInstanceSynchronous`.
//!   - `ContinueProcessOperation#executeMultiInstanceSynchronous` (221–233):
//!     boundary events are created AFTER the MI behavior ran, on the MI root
//!     execution — once per boundary definition, not per instance.
//!   - `ContinueProcessOperation#createBoundaryEvents` (356–388): one child
//!     execution of the MI root per boundary event.
//!   - `ContinueMultiInstanceOperation` (whole file): instance children never
//!     create boundary events.
//!   - `BoundaryTimerEventActivityBehavior#execute` (41–54): exactly one timer
//!     job per boundary event for the whole MI activity.
//!   - `BoundaryEventActivityBehavior#executeInterruptingBehavior` (63–112):
//!     the interrupting trigger deletes all children of the attached scope
//!     (`#deleteChildExecutions`, 157–164) and the scope itself — for an MI
//!     activity that is the MI root plus EVERY instance — then re-parents the
//!     boundary execution to the first parent scope and takes outgoing flows.
//!   - `BoundaryEventActivityBehavior#executeNonInterruptingBehavior`
//!     (114–155): the MI root and all instances stay untouched.
//!
//! Rust has no dedicated boundary child execution; the Java-equivalent host
//! for a whole-MI boundary registration is the MI root execution itself.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::runtime::execution::Execution;
use std::sync::Arc;

const PARALLEL_MI_INTERRUPTING_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miBoundaryInterruptingTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <boundaryEvent id="miTimer" attachedToRef="miTask" cancelActivity="true">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miTimer" targetRef="timeoutTask" />
        <userTask id="timeoutTask" name="Timeout" />
        <sequenceFlow id="f5" sourceRef="timeoutTask" targetRef="timeoutEnd" />
        <endEvent id="timeoutEnd" />
    </process>
</definitions>"#;

const PARALLEL_MI_NON_INTERRUPTING_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miBoundaryNonInterruptingTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <boundaryEvent id="miTimer" attachedToRef="miTask" cancelActivity="false">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miTimer" targetRef="timeoutTask" />
        <userTask id="timeoutTask" name="Timeout" />
        <sequenceFlow id="f5" sourceRef="timeoutTask" targetRef="timeoutEnd" />
        <endEvent id="timeoutEnd" />
    </process>
</definitions>"#;

const PARALLEL_MI_INTERRUPTING_MESSAGE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <message id="cancelMsg" name="cancelMsg" />
    <process id="miBoundaryInterruptingMessage" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <boundaryEvent id="miMsg" attachedToRef="miTask" cancelActivity="true">
            <messageEventDefinition messageRef="cancelMsg" />
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="normalEnd" />
        <endEvent id="normalEnd" />
        <sequenceFlow id="f4" sourceRef="miMsg" targetRef="msgTask" />
        <userTask id="msgTask" name="Message Path" />
        <sequenceFlow id="f5" sourceRef="msgTask" targetRef="msgEnd" />
        <endEvent id="msgEnd" />
    </process>
</definitions>"#;

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

/// Java: one boundary timer per boundary definition for the whole MI activity,
/// hosted on the MI root — never one per instance child.
#[test]
fn parallel_mi_boundary_timer_registers_once_on_mi_root() {
    let (engine, _time) = engine_with_time("mi-boundary-register-on-root");
    let definition_id = deploy(
        &engine,
        "mi_boundary_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["miTask"; 3]);

    let mi_root_id = mi_root_id(&engine, &process_instance.id);
    let child_ids: Vec<String> = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|t| t.execution_id)
        .collect();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let timers: Vec<_> = store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .filter(|j| j.is_boundary && j.activity_id == "miTimer")
        .collect();

    assert_eq!(
        timers.len(),
        1,
        "Java schedules exactly one boundary timer job for the whole MI activity \
         (BoundaryTimerEventActivityBehavior#execute runs once on the MI-root boundary execution)"
    );
    assert_eq!(
        timers[0].execution_id, mi_root_id,
        "the boundary timer must be hosted on the MI root execution"
    );
    assert!(
        !child_ids.contains(&timers[0].execution_id),
        "the boundary timer must not be hosted on an instance child"
    );
}

/// Java `executeInterruptingBehavior`: firing the interrupting boundary timer
/// of an MI activity deletes the MI root and EVERY instance, then continues on
/// the boundary's outgoing flow exactly once.
#[test]
fn interrupting_mi_boundary_timer_cancels_whole_multi_instance() {
    let (engine, time) = engine_with_time("mi-boundary-interrupting-cancel");
    let definition_id = deploy(
        &engine,
        "mi_boundary_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["miTask"; 3]);

    time.advance_time(60 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1, "exactly one boundary timer job exists");

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["timeoutTask"],
        "interrupting boundary on an MI activity cancels ALL instances \
         (Java deletes the MI root and every child) and leaves once"
    );

    let executions = all_executions(&engine, &process_instance.id);
    assert!(
        executions.iter().all(|e| !e.is_multi_instance_root),
        "the MI root execution must be deleted by the interrupting trigger"
    );
    assert!(
        executions
            .iter()
            .all(|e| e.activity_id.as_deref() != Some("miTask")),
        "no instance child execution may survive"
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let timers =
        store.find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(timers.is_empty(), "the fired timer job is consumed");
}

/// Java `executeNonInterruptingBehavior`: firing the non-interrupting boundary
/// leaves the MI root and every instance untouched; the outgoing flow is taken
/// once on a new concurrent execution.
#[test]
fn non_interrupting_mi_boundary_timer_keeps_all_instances_alive() {
    let (engine, time) = engine_with_time("mi-boundary-non-interrupting");
    let definition_id = deploy(
        &engine,
        "mi_boundary_non_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_NON_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["miTask"; 3]);

    time.advance_time(60 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1);

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["miTask", "miTask", "miTask", "timeoutTask"],
        "non-interrupting boundary keeps all 3 instances and takes the \
         outgoing flow exactly once"
    );
    mi_root_id(&engine, &process_instance.id);
}

/// Completing one instance must not touch the boundary registration: Java
/// hosts it on the MI root, which survives individual instance completions.
#[test]
fn completing_one_mi_instance_keeps_root_boundary_timer() {
    let (engine, _time) = engine_with_time("mi-boundary-instance-completion");
    let definition_id = deploy(
        &engine,
        "mi_boundary_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["miTask"; 2]);

    let mi_root_id = mi_root_id(&engine, &process_instance.id);
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let timers: Vec<_> = store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .filter(|j| j.is_boundary && j.activity_id == "miTimer")
        .collect();
    assert_eq!(
        timers.len(),
        1,
        "instance completion must not consume the MI boundary timer"
    );
    assert_eq!(timers[0].execution_id, mi_root_id);
}

/// Regression guard (green before and after): when the whole MI completes
/// normally, `cleanupMiRoot` removes the root-hosted boundary timer and the
/// process continues after the MI activity.
#[test]
fn mi_leave_cleans_root_boundary_timer() {
    let (engine, _time) = engine_with_time("mi-boundary-leave-cleanup");
    let definition_id = deploy(
        &engine,
        "mi_boundary_interrupting_timer.bpmn20.xml",
        PARALLEL_MI_INTERRUPTING_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id);
    let task_service = engine.get_task_service();

    for task in task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
    {
        task_service.complete_task_by_id(task.id).unwrap();
    }
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["afterMi"]);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let timers =
        store.find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        timers.is_empty(),
        "cleanupMiRoot must delete the boundary timer hosted on the MI root"
    );
    drop(session);
    assert!(
        all_executions(&engine, &process_instance.id)
            .iter()
            .all(|e| !e.is_multi_instance_root),
        "cleanupMiRoot must delete the MI root"
    );
}

/// Java: interrupting message boundary on an MI activity cancels the whole MI,
/// same as the timer variant (both go through `executeInterruptingBehavior`).
#[test]
fn interrupting_mi_message_boundary_cancels_whole_multi_instance() {
    let engine = ProcessEngine::new("mi-boundary-message-cancel".to_string());
    let definition_id = deploy(
        &engine,
        "mi_boundary_interrupting_message.bpmn20.xml",
        PARALLEL_MI_INTERRUPTING_MESSAGE_XML,
    );
    let process_instance = start(&engine, definition_id);
    assert_eq!(task_keys(&engine, &process_instance.id), vec!["miTask"; 3]);

    engine
        .get_runtime_service()
        .trigger_boundary_event_by_message_ref(
            "cancelMsg".to_string(),
            process_instance.id.clone(),
        );

    assert_eq!(
        task_keys(&engine, &process_instance.id),
        vec!["msgTask"],
        "interrupting message boundary on an MI activity cancels ALL instances"
    );
    assert!(
        all_executions(&engine, &process_instance.id)
            .iter()
            .all(|e| !e.is_multi_instance_root),
        "the MI root execution must be deleted by the interrupting trigger"
    );
}

/// Regression guard (green before and after): non-MI boundary timer keeps its
/// per-activity hosting — one timer hosted on the task execution itself.
#[test]
fn non_mi_boundary_timer_still_hosts_on_task_execution() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="plainBoundaryTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="hostTask" />
        <userTask id="hostTask" name="Host" />
        <boundaryEvent id="hostTimer" attachedToRef="hostTask" cancelActivity="true">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="hostTask" targetRef="end" />
        <sequenceFlow id="f3" sourceRef="hostTimer" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;
    let (engine, _time) = engine_with_time("plain-boundary-host");
    let definition_id = deploy(&engine, "plain_boundary_timer.bpmn20.xml", xml);
    let process_instance = start(&engine, definition_id);

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let timers: Vec<_> = store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .filter(|j| j.is_boundary && j.activity_id == "hostTimer")
        .collect();
    assert_eq!(timers.len(), 1);
    assert_eq!(
        timers[0].execution_id, tasks[0].execution_id,
        "non-MI boundary timer keeps hosting on the task execution"
    );
}
