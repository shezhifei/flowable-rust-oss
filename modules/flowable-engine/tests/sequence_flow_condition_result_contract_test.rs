//! Java parity contracts for `UelExpressionCondition`.
//!
//! Reference: Flowable Java `UelExpressionCondition.java:39-44` rejects null
//! and non-Boolean results with `FlowableException`. Since sequence-flow
//! selection runs inside the start command, that exception rolls back every
//! runtime mutation made by the command.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use serde_json::json;

fn deploy_condition_process(engine: &ProcessEngine, process_key: &str, condition: &str) -> String {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="{process_key}" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="conditionalFlow" sourceRef="start" targetRef="task">
      <conditionExpression><![CDATA[{condition}]]></conditionExpression>
    </sequenceFlow>
    <userTask id="task" />
  </process>
</definitions>"#
    );

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(process_key.to_string())
                .add_string(format!("{process_key}.bpmn20.xml"), xml),
        )
        .unwrap();

    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.starts_with(&format!("{process_key}:")))
        .expect("deployed process definition")
}

fn assert_no_runtime_state(engine: &ProcessEngine) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(store.snapshot_process_instances(&mut session).is_empty());
    assert!(store.snapshot_executions(&mut session).is_empty());
    assert!(store.snapshot_tasks(&mut session).is_empty());
    session.rollback().unwrap();
}

#[test]
fn missing_comparison_operand_fails_and_rolls_back_start_command() {
    let engine = ProcessEngine::new("p35-missing-condition".to_string());
    let definition_id = deploy_condition_process(&engine, "missingConditionProcess", "${x != 'a'}");

    let error = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .expect_err("a null condition result must abort process start");

    assert!(matches!(
        error,
        FlowableError::ExecutionError(message)
            if message.contains("non-Boolean")
                && message.contains("conditionalFlow")
                && message.ends_with("null")
    ));
    assert_no_runtime_state(&engine);
}

#[test]
fn non_boolean_sequence_flow_condition_fails_and_rolls_back_start_command() {
    let engine = ProcessEngine::new("p35-non-boolean-condition".to_string());
    let definition_id =
        deploy_condition_process(&engine, "nonBooleanConditionProcess", "${decision}");

    let error = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id)
                .variable("decision".to_string(), json!("approve")),
        )
        .expect_err("a non-Boolean condition result must abort process start");

    assert!(matches!(
        error,
        FlowableError::ExecutionError(message)
            if message.contains("non-Boolean")
                && message.contains("conditionalFlow")
                && message.contains("approve")
    ));
    assert_no_runtime_state(&engine);
}
