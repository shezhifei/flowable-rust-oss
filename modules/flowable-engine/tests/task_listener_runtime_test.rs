//! Runtime tests for BPMN taskListener execution via the local registry.

use flowable_engine::bpmn::listener::{
    LocalTaskListener, LocalTaskListenerRegistry, TaskListenerContext,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;
use std::sync::Arc;

struct CreateListener;

impl LocalTaskListener for CreateListener {
    fn notify(&self, ctx: &mut TaskListenerContext<'_>) -> Result<(), FlowableError> {
        ctx.execution.set_process_variable(
            "createListenerFired".to_string(),
            json!(format!("create:{}", ctx.task.task_definition_key)),
        );
        Ok(())
    }
}

struct CompleteListener;

impl LocalTaskListener for CompleteListener {
    fn notify(&self, ctx: &mut TaskListenerContext<'_>) -> Result<(), FlowableError> {
        ctx.execution.set_process_variable(
            "completeListenerFired".to_string(),
            json!(format!("complete:{}", ctx.task.task_definition_key)),
        );
        Ok(())
    }
}

struct AssignmentListener;

impl LocalTaskListener for AssignmentListener {
    fn notify(&self, ctx: &mut TaskListenerContext<'_>) -> Result<(), FlowableError> {
        let assignee = ctx.task.assignee.clone().unwrap_or_default();
        ctx.execution.set_process_variable(
            "assignmentListenerFired".to_string(),
            json!(format!("assignment:{}", assignee)),
        );
        Ok(())
    }
}

struct DeleteListener;

impl LocalTaskListener for DeleteListener {
    fn notify(&self, ctx: &mut TaskListenerContext<'_>) -> Result<(), FlowableError> {
        ctx.execution.set_process_variable(
            "deleteListenerFired".to_string(),
            json!(format!("delete:{}", ctx.task.task_definition_key)),
        );
        Ok(())
    }
}

fn process_xml_with_delete_listener() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="deleteListenerProcess" name="Delete Listener Process" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review" flowable:assignee="kermit">
                <extensionElements>
                    <flowable:taskListener event="delete" class="deleteListener" />
                </extensionElements>
            </userTask>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#
        .to_string()
}

fn engine_with_delete_listener() -> ProcessEngine {
    let mut registry = LocalTaskListenerRegistry::new();
    registry.register("deleteListener", Arc::new(DeleteListener));
    let mut config = ProcessEngineConfiguration::default();
    config.task_listener_registry = Some(registry);
    ProcessEngine::new_with_config("delete-listener-test".to_string(), config)
}

#[test]
fn delete_task_fires_delete_listener() {
    // Java `TaskHelper.java:433-468` (`internalDeleteTask`) fires the
    // `delete` task listener before performing the actual task row
    // deletion. Mirrors the same call from `DeleteTaskCmd` here.
    let process_engine = engine_with_delete_listener();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let builder = repository_service
        .create_deployment()
        .name("Delete Listener Deployment".to_string())
        .add_string(
            "deleteListenerProcess.bpmn20.xml".to_string(),
            process_xml_with_delete_listener(),
        );
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    let task_id = tasks[0].id.clone();

    // Java allows deletion of a task that is part of a running process; the
    // prior Rust guard that refused it was a parity gap. The delete listener
    // is now invoked and the row is actually removed.
    task_service
        .delete_task(task_id.clone(), Some("forced-delete".to_string()), false)
        .expect("delete should succeed for a live-process task");

    let vars = runtime_service
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        vars.get("deleteListenerFired"),
        Some(&json!("delete:userTask1")),
        "delete taskListener should set a process variable"
    );

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after.is_empty(),
        "task row must be removed after delete"
    );
}

fn process_xml_with_task_listeners() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="taskListenerProcess" name="Task Listener Process" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review" flowable:assignee="kermit">
                <extensionElements>
                    <flowable:taskListener event="create" class="createListener" />
                    <flowable:taskListener event="assignment"
                        delegateExpression="${assignmentListenerName}" />
                    <flowable:taskListener event="complete" class="completeListener" />
                </extensionElements>
            </userTask>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#
        .to_string()
}

fn engine_with_task_listeners() -> ProcessEngine {
    let mut registry = LocalTaskListenerRegistry::new();
    registry.register("createListener", Arc::new(CreateListener));
    registry.register("completeListener", Arc::new(CompleteListener));
    registry.register("assignmentListener", Arc::new(AssignmentListener));

    let mut config = ProcessEngineConfiguration::default();
    config.task_listener_registry = Some(registry);
    ProcessEngine::new_with_config("task-listener-test".to_string(), config)
}

#[test]
fn task_listeners_fire_on_create_assignment_and_complete() {
    let process_engine = engine_with_task_listeners();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let builder = repository_service
        .create_deployment()
        .name("Task Listener Deployment".to_string())
        .add_string(
            "taskListenerProcess.bpmn20.xml".to_string(),
            process_xml_with_task_listeners(),
        );
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable(
                    "assignmentListenerName".to_string(),
                    json!("assignmentListener"),
                ),
        )
        .unwrap();

    let vars = runtime_service
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        vars.get("createListenerFired"),
        Some(&json!("create:userTask1")),
        "create taskListener should set a process variable"
    );
    assert_eq!(
        vars.get("assignmentListenerFired"),
        Some(&json!("assignment:kermit")),
        "assignment taskListener should fire when model assignee is set"
    );
    assert!(
        vars.get("completeListenerFired").is_none(),
        "complete listener must not fire before task completion"
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee.as_deref(), Some("kermit"));

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let vars_after = runtime_service
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        vars_after.get("completeListenerFired"),
        Some(&json!("complete:userTask1")),
        "complete taskListener should set a process variable"
    );
}

#[test]
fn unregistered_task_listener_fails_with_clear_error() {
    let process_engine = ProcessEngine::new("task-listener-missing".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="missingTaskListenerProcess" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1">
                <extensionElements>
                    <flowable:taskListener event="create" class="missingTaskListener" />
                </extensionElements>
            </userTask>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Missing Task Listener Deployment".to_string())
        .add_string("missing-task.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let err = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .expect_err("unregistered task listener should fail");

    let msg = err.to_string();
    assert!(
        msg.contains("No local task listener 'missingTaskListener'"),
        "expected clear unregistered-listener error, got: {msg}"
    );
}
