//! P15 contract: terminate end event runtime semantics.
//!
//! Java reference: `TerminateEndEventActivityBehavior.java:60-207` and
//! `TerminateEndEventTest.java`. The current execution is always deleted
//! first, then:
//! - default: `findFirstScope` — a top-level PI terminates entirely, an
//!   embedded SubProcess scope is destroyed and the flow continues along the
//!   SubProcess's outgoing flows, a call activity child PI terminates and the
//!   parent process continues.
//! - `terminateAll=true`: the root process instance (across the call activity
//!   chain) and every child instance are terminated.

use flowable_engine::engine::process_engine::ProcessEngine;

/// 1. Top-level parallel branches: one waits on a user task, the other runs
/// into a terminate end event → the whole PI ends, the waiting task is gone.
#[test]
fn terminate_end_event_ends_whole_top_level_instance() {
    let engine = ProcessEngine::new("terminate-top-level".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="terminateTopLevel" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="f2" sourceRef="fork" targetRef="waitTask" />
            <sequenceFlow id="f3" sourceRef="fork" targetRef="preTerminate" />
            <userTask id="waitTask" />
            <userTask id="preTerminate" />
            <sequenceFlow id="f4" sourceRef="preTerminate" targetRef="terminateEnd" />
            <endEvent id="terminateEnd">
                <terminateEventDefinition />
            </endEvent>
            <sequenceFlow id="f5" sourceRef="waitTask" targetRef="normalEnd" />
            <endEvent id="normalEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("terminate-top-level".to_string())
                .add_string(
                    "terminate_top_level.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
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

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    let mut keys: Vec<_> = tasks
        .iter()
        .map(|t| t.task_definition_key.clone())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["preTerminate", "waitTask"]);

    let pre_terminate = tasks
        .iter()
        .find(|t| t.task_definition_key == "preTerminate")
        .unwrap();
    task_service
        .complete_task_by_id(pre_terminate.id.clone())
        .unwrap();

    // Whole PI terminated: waiting task gone, PI ended, no executions left.
    let tasks_after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert!(
        tasks_after.is_empty(),
        "terminate end event must remove the parallel waiting task, got {:?}",
        tasks_after
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi_row = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .unwrap();
    assert!(pi_row.is_ended, "process instance must be ended");
    let executions = runtime_store.snapshot_executions(&mut session);
    assert!(
        !executions.values().any(|e| {
            e.process_instance_id.as_deref() == Some(pi.id.as_str()) && e.activity_id.is_some()
        }),
        "no activity executions may survive a top-level terminate"
    );
}

/// 2. Terminate inside an embedded subprocess only destroys the subprocess
/// scope; the process continues along the subprocess's outgoing flow.
#[test]
fn terminate_end_event_in_embedded_subprocess_continues_outer_flow() {
    let engine = ProcessEngine::new("terminate-embedded-sub".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="terminateEmbedded" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="sub" />
            <subProcess id="sub">
                <startEvent id="subStart" />
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="fork" />
                <parallelGateway id="fork" />
                <sequenceFlow id="sf2" sourceRef="fork" targetRef="innerWait" />
                <sequenceFlow id="sf3" sourceRef="fork" targetRef="preTerminate" />
                <userTask id="innerWait" />
                <userTask id="preTerminate" />
                <sequenceFlow id="sf4" sourceRef="preTerminate" targetRef="terminateEnd" />
                <endEvent id="terminateEnd">
                    <terminateEventDefinition />
                </endEvent>
                <sequenceFlow id="sf5" sourceRef="innerWait" targetRef="subEnd" />
                <endEvent id="subEnd" />
            </subProcess>
            <sequenceFlow id="f2" sourceRef="sub" targetRef="afterTask" />
            <userTask id="afterTask" />
            <sequenceFlow id="f3" sourceRef="afterTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("terminate-embedded-sub".to_string())
                .add_string("terminate_embedded.bpmn20.xml".to_string(), xml.to_string()),
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

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    let pre_terminate = tasks
        .iter()
        .find(|t| t.task_definition_key == "preTerminate")
        .unwrap();
    task_service
        .complete_task_by_id(pre_terminate.id.clone())
        .unwrap();

    // Only the subprocess scope is destroyed: innerWait is gone, the outer
    // flow reaches afterTask, PI still running.
    let tasks_after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after.len(),
        1,
        "expected only afterTask, got {:?}",
        tasks_after
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(tasks_after[0].task_definition_key, "afterTask");

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi_row = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .unwrap();
    assert!(!pi_row.is_ended, "PI must continue after scope terminate");
    let executions = runtime_store.snapshot_executions(&mut session);
    assert!(
        !executions.values().any(|e| {
            e.process_instance_id.as_deref() == Some(pi.id.as_str())
                && matches!(e.activity_id.as_deref(), Some("sub") | Some("innerWait"))
        }),
        "subprocess scope and its children must be destroyed"
    );
    drop(session);

    task_service
        .complete_task_by_id(tasks_after[0].id.clone())
        .unwrap();
    let mut session = runtime_store.create_session().unwrap();
    let pi_row = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .unwrap();
    assert!(pi_row.is_ended);
}

const TERMINATE_CHILD_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="terminateChild" isExecutable="true">
        <startEvent id="childStart" />
        <sequenceFlow id="cf1" sourceRef="childStart" targetRef="fork" />
        <parallelGateway id="fork" />
        <sequenceFlow id="cf2" sourceRef="fork" targetRef="childWait" />
        <sequenceFlow id="cf3" sourceRef="fork" targetRef="preTerminate" />
        <userTask id="childWait" />
        <userTask id="preTerminate" />
        <sequenceFlow id="cf4" sourceRef="preTerminate" targetRef="terminateEnd" />
        <endEvent id="terminateEnd">
            <terminateEventDefinition />
        </endEvent>
        <sequenceFlow id="cf5" sourceRef="childWait" targetRef="childEnd" />
        <endEvent id="childEnd" />
    </process>
</definitions>"#;

const TERMINATE_ALL_CHILD_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="terminateAllChild" isExecutable="true">
        <startEvent id="childStart" />
        <sequenceFlow id="cf1" sourceRef="childStart" targetRef="preTerminate" />
        <userTask id="preTerminate" />
        <sequenceFlow id="cf2" sourceRef="preTerminate" targetRef="terminateEnd" />
        <endEvent id="terminateEnd">
            <terminateEventDefinition flowable:terminateAll="true" />
        </endEvent>
    </process>
</definitions>"#;

fn parent_xml(process_id: &str, called_element: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="{process_id}" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="callActivity" />
        <callActivity id="callActivity" calledElement="{called_element}" />
        <sequenceFlow id="f2" sourceRef="callActivity" targetRef="outerTask" />
        <userTask id="outerTask" />
        <sequenceFlow id="f3" sourceRef="outerTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />
    </process>
</definitions>"#
    )
}

/// 3. Terminate (without terminateAll) inside a call activity child: the
/// child PI ends, the parent continues from the call activity.
#[test]
fn terminate_end_event_in_call_activity_child_continues_parent() {
    let engine = ProcessEngine::new("terminate-call-activity".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("terminate-call-activity".to_string())
                .add_string(
                    "terminate_parent.bpmn20.xml".to_string(),
                    parent_xml("terminateParent", "terminateChild"),
                )
                .add_string(
                    "terminate_child.bpmn20.xml".to_string(),
                    TERMINATE_CHILD_XML.to_string(),
                ),
        )
        .unwrap();

    let parent_def_id = repository_service
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.starts_with("terminateParent"))
        .unwrap();
    let parent_pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(parent_def_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let child_pi = runtime_store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| pi.id != parent_pi.id && pi.super_execution_id.is_some())
        .expect("child process instance must exist");
    drop(session);

    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child_pi.id.clone())
        .unwrap();
    let pre_terminate = child_tasks
        .iter()
        .find(|t| t.task_definition_key == "preTerminate")
        .unwrap();
    task_service
        .complete_task_by_id(pre_terminate.id.clone())
        .unwrap();

    // Child PI terminated (childWait gone), parent resumed at outerTask.
    let child_tasks_after = task_service
        .get_tasks_by_process_instance_id(child_pi.id.clone())
        .unwrap();
    assert!(
        child_tasks_after.is_empty(),
        "child tasks must be gone after terminate, got {:?}",
        child_tasks_after
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );

    let mut session = runtime_store.create_session().unwrap();
    let child_row = runtime_store
        .find_process_instance(&child_pi.id, &mut session)
        .unwrap();
    assert!(child_row.is_ended, "child PI must be ended");
    let parent_row = runtime_store
        .find_process_instance(&parent_pi.id, &mut session)
        .unwrap();
    assert!(!parent_row.is_ended, "parent PI must keep running");
    drop(session);

    let parent_tasks = task_service
        .get_tasks_by_process_instance_id(parent_pi.id.clone())
        .unwrap();
    assert_eq!(parent_tasks.len(), 1);
    assert_eq!(parent_tasks[0].task_definition_key, "outerTask");
}

/// 4. `terminateAll=true` inside a call activity child terminates the root
/// (parent) process instance as well.
#[test]
fn terminate_all_in_call_activity_child_ends_parent_too() {
    let engine = ProcessEngine::new("terminate-all-call-activity".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("terminate-all-call-activity".to_string())
                .add_string(
                    "terminate_all_parent.bpmn20.xml".to_string(),
                    parent_xml("terminateAllParent", "terminateAllChild"),
                )
                .add_string(
                    "terminate_all_child.bpmn20.xml".to_string(),
                    TERMINATE_ALL_CHILD_XML.to_string(),
                ),
        )
        .unwrap();

    let parent_def_id = repository_service
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.starts_with("terminateAllParent"))
        .unwrap();
    let parent_pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(parent_def_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let child_pi = runtime_store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| pi.id != parent_pi.id && pi.super_execution_id.is_some())
        .expect("child process instance must exist");
    drop(session);

    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child_pi.id.clone())
        .unwrap();
    assert_eq!(child_tasks[0].task_definition_key, "preTerminate");
    task_service
        .complete_task_by_id(child_tasks[0].id.clone())
        .unwrap();

    // terminateAll reaches the root: both PIs ended, no tasks anywhere,
    // parent must NOT continue to outerTask.
    let mut session = runtime_store.create_session().unwrap();
    let child_row = runtime_store
        .find_process_instance(&child_pi.id, &mut session)
        .unwrap();
    assert!(child_row.is_ended, "child PI must be ended");
    let parent_row = runtime_store
        .find_process_instance(&parent_pi.id, &mut session)
        .unwrap();
    assert!(
        parent_row.is_ended,
        "terminateAll must terminate the root (parent) PI too"
    );
    drop(session);

    let parent_tasks = task_service
        .get_tasks_by_process_instance_id(parent_pi.id.clone())
        .unwrap();
    assert!(
        parent_tasks.is_empty(),
        "parent must not continue past the call activity, got {:?}",
        parent_tasks
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
}
