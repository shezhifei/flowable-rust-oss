// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! P1-A — expression-type execution/task listeners must propagate evaluation
//! errors instead of swallowing them.
//!
//! Java evidence:
//! - `ExpressionExecutionListener.java:34-37` — the expression return value is
//!   ignored, but `expression.getValue(execution)` exceptions escape and fail
//!   the command (the surrounding transaction rolls back).
//! - `ExpressionTaskListener.java:31-34` — same contract for task listeners.
//!
//! Undefined variables remain lenient (null) and must NOT fail the listener;
//! only a real evaluation failure (here: a registered bean method returning
//! `Err`) propagates as `FlowableError::ExecutionError`.

use flowable_engine::el::method_registry::ExpressionMethodRegistry;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::{Arc, Mutex};

/// Side-effect sink mirroring `${auditBean.recordStart()}`.
#[derive(Clone, Default)]
struct AuditBean {
    entries: Arc<Mutex<Vec<String>>>,
}

impl AuditBean {
    fn entries(&self) -> Vec<String> {
        self.entries.lock().unwrap().clone()
    }

    fn registry(&self) -> ExpressionMethodRegistry {
        let registry = ExpressionMethodRegistry::new();
        let entries = Arc::clone(&self.entries);
        registry.register_bean_method("auditBean", "recordStart", move |args| {
            let label = args
                .first()
                .and_then(|value| value.as_str())
                .unwrap_or("start")
                .to_string();
            entries.lock().unwrap().push(label);
            Ok(serde_json::Value::Bool(true))
        });
        registry.register_bean_method("auditBean", "failUnconditionally", |_| {
            Err("bean deliberately rejected this listener".to_string())
        });
        registry
    }
}

fn engine_with(audit: &AuditBean, name: &str) -> ProcessEngine {
    let config = ProcessEngineConfiguration {
        expression_method_registry: audit.registry(),
        ..Default::default()
    };
    ProcessEngine::new_with_config(name.to_string(), config).unwrap()
}

fn deploy(engine: &ProcessEngine, resource: &str, xml: &str) -> String {
    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("p1a listener deployment".to_string())
                .add_string(resource.to_string(), xml.to_string()),
        )
        .unwrap();
    repository.get_process_definition_ids().unwrap()[0].clone()
}

/// A failing bean method inside an `expression` execution listener on the start
/// event must fail process instance start (Java lets the exception roll the
/// command back). Before P1-A the listener discarded the result via `let _`.
#[test]
fn failing_bean_method_in_expression_execution_listener_fails_start() {
    let audit = AuditBean::default();
    let engine = engine_with(&audit, "p1a-exec-fail");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="failingExpressionListenerProcess" isExecutable="true">
    <startEvent id="startEvent1">
      <extensionElements>
        <flowable:executionListener event="start"
            expression="${auditBean.failUnconditionally()}" />
      </extensionElements>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "failing-exec-listener.bpmn20.xml", xml);

    let error = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect_err("start must fail when the listener expression errors");

    assert!(
        matches!(error, FlowableError::ExecutionError(_)),
        "expected ExecutionError, got: {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("auditBean.failUnconditionally"),
        "error should identify the listener expression, got: {message}"
    );
    assert!(
        message.contains("bean deliberately rejected this listener"),
        "error should carry the bean method message, got: {message}"
    );
}

/// A successful bean method in an `expression` execution listener still runs
/// for its side effect; the expression value itself is ignored.
#[test]
fn successful_bean_method_expression_execution_listener_runs() {
    let audit = AuditBean::default();
    let engine = engine_with(&audit, "p1a-exec-ok");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="successfulExpressionListenerProcess" isExecutable="true">
    <startEvent id="startEvent1">
      <extensionElements>
        <flowable:executionListener event="start"
            expression="${auditBean.recordStart('start-event')}" />
      </extensionElements>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "successful-exec-listener.bpmn20.xml", xml);

    let instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect("a successful listener expression must not fail start");
    // The instance ran straight through to the end event.
    assert!(
        engine
            .get_runtime_service()
            .get_variables(instance.id)
            .is_ok()
    );
    assert_eq!(audit.entries(), vec!["start-event".to_string()]);
}

/// An undefined variable in a listener expression resolves to lenient null
/// (Java EL's unresolved-variable convention) and the listener succeeds.
#[test]
fn undefined_variable_in_expression_listener_is_lenient_null() {
    let audit = AuditBean::default();
    let engine = engine_with(&audit, "p1a-exec-undefined");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="lenientUndefinedListenerProcess" isExecutable="true">
    <startEvent id="startEvent1">
      <extensionElements>
        <flowable:executionListener event="start"
            expression="${thisVariableIsNeverDefined}" />
      </extensionElements>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "undefined-exec-listener.bpmn20.xml", xml);

    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect("an undefined variable must stay lenient null, not fail the listener");
    assert!(audit.entries().is_empty());
}

/// The same propagation contract applies to an `expression` task listener:
/// its `create` event fires while arriving at the user task during start.
#[test]
fn failing_bean_method_in_expression_task_listener_fails_start() {
    let audit = AuditBean::default();
    let engine = engine_with(&audit, "p1a-task-fail");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="failingTaskListenerProcess" isExecutable="true">
    <startEvent id="startEvent1" />
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
    <userTask id="userTask1" name="Review">
      <extensionElements>
        <flowable:taskListener event="create"
            expression="${auditBean.failUnconditionally()}" />
      </extensionElements>
    </userTask>
    <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "failing-task-listener.bpmn20.xml", xml);

    let error = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect_err("arriving at the user task must fail when its create listener errors");

    assert!(
        matches!(error, FlowableError::ExecutionError(_)),
        "expected ExecutionError, got: {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("taskListener expression"),
        "error should identify the taskListener, got: {message}"
    );
    assert!(
        message.contains("auditBean.failUnconditionally"),
        "error should identify the expression, got: {message}"
    );
}
