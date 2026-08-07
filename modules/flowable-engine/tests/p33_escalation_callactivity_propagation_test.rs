//! P33: escalation propagation across call activities.
//!
//! Java references:
//! - `EscalationPropagation.java:63-81` collects parent process definitions by
//!   following `superExecution` across called process instances.
//! - `EscalationPropagation.java:146-178` records each crossed process
//!   instance and dispatches `PROCESS_COMPLETED_WITH_ESCALATION_END_EVENT`
//!   before executing the parent catcher.
//! - `IntermediateThrowEscalationEventActivityBehavior.java:50-53` propagates
//!   the escalation and then takes the throw event's normal outgoing flow.

use flowable_engine::engine::event_dispatcher::{
    EngineEvent, EngineEventDispatcher, EngineEventListener, EngineEventType,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::{Arc, Mutex};

const ESCALATION_END_CHILD_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
  <process id="escalationEndChild" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="escalationEnd" />
    <endEvent id="escalationEnd">
      <escalationEventDefinition escalationRef="approvalEscalation" />
    </endEvent>
  </process>
</definitions>
"#;

const ESCALATION_THROW_CHILD_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
  <process id="escalationThrowChild" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="throwEscalation" />
    <intermediateThrowEvent id="throwEscalation">
      <escalationEventDefinition escalationRef="approvalEscalation" />
    </intermediateThrowEvent>
    <sequenceFlow id="f2" sourceRef="throwEscalation" targetRef="childAfterThrow" />
    <userTask id="childAfterThrow" name="ChildAfterThrow" />
    <sequenceFlow id="f3" sourceRef="childAfterThrow" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>
"#;

fn parent_xml(process_id: &str, called_element: &str, cancel_activity: bool) -> String {
    format!(
        r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
  <process id="{process_id}" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="callChild" />
    <callActivity id="callChild" calledElement="{called_element}" />
    <boundaryEvent id="catchEscalation" attachedToRef="callChild" cancelActivity="{cancel_activity}">
      <escalationEventDefinition escalationRef="approvalEscalation" />
    </boundaryEvent>
    <sequenceFlow id="catchFlow" sourceRef="catchEscalation" targetRef="parentEscalationTask" />
    <userTask id="parentEscalationTask" name="ParentEscalationTask" />
    <sequenceFlow id="catchEndFlow" sourceRef="parentEscalationTask" targetRef="end" />
    <sequenceFlow id="normalFlow" sourceRef="callChild" targetRef="parentNormalTask" />
    <userTask id="parentNormalTask" name="ParentNormalTask" />
    <sequenceFlow id="normalEndFlow" sourceRef="parentNormalTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>
"#
    )
}

const MIDDLE_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <process id="escalationMiddle" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="callLeaf" />
    <callActivity id="callLeaf" calledElement="escalationEndChild" />
    <sequenceFlow id="f2" sourceRef="callLeaf" targetRef="middleNormalTask" />
    <userTask id="middleNormalTask" name="MiddleNormalTask" />
    <sequenceFlow id="f3" sourceRef="middleNormalTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>
"#;

#[derive(Default)]
struct EscalationCompletionRecorder {
    process_instance_ids: Arc<Mutex<Vec<String>>>,
}

impl EngineEventListener for EscalationCompletionRecorder {
    fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
        if let EngineEvent::Entity { data, .. } = event {
            self.process_instance_ids
                .lock()
                .unwrap()
                .push(data.entity_id.clone());
        }
        Ok(())
    }
}

fn engine_with_completion_recorder(name: &str) -> (ProcessEngine, Arc<Mutex<Vec<String>>>) {
    let process_instance_ids = Arc::new(Mutex::new(Vec::new()));
    let mut dispatcher = EngineEventDispatcher::new();
    dispatcher.add_typed_event_listener(
        EngineEventType::ProcessCompletedWithEscalationEndEvent,
        Arc::new(EscalationCompletionRecorder {
            process_instance_ids: Arc::clone(&process_instance_ids),
        }),
    );
    let mut config = ProcessEngineConfiguration::default();
    config.engine_event_dispatcher = dispatcher;
    (
        ProcessEngine::new_with_config(name.to_string(), config),
        process_instance_ids,
    )
}

fn deploy(engine: &ProcessEngine, name: &str, resources: &[(&str, &str)]) {
    let mut builder = engine
        .get_repository_service()
        .create_deployment()
        .name(name.to_string());
    for (file_name, xml) in resources {
        builder = builder.add_string(file_name.to_string(), xml.to_string());
    }
    engine
        .get_repository_service()
        .deploy(builder)
        .expect("deployment should succeed");
}

fn start_by_key(engine: &ProcessEngine, key: &str) -> String {
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.starts_with(key))
        .unwrap_or_else(|| panic!("process definition for key '{key}'"));
    let runtime_service = engine.get_runtime_service();
    runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(definition_id),
        )
        .unwrap()
        .id
}

fn task_keys(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect()
}

fn process_instances(engine: &ProcessEngine) -> Vec<(String, bool, Option<String>)> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_process_instances(&mut session)
        .into_values()
        .map(|instance| (instance.id, instance.is_ended, instance.super_execution_id))
        .collect()
}

fn runtime_execution_count(engine: &ProcessEngine, process_instance_id: &str) -> usize {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|execution| execution.process_instance_id.as_deref() == Some(process_instance_id))
        .count()
}

#[test]
fn p33_escalation_end_crosses_call_activity_and_completes_child_with_event() {
    let (engine, completed_ids) = engine_with_completion_recorder("p33-end-single");
    let parent = parent_xml("escalationEndParent", "escalationEndChild", true);
    deploy(
        &engine,
        "p33-end-single",
        &[
            ("child.bpmn20.xml", ESCALATION_END_CHILD_XML),
            ("parent.bpmn20.xml", &parent),
        ],
    );

    let parent_id = start_by_key(&engine, "escalationEndParent");
    assert_eq!(task_keys(&engine, &parent_id), vec!["parentEscalationTask"]);

    let instances = process_instances(&engine);
    let child = instances
        .iter()
        .find(|(id, _, super_id)| id != &parent_id && super_id.is_some())
        .expect("call activity child process instance");
    assert!(child.1, "interrupting parent catcher must end child PI");
    assert_eq!(
        runtime_execution_count(&engine, &child.0),
        0,
        "completed escalation child must not retain runtime executions"
    );
    assert_eq!(completed_ids.lock().unwrap().as_slice(), &[child.0.clone()]);
}

#[test]
fn p33_intermediate_throw_uses_parent_call_activity_catcher() {
    let (engine, completed_ids) = engine_with_completion_recorder("p33-throw-single");
    let parent = parent_xml("escalationThrowParent", "escalationThrowChild", true);
    deploy(
        &engine,
        "p33-throw-single",
        &[
            ("child.bpmn20.xml", ESCALATION_THROW_CHILD_XML),
            ("parent.bpmn20.xml", &parent),
        ],
    );

    let parent_id = start_by_key(&engine, "escalationThrowParent");
    assert_eq!(task_keys(&engine, &parent_id), vec!["parentEscalationTask"]);

    let instances = process_instances(&engine);
    let child = instances
        .iter()
        .find(|(id, _, super_id)| id != &parent_id && super_id.is_some())
        .expect("call activity child process instance");
    assert!(child.1, "interrupting parent catcher must end child PI");
    assert!(
        task_keys(&engine, &child.0).is_empty(),
        "stale child throw token must not take its outgoing flow"
    );
    assert_eq!(completed_ids.lock().unwrap().as_slice(), &[child.0.clone()]);
}

#[test]
fn p33_escalation_walks_two_call_activity_levels_and_cleans_crossed_instances() {
    let (engine, completed_ids) = engine_with_completion_recorder("p33-two-level");
    let outer = parent_xml("escalationOuter", "escalationMiddle", true);
    deploy(
        &engine,
        "p33-two-level",
        &[
            ("leaf.bpmn20.xml", ESCALATION_END_CHILD_XML),
            ("middle.bpmn20.xml", MIDDLE_XML),
            ("outer.bpmn20.xml", &outer),
        ],
    );

    let outer_id = start_by_key(&engine, "escalationOuter");
    assert_eq!(task_keys(&engine, &outer_id), vec!["parentEscalationTask"]);

    let instances = process_instances(&engine);
    let crossed_ids: Vec<String> = instances
        .iter()
        .filter(|(id, _, super_id)| id != &outer_id && super_id.is_some())
        .map(|(id, ended, _)| {
            assert!(*ended, "crossed child PI {id} must be ended");
            assert_eq!(runtime_execution_count(&engine, id), 0);
            id.clone()
        })
        .collect();
    assert_eq!(crossed_ids.len(), 2, "leaf + middle must be crossed");

    let mut recorded = completed_ids.lock().unwrap().clone();
    let mut expected = crossed_ids;
    recorded.sort();
    expected.sort();
    assert_eq!(recorded, expected);
}

#[test]
fn p33_non_interrupting_parent_catcher_preserves_child_outgoing_flow() {
    let (engine, _) = engine_with_completion_recorder("p33-non-interrupting");
    let parent = parent_xml(
        "nonInterruptingEscalationParent",
        "escalationThrowChild",
        false,
    );
    deploy(
        &engine,
        "p33-non-interrupting",
        &[
            ("child.bpmn20.xml", ESCALATION_THROW_CHILD_XML),
            ("parent.bpmn20.xml", &parent),
        ],
    );

    let parent_id = start_by_key(&engine, "nonInterruptingEscalationParent");
    assert_eq!(task_keys(&engine, &parent_id), vec!["parentEscalationTask"]);

    let instances = process_instances(&engine);
    let child = instances
        .iter()
        .find(|(id, _, super_id)| id != &parent_id && super_id.is_some())
        .expect("call activity child process instance");
    assert!(
        !child.1,
        "non-interrupting catcher must keep child PI active"
    );
    assert_eq!(task_keys(&engine, &child.0), vec!["childAfterThrow"]);
}
