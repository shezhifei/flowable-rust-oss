//! Contract tests for behavior-internal EL evaluation walking the parent
//! VariableScope chain (P6-B).
//!
//! Companion to `in_flight_el_parent_chain_contract_test.rs` which covers
//! sequence-flow conditions. This file covers the remaining behavior-internal
//! EL evaluation sites: skipExpression, user-task assignee, script-task
//! variable binding, service-task delegateExpression, listener expressions,
//! call-activity IO, job category/retry, and intermediate-throw payload.

use std::sync::Arc;

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;

// ---------------------------------------------------------------------------
// Shared helpers (same shape as in_flight_el_parent_chain_contract_test.rs)
// ---------------------------------------------------------------------------

fn deploy_and_start(
    engine: &ProcessEngine,
    xml: &str,
    resource: &str,
    variables: Vec<(String, serde_json::Value)>,
) -> String {
    let repository_service = engine.get_repository_service();
    let builder = repository_service
        .create_deployment()
        .name(resource.to_string())
        .add_string(format!("{resource}.bpmn20.xml"), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let mut start = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(process_definition_id);
    for (name, value) in variables {
        start = start.variable(name, value);
    }
    engine
        .get_runtime_service()
        .start_process_instance(start)
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

fn complete_task_by_key(engine: &ProcessEngine, process_instance_id: &str, key: &str) {
    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
        .into_iter()
        .find(|task| task.task_definition_key == key)
        .unwrap_or_else(|| panic!("expected task with key {key}"));
    engine
        .get_task_service()
        .complete_task_by_id(task.id)
        .unwrap();
}

// ===========================================================================
// 1. skipExpression
// ===========================================================================

const SKIP_EXPRESSION_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="skipExprFork" name="Skip Expression Fork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />

    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" name="Hold" />

    <sequenceFlow id="toSkip" sourceRef="fork" targetRef="skippableTask" />
    <userTask id="skippableTask" name="Skippable"
              flowable:skipExpression="${skip}" />
    <sequenceFlow id="skipToEnd" sourceRef="skippableTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

/// Gap test (red before P6-B): skipExpression on a forked child user task must
/// resolve the process-level `_FLOWABLE_SKIP_EXPRESSION_ENABLED` switch and the
/// `skip` variable via the parent scope chain. The child's variable maps are
/// empty after the fork (P4-7b), so `is_skip_expression_enabled` returns false
/// and the task is never skipped.
#[test]
fn forked_user_task_skip_expression_resolves_process_variable() {
    let engine = ProcessEngine::new("p6b-skip-expr".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        SKIP_EXPRESSION_FORK_XML,
        "skip_expr_fork",
        vec![
            ("_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(), json!(true)),
            ("skip".to_string(), json!(true)),
        ],
    );

    // skippableTask should be auto-skipped; only holdTask remains.
    let keys = task_keys(&engine, &process_instance_id);
    assert!(
        keys.contains(&"holdTask".to_string()),
        "holdTask must remain; got {keys:?}"
    );
    assert!(
        !keys.contains(&"skippableTask".to_string()),
        "skippableTask must be skipped when skip=true; got {keys:?}"
    );
}

/// Regression guard: when skip=false, the task must NOT be skipped.
/// Must stay green before and after the evaluation fix.
#[test]
fn forked_user_task_skip_expression_false_keeps_task() {
    let engine = ProcessEngine::new("p6b-skip-expr-false".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        SKIP_EXPRESSION_FORK_XML,
        "skip_expr_fork_false",
        vec![
            ("_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(), json!(true)),
            ("skip".to_string(), json!(false)),
        ],
    );

    let keys = task_keys(&engine, &process_instance_id);
    assert!(
        keys.contains(&"holdTask".to_string()),
        "holdTask must remain; got {keys:?}"
    );
    assert!(
        keys.contains(&"skippableTask".to_string()),
        "skippableTask must NOT be skipped when skip=false; got {keys:?}"
    );
}

/// Regression guard: without the enabled switch, skipExpression must not fire
/// even if `skip=true`. Must stay green before and after the fix.
#[test]
fn forked_user_task_skip_expression_disabled_switch_keeps_task() {
    let engine = ProcessEngine::new("p6b-skip-expr-disabled".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        SKIP_EXPRESSION_FORK_XML,
        "skip_expr_fork_disabled",
        vec![("skip".to_string(), json!(true))],
    );

    let keys = task_keys(&engine, &process_instance_id);
    assert!(
        keys.contains(&"skippableTask".to_string()),
        "skippableTask must NOT be skipped when switch is off; got {keys:?}"
    );
}

// ===========================================================================
// 2. user_task assignee expression
// ===========================================================================

const ASSIGNEE_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="assigneeFork" name="Assignee Fork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />

    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" name="Hold" />

    <sequenceFlow id="toAssignee" sourceRef="fork" targetRef="assigneeTask" />
    <userTask id="assigneeTask" name="Assignee"
              flowable:assignee="${assignee}" />
  </process>
</definitions>"#;

/// Gap test (red before P6-B): assignee expression on a forked child user task
/// must resolve the process-level `assignee` variable via the parent scope
/// chain. The child's variable maps are empty after the fork, so the expression
/// evaluates to None and the task is created without an assignee.
#[test]
fn forked_user_task_assignee_resolves_process_variable() {
    let engine = ProcessEngine::new("p6b-assignee".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        ASSIGNEE_FORK_XML,
        "assignee_fork",
        vec![("assignee".to_string(), json!("john"))],
    );

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap();
    let assignee_task = tasks
        .iter()
        .find(|t| t.task_definition_key == "assigneeTask")
        .expect("assigneeTask must exist");
    assert_eq!(
        assignee_task.assignee.as_deref(),
        Some("john"),
        "assignee expression must resolve process variable via parent chain"
    );
}

/// Regression guard: literal assignee (not an expression) must still work.
/// Must stay green before and after the fix.
#[test]
fn forked_user_task_literal_assignee_still_works() {
    let engine = ProcessEngine::new("p6b-assignee-literal".to_string());
    let xml = ASSIGNEE_FORK_XML.replace(
        r#"flowable:assignee="${assignee}""#,
        r#"flowable:assignee="literalUser""#,
    );
    let process_instance_id = deploy_and_start(&engine, &xml, "assignee_literal", vec![]);

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap();
    let assignee_task = tasks
        .iter()
        .find(|t| t.task_definition_key == "assigneeTask")
        .expect("assigneeTask must exist");
    assert_eq!(
        assignee_task.assignee.as_deref(),
        Some("literalUser"),
        "literal assignee must work without parent chain"
    );
}

// ===========================================================================
// 3. script_task process_variables() binding
// ===========================================================================

const SCRIPT_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="scriptFork" name="Script Fork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />

    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" name="Hold" />

    <sequenceFlow id="toScript" sourceRef="fork" targetRef="scriptTask" />
    <scriptTask id="scriptTask" name="Resolve" scriptFormat="javascript"
                flowable:autoStoreVariables="true">
      <script>var resolvedSrc = srcVar;</script>
    </scriptTask>
    <sequenceFlow id="scriptToWait" sourceRef="scriptTask" targetRef="waitTask" />
    <userTask id="waitTask" name="Wait" />
  </process>
</definitions>"#;

fn secure_script_engine(name: &str) -> ProcessEngine {
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string()],
        ..Default::default()
    };
    ProcessEngine::new_with_config(name.to_string(), config)
}

/// Gap test (red before P6-B): a script task on a forked child execution must
/// read the process-level `srcVar` variable via the parent scope chain. The
/// child's variable maps are empty after the fork, so the script resolves
/// `srcVar` to null and `resolvedSrc` is never populated with the expected
/// value.
#[test]
fn forked_script_task_resolves_process_variable() {
    let engine = secure_script_engine("p6b-script");
    let process_instance_id = deploy_and_start(
        &engine,
        SCRIPT_FORK_XML,
        "script_fork",
        vec![("srcVar".to_string(), json!("hello"))],
    );

    // The script writes `resolvedSrc` back to its execution variables. After
    // the script task completes, the child execution advances to `waitTask`.
    // We snapshot all executions and look for the one carrying `resolvedSrc`.
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();

    let resolved = executions
        .values()
        .find_map(|e| e.variables.get("resolvedSrc").cloned())
        .unwrap_or_else(|| {
            panic!(
                "expected `resolvedSrc` to be written by the script task; executions = {executions:?}"
            )
        });
    assert_eq!(
        resolved,
        json!("hello"),
        "script task must resolve process-level `srcVar` via parent chain"
    );

    // Sanity: the holdTask must still be active (fork survived).
    let keys = task_keys(&engine, &process_instance_id);
    assert!(
        keys.contains(&"holdTask".to_string()),
        "holdTask must remain; got {keys:?}"
    );
    assert!(
        keys.contains(&"waitTask".to_string()),
        "waitTask must remain after script task; got {keys:?}"
    );
}

// ===========================================================================
// 4. service_task delegateExpression + field expression
// ===========================================================================

use flowable_engine::bpmn::behavior::service_task_activity_behavior::{
    LocalServiceTaskDelegate, LocalServiceTaskDelegateContext, LocalServiceTaskDelegateRegistry,
};
use flowable_engine::error::FlowableError;

const DELEGATE_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="delegateFork" name="Delegate Fork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />

    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" name="Hold" />

    <sequenceFlow id="toDelegate" sourceRef="fork" targetRef="delegateTask" />
    <serviceTask id="delegateTask" name="Resolve Delegate"
                 flowable:delegateExpression="${delegateName}">
      <extensionElements>
        <flowable:field name="greeting" expression="${greetingVar}" />
      </extensionElements>
    </serviceTask>
    <sequenceFlow id="delegateToWait" sourceRef="delegateTask" targetRef="waitTask" />
    <userTask id="waitTask" name="Wait" />
  </process>
</definitions>"#;

struct EchoDelegate;

impl LocalServiceTaskDelegate for EchoDelegate {
    fn execute(
        &self,
        context: &mut LocalServiceTaskDelegateContext<'_>,
    ) -> Result<serde_json::Value, FlowableError> {
        let greeting = context
            .fields
            .get("greeting")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        context.execution.set_process_variable(
            "delegateFired".to_string(),
            serde_json::json!({ "greeting": greeting }),
        );
        Ok(serde_json::json!({ "ok": true }))
    }
}

fn engine_with_echo_delegate(name: &str) -> ProcessEngine {
    let mut registry = LocalServiceTaskDelegateRegistry::new();
    registry.register("echoDelegate", std::sync::Arc::new(EchoDelegate));
    let config = ProcessEngineConfiguration {
        service_task_delegate_registry: Some(registry),
        ..Default::default()
    };
    ProcessEngine::new_with_config(name.to_string(), config)
}

/// Gap test (red before P6-B): `delegateExpression="${delegateName}"` on a
/// forked child service task must resolve the process-level `delegateName`
/// variable via the parent scope chain, and the field `expression="${greetingVar}"`
/// must resolve `greetingVar` the same way. The child's variable maps are empty
/// after the fork, so the delegateExpression fails to resolve and the process
/// start errors out.
#[test]
fn forked_service_task_delegate_expression_resolves_process_variable() {
    let engine = engine_with_echo_delegate("p6b-delegate");
    let process_instance_id = deploy_and_start(
        &engine,
        DELEGATE_FORK_XML,
        "delegate_fork",
        vec![
            ("delegateName".to_string(), json!("echoDelegate")),
            ("greetingVar".to_string(), json!("hi-from-pi")),
        ],
    );

    // The delegate writes `delegateFired` to the child execution that ran the
    // service task. Snapshot all executions and look for the marker.
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();

    let fired = executions
        .values()
        .find_map(|e| e.variables.get("delegateFired").cloned())
        .unwrap_or_else(|| {
            panic!(
                "expected `delegateFired` to be written by the delegate; executions = {executions:?}"
            )
        });
    assert_eq!(
        fired,
        json!({ "greeting": "hi-from-pi" }),
        "delegateExpression and field expression must resolve process variables via parent chain"
    );

    let keys = task_keys(&engine, &process_instance_id);
    assert!(
        keys.contains(&"holdTask".to_string()),
        "holdTask must remain; got {keys:?}"
    );
    assert!(
        keys.contains(&"waitTask".to_string()),
        "waitTask must remain after delegate; got {keys:?}"
    );
}

// ===========================================================================
// 5. task/execution listener field expressions
// ===========================================================================

use flowable_engine::bpmn::listener::{
    ExecutionListenerContext, LocalExecutionListener, LocalExecutionListenerRegistry,
    LocalTaskListener, LocalTaskListenerRegistry, TaskListenerContext,
};

struct CaptureTaskListenerField;

impl LocalTaskListener for CaptureTaskListenerField {
    fn notify(&self, ctx: &mut TaskListenerContext<'_>) -> Result<(), FlowableError> {
        let value = ctx
            .fields
            .get("marker")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        ctx.execution
            .set_process_variable("taskListenerField".to_string(), value);
        Ok(())
    }
}

struct CaptureExecutionListenerField;

impl LocalExecutionListener for CaptureExecutionListenerField {
    fn notify(&self, ctx: &mut ExecutionListenerContext<'_>) -> Result<(), FlowableError> {
        let value = ctx
            .fields
            .get("marker")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        ctx.execution
            .set_process_variable("executionListenerField".to_string(), value);
        Ok(())
    }
}

const TASK_LISTENER_FIELD_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="taskListenerFieldFork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />
    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" />
    <sequenceFlow id="toListener" sourceRef="fork" targetRef="listenerTask" />
    <userTask id="listenerTask">
      <extensionElements>
        <flowable:taskListener event="create" class="captureTaskField">
          <flowable:field name="marker" expression="${listenerValue}" />
        </flowable:taskListener>
      </extensionElements>
    </userTask>
  </process>
</definitions>"#;

const EXECUTION_LISTENER_FIELD_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="executionListenerFieldFork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />
    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" />
    <sequenceFlow id="toListener" sourceRef="fork" targetRef="listenerTask" />
    <userTask id="listenerTask">
      <extensionElements>
        <flowable:executionListener event="start" class="captureExecutionField">
          <flowable:field name="marker" expression="${listenerValue}" />
        </flowable:executionListener>
      </extensionElements>
    </userTask>
  </process>
</definitions>"#;

#[test]
fn forked_task_listener_field_expression_resolves_process_variable() {
    let mut registry = LocalTaskListenerRegistry::new();
    registry.register(
        "captureTaskField",
        std::sync::Arc::new(CaptureTaskListenerField),
    );
    let config = ProcessEngineConfiguration {
        task_listener_registry: Some(registry),
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("p6b-task-listener-field".to_string(), config);
    deploy_and_start(
        &engine,
        TASK_LISTENER_FIELD_FORK_XML,
        "task_listener_field_fork",
        vec![("listenerValue".to_string(), json!("from-pi"))],
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();
    assert!(
        executions.values().any(|execution| {
            execution.variables.get("taskListenerField") == Some(&json!("from-pi"))
        }),
        "task listener field expression must resolve the PI variable; executions = {executions:?}"
    );
}

#[test]
fn forked_execution_listener_field_expression_resolves_process_variable() {
    let mut registry = LocalExecutionListenerRegistry::new();
    registry.register(
        "captureExecutionField",
        std::sync::Arc::new(CaptureExecutionListenerField),
    );
    let config = ProcessEngineConfiguration {
        execution_listener_registry: Some(registry),
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("p6b-execution-listener-field".to_string(), config);
    deploy_and_start(
        &engine,
        EXECUTION_LISTENER_FIELD_FORK_XML,
        "execution_listener_field_fork",
        vec![("listenerValue".to_string(), json!("from-pi"))],
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();
    assert!(
        executions.values().any(|execution| {
            execution.variables.get("executionListenerField") == Some(&json!("from-pi"))
        }),
        "execution listener field expression must resolve the PI variable; executions = {executions:?}"
    );
}

// ===========================================================================
// 6. call_activity calledElement + in/out parameter expressions
// ===========================================================================

const CALL_ACTIVITY_PARENT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="callActivityParent" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toCall" sourceRef="start" targetRef="callActivity" />
    <callActivity id="callActivity" calledElement="${calleeKey}">
      <extensionElements>
        <flowable:in sourceExpression="${parentInput}" target="childInput" />
        <flowable:out sourceExpression="${childResult}" target="parentResult" />
      </extensionElements>
    </callActivity>
    <sequenceFlow id="callToWait" sourceRef="callActivity" targetRef="waitTask" />
    <userTask id="waitTask" name="Wait" />
    <sequenceFlow id="waitToEnd" sourceRef="waitTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

const CALL_ACTIVITY_CHILD_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="callActivityChild" isExecutable="true">
    <startEvent id="childStart" />
    <sequenceFlow id="childFlow1" sourceRef="childStart" targetRef="childTask" />
    <userTask id="childTask" name="Child Task" />
    <sequenceFlow id="childFlow2" sourceRef="childTask" targetRef="childEnd" />
    <endEvent id="childEnd" />
  </process>
</definitions>"#;

/// Regression guard: call activity `calledElement` expression and in-parameter
/// `sourceExpression` must still resolve process-level variables after the
/// P6-B `evaluation_execution` change. On the root execution (no fork) the
/// variables are already present, so this stays green before and after the fix.
#[test]
fn call_activity_expression_resolves_process_variable() {
    let engine = ProcessEngine::new("p6b-call-activity".to_string());
    let repository_service = engine.get_repository_service();
    let builder = repository_service
        .create_deployment()
        .name("call_activity".to_string())
        .add_string(
            "call_activity_parent.bpmn20.xml".to_string(),
            CALL_ACTIVITY_PARENT_XML.to_string(),
        )
        .add_string(
            "call_activity_child.bpmn20.xml".to_string(),
            CALL_ACTIVITY_CHILD_XML.to_string(),
        );
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service
        .get_process_definition_ids()
        .unwrap()
        .iter()
        .find(|id| id.starts_with("callActivityParent"))
        .cloned()
        .expect("parent process definition must be deployed");

    let start = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .variable("calleeKey".to_string(), json!("callActivityChild"))
        .variable("parentInput".to_string(), json!("hello-from-pi"));
    let process_instance_id = engine
        .get_runtime_service()
        .start_process_instance(start)
        .unwrap()
        .id;

    // The call activity starts a child process instance. The child's
    // `childInput` variable should be populated with `parentInput` value.
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let all_instances = runtime_store.snapshot_process_instances(&mut session);
    session.rollback().unwrap();

    let child = all_instances
        .values()
        .find(|pi| pi.super_execution_id.is_some())
        .expect("child process instance must be started by the call activity");

    let mut session = runtime_store.create_session().unwrap();
    let child_execution = runtime_store
        .find_execution(&child.id, &mut session)
        .expect("child process instance scope execution must exist");
    session.rollback().unwrap();

    assert_eq!(
        child_execution.variables.get("childInput"),
        Some(&json!("hello-from-pi")),
        "call activity in-parameter sourceExpression must resolve PI variable"
    );

    // The child process instance has a `childTask` that is active while the
    // parent call activity waits for the child to complete.
    let child_keys = task_keys(&engine, &child.id);
    assert!(
        child_keys.contains(&"childTask".to_string()),
        "childTask must be active in the child process; got {child_keys:?}"
    );
}

// ===========================================================================
// 7. job_category expression on async continuation / boundary timer
// ===========================================================================

const ASYNC_CATEGORY_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="asyncCategoryFork" name="Async Category Fork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />

    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" name="Hold" />

    <sequenceFlow id="toAsync" sourceRef="fork" targetRef="asyncTask" />
    <serviceTask id="asyncTask" name="Async" flowable:async="true">
      <extensionElements>
        <flowable:jobCategory>${categoryValue}</flowable:jobCategory>
      </extensionElements>
    </serviceTask>
    <sequenceFlow id="asyncToEnd" sourceRef="asyncTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

/// Gap test (red before P6-B): an async-before continuation job created on a
/// forked child execution must resolve the process-level `categoryValue`
/// variable via the parent scope chain when evaluating `flowable:jobCategory`.
/// The child's variable maps are empty after the fork, so without parent-chain
/// resolution the category silently drops to `None`.
#[test]
fn forked_async_continuation_job_category_resolves_process_variable() {
    let engine = ProcessEngine::new("p6b-async-category".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        ASYNC_CATEGORY_FORK_XML,
        "async_category_fork",
        vec![("categoryValue".to_string(), json!("orders"))],
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance_id, &mut session);
    session.rollback().unwrap();

    let async_job = jobs
        .iter()
        .find(|j| j.activity_id == "asyncTask")
        .expect("async continuation job for asyncTask must exist");
    assert_eq!(
        async_job.category.as_deref(),
        Some("orders"),
        "jobCategory expression must resolve process variable via parent chain"
    );
}

const BOUNDARY_CATEGORY_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="boundaryCategoryFork" name="Boundary Category Fork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />

    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" name="Hold" />

    <sequenceFlow id="toTimed" sourceRef="fork" targetRef="timedTask" />
    <userTask id="timedTask" name="Timed" />
    <boundaryEvent id="boundaryTimer" attachedToRef="timedTask" cancelActivity="true">
      <extensionElements>
        <flowable:jobCategory>${categoryValue}</flowable:jobCategory>
      </extensionElements>
      <timerEventDefinition>
        <timeDuration>PT1H</timeDuration>
      </timerEventDefinition>
    </boundaryEvent>
    <sequenceFlow id="timedToEnd" sourceRef="timedTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

/// Gap test (red before P6-B): a boundary-timer job created on a forked child
/// user-task execution must resolve the process-level `categoryValue` variable
/// via the parent scope chain when evaluating `flowable:jobCategory` on the
/// boundary event. The child's variable maps are empty after the fork, so
/// without parent-chain resolution the category silently drops to `None`.
#[test]
fn forked_boundary_timer_job_category_resolves_process_variable() {
    let engine = ProcessEngine::new("p6b-boundary-category".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        BOUNDARY_CATEGORY_FORK_XML,
        "boundary_category_fork",
        vec![("categoryValue".to_string(), json!("orders"))],
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance_id, &mut session);
    session.rollback().unwrap();

    let boundary_job = jobs
        .iter()
        .find(|j| j.activity_id == "boundaryTimer")
        .expect("boundary timer job must exist");
    assert_eq!(
        boundary_job.category.as_deref(),
        Some("orders"),
        "boundary jobCategory expression must resolve process variable via parent chain"
    );
}

const ASYNC_RETRY_CYCLE_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="asyncRetryCycleFork" name="Async Retry Cycle Fork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />

    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" name="Hold" />

    <sequenceFlow id="toAsync" sourceRef="fork" targetRef="asyncTask" />
    <serviceTask id="asyncTask" name="Async" flowable:async="true">
      <extensionElements>
        <flowable:failedJobRetryTimeCycle>${retryCycle}</flowable:failedJobRetryTimeCycle>
      </extensionElements>
    </serviceTask>
    <sequenceFlow id="asyncToEnd" sourceRef="asyncTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

/// Gap test (red before P6-B): `failedJobRetryTimeCycle` expression on a forked
/// child execution's async job must resolve the process-level `retryCycle`
/// variable via the parent scope chain when the job fails. The forked child's
/// variable maps are empty, so without parent-chain resolution the expression
/// resolves to no value and the failure-recording command errors out.
#[test]
fn forked_async_job_retry_cycle_resolves_process_variable() {
    use flowable_engine::cmd::record_failed_timer_work_cmd::RecordFailedTimerWorkCmd;
    use flowable_engine::engine::timer_worker::TimerWork;
    use flowable_engine::error::FlowableError;
    use flowable_engine::interceptor::command_executor::CommandExecutor;

    let engine = ProcessEngine::new("p6b-retry-cycle".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        ASYNC_RETRY_CYCLE_FORK_XML,
        "async_retry_cycle_fork",
        vec![("retryCycle".to_string(), json!("R2/PT1M"))],
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let job = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance_id, &mut session)
        .into_iter()
        .find(|j| j.activity_id == "asyncTask")
        .expect("async continuation job for asyncTask must exist");
    session.rollback().unwrap();

    // Simulate a job failure. The retry cycle expression `${retryCycle}`
    // references a PI-level variable reachable only via the parent scope chain
    // from the forked child execution.
    let simulated_failure = FlowableError::ExecutionError("simulated job failure".to_string());
    let command =
        RecordFailedTimerWorkCmd::new(TimerWork::RuntimeJob(job.clone()), &simulated_failure);
    engine
        .get_command_executor()
        .execute(&command)
        .expect("retry cycle expression must resolve via parent chain");

    let mut session = runtime_store.create_session().unwrap();
    let updated_job = runtime_store
        .find_timer_job_state(&job.timer_job_id, &mut session)
        .expect("updated job must exist after failure recording");
    session.rollback().unwrap();

    // R2/PT1M → repetitions=2; first failure → retries = 2 - 1 = 1.
    assert_eq!(
        updated_job.retries,
        Some(1),
        "failedJobRetryTimeCycle expression must resolve to R2/PT1M via parent chain"
    );
    assert_eq!(
        updated_job.job_state.as_deref(),
        Some("timer"),
        "job must wait for retry (not deadletter) when retries remain"
    );
}

// ===========================================================================
// 9. business_rule_task DMN input variable resolution
// ===========================================================================
//
// `BusinessRuleTaskActivityBehavior` reads DMN input variables via
// `execution.process_variable(name)`, which only inspects the execution's own
// variable maps. On a forked child execution those maps are empty (P4-7b), so
// PI-level inputs are silently replaced with `Value::Null` and the decision
// matches the wrong rule. The fix routes input resolution through
// `evaluation_execution` so the parent scope chain is walked.

use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnHitPolicy, DmnInputClause, DmnModel,
    DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry, DmnUnaryTest,
};

fn business_rule_engine(name: &str) -> ProcessEngine {
    let dmn_engine = Arc::new(DmnEngine::new_in_memory().expect("dmn engine"));
    dmn_engine
        .deploy(
            DmnDeploymentRequest::new("p6b business rule decisions").with_resource(
                "p6b-credit-eligibility.dmn",
                DmnModel::new(vec![DmnDecision::new(
                    "creditEligibility",
                    "creditEligibility",
                    "Credit Eligibility",
                    DmnHitPolicy::First,
                    vec![DmnInputClause::new("input-1", "creditScore")],
                    vec![
                        DmnOutputClause::new("output-1", "approved"),
                        DmnOutputClause::new("output-2", "riskBand"),
                    ],
                    vec![
                        DmnRule::new(
                            "rule-1",
                            vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!(730)))],
                            vec![
                                DmnRuleOutputEntry::new(json!(true)),
                                DmnRuleOutputEntry::new(json!("LOW")),
                            ],
                        ),
                        DmnRule::new(
                            "rule-2",
                            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                            vec![
                                DmnRuleOutputEntry::new(json!(false)),
                                DmnRuleOutputEntry::new(json!("HIGH")),
                            ],
                        ),
                    ],
                )]),
            ),
        )
        .expect("dmn deployment");

    let config = ProcessEngineConfiguration {
        dmn_engine: Some(dmn_engine),
        ..Default::default()
    };
    ProcessEngine::new_with_config(name.to_string(), config)
}

const BUSINESS_RULE_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="businessRuleFork" name="Business Rule Fork" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />

    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" name="Hold" />

    <sequenceFlow id="toRule" sourceRef="fork" targetRef="ruleTask" />
    <businessRuleTask id="ruleTask" name="Evaluate"
                      flowable:decisionRef="creditEligibility"
                      flowable:ruleVariablesInput="creditScore"
                      flowable:resultVariable="decisionResult" />
    <sequenceFlow id="ruleToWait" sourceRef="ruleTask" targetRef="waitTask" />
    <userTask id="waitTask" name="Wait" />
  </process>
</definitions>"#;

/// Gap test (red before P6-B): a business rule task on a forked child execution
/// must resolve the process-level `creditScore` variable via the parent scope
/// chain when building DMN inputs. The child's variable maps are empty after
/// the fork, so without parent-chain resolution `creditScore` is replaced with
/// `Value::Null`, the decision matches the catch-all rule, and `approved`
/// becomes `false` instead of `true`.
#[test]
fn forked_business_rule_task_input_resolves_process_variable() {
    let engine = business_rule_engine("p6b-business-rule");
    let process_instance_id = deploy_and_start(
        &engine,
        BUSINESS_RULE_FORK_XML,
        "business_rule_fork",
        vec![("creditScore".to_string(), json!(730))],
    );

    // The forked child must have completed the business rule task and reached
    // `waitTask`. The decision result is written back onto the child
    // execution's variables; we snapshot all executions to find it.
    let keys = task_keys(&engine, &process_instance_id);
    assert!(
        keys.contains(&"holdTask".to_string()),
        "holdTask must remain; got {keys:?}"
    );
    assert!(
        keys.contains(&"waitTask".to_string()),
        "waitTask must be reached after business rule task; got {keys:?}"
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();

    let decision_result = executions
        .values()
        .find_map(|e| e.variables.get("decisionResult").cloned())
        .unwrap_or_else(|| {
            panic!("expected `decisionResult` to be written by the business rule task; executions = {executions:?}")
        });
    assert_eq!(
        decision_result["approved"],
        json!(true),
        "creditScore must resolve via parent chain so rule-1 (==730) matches; got {decision_result}"
    );
    assert_eq!(
        decision_result["riskBand"],
        json!("LOW"),
        "riskBand must reflect rule-1 match; got {decision_result}"
    );
}

/// Regression guard: when the input variable is missing entirely, the
/// catch-all rule must fire and produce `approved=false`. Must stay green
/// before and after the parent-chain fix.
#[test]
fn forked_business_rule_task_missing_input_falls_back_to_catch_all() {
    let engine = business_rule_engine("p6b-business-rule-missing");
    deploy_and_start(
        &engine,
        BUSINESS_RULE_FORK_XML,
        "business_rule_fork_missing",
        vec![],
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();

    let decision_result = executions
        .values()
        .find_map(|e| e.variables.get("decisionResult").cloned())
        .unwrap_or_else(|| {
            panic!("expected `decisionResult` to be written by the business rule task; executions = {executions:?}")
        });
    assert_eq!(
        decision_result["approved"],
        json!(false),
        "missing creditScore must fall back to catch-all rule-2; got {decision_result}"
    );
    assert_eq!(
        decision_result["riskBand"],
        json!("HIGH"),
        "riskBand must reflect rule-2 match; got {decision_result}"
    );
}
