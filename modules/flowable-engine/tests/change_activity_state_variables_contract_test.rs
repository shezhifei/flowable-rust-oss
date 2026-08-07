//! Contract tests for change-activity-state variable injection, mirroring Java
//! `org.flowable.engine.runtime.ChangeActivityStateBuilder` /
//! `AbstractDynamicStateManager#doMoveExecutionState` semantics:
//!   - `processVariables` are written to the process instance execution *before* the
//!     move is actioned, so they are visible to the activities started by the move;
//!   - `localVariables` are keyed by target activity id and written as execution-local
//!     variables on the executions created at that activity;
//!   - a local variable keyed by an activity that is not started by the move is ignored;
//!   - variable injection is available on both the process-instance-level and the
//!     execution-level entry points.

use flowable_engine::bpmn::listener::{
    ExecutionListenerContext, LocalExecutionListener, LocalExecutionListenerRegistry,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::runtime::process_instance::ProcessInstance;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

fn review_chain_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="changeStateVariablesProcess" name="Change State Variables Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="reviewA" />
            <userTask id="reviewA" name="Review A" />
            <sequenceFlow id="f2" sourceRef="reviewA" targetRef="reviewB" />
            <userTask id="reviewB" name="Review B" />
            <sequenceFlow id="f3" sourceRef="reviewB" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
        .to_string()
}

/// Copies the execution-local variable `localNote` into a persisted process variable so a
/// test can observe whether the injected local variable was visible to the activity the
/// move started, within the same transaction.
struct EchoLocalVariableListener;

impl LocalExecutionListener for EchoLocalVariableListener {
    fn notify(&self, ctx: &mut ExecutionListenerContext<'_>) -> Result<(), FlowableError> {
        let observed = ctx
            .execution
            .local_variables
            .get("localNote")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        ctx.execution
            .set_process_variable("observedLocalNote".to_string(), observed);
        Ok(())
    }
}

fn review_chain_xml_with_listener() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="changeStateVariablesProcess" name="Change State Variables Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="reviewA" />
            <userTask id="reviewA" name="Review A" />
            <sequenceFlow id="f2" sourceRef="reviewA" targetRef="reviewB" />
            <userTask id="reviewB" name="Review B">
                <extensionElements>
                    <flowable:executionListener event="start" class="echoLocal" />
                </extensionElements>
            </userTask>
            <sequenceFlow id="f3" sourceRef="reviewB" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
        .to_string()
}

fn engine_with_echo_listener(name: &str) -> ProcessEngine {
    let mut registry = LocalExecutionListenerRegistry::new();
    registry.register("echoLocal", Arc::new(EchoLocalVariableListener));
    let mut config = ProcessEngineConfiguration::default();
    config.execution_listener_registry = Some(registry);
    ProcessEngine::new_with_config(name.to_string(), config)
}

fn deploy_and_start_xml(engine: &ProcessEngine, xml: String) -> ProcessInstance {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("changeStateVariables.bpmn20.xml".to_string(), xml),
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

fn deploy_and_start(engine: &ProcessEngine) -> ProcessInstance {
    let repo = engine.get_repository_service();
    repo.deploy(repo.create_deployment().add_string(
        "changeStateVariables.bpmn20.xml".to_string(),
        review_chain_xml(),
    ))
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

fn process_variables(engine: &ProcessEngine, id: &str) -> HashMap<String, serde_json::Value> {
    engine
        .get_runtime_service()
        .get_variables(id.to_string())
        .unwrap()
}

fn execution_at_activity(
    engine: &ProcessEngine,
    process_instance_id: &str,
    activity_id: &str,
) -> flowable_engine::runtime::execution::Execution {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && execution.activity_id.as_deref() == Some(activity_id)
                && !execution.is_ended
        })
        .expect("an active execution should exist at the requested activity")
}

#[test]
fn process_instance_change_state_injects_process_variables() {
    let engine = ProcessEngine::new("change-state-vars-process-level".to_string());
    let instance = deploy_and_start(&engine);

    let mut variables = HashMap::new();
    variables.insert("reviewer".to_string(), json!("alice"));
    variables.insert("escalated".to_string(), json!(true));

    engine
        .get_runtime_service()
        .change_process_instance_activity_state_with_variables(
            instance.id.clone(),
            vec!["reviewA".to_string()],
            vec!["reviewB".to_string()],
            variables,
            HashMap::new(),
        )
        .expect("change state with process variables should succeed");

    let stored = process_variables(&engine, &instance.id);
    assert_eq!(stored.get("reviewer"), Some(&json!("alice")));
    assert_eq!(stored.get("escalated"), Some(&json!(true)));
}

/// Java applies `localVariables` to the executions created at the moved-to activity so the
/// started activity can read them, and they persist in that execution's local scope. Asserts
/// both: the injected variable is readable by the started activity, and it is still readable
/// as an execution-local variable after the command commits.
#[test]
fn process_instance_change_state_local_variables_are_visible_to_started_activity() {
    let engine = engine_with_echo_listener("change-state-vars-local");
    let instance = deploy_and_start_xml(&engine, review_chain_xml_with_listener());

    let mut local_for_review_b = HashMap::new();
    local_for_review_b.insert("localNote".to_string(), json!("scoped-to-reviewB"));
    let mut local_variables = HashMap::new();
    local_variables.insert("reviewB".to_string(), local_for_review_b);
    // Keyed by an activity the move does not start: Java only applies local variables
    // to executions created at a moved-to activity, so this entry must be ignored.
    let mut ignored = HashMap::new();
    ignored.insert("neverApplied".to_string(), json!("ignored"));
    local_variables.insert("reviewA".to_string(), ignored);

    engine
        .get_runtime_service()
        .change_process_instance_activity_state_with_variables(
            instance.id.clone(),
            vec!["reviewA".to_string()],
            vec!["reviewB".to_string()],
            HashMap::new(),
            local_variables,
        )
        .expect("change state with local variables should succeed");

    let stored = process_variables(&engine, &instance.id);
    assert_eq!(
        stored.get("observedLocalNote"),
        Some(&json!("scoped-to-reviewB")),
        "the local variable keyed by the moved-to activity must be readable by that activity"
    );
    // A key for an activity that was not started must never be applied.
    assert_eq!(stored.get("neverApplied"), None);

    // The injected local variable now outlives the command, in the local scope of the
    // execution the move started.
    let moved_execution = execution_at_activity(&engine, &instance.id, "reviewB");
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(moved_execution.id, "localNote".to_string())
            .unwrap(),
        Some(json!("scoped-to-reviewB"))
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .has_variable_local(instance.id, "neverApplied".to_string())
            .unwrap(),
        false
    );
}

/// A local-variable key that matches no moved-to activity is ignored entirely.
#[test]
fn change_state_local_variables_for_unstarted_activity_are_ignored() {
    let engine = engine_with_echo_listener("change-state-vars-local-unmatched");
    let instance = deploy_and_start_xml(&engine, review_chain_xml_with_listener());

    let mut unmatched = HashMap::new();
    unmatched.insert("localNote".to_string(), json!("never-reaches-reviewB"));
    let mut local_variables = HashMap::new();
    local_variables.insert("someOtherActivity".to_string(), unmatched);

    engine
        .get_runtime_service()
        .change_process_instance_activity_state_with_variables(
            instance.id.clone(),
            vec!["reviewA".to_string()],
            vec!["reviewB".to_string()],
            HashMap::new(),
            local_variables,
        )
        .expect("change state should succeed");

    let stored = process_variables(&engine, &instance.id);
    assert_eq!(stored.get("observedLocalNote"), Some(&json!(null)));
    assert_eq!(stored.get("localNote"), None);
}

#[test]
fn execution_change_state_injects_process_and_local_variables() {
    let engine = engine_with_echo_listener("change-state-vars-execution-level");
    let instance = deploy_and_start_xml(&engine, review_chain_xml_with_listener());
    let execution_id = execution_at_activity(&engine, &instance.id, "reviewA").id;

    let mut variables = HashMap::new();
    variables.insert("origin".to_string(), json!("execution-level"));
    let mut local_for_review_b = HashMap::new();
    local_for_review_b.insert("localNote".to_string(), json!("execution-local"));
    let mut local_variables = HashMap::new();
    local_variables.insert("reviewB".to_string(), local_for_review_b);

    engine
        .get_runtime_service()
        .change_execution_activity_state_with_variables(
            execution_id,
            vec!["reviewA".to_string()],
            vec!["reviewB".to_string()],
            variables,
            local_variables,
        )
        .expect("execution-level change state with variables should succeed");

    let stored = process_variables(&engine, &instance.id);
    assert_eq!(stored.get("origin"), Some(&json!("execution-level")));
    assert_eq!(
        stored.get("observedLocalNote"),
        Some(&json!("execution-local"))
    );
}

#[test]
fn change_state_variables_are_not_written_when_the_move_fails() {
    let engine = ProcessEngine::new("change-state-vars-rollback".to_string());
    let instance = deploy_and_start(&engine);

    let mut variables = HashMap::new();
    variables.insert("shouldNotPersist".to_string(), json!("nope"));

    let error = engine
        .get_runtime_service()
        .change_process_instance_activity_state_with_variables(
            instance.id.clone(),
            vec!["reviewA".to_string()],
            vec!["missingActivity".to_string()],
            variables,
            HashMap::new(),
        )
        .expect_err("moving to an unknown activity must fail");
    assert!(error.to_string().contains("missingActivity"));

    let stored = process_variables(&engine, &instance.id);
    assert_eq!(stored.get("shouldNotPersist"), None);
}
