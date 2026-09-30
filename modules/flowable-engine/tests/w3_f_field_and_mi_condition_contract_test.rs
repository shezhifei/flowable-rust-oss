// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! W3 / C3 — F-group user-task name/category/formKey contracts.
//!
//! Java evidence (`UserTaskActivityBehavior.java:208-324`):
//! - handleName / handleCategory / handleFormKey wrap `getValue` in
//!   `try/catch (FlowableException)` → fall back to the **model text** + warn.
//! - `createExpression` parse failures throw `ELException` (not
//!   `FlowableException`) and are NOT caught → propagate (N4-1).
//! - A successful evaluation to null sets the field to null (not empty-string
//!   disguise, not activity_id).

use flowable_engine::el::method_registry::ExpressionMethodRegistry;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;

fn engine_with_registry(name: &str, registry: ExpressionMethodRegistry) -> ProcessEngine {
    let config = ProcessEngineConfiguration {
        expression_method_registry: registry,
        ..Default::default()
    };
    ProcessEngine::new_with_config(name.to_string(), config).unwrap()
}

fn deploy_user_task(
    engine: &ProcessEngine,
    resource: &str,
    name_attr: &str,
    category_attr: &str,
    form_key_attr: &str,
) -> String {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="fFieldProcess" isExecutable="true">
    <startEvent id="startEvent1" />
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
    <userTask id="userTask1" name="{name_attr}" flowable:category="{category_attr}" flowable:formKey="{form_key_attr}" />
    <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#
    );
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("f-field".to_string())
                .add_string(resource.to_string(), xml),
        )
        .unwrap();
    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone()
}

fn first_task_fields(engine: &ProcessEngine) -> (String, Option<String>, Option<String>) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let tasks = store.snapshot_tasks(&mut session);
    session.rollback().unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one task, got {}", tasks.len());
    let task = tasks.values().next().unwrap();
    (
        task.name.clone(),
        task.category.clone(),
        task.form_key.clone(),
    )
}

/// Literal model text is returned unchanged (Java createExpression("My Task")
/// evaluates to the literal).
#[test]
fn f_field_literal_text_is_used_unchanged() {
    let engine = ProcessEngine::new("f-literal".to_string()).unwrap();
    let definition_id = deploy_user_task(
        &engine,
        "f-literal.bpmn20.xml",
        "Approve Request",
        "orders",
        "form-1",
    );
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .unwrap();
    let (name, category, form_key) = first_task_fields(&engine);
    assert_eq!(name, "Approve Request");
    assert_eq!(category.as_deref(), Some("orders"));
    assert_eq!(form_key.as_deref(), Some("form-1"));
}

/// Eval-time failure (undefined variable) → fallback to the **model text** +
/// warn. Java handleName catch(FlowableException) keeps `beforeContext.getName()`.
#[test]
fn f_field_eval_failure_falls_back_to_model_text() {
    let engine = ProcessEngine::new("f-fallback".to_string()).unwrap();
    let definition_id = deploy_user_task(
        &engine,
        "f-fallback.bpmn20.xml",
        "${thisVarIsNeverDefined}",
        "${thisVarIsNeverDefined}",
        "${thisVarIsNeverDefined}",
    );
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect("eval failure must NOT fail the command (Java catch → fallback)");
    let (name, category, form_key) = first_task_fields(&engine);
    // Fallback is the raw model expression text — never empty, never activity_id.
    assert_eq!(name, "${thisVarIsNeverDefined}");
    assert_eq!(category.as_deref(), Some("${thisVarIsNeverDefined}"));
    assert_eq!(form_key.as_deref(), Some("${thisVarIsNeverDefined}"));
}

/// Successful evaluation to a defined-null value → field cleared (Java
/// setName(null) / setCategory(null) / setFormKey(null)). C4: must NOT become
/// activity_id for name. Use `${null}` literal (parses to Value::Null).
#[test]
fn f_field_legal_null_clears_field() {
    let engine = ProcessEngine::new("f-null".to_string()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="fFieldProcess" isExecutable="true">
    <startEvent id="startEvent1" />
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
    <userTask id="userTask1" name="${null}" flowable:category="${null}" flowable:formKey="${null}" />
    <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#;
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("f-null".to_string())
                .add_string("f-null.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect("legal null must not fail the command");
    let (name, category, form_key) = first_task_fields(&engine);
    // C4: name is NOT activity_id. Task.name is String so Java null → "".
    assert_ne!(name, "userTask1", "legal null name must not become activity_id");
    assert_eq!(name, "", "Java setName(null) maps to empty String in Rust");
    assert_eq!(category, None, "legal null category clears the field");
    assert_eq!(form_key, None, "legal null formKey clears the field");
}

/// Successful evaluation to a value uses the value (Java toString()).
#[test]
fn f_field_expression_value_is_used() {
    let registry = ExpressionMethodRegistry::new();
    let engine = engine_with_registry("f-value", registry);
    let definition_id = deploy_user_task(
        &engine,
        "f-value.bpmn20.xml",
        "${nameVar}",
        "${catVar}",
        "${keyVar}",
    );
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id)
                .variable("nameVar".to_string(), serde_json::json!("Resolved Name"))
                .variable("catVar".to_string(), serde_json::json!("resolved-cat"))
                .variable("keyVar".to_string(), serde_json::json!("resolved-key")),
        )
        .unwrap();
    let (name, category, form_key) = first_task_fields(&engine);
    assert_eq!(name, "Resolved Name");
    assert_eq!(category.as_deref(), Some("resolved-cat"));
    assert_eq!(form_key.as_deref(), Some("resolved-key"));
}

/// N4-1: parse/compile failure propagates (Java createExpression ELException is
/// NOT caught by handleName's catch(FlowableException)).
#[test]
fn f_field_compile_failure_propagates() {
    let engine = ProcessEngine::new("f-compile".to_string()).unwrap();
    // `${1 +}` is well-formed EL wrapper but fails to parse.
    let definition_id = deploy_user_task(
        &engine,
        "f-compile.bpmn20.xml",
        "${1 +}",
        "orders",
        "form-1",
    );
    let error = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect_err("compile failure must propagate (N4-1, Java ELException)");
    let message = error.to_string();
    assert!(
        message.contains("compile") || message.contains("Error while evaluating"),
        "error should identify the compile failure, got: {message}"
    );
}

/// C1 regression: multi_instance completionCondition null result must fail the
/// command on BOTH paths (multi_instance_support AND task_service), matching
/// UelExpressionCondition.java:39-40 / MultiInstanceActivityBehavior.java:390-394.
#[test]
fn mi_completion_condition_null_fails_command() {
    let engine = ProcessEngine::new("mi-null-cond".to_string()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="miNullCondProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
    <userTask id="miTask" name="MI">
      <multiInstanceLoopCharacteristics isSequential="false">
        <loopCardinality>2</loopCardinality>
        <completionCondition>${maybeDone}</completionCondition>
      </multiInstanceLoopCharacteristics>
    </userTask>
    <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("mi-null".to_string())
                .add_string("mi-null.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    // maybeDone is undefined → eval error (not null message), OR if defined-null
    // → "returns null" message. Either way the command must fail (not Ok(false)).
    let error = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect_err("null/undefined completionCondition must fail the command");
    let message = error.to_string();
    assert!(
        message.contains("condition expression")
            || message.contains("completionCondition")
            || message.contains("Unknown property")
            || message.contains("returns null")
            || message.contains("non-Boolean"),
        "error should identify the condition failure, got: {message}"
    );
}

/// C1 regression: defined-null completionCondition → "returns null" message.
#[test]
fn mi_completion_condition_defined_null_fails_with_null_message() {
    let engine = ProcessEngine::new("mi-null2".to_string()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="miNullCondProcess2" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
    <userTask id="miTask" name="MI">
      <multiInstanceLoopCharacteristics isSequential="false">
        <loopCardinality>2</loopCardinality>
        <completionCondition>${doneFlag}</completionCondition>
      </multiInstanceLoopCharacteristics>
    </userTask>
    <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("mi-null2".to_string())
                .add_string("mi-null2.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let error = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id)
                .variable("doneFlag".to_string(), serde_json::Value::Null),
        )
        .expect_err("defined-null completionCondition must fail with 'returns null'");
    let message = error.to_string();
    assert!(
        message.contains("returns null"),
        "expected 'returns null' message, got: {message}"
    );
}

/// C1 residual close: the **complete-task** MI path (task_service) must use the
/// same three-way classification as the MI runtime path. Before the dual-copy
/// dedupe this path was a separate function; a null/undefined
/// completionCondition must fail `complete_task`, not be swallowed as
/// `Ok(false)`.
#[test]
fn mi_completion_condition_null_fails_complete_task_path() {
    let engine = ProcessEngine::new("mi-null-complete".to_string()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="miNullCompleteProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
    <userTask id="miTask" name="MI">
      <multiInstanceLoopCharacteristics isSequential="false">
        <loopCardinality>2</loopCardinality>
        <completionCondition>${doneFlag}</completionCondition>
      </multiInstanceLoopCharacteristics>
    </userTask>
    <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("mi-null-complete".to_string())
                .add_string("mi-null-complete.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    // Start with a legal Boolean so instance creation can evaluate
    // completionCondition to `false`. Then flip the variable to null and
    // complete a task: the complete-task path must fail, not swallow.
    let instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id)
                .variable("doneFlag".to_string(), serde_json::Value::Bool(false)),
        )
        .unwrap();
    let process_instance_id = instance.id.clone();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let tasks = store.snapshot_tasks(&mut session);
    session.rollback().unwrap();
    assert_eq!(tasks.len(), 2, "expected two MI child tasks, got {}", tasks.len());
    let task_id = tasks.values().next().unwrap().id.clone();

    engine
        .get_runtime_service()
        .set_variable(
            process_instance_id.clone(),
            "doneFlag".to_string(),
            serde_json::Value::Null,
        )
        .unwrap();

    let error = engine
        .get_task_service()
        .complete_task_by_id(task_id)
        .expect_err(
            "complete-task MI path must fail on defined-null completionCondition \
             (shared three-way classification, not Ok(false))",
        );
    let message = error.to_string();
    assert!(
        message.contains("returns null") || message.contains("completionCondition"),
        "expected null/condition failure message, got: {message}"
    );
}

/// C1 residual close: complete-task path with a **non-Boolean** result must
/// fail (third arm of the three-way classification).
#[test]
fn mi_completion_condition_non_boolean_fails_complete_task_path() {
    let engine = ProcessEngine::new("mi-nonbool-complete".to_string()).unwrap();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="miNonBoolCompleteProcess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
    <userTask id="miTask" name="MI">
      <multiInstanceLoopCharacteristics isSequential="false">
        <loopCardinality>2</loopCardinality>
        <completionCondition>${doneFlag}</completionCondition>
      </multiInstanceLoopCharacteristics>
    </userTask>
    <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("mi-nonbool-complete".to_string())
                .add_string(
                    "mi-nonbool-complete.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id)
                .variable("doneFlag".to_string(), serde_json::Value::Bool(false)),
        )
        .unwrap();
    let process_instance_id = instance.id.clone();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let tasks = store.snapshot_tasks(&mut session);
    session.rollback().unwrap();
    let task_id = tasks.values().next().unwrap().id.clone();

    engine
        .get_runtime_service()
        .set_variable(
            process_instance_id.clone(),
            "doneFlag".to_string(),
            serde_json::Value::String("not-a-bool".to_string()),
        )
        .unwrap();

    let error = engine
        .get_task_service()
        .complete_task_by_id(task_id)
        .expect_err(
            "complete-task MI path must fail on non-Boolean completionCondition \
             (shared three-way classification)",
        );
    let message = error.to_string();
    assert!(
        message.contains("non-Boolean") || message.contains("completionCondition"),
        "expected non-Boolean failure message, got: {message}"
    );
}
