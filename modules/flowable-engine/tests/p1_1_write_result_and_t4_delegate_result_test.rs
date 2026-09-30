// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over.
#![allow(clippy::unwrap_used)]

//! P1-1 three-state result write + T4 class/delegate resultVariableName.
//!
//! R1 matrix (ScriptTask) — 禁止单测 `null;` 假绿:
//! - R1-NONE: 无表达式收尾 `var _x = 1;` → 写 Null（覆盖旧值）
//! - R1-NULL: `null;` → 写 Null
//! - R1-VAL: `var _x = 1; 42` → 写 42
//! - R1-SKIP: 不配置 resultVariable → 不创建变量
//!
//! R2 matrix (ServiceTask expression) — same S-WRITE-NULL contract.
//! T4: class/delegateExpression + resultVariableName is a deploy error
//! (Java ServiceTaskValidator:97-103) and runtime S-SKIP.

use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use flowable_engine::validation::unsupported_model_validator::UnsupportedModelValidator;
use serde_json::{Value, json};

fn script_task_xml(result_attr: &str, script: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="r1Process" name="R1" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="scriptTask1" />
        <scriptTask id="scriptTask1" scriptFormat="javascript" {result_attr}>
            <script>{script}</script>
        </scriptTask>
        <sequenceFlow id="flow2" sourceRef="scriptTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After Script" />
    </process>
</definitions>"#
    )
}

fn start_script_and_vars(
    xml: &str,
) -> Result<std::collections::HashMap<String, Value>, FlowableError> {
    start_script_and_vars_with(xml, &[])
}

fn start_script_and_vars_with(
    xml: &str,
    initial: &[(&str, Value)],
) -> Result<std::collections::HashMap<String, Value>, FlowableError> {
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string()],
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config(
        format!(
            "p1-1-r1-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ),
        config,
    )?;
    let repository = engine.get_repository_service();
    let builder = repository
        .create_deployment()
        .name("r1".to_string())
        .add_string("r1.bpmn20.xml".to_string(), xml.to_string());
    repository.deploy(builder)?;
    let def_id = repository.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let mut builder = runtime
        .create_process_instance_builder()
        .process_definition_id(def_id);
    for (name, value) in initial {
        builder = builder.variable(name.to_string(), value.clone());
    }
    let instance = runtime.start_process_instance(builder)?;
    let vars = runtime.get_variables(instance.id.clone())?;
    Ok(vars)
}

// ── R1 ScriptTask matrix ─────────────────────────────────────────────────────

#[test]
fn r1_none_endless_statement_writes_null() {
    // R1-NONE: 无表达式收尾 — must WRITE Null (S-WRITE-NULL).
    // `null;` alone is a forbidden fake-green script.
    let xml = script_task_xml(r#"flowable:resultVariable="r1out""#, "var _x = 1;");
    let vars = start_script_and_vars(&xml).expect("R1-NONE must run");
    assert!(
        vars.contains_key("r1out"),
        "R1-NONE must WRITE the result variable as Null (S-WRITE-NULL), vars={vars:?}"
    );
    assert_eq!(
        vars.get("r1out"),
        Some(&Value::Null),
        "R1-NONE result must be Null, not missing"
    );
}

#[test]
fn r1_none_overwrites_existing_non_null_value_with_null() {
    // M-3 / plan risk: S-WRITE-NULL must overwrite a prior non-Null value
    // (Java setVariable(result, null) overwrites). Seed r1out=42, then R1-NONE.
    let xml = script_task_xml(r#"flowable:resultVariable="r1out""#, "var _x = 1;");
    let vars = start_script_and_vars_with(&xml, &[("r1out", json!(42))])
        .expect("R1-NONE overwrite must run");
    assert!(
        vars.contains_key("r1out"),
        "overwrite must keep the key present, vars={vars:?}"
    );
    assert_eq!(
        vars.get("r1out"),
        Some(&Value::Null),
        "R1-NONE must OVERWRITE prior 42 with Null, vars={vars:?}"
    );
}

#[test]
fn r1_null_explicit_writes_null() {
    let xml = script_task_xml(r#"flowable:resultVariable="r1out""#, "null;");
    let vars = start_script_and_vars(&xml).expect("run");
    assert!(vars.contains_key("r1out"), "vars={vars:?}");
    assert_eq!(vars.get("r1out"), Some(&Value::Null), "vars={vars:?}");
}

#[test]
fn r1_val_writes_returned_value() {
    let xml = script_task_xml(r#"flowable:resultVariable="r1out""#, "var _x = 1; 42");
    let vars = start_script_and_vars(&xml).expect("run");
    assert_eq!(vars.get("r1out"), Some(&json!(42)), "vars={vars:?}");
}

#[test]
fn r1_skip_no_result_variable_is_not_created() {
    let xml = script_task_xml("", "var _x = 1; 42");
    let vars = start_script_and_vars(&xml).expect("R1-SKIP must run");
    assert!(
        !vars.contains_key("r1out"),
        "R1-SKIP must not create a result variable"
    );
}

// ── R2 ServiceTask expression matrix (S-WRITE-NULL) ──────────────────────────

fn expression_task_xml(result_attr: &str, expression: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="r2Process" name="R2" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="exprTask1" />
        <serviceTask id="exprTask1" flowable:expression="{expression}" {result_attr} />
        <sequenceFlow id="flow2" sourceRef="exprTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After Expression" />
    </process>
</definitions>"#
    )
}

fn run_expression_service_task(
    xml: &str,
) -> Result<(Option<Value>, bool /*present*/), FlowableError> {
    // Full deploy + start path: expression type is now deploy-allowed
    // (Java ServiceTaskValidator.verifyImplementation:49), so R2 is
    // production-reachable. Validator + runtime write are both exercised.
    let vars = start_script_and_vars(xml)?;
    let value = vars.get("r2out").cloned();
    let present = vars.contains_key("r2out");
    Ok((value, present))
}

#[test]
fn r2_expression_val_writes_result() {
    let xml = expression_task_xml(r#"flowable:resultVariableName="r2out""#, "${40+2}");
    let (value, present) = run_expression_service_task(&xml).expect("R2-VAL must run");
    assert!(present, "R2-VAL must write r2out");
    assert_eq!(value, Some(json!(42)));
}

#[test]
fn r2_expression_null_writes_null() {
    let xml = expression_task_xml(r#"flowable:resultVariableName="r2out""#, "${null}");
    let (value, present) = run_expression_service_task(&xml).expect("R2-NULL must run");
    assert!(present, "R2-NULL must write the variable as Null");
    assert_eq!(value, Some(Value::Null));
}

#[test]
fn r2_expression_skip_does_not_create_variable() {
    let xml = expression_task_xml("", "${40+2}");
    let (value, present) = run_expression_service_task(&xml).expect("R2-SKIP must run");
    assert!(!present, "R2-SKIP must not create a result variable, got {value:?}");
}

// ── T4 class/delegate + resultVariableName ───────────────────────────────────

const CLASS_WITH_RESULT_VAR_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="t4Process" name="T4" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="delegateTask1" />
        <serviceTask id="delegateTask1"
                     flowable:class="com.example.MyDelegate"
                     flowable:resultVariableName="delegateResult" />
        <sequenceFlow id="flow2" sourceRef="delegateTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After" />
    </process>
</definitions>"#;

const DELEGATE_EXPR_WITH_RESULT_VAR_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="t4Process" name="T4" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="delegateTask1" />
        <serviceTask id="delegateTask1"
                     flowable:delegateExpression="${delegateName}"
                     flowable:resultVariableName="delegateResult" />
        <sequenceFlow id="flow2" sourceRef="delegateTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After" />
    </process>
</definitions>"#;

#[test]
fn t4_class_with_result_variable_name_is_deploy_rejected() {
    let model = BpmnXMLConverter::new().convert_to_bpmn_model(CLASS_WITH_RESULT_VAR_XML);
    let err = UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect_err("class + resultVariableName must be a deployment error (Java :97-103)");
    let message = err.to_string();
    assert!(
        message.contains("resultVariableName"),
        "error must mirror Java ServiceTaskValidator message, got: {message}"
    );
    assert!(
        message.contains("class") && message.contains("delegateExpression"),
        "error must name both implementation kinds, got: {message}"
    );
}

#[test]
fn t4_delegate_expression_with_result_variable_name_is_deploy_rejected() {
    let model = BpmnXMLConverter::new().convert_to_bpmn_model(DELEGATE_EXPR_WITH_RESULT_VAR_XML);
    let err = UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect_err("delegateExpression + resultVariableName must be a deployment error");
    assert!(err.to_string().contains("resultVariableName"));
}

#[test]
fn t4_class_with_result_variable_name_fails_real_repository_deploy() {
    // X2: real `repository.deploy(...)` path (not just UnsupportedModelValidator).
    let engine = ProcessEngine::new_with_config(
        format!(
            "t4-deploy-reject-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ),
        ProcessEngineConfiguration {
            enable_secure_scripting: true,
            ..Default::default()
        },
    )
    .expect("engine");
    let repository = engine.get_repository_service();
    let builder = repository
        .create_deployment()
        .name("t4".to_string())
        .add_string(
            "t4.bpmn20.xml".to_string(),
            CLASS_WITH_RESULT_VAR_XML.to_string(),
        );
    let err = repository
        .deploy(builder)
        .expect_err("class + resultVariableName must fail repository.deploy");
    assert!(
        err.to_string().contains("resultVariableName"),
        "deploy error must mention resultVariableName, got: {err}"
    );
}

#[test]
fn t4_expression_with_result_variable_name_passes_real_repository_deploy() {
    // X2 positive: expression + resultVariableName deploys through the real path.
    let engine = ProcessEngine::new_with_config(
        format!(
            "t4-deploy-expr-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ),
        ProcessEngineConfiguration {
            enable_secure_scripting: true,
            ..Default::default()
        },
    )
    .expect("engine");
    let repository = engine.get_repository_service();
    let builder = repository.create_deployment().name("t4e".to_string()).add_string(
        "t4e.bpmn20.xml".to_string(),
        expression_task_xml(r#"flowable:resultVariableName="r2out""#, "${40+2}"),
    );
    repository
        .deploy(builder)
        .expect("expression + resultVariableName must deploy (Java-aligned)");
}
