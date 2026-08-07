//! Contract tests for the persistent execution-local variable scope, mirroring Java
//! `RuntimeService#setVariableLocal` / `#getVariablesLocal` / `#removeVariableLocal` and
//! `ExecutionEntity` scope resolution:
//!   - execution-local variables survive the command transaction;
//!   - a child execution's local variable is invisible from its parent / the process instance;
//!   - a local variable shadows a same-named variable in an ancestor scope on read, without
//!     overwriting the ancestor's value;
//!   - `setVariable` resolves to the scope that already owns the name, otherwise the root;
//!   - `removeVariableLocal` only removes the execution's own copy;
//!   - `hasVariableLocal` is scope-strict while `hasVariable` follows the parent chain.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::runtime::process_instance::ProcessInstance;
use serde_json::json;
use std::collections::HashMap;

/// An embedded subprocess containing a parallel fork. This is the topology that gives a
/// genuine ancestor execution row (the scope execution, whose id equals the process instance
/// id) plus two sibling child executions under it, which is what the scope-resolution
/// assertions need. A top-level parallel fork is not usable here: these
/// assertions need the embedded-subprocess scope row with sibling child
/// executions beneath it.
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

fn deploy_and_start(engine: &ProcessEngine) -> ProcessInstance {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("localScope.bpmn20.xml".to_string(), subprocess_fork_xml()),
    )
    .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap()
}

/// Id of the child execution sitting at `activity_id` under the subprocess scope execution.
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

/// The subprocess scope execution, which is the ancestor scope of both branch executions.
/// Its id equals the process instance id in this topology.
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

#[test]
fn local_variable_survives_the_command_transaction() {
    let engine = ProcessEngine::new("local-scope-persistence".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let execution_id = child_execution_id(&engine, "taskA");

    runtime
        .set_variable_local(execution_id.clone(), "branch".to_string(), json!("a"))
        .expect("setting a local variable should succeed");

    // A separate command must still observe it: this is what `skip_serializing` prevented.
    assert_eq!(
        runtime
            .get_variable_local(execution_id.clone(), "branch".to_string())
            .unwrap(),
        Some(json!("a"))
    );
    let locals = runtime.get_variables_local(execution_id).unwrap();
    assert_eq!(locals.get("branch"), Some(&json!("a")));
}

#[test]
fn child_local_variable_is_invisible_from_parent_scope() {
    let engine = ProcessEngine::new("local-scope-isolation".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let scope = scope_execution_id(&engine);
    let task_a = child_execution_id(&engine, "taskA");
    let task_b = child_execution_id(&engine, "taskB");

    runtime
        .set_variable_local(task_a.clone(), "branchOnly".to_string(), json!("a"))
        .unwrap();

    // Invisible from the ancestor scope and from a sibling branch.
    assert_eq!(
        runtime
            .get_variables_local(scope.clone())
            .unwrap()
            .get("branchOnly"),
        None
    );
    assert_eq!(
        runtime
            .get_variable(scope, "branchOnly".to_string())
            .unwrap(),
        None
    );
    assert_eq!(
        runtime
            .get_variable(task_b, "branchOnly".to_string())
            .unwrap(),
        None
    );
    // But visible from the owning execution.
    assert_eq!(
        runtime
            .get_variable(task_a, "branchOnly".to_string())
            .unwrap(),
        Some(json!("a"))
    );
}

#[test]
fn local_variable_shadows_ancestor_value_without_overwriting_it() {
    let engine = ProcessEngine::new("local-scope-shadowing".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let scope = scope_execution_id(&engine);
    let task_a = child_execution_id(&engine, "taskA");
    let task_b = child_execution_id(&engine, "taskB");

    runtime
        .set_variable(scope.clone(), "reviewer".to_string(), json!("global"))
        .unwrap();
    runtime
        .set_variable_local(task_a.clone(), "reviewer".to_string(), json!("local-a"))
        .unwrap();

    // Nearest scope wins on read from the owning execution.
    assert_eq!(
        runtime
            .get_variable(task_a.clone(), "reviewer".to_string())
            .unwrap(),
        Some(json!("local-a"))
    );
    // The ancestor value is untouched, and a sibling still resolves to it.
    assert_eq!(
        runtime.get_variable(scope, "reviewer".to_string()).unwrap(),
        Some(json!("global"))
    );
    assert_eq!(
        runtime
            .get_variable(task_b, "reviewer".to_string())
            .unwrap(),
        Some(json!("global"))
    );
    // And the merged view from the owning execution shows the shadowing value.
    assert_eq!(
        runtime.get_variables(task_a).unwrap().get("reviewer"),
        Some(&json!("local-a"))
    );
}

#[test]
fn remove_variable_local_only_removes_the_own_copy() {
    let engine = ProcessEngine::new("local-scope-remove".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let scope = scope_execution_id(&engine);
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .set_variable(scope.clone(), "reviewer".to_string(), json!("global"))
        .unwrap();
    runtime
        .set_variable_local(task_a.clone(), "reviewer".to_string(), json!("local-a"))
        .unwrap();

    runtime
        .remove_variable_local(task_a.clone(), "reviewer".to_string())
        .expect("removing the local copy should succeed");

    assert_eq!(
        runtime
            .get_variable_local(task_a.clone(), "reviewer".to_string())
            .unwrap(),
        None
    );
    // The ancestor variable survives and is again what the child resolves to.
    assert_eq!(
        runtime
            .get_variable(task_a, "reviewer".to_string())
            .unwrap(),
        Some(json!("global"))
    );
    assert_eq!(
        runtime.get_variable(scope, "reviewer".to_string()).unwrap(),
        Some(json!("global"))
    );
}

#[test]
fn has_variable_local_is_scope_strict_while_has_variable_walks_the_chain() {
    let engine = ProcessEngine::new("local-scope-has".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let scope = scope_execution_id(&engine);
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .set_variable(scope.clone(), "globalOnly".to_string(), json!(1))
        .unwrap();
    runtime
        .set_variable_local(task_a.clone(), "localOnly".to_string(), json!(2))
        .unwrap();

    assert!(
        !runtime
            .has_variable_local(task_a.clone(), "globalOnly".to_string())
            .unwrap(),
        "an ancestor-owned variable is not local to the child"
    );
    assert!(
        runtime
            .has_variable(task_a.clone(), "globalOnly".to_string())
            .unwrap(),
        "hasVariable follows the parent chain"
    );
    assert!(
        runtime
            .has_variable_local(task_a.clone(), "localOnly".to_string())
            .unwrap()
    );
    assert!(
        !runtime
            .has_variable_local(scope, "localOnly".to_string())
            .unwrap(),
        "a child's local variable is not local to the parent"
    );
}

#[test]
fn set_variable_resolves_to_the_owning_scope_otherwise_root() {
    let engine = ProcessEngine::new("local-scope-set-resolution".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let scope = scope_execution_id(&engine);
    let task_a = child_execution_id(&engine, "taskA");

    // Owned locally: a global-style set must update the local owner, not the root.
    runtime
        .set_variable_local(task_a.clone(), "owned".to_string(), json!("local-v1"))
        .unwrap();
    runtime
        .set_variable(task_a.clone(), "owned".to_string(), json!("local-v2"))
        .unwrap();
    assert_eq!(
        runtime
            .get_variable_local(task_a.clone(), "owned".to_string())
            .unwrap(),
        Some(json!("local-v2"))
    );
    assert_eq!(
        runtime
            .get_variables_local(scope.clone())
            .unwrap()
            .get("owned"),
        None,
        "the ancestor must not receive a copy when a descendant already owns the name"
    );

    // Not owned anywhere: falls back to the root execution of the chain.
    runtime
        .set_variable(task_a.clone(), "unowned".to_string(), json!("root-v1"))
        .unwrap();
    assert_eq!(
        runtime.get_variable(scope, "unowned".to_string()).unwrap(),
        Some(json!("root-v1")),
        "an unowned name resolves to the root of the execution chain"
    );
    assert_eq!(
        runtime
            .get_variable_local(task_a, "unowned".to_string())
            .unwrap(),
        None
    );
}

#[test]
fn bulk_local_variable_operations() {
    let engine = ProcessEngine::new("local-scope-bulk".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let task_a = child_execution_id(&engine, "taskA");

    let mut variables = HashMap::new();
    variables.insert("one".to_string(), json!(1));
    variables.insert("two".to_string(), json!(2));
    variables.insert("three".to_string(), json!(3));
    runtime
        .set_variables_local(task_a.clone(), variables)
        .expect("bulk local set should succeed");

    let locals = runtime.get_variables_local(task_a.clone()).unwrap();
    assert_eq!(locals.get("one"), Some(&json!(1)));
    assert_eq!(locals.get("two"), Some(&json!(2)));
    assert_eq!(locals.get("three"), Some(&json!(3)));

    runtime
        .remove_variables_local(task_a.clone(), vec!["one".to_string(), "three".to_string()])
        .expect("bulk local removal should succeed");

    let remaining = runtime.get_variables_local(task_a).unwrap();
    assert_eq!(remaining.get("one"), None);
    assert_eq!(remaining.get("two"), Some(&json!(2)));
    assert_eq!(remaining.get("three"), None);
}

#[test]
fn local_variable_apis_reject_unknown_execution() {
    let engine = ProcessEngine::new("local-scope-unknown".to_string());
    let runtime = engine.get_runtime_service();

    let set_error = runtime
        .set_variable_local("missing-execution".to_string(), "x".to_string(), json!(1))
        .expect_err("unknown execution must not be writable");
    assert!(matches!(set_error, FlowableError::NotFound(_)));
    assert!(set_error.to_string().contains("missing-execution"));

    let get_error = runtime
        .get_variables_local("missing-execution".to_string())
        .expect_err("unknown execution must not be readable");
    assert!(matches!(get_error, FlowableError::NotFound(_)));

    let remove_error = runtime
        .remove_variable_local("missing-execution".to_string(), "x".to_string())
        .expect_err("unknown execution must not be mutable");
    assert!(matches!(remove_error, FlowableError::NotFound(_)));

    let has_error = runtime
        .has_variable_local("missing-execution".to_string(), "x".to_string())
        .expect_err("unknown execution must not be queryable");
    assert!(matches!(has_error, FlowableError::NotFound(_)));
}
