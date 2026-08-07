//! P18-C contract test: a compensation handler executes against a SNAPSHOT of
//! the scope variables taken when the compensated activity completed.
//!
//! Java evidence: `ScopeUtil.createCopyOfSubProcessExecutionForCompensation`
//! copies the scope's (non-transient) local variables onto the compensation
//! event-scope execution at subscription-creation time;
//! `CompensationEventHandler` later runs the handler against that copy. Thus
//! variable writes performed AFTER the activity completed (but before the
//! throw) are invisible to the handler.

use flowable_engine::bpmn::behavior::service_task_activity_behavior::{
    LocalServiceTaskDelegate, LocalServiceTaskDelegateContext, LocalServiceTaskDelegateRegistry,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const RECORD_X_CLASS: &str = "com.example.p18.RecordXDelegate";

static SEEN_X: Mutex<Option<Value>> = Mutex::new(None);

/// Records the value of `x` visible to the compensation handler execution.
struct RecordXDelegate;

impl LocalServiceTaskDelegate for RecordXDelegate {
    fn execute(
        &self,
        context: &mut LocalServiceTaskDelegateContext<'_>,
    ) -> Result<Value, FlowableError> {
        *SEEN_X.lock().unwrap() = context.execution.process_variable("x");
        Ok(Value::Null)
    }
}

const SNAPSHOT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="compensationSnapshotP18" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="work" />
        <userTask id="work" name="Work" />
        <boundaryEvent id="workCompensation" attachedToRef="work">
            <compensateEventDefinition />
        </boundaryEvent>
        <serviceTask id="recordX"
                     name="Record X"
                     isForCompensation="true"
                     flowable:class="com.example.p18.RecordXDelegate" />
        <sequenceFlow id="flow2" sourceRef="work" targetRef="gate" />
        <userTask id="gate" name="Gate" />
        <sequenceFlow id="flow3" sourceRef="gate" targetRef="throwComp" />
        <intermediateThrowEvent id="throwComp">
            <compensateEventDefinition />
        </intermediateThrowEvent>
        <sequenceFlow id="flow4" sourceRef="throwComp" targetRef="after" />
        <userTask id="after" name="After" />
        <sequenceFlow id="flow5" sourceRef="after" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

fn engine_with_record_x_delegate() -> ProcessEngine {
    let mut registry = LocalServiceTaskDelegateRegistry::new();
    registry.register(RECORD_X_CLASS, Arc::new(RecordXDelegate));

    let mut config = ProcessEngineConfiguration::default();
    config.service_task_delegate_registry = Some(registry);
    ProcessEngine::new_with_config("p18-compensation-snapshot".to_string(), config)
}

fn complete_single_task(engine: &ProcessEngine, process_instance_id: &str, expected_key: &str) {
    let task_service = engine.get_task_service();
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one open task");
    assert_eq!(tasks[0].task_definition_key, expected_key);
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
}

/// x=1 while `work` completes (snapshot taken), x=2 before the throw — the
/// compensation handler must still see x=1 (Java snapshot-copy semantics),
/// while the live process variable stays 2.
#[test]
fn compensation_handler_sees_variable_snapshot_taken_at_activity_completion() {
    let engine = engine_with_record_x_delegate();
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Compensation Snapshot Deployment".to_string())
                .add_string(
                    "compensation_snapshot_p18.bpmn20.xml".to_string(),
                    SNAPSHOT_XML.to_string(),
                ),
        )
        .unwrap();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_key("compensationSnapshotP18".to_string())
                .variable("x".to_string(), json!(1)),
        )
        .unwrap();

    // `work` completes while x == 1 → the compensation subscription must
    // snapshot that state.
    complete_single_task(&engine, &process_instance.id, "work");

    // Mutate the variable AFTER the activity completed but BEFORE the throw.
    runtime_service
        .set_variable(process_instance.id.clone(), "x".to_string(), json!(2))
        .unwrap();

    *SEEN_X.lock().unwrap() = None;
    complete_single_task(&engine, &process_instance.id, "gate");

    assert_eq!(
        *SEEN_X.lock().unwrap(),
        Some(json!(1)),
        "compensation handler must see the variable snapshot taken when the \
         compensated activity completed, not the later value"
    );
    assert_eq!(
        runtime_service
            .get_variable(process_instance.id.clone(), "x".to_string())
            .unwrap(),
        Some(json!(2)),
        "the live process variable must keep the post-completion value"
    );

    let task_keys = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    assert_eq!(task_keys, vec!["after".to_string()]);
}
