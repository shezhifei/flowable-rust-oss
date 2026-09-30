// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! W2 residual S-group / W4 R1c — evaluation errors must fail the command,
//! never be disguised as null / empty / "could not resolve" without cause.
//!
//! Java evidence (research A.2):
//! - call_activity: `CallActivityBehavior.java:343,125` / `IOParameterUtil.java:74`
//!   (`expression.getValue` has no catch).
//! - case_task: `CaseTaskActivityBehavior.java:124` / `IOParameterUtil:74,83`.
//! - external_worker: `ExternalWorkerTaskActivityBehavior.java:98,119`.
//! - listener field/delegate: `ExpressionExecutionListener.java:36` family.
//! - task_service aggregation: `UelExpressionCondition` getValue family.
//! - event_registry_correlation: correlation-key getValue family.
//! - service_task R1c: `ServiceTaskExpressionActivityBehavior.java:107-114`
//!   handleException → ErrorPropagation (BpmnError or rethrow; never silent null).

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;

fn deploy(engine: &ProcessEngine, resource: &str, xml: &str) -> String {
    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("w2-s-group".to_string())
                .add_string(resource.to_string(), xml.to_string()),
        )
        .unwrap();
    repository.get_process_definition_ids().unwrap()[0].clone()
}

fn start_must_fail(engine: &ProcessEngine, definition_id: &str, label: &str) -> String {
    let error = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id.to_string()),
        )
        .expect_err(label);
    let message = error.to_string();
    assert!(
        matches!(error, FlowableError::ExecutionError(_))
            || matches!(error, FlowableError::Generic(_)),
        "expected a command failure, got: {error:?}"
    );
    message
}

// ─── call_activity (A.2 #8-11) ───────────────────────────────────────────────

/// `calledElement="${undefinedVar}"` must fail start (Java getValue has no
/// catch). Before W2 the lenient path turned the evaluation error into a
/// generic "could not be resolved" or silently used the raw text.
#[test]
fn call_activity_called_element_undefined_variable_fails_start() {
    let engine = ProcessEngine::new("w2-call-activity-key".into()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="callActivityKeyProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call1" />
    <callActivity id="call1" calledElement="${undefinedVar}" />
    <sequenceFlow id="f2" sourceRef="call1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "call-key.bpmn20.xml", xml);
    let message = start_must_fail(
        &engine,
        &definition_id,
        "start must fail when calledElement expression errors",
    );
    assert!(
        message.contains("calledElement") || message.contains("undefinedVar") || message.contains("Unknown property"),
        "error should identify the evaluation failure, got: {message}"
    );
}

/// In-parameter `sourceExpression` with an undefined variable must fail
/// (IOParameterUtil.java:74) — not silently map Null.
#[test]
fn call_activity_in_source_expression_undefined_variable_fails_start() {
    let engine = ProcessEngine::new("w2-call-activity-in".into()).unwrap();
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .name("child".to_string())
            .add_string(
                "child.bpmn20.xml".to_string(),
                r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="childProc" isExecutable="true">
    <startEvent id="cs" />
    <sequenceFlow id="cf1" sourceRef="cs" targetRef="ct" />
    <userTask id="ct" name="child task" />
    <sequenceFlow id="cf2" sourceRef="ct" targetRef="ce" />
    <endEvent id="ce" />
  </process>
</definitions>"#
                .to_string(),
            ),
    )
    .unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="callActivityInProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call1" />
    <callActivity id="call1" calledElement="childProc">
      <extensionElements>
        <flowable:in sourceExpression="${missingSourceVar}" target="childIn" />
      </extensionElements>
    </callActivity>
    <sequenceFlow id="f2" sourceRef="call1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "call-in.bpmn20.xml", xml);
    let message = start_must_fail(
        &engine,
        &definition_id,
        "start must fail when in sourceExpression errors",
    );
    assert!(
        message.contains("sourceExpression") || message.contains("missingSourceVar") || message.contains("Unknown property"),
        "error should identify the in-parameter expression, got: {message}"
    );
}

// ─── case_task (A.2 #12-14) ──────────────────────────────────────────────────

/// `caseDefinitionKey="${undefinedVar}"` must fail (CaseTaskActivityBehavior.java:124).
#[test]
fn case_task_definition_key_undefined_variable_fails_start() {
    let engine = ProcessEngine::new("w2-case-task-key".into()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="caseTaskKeyProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="case1" />
    <serviceTask id="case1" flowable:type="case"
                 flowable:caseDefinitionKey="${undefinedCaseKey}" />
    <sequenceFlow id="f2" sourceRef="case1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "case-key.bpmn20.xml", xml);
    let message = start_must_fail(
        &engine,
        &definition_id,
        "start must fail when caseDefinitionKey expression errors",
    );
    assert!(
        message.contains("caseDefinitionKey") || message.contains("undefinedCaseKey") || message.contains("Unknown property"),
        "error should identify the case key expression, got: {message}"
    );
}

// ─── external_worker (A.2 #35-38) ────────────────────────────────────────────

/// External-worker `topic="${undefinedVar}"` must fail (getValue family).
#[test]
fn external_worker_topic_undefined_variable_fails_start() {
    let engine = ProcessEngine::new("w2-ext-worker-topic".into()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="extWorkerTopicProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="ext1" />
    <serviceTask id="ext1" flowable:type="external-worker"
                 flowable:topic="${undefinedTopicVar}" />
    <sequenceFlow id="f2" sourceRef="ext1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "ext-topic.bpmn20.xml", xml);
    let message = start_must_fail(
        &engine,
        &definition_id,
        "start must fail when external-worker topic expression errors",
    );
    assert!(
        message.contains("topic") || message.contains("undefinedTopicVar") || message.contains("Unknown property"),
        "error should identify the topic expression, got: {message}"
    );
}

// ─── listener field expression (A.2 #31-34) ──────────────────────────────────

/// A task-listener field `expression` with an undefined variable must fail the
/// command (ExpressionExecutionListener family — no catch).
#[test]
fn task_listener_field_expression_undefined_variable_fails_complete() {
    let engine = ProcessEngine::new("w2-listener-field".into()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="listenerFieldProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="user1" />
    <userTask id="user1" name="listener target">
      <extensionElements>
        <flowable:taskListener event="complete" class="noop">
          <flowable:field name="payload">
            <flowable:expression>${undefinedFieldVar}</flowable:expression>
          </flowable:field>
        </flowable:taskListener>
      </extensionElements>
    </userTask>
    <sequenceFlow id="f2" sourceRef="user1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "listener-field.bpmn20.xml", xml);
    let instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .unwrap();
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    let task_id = tasks[0].id.clone();
    let error = engine
        .get_task_service()
        .complete_task_by_id(task_id)
        .expect_err("complete must fail when the listener field expression errors");
    let message = error.to_string();
    assert!(
        message.contains("undefinedFieldVar")
            || message.contains("payload")
            || message.contains("Unknown property")
            || message.contains("failed"),
        "error should identify the field expression failure, got: {message}"
    );
}

// ─── service_task R1c (A.2 #17-28) ───────────────────────────────────────────

/// Delegate-expression service task with an unresolved expression must fail
/// the command (not silently complete with a null result).
#[test]
fn service_task_delegate_expression_undefined_variable_fails_start() {
    let engine = ProcessEngine::new("w2-service-delegate".into()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="serviceDelegateProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="svc1" />
    <serviceTask id="svc1" flowable:delegateExpression="${undefinedDelegateVar}" />
    <sequenceFlow id="f2" sourceRef="svc1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "svc-delegate.bpmn20.xml", xml);
    let message = start_must_fail(
        &engine,
        &definition_id,
        "start must fail when delegateExpression evaluation errors",
    );
    assert!(
        message.contains("delegateExpression")
            || message.contains("undefinedDelegateVar")
            || message.contains("Unknown property")
            || message.contains("failed"),
        "error should identify the delegate expression failure, got: {message}"
    );
}

/// Service-task field extension `expression` with an undefined variable must
/// fail (R1c — never silently resolve to null).
///
/// A noop delegate is registered so the field-expression path is exercised in
/// isolation: the error must name the field expression, not the delegate.
#[test]
fn service_task_field_expression_undefined_variable_fails_start() {
    use flowable_engine::bpmn::behavior::service_task_activity_behavior::{
        LocalServiceTaskDelegate, LocalServiceTaskDelegateContext, LocalServiceTaskDelegateRegistry,
    };
    use std::sync::Arc;

    struct NoopDelegate;
    impl LocalServiceTaskDelegate for NoopDelegate {
        fn execute(
            &self,
            _context: &mut LocalServiceTaskDelegateContext<'_>,
        ) -> Result<serde_json::Value, FlowableError> {
            Ok(serde_json::Value::Null)
        }
    }

    let mut registry = LocalServiceTaskDelegateRegistry::new();
    registry.register("noopDelegate", Arc::new(NoopDelegate));
    let mut config = ProcessEngineConfiguration::default();
    config.service_task_delegate_registry = Some(registry);
    let engine =
        ProcessEngine::new_with_config("w2-service-field".into(), config).unwrap();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="serviceFieldProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="svc1" />
    <serviceTask id="svc1" flowable:class="noopDelegate">
      <extensionElements>
        <flowable:field name="payload">
          <flowable:expression>${undefinedFieldVar}</flowable:expression>
        </flowable:field>
      </extensionElements>
    </serviceTask>
    <sequenceFlow id="f2" sourceRef="svc1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "svc-field.bpmn20.xml", xml);
    let result = engine.get_runtime_service().start_process_instance(
        engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(definition_id),
    );
    let error = result.expect_err(
        "start must fail when the field expression errors (delegate is registered, so the \
         failure must come from the field expression, not a missing delegate)",
    );
    let message = error.to_string();
    // Precise: the error must identify the field-expression evaluation failure.
    // "delegate" / "failed" alone are NOT acceptable — a missing-delegate error
    // would also contain those words and would mask a false pass.
    assert!(
        message.contains("undefinedFieldVar") || message.contains("payload"),
        "error must name the field expression (undefinedFieldVar / payload), got: {message}"
    );
    assert!(
        message.contains("Unknown property") || message.contains("failed"),
        "error must indicate an evaluation failure, got: {message}"
    );
}

// ─── event_registry_correlation (A.2 #4) ─────────────────────────────────────

/// A correlation-parameter value expression with an undefined variable must
/// fail at runtime when the receive/subscription is created — not fall back to
/// the raw expression text.
#[test]
fn correlation_parameter_undefined_variable_fails_subscription() {
    let engine = ProcessEngine::new("w2-correlation".into()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="correlationProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="receive1" />
    <receiveTask id="receive1" messageRef="waitMsg">
      <extensionElements>
        <flowable:eventCorrelationParameter name="orderId"
            value="${undefinedCorrelationVar}" />
      </extensionElements>
    </receiveTask>
    <sequenceFlow id="f2" sourceRef="receive1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "correlation.bpmn20.xml", xml);
    let message = start_must_fail(
        &engine,
        &definition_id,
        "start must fail when correlation value expression errors",
    );
    assert!(
        message.contains("Correlation")
            || message.contains("undefinedCorrelationVar")
            || message.contains("Unknown property")
            || message.contains("failed"),
        "error should identify the correlation expression failure, got: {message}"
    );
}

// ─── execution_listener field expression (A.2 #31-34) ────────────────────────

/// An execution-listener field `expression` with an undefined variable must fail
/// the command (ExpressionExecutionListener family — no catch). Complements the
/// task-listener case above so both listener surfaces are covered.
#[test]
fn execution_listener_field_expression_undefined_variable_fails_start() {
    let engine = ProcessEngine::new("w2-exec-listener-field".into()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="execListenerFieldProcess" isExecutable="true">
    <startEvent id="start">
      <extensionElements>
        <flowable:executionListener event="start" class="noop">
          <flowable:field name="payload">
            <flowable:expression>${undefinedExecFieldVar}</flowable:expression>
          </flowable:field>
        </flowable:executionListener>
      </extensionElements>
    </startEvent>
    <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "exec-listener-field.bpmn20.xml", xml);
    let message = start_must_fail(
        &engine,
        &definition_id,
        "start must fail when the execution listener field expression errors",
    );
    assert!(
        message.contains("undefinedExecFieldVar")
            || message.contains("payload")
            || message.contains("Unknown property")
            || message.contains("failed"),
        "error should identify the execution listener field expression failure, got: {message}"
    );
}

// ─── task_service aggregation sourceExpression (A.2 #39-40) ─────────────────

/// Multi-instance variable-aggregation `sourceExpression` with an undefined
/// variable must fail when the aggregation runs (getValue family — no catch).
#[test]
fn task_service_aggregation_source_expression_undefined_variable_fails_complete() {
    let engine = ProcessEngine::new("w2-task-agg".into()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="aggProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
    <userTask id="miTask" name="MI Task">
      <multiInstanceLoopCharacteristics isSequential="false">
        <extensionElements>
          <flowable:variableAggregation target="reviews">
            <variable sourceExpression="${undefinedAggVar}" target="userId" />
          </flowable:variableAggregation>
        </extensionElements>
        <loopCardinality>1</loopCardinality>
      </multiInstanceLoopCharacteristics>
    </userTask>
    <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "agg.bpmn20.xml", xml);
    let instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .unwrap();
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "loopCardinality=1 should create one task");
    let error = engine
        .get_task_service()
        .complete_task_by_id(tasks[0].id.clone())
        .expect_err("complete must fail when the aggregation sourceExpression errors");
    let message = error.to_string();
    assert!(
        message.contains("undefinedAggVar")
            || message.contains("sourceExpression")
            || message.contains("aggregation")
            || message.contains("Unknown property")
            || message.contains("failed"),
        "error should identify the aggregation sourceExpression failure, got: {message}"
    );
}

// ─── external_worker in sourceExpression (A.2 #37) ─────────────────────────

/// External-worker in-parameter `sourceExpression` with an undefined variable
/// must fail the command — never silently drop the parameter or disguise the
/// error as an empty map. The service-task in-parameter path evaluates at
/// activity entry (`apply_service_task_in_parameters`), so start fails here.
#[test]
fn external_worker_in_source_expression_undefined_variable_fails_start() {
    let engine = ProcessEngine::new("w2-ext-in-expr".into()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="extInExprProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="ext1" />
    <serviceTask id="ext1" flowable:type="external-worker" flowable:topic="orders">
      <extensionElements>
        <flowable:in sourceExpression="${undefinedInVar}" target="childIn" />
      </extensionElements>
    </serviceTask>
    <sequenceFlow id="f2" sourceRef="ext1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    let definition_id = deploy(&engine, "ext-in-expr.bpmn20.xml", xml);
    let message = start_must_fail(
        &engine,
        &definition_id,
        "start must fail when the external worker in sourceExpression errors",
    );
    assert!(
        message.contains("undefinedInVar")
            || message.contains("sourceExpression")
            || message.contains("Unknown property")
            || message.contains("failed"),
        "error should identify the in sourceExpression failure, got: {message}"
    );
}
