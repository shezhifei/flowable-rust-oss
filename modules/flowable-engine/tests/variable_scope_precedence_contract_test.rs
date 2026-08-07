//! Contract tests for `collect_execution_variables` vs `Execution::process_variable`
//! in-row precedence consistency.
//!
//! Background: `Execution::process_variable(name)` resolves transient → local → variables.
//! `collect_execution_variables` inserts `variables` first, then `local_variables`, then
//! `transient_variables` — all via `or_insert`. Within a single row, this means `variables`
//! wins over `local_variables`, which is the *opposite* of `process_variable`.
//!
//! The write paths (P3-1, P3-2) are designed to never produce a state where the same
//! execution row holds the same key in both `variables` and `local_variables`. This test
//! constructs that state via direct store manipulation to determine whether the inconsistency
//! is observable through public APIs.

use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

const SIMPLE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="simpleProcess" name="Simple Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Task 1" />
        <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

fn subprocess_fork_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="localScopeProcess" name="Local Scope Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="outerSub" />
            <subProcess id="outerSub" name="Outer Sub">
                <startEvent id="subStart" />
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="fork" />
                <parallelGateway id="fork" />
                <sequenceFlow id="sf2" sourceRef="fork" targetRef="taskA" />
                <userTask id="taskA" name="Task A" />
                <sequenceFlow id="sf3" sourceRef="fork" targetRef="taskB" />
                <userTask id="taskB" name="Task B" />
                <sequenceFlow id="sf4" sourceRef="taskA" targetRef="join" />
                <sequenceFlow id="sf5" sourceRef="taskB" targetRef="join" />
                <parallelGateway id="join" />
                <sequenceFlow id="sf6" sourceRef="join" targetRef="subEnd" />
                <endEvent id="subEnd" />
            </subProcess>
            <sequenceFlow id="f2" sourceRef="outerSub" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
        .to_string()
}

fn deploy_simple(engine: &ProcessEngine) -> String {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("simple.bpmn20.xml".to_string(), SIMPLE_TASK_XML.to_string()),
    )
    .unwrap();
    let definition_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap();
    pi.id
}

fn deploy_fork(engine: &ProcessEngine) -> String {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("fork.bpmn20.xml".to_string(), subprocess_fork_xml()),
    )
    .unwrap();
    let definition_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap();
    pi.id
}

fn child_execution_id(engine: &ProcessEngine, activity_id: &str) -> String {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| {
            execution.activity_id.as_deref() == Some(activity_id)
                && execution.parent_id.is_some()
                && !execution.is_ended
        })
        .expect("a child execution should exist at the requested activity")
        .id
}

fn scope_execution_id(engine: &ProcessEngine) -> String {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| execution.parent_id.is_none() && !execution.is_ended)
        .expect("the subprocess scope execution should exist")
        .id
}

// ═══════════════════════════════════════════════════════════════════════════
// Regression guard: cross-row nearest-scope-wins works correctly.
// This test must pass BEFORE any fix to collect_execution_variables.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn cross_row_nearest_scope_wins_in_collect() {
    let engine = ProcessEngine::new("precedence-cross-row".to_string());
    deploy_fork(&engine);
    let runtime = engine.get_runtime_service();
    let scope = scope_execution_id(&engine);
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .set_variable(scope.clone(), "shared".to_string(), json!("ancestor"))
        .unwrap();
    runtime
        .set_variable_local(task_a.clone(), "shared".to_string(), json!("child"))
        .unwrap();

    // get_variables (collect_execution_variables) must return the child's value
    // because the child row is the starting row and is inserted first (nearest scope).
    let vars = runtime.get_variables(task_a.clone()).unwrap();
    assert_eq!(
        vars.get("shared"),
        Some(&json!("child")),
        "cross-row: nearest scope must win in collect_execution_variables"
    );

    // get_variable (find_execution_variable -> process_variable) agrees.
    assert_eq!(
        runtime.get_variable(task_a, "shared".to_string()).unwrap(),
        Some(json!("child")),
        "cross-row: nearest scope must win in find_execution_variable"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Observability probe: within a single row, do get_variable and get_variables
// return different values when variables and local_variables both hold same key?
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn in_row_precedence_is_consistent_between_get_variable_and_get_variables() {
    let engine = ProcessEngine::new("precedence-in-row".to_string());
    let _pi_id = deploy_simple(&engine);
    let runtime = engine.get_runtime_service();

    // Find the execution row (simple process has one execution row = root).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let exec_id = {
        let execs = store.snapshot_executions(&mut session);
        execs
            .into_values()
            .find(|e| !e.is_ended)
            .expect("should have an active execution")
            .id
    };

    // Construct the conflicting state: same key in both `variables` and
    // `local_variables` with different values. Normal write paths prevent this,
    // so we use direct store manipulation.
    let mut execution = store
        .find_execution(&exec_id, &mut session)
        .expect("execution must exist");
    execution
        .variables
        .insert("conflict".to_string(), json!("from-variables"));
    execution
        .local_variables
        .insert("conflict".to_string(), json!("from-local"));
    store.update_execution(&execution, &mut session);
    session.flush_and_commit().unwrap();

    // get_variable uses process_variable(): transient -> local -> variables
    // Expected: "from-local"
    let single = runtime
        .get_variable(exec_id.clone(), "conflict".to_string())
        .unwrap();

    // get_variables uses collect_execution_variables which inserts variables first
    // Expected (before fix): "from-variables" — DIFFERENT from get_variable
    let all = runtime.get_variables(exec_id.clone()).unwrap();

    // The contract: both APIs must agree.
    assert_eq!(
        single.as_ref(),
        all.get("conflict"),
        "in-row precedence must be consistent: get_variable returned {:?} \
         but get_variables returned {:?}",
        single,
        all.get("conflict")
    );

    // Additionally, local_variables should win (matching process_variable semantics).
    assert_eq!(
        all.get("conflict"),
        Some(&json!("from-local")),
        "local_variables must take precedence over variables within the same row"
    );
}
