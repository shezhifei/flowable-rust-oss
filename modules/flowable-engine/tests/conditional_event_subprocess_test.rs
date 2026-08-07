//! P11: Conditional event subprocess repeat semantics — Java parity probes.
//!
//! Java reference: `ConditionalEventSubprocessTest` /
//! `EvaluateConditionalEventsOperation.evaluateEventSubProcesses`

use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;
use std::collections::HashMap;

/// Interrupting conditional event subprocess: `isInterrupting="true"`.
/// Java: `testSimpleInterruptingEventSubProcess` / `testInterruptingSubProcess`.
const INTERRUPTING_CONDITIONAL_ES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="interruptingConditionalES" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="task" />
    <userTask id="task" name="Main Task" />
    <sequenceFlow id="flow2" sourceRef="task" targetRef="end" />
    <endEvent id="end" />
    <subProcess id="conditionalEventSubProcess" triggeredByEvent="true">
      <startEvent id="esConditionalStart" isInterrupting="true">
        <conditionalEventDefinition>
          <condition>${myVar == 'test'}</condition>
        </conditionalEventDefinition>
      </startEvent>
      <sequenceFlow id="esFlow1" sourceRef="esConditionalStart" targetRef="esTask" />
      <userTask id="esTask" name="ES Task" />
      <sequenceFlow id="esFlow2" sourceRef="esTask" targetRef="esEnd" />
      <endEvent id="esEnd" />
    </subProcess>
  </process>
</definitions>"#;

/// Non-interrupting conditional event subprocess: `isInterrupting="false"`.
/// Java: `testNonInterruptingSubProcess`.
const NON_INTERRUPTING_CONDITIONAL_ES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="nonInterruptingConditionalES" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="task" />
    <userTask id="task" name="Main Task" />
    <sequenceFlow id="flow2" sourceRef="task" targetRef="end" />
    <endEvent id="end" />
    <subProcess id="conditionalEventSubProcess" triggeredByEvent="true">
      <startEvent id="esConditionalStart" isInterrupting="false">
        <conditionalEventDefinition>
          <condition>${myVar == 'test'}</condition>
        </conditionalEventDefinition>
      </startEvent>
      <sequenceFlow id="esFlow1" sourceRef="esConditionalStart" targetRef="esTask" />
      <userTask id="esTask" name="ES Task" />
      <sequenceFlow id="esFlow2" sourceRef="esTask" targetRef="esEnd" />
      <endEvent id="esEnd" />
    </subProcess>
  </process>
</definitions>"#;

fn deploy_and_start(engine: &ProcessEngine, xml: &str, resource: &str) -> String {
    let repository_service = engine.get_repository_service();
    let builder = repository_service
        .create_deployment()
        .name(resource.to_string())
        .add_string(format!("{resource}.bpmn20.xml"), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap()
        .id
}

/// Java parity: non-interrupting conditional event subprocess fires when the
/// condition becomes true, leaving the main flow untouched.
#[test]
fn non_interrupting_conditional_event_subprocess_fires() {
    let engine = ProcessEngine::new("p11-ni-es-fire".to_string());
    let pi_id = deploy_and_start(
        &engine,
        NON_INTERRUPTING_CONDITIONAL_ES_XML,
        "ni_conditional_es",
    );

    let task_service = engine.get_task_service();
    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "main task should be present");
    assert_eq!(tasks[0].task_definition_key, "task");

    // Evaluate conditional events: condition is true, event subprocess should fire.
    let mut vars = HashMap::new();
    vars.insert("myVar".to_string(), json!("test"));
    engine
        .get_runtime_service()
        .evaluate_conditional_events(pi_id.clone(), vars)
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        2,
        "main task + event subprocess task should be present"
    );
    let keys: Vec<_> = tasks.iter().map(|t| t.task_definition_key.clone()).collect();
    assert!(keys.contains(&"task".to_string()));
    assert!(keys.contains(&"esTask".to_string()));
}

/// Java parity: non-interrupting conditional event subprocess is repeatable
/// — each call to evaluateConditionalEvents with a true condition creates
/// another subprocess instance.
#[test]
fn non_interrupting_conditional_event_subprocess_is_repeatable() {
    let engine = ProcessEngine::new("p11-ni-es-repeat".to_string());
    let pi_id = deploy_and_start(
        &engine,
        NON_INTERRUPTING_CONDITIONAL_ES_XML,
        "ni_conditional_es_repeat",
    );

    let task_service = engine.get_task_service();

    let mut vars = HashMap::new();
    vars.insert("myVar".to_string(), json!("test"));

    // First call: 1 main + 1 es = 2 tasks
    engine
        .get_runtime_service()
        .evaluate_conditional_events(pi_id.clone(), vars.clone())
        .unwrap();
    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);

    // Second call: 1 main + 2 es = 3 tasks
    engine
        .get_runtime_service()
        .evaluate_conditional_events(pi_id.clone(), vars.clone())
        .unwrap();
    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        3,
        "Java parity: second evaluateConditionalEvents with same true condition must create another ES instance"
    );

    // Third call with different variable: condition is false, no new instance
    let mut vars2 = HashMap::new();
    vars2.insert("myVar".to_string(), json!("test2"));
    engine
        .get_runtime_service()
        .evaluate_conditional_events(pi_id.clone(), vars2)
        .unwrap();
    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        3,
        "Java parity: false condition must not create a new ES instance"
    );
}

/// Java parity: interrupting conditional event subprocess fires and cancels
/// the main flow's activities.
#[test]
fn interrupting_conditional_event_subprocess_fires_and_cancels_main_flow() {
    let engine = ProcessEngine::new("p11-i-es-fire".to_string());
    let pi_id = deploy_and_start(
        &engine,
        INTERRUPTING_CONDITIONAL_ES_XML,
        "i_conditional_es",
    );

    let task_service = engine.get_task_service();
    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "main task should be present");
    assert_eq!(tasks[0].task_definition_key, "task");

    // Evaluate conditional events: condition is true, event subprocess should fire
    // and cancel the main task.
    let mut vars = HashMap::new();
    vars.insert("myVar".to_string(), json!("test"));
    engine
        .get_runtime_service()
        .evaluate_conditional_events(pi_id.clone(), vars)
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "only event subprocess task should remain; main task was cancelled"
    );
    assert_eq!(tasks[0].task_definition_key, "esTask");
}