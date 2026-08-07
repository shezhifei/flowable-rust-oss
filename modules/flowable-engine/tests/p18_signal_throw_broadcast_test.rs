//! P18-A contract tests: signal intermediate throw is an ENGINE-WIDE broadcast.
//!
//! Java evidence: `IntermediateThrowSignalEventActivityBehavior#execute`
//! (flowable-engine, lines 79-103) — by default the throw queries all signal
//! event subscriptions by event NAME across the whole engine (all process
//! instances, plus signal start events which spawn new instances). Only when
//! the signal is declared with `flowable:scope="processInstance"` is delivery
//! narrowed to the throwing process instance.

use flowable_engine::engine::process_engine::ProcessEngine;

const CATCHER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <signal id="alertSignal" name="alert" />
    <process id="signalCatcherP18" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="catchAlert" />
        <intermediateCatchEvent id="catchAlert">
            <signalEventDefinition signalRef="alertSignal" />
        </intermediateCatchEvent>
        <sequenceFlow id="flow2" sourceRef="catchAlert" targetRef="afterCatch" />
        <userTask id="afterCatch" name="After Catch" />
        <sequenceFlow id="flow3" sourceRef="afterCatch" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const THROWER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <signal id="alertSignal" name="alert" />
    <process id="signalThrowerP18" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="throwAlert" />
        <intermediateThrowEvent id="throwAlert">
            <signalEventDefinition signalRef="alertSignal" />
        </intermediateThrowEvent>
        <sequenceFlow id="flow2" sourceRef="throwAlert" targetRef="afterThrow" />
        <userTask id="afterThrow" name="After Throw" />
        <sequenceFlow id="flow3" sourceRef="afterThrow" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

/// Throwing process declares the signal with `flowable:scope="processInstance"`
/// — the Java constructor flips `processInstanceScope=true` for it.
const SCOPED_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <signal id="alertSignal" name="alert" flowable:scope="processInstance" />
    <process id="signalScopedForkP18" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="fork" />
        <parallelGateway id="fork" />
        <sequenceFlow id="flow2" sourceRef="fork" targetRef="catchAlert" />
        <intermediateCatchEvent id="catchAlert">
            <signalEventDefinition signalRef="alertSignal" />
        </intermediateCatchEvent>
        <sequenceFlow id="flow3" sourceRef="catchAlert" targetRef="afterCatch" />
        <userTask id="afterCatch" name="After Catch" />
        <sequenceFlow id="flow4" sourceRef="fork" targetRef="throwAlert" />
        <intermediateThrowEvent id="throwAlert">
            <signalEventDefinition signalRef="alertSignal" />
        </intermediateThrowEvent>
        <sequenceFlow id="flow5" sourceRef="throwAlert" targetRef="afterThrow" />
        <userTask id="afterThrow" name="After Throw" />
    </process>
</definitions>"#;

const SIGNAL_START_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <signal id="alertSignal" name="alert" />
    <process id="signalStartP18" isExecutable="true">
        <startEvent id="sigStart">
            <signalEventDefinition signalRef="alertSignal" />
        </startEvent>
        <sequenceFlow id="flow1" sourceRef="sigStart" targetRef="startedTask" />
        <userTask id="startedTask" name="Started By Signal" />
        <sequenceFlow id="flow2" sourceRef="startedTask" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

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

fn task_keys(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    let mut keys = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

/// Java default: a signal throw wakes ALL process instances waiting on the
/// same signal, not just the throwing one.
#[test]
fn signal_throw_broadcasts_to_all_waiting_process_instances() {
    let engine = ProcessEngine::new("p18-signal-broadcast".to_string());
    deploy(&engine, "signal_catcher_p18", CATCHER_XML);
    deploy(&engine, "signal_thrower_p18", THROWER_XML);

    let catcher_one = start_by_key(&engine, "signalCatcherP18");
    let catcher_two = start_by_key(&engine, "signalCatcherP18");
    assert!(task_keys(&engine, &catcher_one).is_empty());
    assert!(task_keys(&engine, &catcher_two).is_empty());

    let thrower = start_by_key(&engine, "signalThrowerP18");

    assert_eq!(
        task_keys(&engine, &catcher_one),
        vec!["afterCatch".to_string()],
        "first waiting process instance must be woken by the engine-wide signal throw"
    );
    assert_eq!(
        task_keys(&engine, &catcher_two),
        vec!["afterCatch".to_string()],
        "second waiting process instance must be woken by the engine-wide signal throw"
    );
    assert_eq!(task_keys(&engine, &thrower), vec!["afterThrow".to_string()]);
}

/// `flowable:scope="processInstance"` narrows delivery to the throwing
/// process instance only (Java `Signal.SCOPE_PROCESS_INSTANCE`).
#[test]
fn signal_throw_with_process_instance_scope_only_wakes_own_instance() {
    let engine = ProcessEngine::new("p18-signal-pi-scope".to_string());
    deploy(&engine, "signal_scoped_fork_p18", SCOPED_FORK_XML);

    // Another instance of the same definition waits on the same signal.
    // Its throw branch must not wake the other instance's catch, so start
    // both and inspect afterwards: each instance only woke its own catch.
    let instance_one = start_by_key(&engine, "signalScopedForkP18");
    assert_eq!(
        task_keys(&engine, &instance_one),
        vec!["afterCatch".to_string(), "afterThrow".to_string()],
        "processInstance-scoped throw must wake the catch in its own instance"
    );

    let instance_two = start_by_key(&engine, "signalScopedForkP18");
    assert_eq!(
        task_keys(&engine, &instance_two),
        vec!["afterCatch".to_string(), "afterThrow".to_string()]
    );
    assert_eq!(
        task_keys(&engine, &instance_one),
        vec!["afterCatch".to_string(), "afterThrow".to_string()],
        "instance two's scoped throw must not have touched instance one"
    );
}

/// Guard: the scoped fork must NOT broadcast — a separate catcher process
/// instance waiting on the same signal stays asleep.
#[test]
fn signal_throw_with_process_instance_scope_leaves_other_instances_waiting() {
    let engine = ProcessEngine::new("p18-signal-pi-scope-guard".to_string());
    deploy(&engine, "signal_scoped_fork_p18", SCOPED_FORK_XML);
    deploy(&engine, "signal_catcher_p18", CATCHER_XML);

    let catcher = start_by_key(&engine, "signalCatcherP18");
    assert!(task_keys(&engine, &catcher).is_empty());

    let scoped = start_by_key(&engine, "signalScopedForkP18");
    assert_eq!(
        task_keys(&engine, &scoped),
        vec!["afterCatch".to_string(), "afterThrow".to_string()]
    );
    assert!(
        task_keys(&engine, &catcher).is_empty(),
        "processInstance-scoped signal throw must not wake other process instances"
    );
}

/// Java default: the engine-wide subscription query also matches signal START
/// event subscriptions, so a throw spawns new process instances.
#[test]
fn signal_throw_triggers_matching_signal_start_event() {
    let engine = ProcessEngine::new("p18-signal-start-trigger".to_string());
    deploy(&engine, "signal_start_p18", SIGNAL_START_XML);
    deploy(&engine, "signal_thrower_p18", THROWER_XML);

    let thrower = start_by_key(&engine, "signalThrowerP18");
    assert_eq!(task_keys(&engine, &thrower), vec!["afterThrow".to_string()]);

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let started = runtime_store
        .snapshot_process_instances(&mut session)
        .into_values()
        .filter(|pi| pi.process_definition_key == "signalStartP18")
        .collect::<Vec<_>>();
    session.rollback().unwrap();

    assert_eq!(
        started.len(),
        1,
        "signal throw must trigger the signal start event and spawn a new process instance"
    );
    assert_eq!(
        task_keys(&engine, &started[0].id),
        vec!["startedTask".to_string()]
    );
}
