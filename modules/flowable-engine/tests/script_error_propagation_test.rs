//! Java ScriptingEngines.evaluate / ScriptTaskActivityBehavior.safelyExecuteScript:
//! a script failure must remain an error regardless of the exception message.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::scripting::secure_context::SecureScriptContext;
use flowable_engine::scripting::secure_engine::SecureScriptEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::{Value, json};
use std::collections::HashMap;

fn evaluate(script: &str) -> Result<Option<Value>, FlowableError> {
    let engine = SecureScriptEngine::new(vec!["javascript".to_string()]);
    let mut context = SecureScriptContext::from_variables(HashMap::new());
    engine.execute("javascript", script, &mut context)
}

#[test]
fn real_error_containing_return_marker_is_not_a_successful_null() {
    for marker in [
        "ordinary failure",
        "__RETURN__:null",
        "__RETURN__:{invalid}",
    ] {
        let script = format!("function fail() {{ return 1 - '{marker}'; }} fail();");
        let error = evaluate(&script).expect_err("the arithmetic error must escape the function");
        assert!(matches!(&error, FlowableError::ExecutionError(_)));
        assert!(error.to_string().contains("Cannot apply '-'"), "{error}");
        assert!(error.to_string().contains(marker), "{error}");
    }
}

#[test]
fn nested_function_error_keeps_its_original_cause() {
    let error = evaluate(
        "function inner() { return 1 - '__RETURN__:null'; }
         function outer() { return inner(); }
         outer();",
    )
    .expect_err("every call frame must propagate the original error");
    assert!(error.to_string().contains("Cannot apply '-'"), "{error}");
    assert!(error.to_string().contains("__RETURN__:null"), "{error}");
}

#[test]
fn typed_return_escapes_blocks_and_loops_without_running_later_statements() {
    let cases = [
        "function f() { if (true) { return 42; } missing(); } f();",
        "function f() { for (var i = 0; i < 2; i += 1) { return 42; } missing(); } f();",
        "function f() { while (true) { if (true) { return 42; } } missing(); } f();",
        "if (true) { return 42; } missing();",
    ];
    for script in cases {
        assert_eq!(evaluate(script).unwrap(), Some(json!(42)), "{script}");
    }
}

#[test]
fn return_preserves_structured_values_and_marker_text_as_data() {
    assert_eq!(
        evaluate(
            "function f() { return {message: '__RETURN__:null', items: [1, null, 'é']}; } f();"
        )
        .unwrap(),
        Some(json!({"message": "__RETURN__:null", "items": [1, null, "é"]}))
    );
    assert_eq!(
        evaluate("function f() { return; } f();").unwrap(),
        Some(Value::Null)
    );
    assert_eq!(evaluate("var value = 1;").unwrap(), None);
}

#[test]
fn script_task_error_with_return_marker_rolls_back_process_start() {
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string()],
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("script-error-parity".to_string(), config).unwrap();
    let repository = engine.get_repository_service();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Tests">
  <process id="scriptErrorParity" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toScript" sourceRef="start" targetRef="script" />
    <scriptTask id="script" scriptFormat="javascript" flowable:resultVariable="result">
      <script><![CDATA[
        execution.setVariable('sideEffect', 42);
        function fail() { return 1 - '__RETURN__:null'; }
        fail();
      ]]></script>
    </scriptTask>
    <sequenceFlow id="toWait" sourceRef="script" targetRef="wait" />
    <userTask id="wait" />
  </process>
</definitions>"#;
    repository
        .deploy(
            repository
                .create_deployment()
                .add_string("error.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let definition_id = repository.get_process_definition_ids().unwrap().remove(0);
    let runtime = engine.get_runtime_service();
    let error = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect_err("a script exception must fail process start, not advance to the wait task");
    assert!(error.to_string().contains("Cannot apply '-'"), "{error}");
    assert!(error.to_string().contains("__RETURN__:null"), "{error}");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store.snapshot_executions(&mut session).is_empty(),
        "failed start must leave no execution or variables"
    );
    session
        .flush_and_commit()
        .expect("rollback verification must not hide a sticky storage error");
}
