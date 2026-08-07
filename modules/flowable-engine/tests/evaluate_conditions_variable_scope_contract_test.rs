//! Contract tests for the variable scope a conditional-event condition is
//! evaluated against.
//!
//! Java reference: `EvaluateConditionalEventsCmd` plans an agenda operation and
//! `EvaluateConditionalEventsOperation` evaluates each condition through
//! `ConditionUtil.hasTrueCondition(..., DelegateExecution)`. The expression
//! resolves names via `VariableScopeImpl#getVariable`, which checks the
//! execution's own scope and then delegates to the PARENT scope chain. So the
//! effective scope is "own variables plus every ancestor's variables", nearest
//! scope winning.
//!
//! Rust stores one execution row as three maps (`variables`, `local_variables`,
//! `transient_variables`) and additionally keeps process-level variables on the
//! `ProcessInstance`. Every one of those has to participate, or a condition
//! silently evaluates to false against a variable the caller can read back
//! through `RuntimeService`.

use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;
use std::collections::HashMap;

/// An embedded subprocess with a parallel fork: one branch waits on a
/// conditional intermediate catch event, the other on a user task. This gives a
/// child execution whose condition has to resolve through its ancestor scope
/// row, which is the only way the Rust engine materializes a real ancestor.
const SUBPROCESS_CONDITION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="condScopeProcess" name="Condition Scope" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toSub" sourceRef="start" targetRef="outerSub" />
    <subProcess id="outerSub" name="Outer Subprocess">
      <startEvent id="subStart" />
      <sequenceFlow id="toFork" sourceRef="subStart" targetRef="fork" />
      <parallelGateway id="fork" />
      <sequenceFlow id="toCatch" sourceRef="fork" targetRef="catchApproved" />
      <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
      <intermediateCatchEvent id="catchApproved" name="Catch Approved">
        <conditionalEventDefinition>
          <condition>${approved == true}</condition>
        </conditionalEventDefinition>
      </intermediateCatchEvent>
      <sequenceFlow id="afterCatch" sourceRef="catchApproved" targetRef="afterConditionTask" />
      <userTask id="afterConditionTask" name="After Condition" />
      <userTask id="holdTask" name="Hold" />
      <sequenceFlow id="toSubEnd" sourceRef="afterConditionTask" targetRef="subEnd" />
      <endEvent id="subEnd" />
    </subProcess>
    <sequenceFlow id="toEnd" sourceRef="outerSub" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

fn deploy_and_start(engine: &ProcessEngine, xml: &str, resource: &str) -> String {
    let repository_service = engine.get_repository_service();
    let builder = repository_service
        .create_deployment()
        .name(resource.to_string())
        .add_string(format!("{resource}.bpmn20.xml"), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap()
        .id
}

/// The scope execution row of the embedded subprocess (`parent_id == None`).
fn scope_execution_id(engine: &ProcessEngine) -> String {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| execution.parent_id.is_none() && !execution.is_ended)
        .map(|execution| execution.id)
        .expect("no scope execution")
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

/// A variable an ancestor execution holds in its execution-local scope must be
/// visible to a descendant's condition, exactly like Java's parent-scope
/// delegation. Before the fix only the ancestor's `variables` map was consulted,
/// so a local variable made the condition evaluate to false.
#[test]
fn ancestor_local_variables_are_visible_to_condition_evaluation() {
    let engine = ProcessEngine::new("cond-scope-ancestor-local".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        SUBPROCESS_CONDITION_XML,
        "cond_scope_ancestor_local",
    );
    let scope_id = scope_execution_id(&engine);

    assert_eq!(
        task_keys(&engine, &process_instance_id),
        vec!["holdTask".to_string()]
    );

    // Written into the ancestor's execution-local scope, not its process
    // variables.
    engine
        .get_runtime_service()
        .set_variable_local(scope_id.clone(), "approved".to_string(), json!(true))
        .unwrap();

    // Sanity check on the premise: the value really lives in the local map only.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let scope_execution = store.find_execution(&scope_id, &mut session).unwrap();
    drop(session);
    assert_eq!(
        scope_execution.local_variables.get("approved"),
        Some(&json!(true))
    );
    assert_eq!(
        scope_execution.variables.get("approved"),
        None,
        "the premise of this test is a local-only variable"
    );

    engine
        .get_runtime_service()
        .evaluate_conditional_events(process_instance_id.clone(), HashMap::new())
        .unwrap();

    let mut keys = task_keys(&engine, &process_instance_id);
    keys.sort();
    assert_eq!(
        keys,
        vec!["afterConditionTask".to_string(), "holdTask".to_string()],
        "the condition must see the ancestor's execution-local variable"
    );
}

/// Regression guard: the pre-existing paths that already worked keep working —
/// variables passed to the command itself, and process variables held on the
/// process instance / root execution row.
#[test]
fn condition_variables_passed_to_the_command_still_resolve() {
    let engine = ProcessEngine::new("cond-scope-command-variables".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        SUBPROCESS_CONDITION_XML,
        "cond_scope_command_variables",
    );

    engine
        .get_runtime_service()
        .evaluate_conditional_events(
            process_instance_id.clone(),
            HashMap::from([("approved".to_string(), json!(true))]),
        )
        .unwrap();

    assert_eq!(
        task_keys(&engine, &process_instance_id),
        vec!["afterConditionTask".to_string(), "holdTask".to_string()],
        "variables supplied to the command must reach the condition"
    );
}

/// A variable the descendant owns itself shadows the ancestor's value, matching
/// the nearest-scope-wins rule of `VariableScopeImpl#getVariable`.
#[test]
fn own_scope_shadows_the_ancestor_value_during_evaluation() {
    let engine = ProcessEngine::new("cond-scope-shadowing".to_string());
    let process_instance_id =
        deploy_and_start(&engine, SUBPROCESS_CONDITION_XML, "cond_scope_shadowing");
    let scope_id = scope_execution_id(&engine);

    // Ancestor says true, the waiting child says false: the child's own scope
    // must win, so the condition stays unsatisfied.
    engine
        .get_runtime_service()
        .set_variable_local(scope_id, "approved".to_string(), json!(true))
        .unwrap();

    let waiting_execution_id = {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store
            .snapshot_executions(&mut session)
            .into_values()
            .find(|execution| {
                execution.activity_id.as_deref() == Some("catchApproved")
                    && execution.parent_id.is_some()
                    && !execution.is_ended
            })
            .map(|execution| execution.id)
            .expect("no execution waiting on the conditional catch event")
    };
    engine
        .get_runtime_service()
        .set_variable_local(waiting_execution_id, "approved".to_string(), json!(false))
        .unwrap();

    engine
        .get_runtime_service()
        .evaluate_conditional_events(process_instance_id.clone(), HashMap::new())
        .unwrap();

    assert_eq!(
        task_keys(&engine, &process_instance_id),
        vec!["holdTask".to_string()],
        "the descendant's own value must shadow the ancestor's"
    );
}
