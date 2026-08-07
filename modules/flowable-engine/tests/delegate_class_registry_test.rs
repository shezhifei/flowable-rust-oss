//! M76: `implementation_type=class` resolves via the same local delegate registry
//! as `delegateExpression` (registry key, not JVM classloading).

use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_engine::bpmn::behavior::service_task_activity_behavior::{
    LocalServiceTaskDelegate, LocalServiceTaskDelegateContext, LocalServiceTaskDelegateRegistry,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use flowable_engine::validation::unsupported_model_validator::UnsupportedModelValidator;
use serde_json::{Value, json};
use std::sync::Arc;

const MY_DELEGATE_CLASS: &str = "com.example.MyDelegate";

const CLASS_DELEGATE_SERVICE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="classDelegateProcess" name="Class Delegate Process" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="delegateTask1" />
        <serviceTask id="delegateTask1"
                     name="Invoke Class Delegate"
                     flowable:class="com.example.MyDelegate"
                     flowable:resultVariableName="delegateResult">
            <extensionElements>
                <flowable:field name="greeting" stringValue="hello-from-class" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="delegateTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After Class Delegate" />
    </process>
</definitions>"#;

struct MyDelegate;

impl LocalServiceTaskDelegate for MyDelegate {
    fn execute(
        &self,
        context: &mut LocalServiceTaskDelegateContext<'_>,
    ) -> Result<Value, FlowableError> {
        let greeting = context
            .fields
            .get("greeting")
            .cloned()
            .unwrap_or(Value::Null);
        context.execution.set_process_variable(
            "classDelegateFired".to_string(),
            json!(format!(
                "{}:{}",
                context.service_task_id,
                greeting.as_str().unwrap_or("")
            )),
        );
        Ok(json!({
            "delegate": "com.example.MyDelegate",
            "activityId": context.service_task_id,
            "fields": context.fields,
        }))
    }
}

fn engine_with_class_delegate() -> ProcessEngine {
    let mut registry = LocalServiceTaskDelegateRegistry::new();
    registry.register(MY_DELEGATE_CLASS, Arc::new(MyDelegate));

    let mut config = ProcessEngineConfiguration::default();
    config.service_task_delegate_registry = Some(registry);
    ProcessEngine::new_with_config("delegate-class-registry-test".to_string(), config)
}

#[test]
fn validator_allows_class_implementation_fqcn() {
    let model = BpmnXMLConverter::new().convert_to_bpmn_model(CLASS_DELEGATE_SERVICE_TASK_XML);
    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("class FQCN should be a valid registry-key implementation");
}

#[test]
fn class_implementation_resolves_via_local_delegate_registry() {
    let process_engine = engine_with_class_delegate();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let builder = repository_service
        .create_deployment()
        .name("Class Delegate Deployment".to_string())
        .add_string(
            "classDelegateProcess.bpmn20.xml".to_string(),
            CLASS_DELEGATE_SERVICE_TASK_XML.to_string(),
        );
    repository_service
        .deploy(builder)
        .expect("class delegate process should deploy");

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .expect("class delegate service task should execute via registry");

    let vars = runtime_service
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        vars.get("classDelegateFired"),
        Some(&json!("delegateTask1:hello-from-class")),
        "registered class delegate should write a process-variable side effect"
    );
    assert_eq!(
        vars.get("delegateResult")
            .and_then(|v| v.get("delegate"))
            .and_then(|v| v.as_str()),
        Some(MY_DELEGATE_CLASS),
        "resultVariableName should capture the delegate return value"
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "After Class Delegate");
}

#[test]
fn unregistered_class_delegate_fails_with_clear_error() {
    // Engine has no registry entries for the class name.
    let process_engine = ProcessEngine::new("delegate-class-missing".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("Missing Class Delegate Deployment".to_string())
        .add_string(
            "classDelegateProcess.bpmn20.xml".to_string(),
            CLASS_DELEGATE_SERVICE_TASK_XML.to_string(),
        );
    repository_service
        .deploy(builder)
        .expect("deployment should succeed; resolution is at runtime");

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let err = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .expect_err("unregistered class delegate should fail at execution");

    let msg = err.to_string();
    assert!(
        msg.contains("No local service task delegate 'com.example.MyDelegate'"),
        "expected clear unregistered-class-delegate error, got: {msg}"
    );
}
