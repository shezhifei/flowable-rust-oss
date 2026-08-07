//! P24 sub-item 3: variable-listener event subprocess wiring.
//! Java parity: VariableListenerEventSubprocessTest.testInterruptingSubProcess.

use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};
use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

const INTERRUPTING_VL_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="process" isExecutable="true">
    <startEvent id="theStart" />
    <sequenceFlow sourceRef="theStart" targetRef="task" />
    <userTask id="task" name="Task" />
    <sequenceFlow sourceRef="task" targetRef="theEnd" />
    <endEvent id="theEnd" />
    <subProcess triggeredByEvent="true" id="eventSubProcess">
      <startEvent id="eventProcessStart" isInterrupting="true">
        <extensionElements>
          <flowable:variableListenerEventDefinition variableName="var1" />
        </extensionElements>
      </startEvent>
      <sequenceFlow sourceRef="eventProcessStart" targetRef="eventSubProcessTask" />
      <userTask id="eventSubProcessTask" name="ESP Task" />
      <sequenceFlow sourceRef="eventSubProcessTask" targetRef="eventSubProcessEnd" />
      <endEvent id="eventSubProcessEnd" />
    </subProcess>
  </process>
</definitions>"#;

fn has_variable_listener(elements: &[FlowElementEnum]) -> bool {
    for fe in elements {
        match fe {
            FlowElementEnum::StartEvent(se) => {
                for def in &se.event.event_definitions {
                    if let EventDefinitionEnum::VariableListenerEventDefinition(vl) = def {
                        if vl.variable_name.as_deref() == Some("var1") {
                            return true;
                        }
                    }
                }
            }
            FlowElementEnum::SubProcess(s) => {
                if has_variable_listener(&s.flow_elements) {
                    return true;
                }
            }
            FlowElementEnum::EventSubProcess(e) => {
                if has_variable_listener(&e.sub_process.flow_elements) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[test]
fn test_variable_listener_definition_parsed() {
    let model = BpmnXMLConverter::new().convert_to_bpmn_model(INTERRUPTING_VL_XML);
    let process = model.main_process.as_ref().unwrap();
    assert!(
        has_variable_listener(&process.flow_elements),
        "variableListenerEventDefinition was not parsed onto start event"
    );
}

#[test]
fn test_interrupting_variable_listener_event_subprocess() {
    let engine = ProcessEngine::new("default".to_string());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_svc = engine.get_task_service();

    repo.deploy(
        repo.create_deployment().add_string(
            "vl-esp.bpmn20.xml".to_string(),
            INTERRUPTING_VL_XML.to_string(),
        ),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime.start_process_instance_by_id(def_id, None).unwrap();

    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key.as_str(), "task");

    // Wrong variable name must not trigger.
    runtime
        .set_variable(pi.id.clone(), "var2".to_string(), json!("x"))
        .unwrap();
    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key.as_str(), "task");

    // Matching variable triggers interrupting event subprocess.
    runtime
        .set_variable(pi.id.clone(), "var1".to_string(), json!("test"))
        .unwrap();

    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "expected only event-subprocess task after interrupt, got {:?}",
        tasks
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(tasks[0].task_definition_key.as_str(), "eventSubProcessTask");

    task_svc.complete_task_by_id(tasks[0].id.clone()).unwrap();
    let remaining = task_svc
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert!(remaining.is_empty());
}
