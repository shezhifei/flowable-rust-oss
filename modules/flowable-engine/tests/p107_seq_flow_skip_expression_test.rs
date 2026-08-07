//! P107 — sequence-flow `flowable:skipExpression` converter parse, end-to-end.
//!
//! P106 wired the engine-side consumption (`should_skip_sequence_flow`,
//! `skip_expression.rs`) but the BPMN converter did not parse the attribute
//! (Java `SequenceFlowXMLConverter.java:46`), blocking XML end-to-end coverage.
//! These tests pin the full chain: XML attribute → model → gateway selection.

use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

const SKIP_EXPR_GATEWAY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="skipExprGatewayProcess" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow0" sourceRef="startEvent1" targetRef="gw" />
        <exclusiveGateway id="gw" />
        <sequenceFlow id="flowA" sourceRef="gw" targetRef="taskA"
                      flowable:skipExpression="${true}">
            <conditionExpression xsi:type="tFormalExpression"
                                 xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">${false}</conditionExpression>
        </sequenceFlow>
        <sequenceFlow id="flowB" sourceRef="gw" targetRef="taskB">
            <conditionExpression xsi:type="tFormalExpression"
                                 xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">${true}</conditionExpression>
        </sequenceFlow>
        <userTask id="taskA" name="Task A" />
        <userTask id="taskB" name="Task B" />
        <sequenceFlow id="flowA1" sourceRef="taskA" targetRef="endEvent1" />
        <sequenceFlow id="flowB1" sourceRef="taskB" targetRef="endEvent1" />
        <endEvent id="endEvent1" />
    </process>
</definitions>"#;

fn deploy_and_start(skip_enabled: bool) -> (ProcessEngine, String) {
    let engine = ProcessEngine::new(format!("p107-skip-expr-{skip_enabled}"));
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("Skip Expression Deployment".to_string())
        .add_string(
            "skipExprGatewayProcess.bpmn20.xml".to_string(),
            SKIP_EXPR_GATEWAY_XML.to_string(),
        );
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable(
                    "_ACTIVITI_SKIP_EXPRESSION_ENABLED".to_string(),
                    json!(skip_enabled),
                ),
        )
        .unwrap();
    (engine, instance.id)
}

/// Java `TakeOutgoingSequenceFlowsOperation.java:215-228`: with skip
/// expressions enabled, a sequence flow whose skipExpression evaluates to true
/// is selected directly, skipping its (false) condition — so taskA, not the
/// condition-true taskB, becomes the active task.
#[test]
fn skip_expression_on_sequence_flow_selects_flow_with_false_condition() {
    let (engine, process_instance_id) = deploy_and_start(true);
    let task_service = engine.get_task_service();
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].task_definition_key.as_str(),
        "taskA",
        "skipExpression=true must select flowA despite its false condition"
    );
}

/// Guard: with the skip machinery disabled the same model follows normal
/// condition evaluation and lands on taskB (condition true).
#[test]
fn skip_expression_disabled_falls_back_to_condition() {
    let (engine, process_instance_id) = deploy_and_start(false);
    let task_service = engine.get_task_service();
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].task_definition_key.as_str(),
        "taskB",
        "disabled skip machinery must evaluate conditions normally"
    );
}
