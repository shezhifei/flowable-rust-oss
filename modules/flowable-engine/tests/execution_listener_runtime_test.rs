//! Runtime tests for BPMN executionListener execution via the local registry.

use flowable_engine::bpmn::listener::{
    ExecutionListenerContext, LocalExecutionListener, LocalExecutionListenerRegistry,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;
use std::sync::Arc;

struct SetVariableOnStartListener;

impl LocalExecutionListener for SetVariableOnStartListener {
    fn notify(&self, ctx: &mut ExecutionListenerContext<'_>) -> Result<(), FlowableError> {
        ctx.execution.set_process_variable(
            "startListenerFired".to_string(),
            json!(format!("start:{}", ctx.activity_id.unwrap_or("unknown"))),
        );
        Ok(())
    }
}

struct SetVariableOnEndListener;

impl LocalExecutionListener for SetVariableOnEndListener {
    fn notify(&self, ctx: &mut ExecutionListenerContext<'_>) -> Result<(), FlowableError> {
        ctx.execution.set_process_variable(
            "endListenerFired".to_string(),
            json!(format!("end:{}", ctx.activity_id.unwrap_or("unknown"))),
        );
        Ok(())
    }
}

fn process_xml_with_execution_listeners() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="executionListenerProcess" name="Execution Listener Process" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review">
                <extensionElements>
                    <flowable:executionListener event="start"
                        delegateExpression="${startListenerName}" />
                    <flowable:executionListener event="end"
                        class="endListener" />
                </extensionElements>
            </userTask>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#
    .to_string()
}

fn engine_with_execution_listeners() -> ProcessEngine {
    let mut registry = LocalExecutionListenerRegistry::new();
    registry.register("startListener", Arc::new(SetVariableOnStartListener));
    registry.register("endListener", Arc::new(SetVariableOnEndListener));

    let mut config = ProcessEngineConfiguration::default();
    config.execution_listener_registry = Some(registry);
    ProcessEngine::new_with_config("execution-listener-test".to_string(), config)
}

#[test]
fn execution_listeners_fire_on_user_task_start_and_end() {
    let process_engine = engine_with_execution_listeners();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let builder = repository_service
        .create_deployment()
        .name("Execution Listener Deployment".to_string())
        .add_string(
            "executionListenerProcess.bpmn20.xml".to_string(),
            process_xml_with_execution_listeners(),
        );
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("startListenerName".to_string(), json!("startListener")),
        )
        .unwrap();

    let vars = runtime_service
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        vars.get("startListenerFired"),
        Some(&json!("start:userTask1")),
        "start executionListener should set a process variable"
    );
    assert!(
        vars.get("endListenerFired").is_none(),
        "end listener must not fire while the user task is still waiting"
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let vars_after = runtime_service
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        vars_after.get("startListenerFired"),
        Some(&json!("start:userTask1"))
    );
    assert_eq!(
        vars_after.get("endListenerFired"),
        Some(&json!("end:userTask1")),
        "end executionListener should set a process variable on leave"
    );
}

#[test]
fn unregistered_execution_listener_fails_with_clear_error() {
    // Engine has no registry entries.
    let process_engine = ProcessEngine::new("execution-listener-missing".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="missingListenerProcess" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1">
                <extensionElements>
                    <flowable:executionListener event="start" class="doesNotExist" />
                </extensionElements>
            </userTask>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Missing Listener Deployment".to_string())
        .add_string("missing.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let err = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .expect_err("unregistered listener should fail");

    let msg = err.to_string();
    assert!(
        msg.contains("No local execution listener 'doesNotExist'"),
        "expected clear unregistered-listener error, got: {msg}"
    );
}
