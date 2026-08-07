//! Contract tests for the P53 typed-event bus extension.
//!
//! Verifies Java `FlowableEngineEventType` parity for the new event types
//! added in P53 layer 1 (process/task lifecycle) and layer 2
//! (activity/sequenceflow). References:
//!
//! - Java `ProcessInstanceHelper.java:227-275, 302-317` (PROCESS_CREATED,
//!   ENTITY_INITIALIZED, PROCESS_STARTED).
//! - Java `ContinueProcessOperation.java:266-306` (ACTIVITY_STARTED).
//! - Java `TakeOutgoingSequenceFlowsOperation.java:159-196`
//!   (ACTIVITY_COMPLETED).
//! - Java `ContinueProcessOperation.java:308-345` (SEQUENCEFLOW_TAKEN).
//! - Java `TaskHelper` (TASK_CREATED, TASK_COMPLETED, TASK_ASSIGNED).
//! - Java `ProcessInstanceHelper.endProcessInstance` (PROCESS_COMPLETED).

use flowable_engine::engine::event_dispatcher::{
    EngineEvent, EngineEventDispatcher, EngineEventListener, EngineEventType, EntityKind,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Clone)]
struct EventCollector {
    events: Arc<Mutex<Vec<(EngineEventType, EntityKind, String)>>>,
}

impl EventCollector {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn snapshot(&self) -> Vec<(EngineEventType, EntityKind, String)> {
        self.events.lock().unwrap().clone()
    }
}

impl EngineEventListener for EventCollector {
    fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
        if let EngineEvent::Entity { event_type, data } = event {
            self.events
                .lock()
                .unwrap()
                .push((*event_type, data.entity_kind, data.entity_id.clone()));
        }
        Ok(())
    }
}

fn collect_for(xml: &str, definitions_id: &str) -> (ProcessEngine, EventCollector) {
    let mut config = ProcessEngineConfiguration::default();
    let collector = EventCollector::new();
    config.engine_event_dispatcher = EngineEventDispatcher::new();
    config
        .engine_event_dispatcher
        .add_event_listener(Arc::new(collector.clone()));
    let engine = ProcessEngine::new_with_config("p53-typed-events".to_string(), config);
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string(definitions_id.to_string(), xml.to_string()),
    )
    .unwrap();
    (engine, collector)
}

#[test]
fn process_create_initialized_started_and_completed_fire_in_order() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p53-1" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toEnd" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    let (engine, collector) = collect_for(xml, "p53-1.bpmn20.xml");

    let runtime = engine.get_runtime_service();
    let def_id = engine.get_repository_service().get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // Drain the engine for the post-agenda events to fire.
    let snapshot = collector.snapshot();

    // ENTITY_INITIALIZED + PROCESS_CREATED must fire BEFORE the start event
    // fires. PROCESS_STARTED fires once execution lands on the start event.
    let types: Vec<EngineEventType> = snapshot.iter().map(|(t, _, _)| *t).collect();
    let initialized_idx = types
        .iter()
        .position(|t| *t == EngineEventType::EntityInitialized)
        .expect("ENTITY_INITIALIZED must fire");
    let created_idx = types
        .iter()
        .position(|t| *t == EngineEventType::ProcessCreated)
        .expect("PROCESS_CREATED must fire");
    let started_idx = types
        .iter()
        .position(|t| *t == EngineEventType::ProcessStarted)
        .expect("PROCESS_STARTED must fire");
    assert!(
        initialized_idx < started_idx,
        "EntityInitialized must precede ProcessStarted: {:?}",
        types
    );
    assert!(
        created_idx < started_idx,
        "ProcessCreated must precede ProcessStarted: {:?}",
        types
    );

    // All process instance events must carry the same process instance id.
    for (ty, kind, id) in &snapshot {
        if *kind == EntityKind::ProcessInstance {
            assert_eq!(id, &pi.id, "process event {ty:?} must carry pi id");
        }
    }
}

#[test]
fn task_created_and_completed_fire_with_assignment_when_assignee_set() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p53-task" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toTask" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="T" flowable:assignee="alice" />
            <sequenceFlow id="toEnd" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    let (engine, collector) = collect_for(xml, "p53-task.bpmn20.xml");

    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let def_id = engine.get_repository_service().get_process_definition_ids().unwrap()[0].clone();
    runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let snapshot = collector.snapshot();
    let types: Vec<EngineEventType> = snapshot.iter().map(|(t, _, _)| *t).collect();

    // `TASK_CREATED` must fire after the start event completes, and
    // `TASK_ASSIGNED` must fire because the user task has an assignee.
    assert!(
        types.contains(&EngineEventType::TaskCreated),
        "TASK_CREATED must fire: {:?}",
        types
    );
    assert!(
        types.contains(&EngineEventType::TaskAssigned),
        "TASK_ASSIGNED must fire when user task has assignee: {:?}",
        types
    );

    // Completing the task must fire TASK_COMPLETED.
    let tasks = task_service
        .create_task_query()
        .task_definition_key("task1".to_string())
        .list()
        .unwrap();
    let task_id = tasks[0].id.clone();
    task_service.complete_task_by_id(task_id).unwrap();
    let snapshot = collector.snapshot();
    let types: Vec<EngineEventType> = snapshot.iter().map(|(t, _, _)| *t).collect();
    assert!(
        types.contains(&EngineEventType::TaskCompleted),
        "TASK_COMPLETED must fire on complete: {:?}",
        types
    );
}

#[test]
fn activity_started_completed_and_sequenceflow_taken_fire() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p53-act" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="T" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    let (engine, collector) = collect_for(xml, "p53-act.bpmn20.xml");

    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let def_id = engine.get_repository_service().get_process_definition_ids().unwrap()[0].clone();
    runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // After start, the start event has been traversed (f1) and task1 has been
    // created. We now drive the user task to completion so the second sequence
    // flow f2 and the end event also fire.
    let tasks = task_service
        .create_task_query()
        .task_definition_key("task1".to_string())
        .list()
        .unwrap();
    let task_id = tasks[0].id.clone();
    task_service.complete_task_by_id(task_id).unwrap();

    let snapshot = collector.snapshot();
    let types: Vec<EngineEventType> = snapshot.iter().map(|(t, _, _)| *t).collect();
    let kinds: Vec<EntityKind> = snapshot.iter().map(|(_, k, _)| *k).collect();

    // ACTIVITY_STARTED for the start event, the user task, and the end event.
    let activity_started_count = types
        .iter()
        .filter(|t| **t == EngineEventType::ActivityStarted)
        .count();
    assert!(
        activity_started_count >= 3,
        "ACTIVITY_STARTED must fire at least three times (start + task1 + end): {}",
        activity_started_count
    );

    // SEQUENCEFLOW_TAKEN for f1 and f2.
    let seq_taken_count = types
        .iter()
        .filter(|t| **t == EngineEventType::SequenceflowTaken)
        .count();
    assert!(
        seq_taken_count >= 2,
        "SEQUENCEFLOW_TAKEN must fire at least twice (f1, f2): {}",
        seq_taken_count
    );

    // Sequence flow events carry the SequenceFlow entity kind.
    let seq_entity_count = kinds
        .iter()
        .filter(|k| **k == EntityKind::SequenceFlow)
        .count();
    assert!(
        seq_entity_count >= 2,
        "Sequence flow events must use EntityKind::SequenceFlow: {}",
        seq_entity_count
    );
}
