//! Contract tests for the process-instance scope execution row in top-level
//! parallel / inclusive topologies, mirroring the Java model where the process
//! instance IS an `ExecutionEntity` (`ExecutionEntityImpl`) and therefore an
//! execution row for the process instance always exists:
//!   - a top-level parallel fork must not delete the process-instance scope
//!     execution row; it is kept as the inactive scope parent of the branches;
//!   - a top-level inclusive split behaves the same;
//!   - branch executions resolve process-level variables through the `parent_id`
//!     chain, so a variable set on the process instance AFTER the fork is
//!     visible from a branch execution (Java `VariableScopeImpl#getVariable`).
//!
//! Before the fix the fork deleted the root row and the branch executions were
//! cloned with a snapshot copy of its variables, which masked the broken chain
//! for start variables but made every later process-level write unreachable.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::runtime::process_instance::ProcessInstance;
use serde_json::json;

fn top_level_parallel_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="topLevelParallel" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="f2" sourceRef="fork" targetRef="taskA" />
            <userTask id="taskA" name="Task A" />
            <sequenceFlow id="f3" sourceRef="fork" targetRef="taskB" />
            <userTask id="taskB" name="Task B" />
            <sequenceFlow id="f4" sourceRef="taskA" targetRef="join" />
            <sequenceFlow id="f5" sourceRef="taskB" targetRef="join" />
            <parallelGateway id="join" />
            <sequenceFlow id="f6" sourceRef="join" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
        .to_string()
}

fn top_level_inclusive_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="topLevelInclusive" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="fork" />
            <inclusiveGateway id="fork" />
            <sequenceFlow id="f2" sourceRef="fork" targetRef="taskA" />
            <userTask id="taskA" name="Task A" />
            <sequenceFlow id="f3" sourceRef="fork" targetRef="taskB" />
            <userTask id="taskB" name="Task B" />
            <sequenceFlow id="f4" sourceRef="taskA" targetRef="join" />
            <sequenceFlow id="f5" sourceRef="taskB" targetRef="join" />
            <inclusiveGateway id="join" />
            <sequenceFlow id="f6" sourceRef="join" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
        .to_string()
}

fn deploy_and_start(engine: &ProcessEngine, resource_name: &str, xml: String) -> ProcessInstance {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string(resource_name.to_string(), xml),
    )
    .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(definition_id)
        .variable("orderId".to_string(), json!("ORD-1"));
    engine
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap()
}

fn find_execution(engine: &ProcessEngine, execution_id: &str) -> Option<Execution> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.find_execution(execution_id, &mut session)
}

fn child_execution_id(engine: &ProcessEngine, activity_id: &str) -> String {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| {
            execution.activity_id.as_deref() == Some(activity_id)
                && execution.parent_id.is_some()
                && !execution.is_ended
        })
        .expect("a branch execution should exist at the requested activity")
        .id
}

/// The process-instance scope execution row survives a top-level parallel fork
/// as the (inactive) scope parent of the branch executions.
#[test]
fn top_level_parallel_fork_preserves_process_instance_scope_execution() {
    let engine = ProcessEngine::new("top-level-parallel-scope-row".to_string());
    let process_instance = deploy_and_start(
        &engine,
        "top_level_parallel.bpmn20.xml",
        top_level_parallel_xml(),
    );

    let root = find_execution(&engine, &process_instance.id)
        .expect("the process instance scope execution row must survive the fork");
    assert!(
        root.is_scope,
        "the preserved process instance row must be the scope execution"
    );
    assert!(
        !root.is_active,
        "the preserved process instance row is inactive while branches run"
    );
    assert_eq!(root.parent_id, None);

    let branch_a = child_execution_id(&engine, "taskA");
    let branch_b = child_execution_id(&engine, "taskB");
    let branch_a_row = find_execution(&engine, &branch_a).unwrap();
    let branch_b_row = find_execution(&engine, &branch_b).unwrap();
    assert_eq!(
        branch_a_row.parent_id.as_deref(),
        Some(process_instance.id.as_str())
    );
    assert_eq!(
        branch_b_row.parent_id.as_deref(),
        Some(process_instance.id.as_str())
    );
}

/// Java parity: `VariableScopeImpl#getVariable` walks the parent scope chain.
/// A variable written to the process instance scope AFTER the fork must be
/// visible from a branch execution. Before the fix the write had no execution
/// row to land on and the branch's parent chain pointed at a deleted row.
#[test]
fn top_level_parallel_branch_resolves_process_variable_written_after_fork() {
    let engine = ProcessEngine::new("top-level-parallel-parent-chain".to_string());
    let process_instance = deploy_and_start(
        &engine,
        "top_level_parallel.bpmn20.xml",
        top_level_parallel_xml(),
    );
    let runtime = engine.get_runtime_service();

    runtime
        .set_variable(
            process_instance.id.clone(),
            "afterFork".to_string(),
            json!("late"),
        )
        .expect("writing a process variable after the fork should succeed");

    let branch_a = child_execution_id(&engine, "taskA");
    assert_eq!(
        runtime
            .get_variable(branch_a.clone(), "afterFork".to_string())
            .unwrap(),
        Some(json!("late")),
        "the branch must resolve the process-level variable through the parent chain"
    );
    // Start variables resolve through the parent chain as well.
    assert_eq!(
        runtime
            .get_variable(branch_a, "orderId".to_string())
            .unwrap(),
        Some(json!("ORD-1"))
    );
    // And the process instance scope itself answers getVariables.
    let variables = runtime.get_variables(process_instance.id.clone()).unwrap();
    assert_eq!(variables.get("afterFork"), Some(&json!("late")));
    assert_eq!(variables.get("orderId"), Some(&json!("ORD-1")));
}

/// The top-level inclusive split goes through
/// `TakeOutgoingSequenceFlowsOperation`, which deleted the arriving execution
/// unconditionally. The process-instance scope row must survive here too.
#[test]
fn top_level_inclusive_split_preserves_process_instance_scope_execution() {
    let engine = ProcessEngine::new("top-level-inclusive-scope-row".to_string());
    let process_instance = deploy_and_start(
        &engine,
        "top_level_inclusive.bpmn20.xml",
        top_level_inclusive_xml(),
    );

    let root = find_execution(&engine, &process_instance.id)
        .expect("the process instance scope execution row must survive the inclusive split");
    assert!(root.is_scope);
    assert!(!root.is_active);

    let branch_a = child_execution_id(&engine, "taskA");
    let branch_a_row = find_execution(&engine, &branch_a).unwrap();
    assert_eq!(
        branch_a_row.parent_id.as_deref(),
        Some(process_instance.id.as_str())
    );
}

/// Regression guard (green before and after the fix): the fork/join flow itself
/// is unchanged — both branches get their task, and completing both ends the
/// process instance.
#[test]
fn top_level_parallel_fork_join_semantics_unchanged() {
    let engine = ProcessEngine::new("top-level-parallel-regression".to_string());
    let process_instance = deploy_and_start(
        &engine,
        "top_level_parallel.bpmn20.xml",
        top_level_parallel_xml(),
    );
    let task_service = engine.get_task_service();

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();
    assert_eq!(task_keys, vec!["taskA".to_string(), "taskB".to_string()]);

    for task in task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
    {
        task_service.complete_task_by_id(task.id).unwrap();
    }

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored_pi = store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(stored_pi.is_ended);
}

/// Single-storage invariant (Java parity: the process instance IS an
/// `ExecutionEntity`, so there is exactly one process-level variable store):
/// `ProcessInstance` carries no variable map at all — start variables live
/// only on the process-instance scope execution row and are read back through
/// `get_variables(process_instance_id)`.
#[test]
fn process_level_variables_live_only_on_the_scope_execution_row() {
    let engine = ProcessEngine::new("top-level-parallel-single-storage".to_string());
    let process_instance = deploy_and_start(
        &engine,
        "top_level_parallel.bpmn20.xml",
        top_level_parallel_xml(),
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored_pi = store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(!stored_pi.is_ended);
    drop(session);

    let variables = engine
        .get_runtime_service()
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(variables.get("orderId"), Some(&json!("ORD-1")));
}
