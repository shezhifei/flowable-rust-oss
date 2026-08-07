//! P106 — listener trigger surface completion contract tests.
//!
//! Covers three of the four P106 surface gaps via end-to-end XML models:
//!   1. Sequence-flow execution listeners (`start`/`take`/`end`) fire in Java
//!      order on the take path (`ContinueProcessOperation.java:308-319`).
//!   2. Process-level execution listeners for `start`/`end`
//!      (`ContinueProcessOperation.java:96-98,105-111` and
//!      `EndExecutionOperation.java:126-131`).
//!   3. taskListener `event="allEvents"` matches every fired event
//!      (`ListenerNotificationHelper.java:122`).
//!
//! Item 4 (sequence-flow skipExpression) is covered by unit tests inside
//! `take_outgoing_sequence_flows_operation.rs`, because the Rust BPMN
//! converter does not (yet) parse the `flowable:skipExpression` attribute on
//! sequence flows (Java `SequenceFlowXMLConverter.java:46`).

use flowable_engine::bpmn::listener::{
    ExecutionListenerContext, LocalExecutionListener, LocalExecutionListenerRegistry,
    LocalTaskListener, LocalTaskListenerRegistry, TaskListenerContext,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::{Arc, Mutex};

/// Shared in-memory recorder shared by all listener instances registered under
/// one name. Entries are appended in invocation order.
#[derive(Clone)]
struct EventRecorder {
    events: Arc<Mutex<Vec<String>>>,
}

impl EventRecorder {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn record(&self, entry: String) {
        self.events.lock().unwrap().push(entry);
    }

    fn snapshot(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

struct RecordingExecutionListener {
    recorder: EventRecorder,
    /// Record `"<event>:<activityId>"` (seq-flow listeners) or just
    /// `"<event>"` (process-level listeners, whose activity id is not
    /// asserted).
    include_activity: bool,
}

impl LocalExecutionListener for RecordingExecutionListener {
    fn notify(&self, ctx: &mut ExecutionListenerContext<'_>) -> Result<(), FlowableError> {
        if self.include_activity {
            self.recorder.record(format!(
                "{}:{}",
                ctx.event,
                ctx.activity_id.unwrap_or("unknown")
            ));
        } else {
            self.recorder.record(ctx.event.to_string());
        }
        Ok(())
    }
}

struct RecordingTaskListener {
    recorder: EventRecorder,
}

impl LocalTaskListener for RecordingTaskListener {
    fn notify(&self, ctx: &mut TaskListenerContext<'_>) -> Result<(), FlowableError> {
        self.recorder.record(ctx.event.to_string());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 1. Sequence-flow execution listeners
// ---------------------------------------------------------------------------

const SEQ_FLOW_LISTENER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="seqFlowListenerProcess" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1">
            <extensionElements>
                <flowable:executionListener event="start" class="flowListener" />
                <flowable:executionListener event="take" class="flowListener" />
                <flowable:executionListener event="end" class="flowListener" />
            </extensionElements>
        </sequenceFlow>
        <userTask id="userTask1" name="Review" />
        <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1">
            <extensionElements>
                <flowable:executionListener event="start" class="flowListener" />
                <flowable:executionListener event="take" class="flowListener" />
                <flowable:executionListener event="end" class="flowListener" />
            </extensionElements>
        </sequenceFlow>
        <endEvent id="endEvent1" />
    </process>
</definitions>"#;

/// Java `ContinueProcessOperation.java:308-319`: a sequence flow fires its
/// execution listeners for `start`, `take` and `end` — all three, in that
/// order — while the execution is on the flow. The listener context sees the
/// flow's id as activity id.
#[test]
fn sequence_flow_execution_listeners_fire_start_take_end_in_order() {
    let recorder = EventRecorder::new();
    let mut registry = LocalExecutionListenerRegistry::new();
    registry.register(
        "flowListener",
        Arc::new(RecordingExecutionListener {
            recorder: recorder.clone(),
            include_activity: true,
        }),
    );
    let mut config = ProcessEngineConfiguration::default();
    config.execution_listener_registry = Some(registry);
    let engine = ProcessEngine::new_with_config("p106-seq-flow-listener".to_string(), config);

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let builder = repository_service
        .create_deployment()
        .name("Seq Flow Listener Deployment".to_string())
        .add_string("seqFlowListenerProcess.bpmn20.xml".to_string(), SEQ_FLOW_LISTENER_XML.to_string());
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    // Traversing flow1 (start -> userTask) fires start/take/end in order.
    assert_eq!(
        recorder.snapshot(),
        vec!["start:flow1", "take:flow1", "end:flow1"],
        "seq-flow listeners must fire start/take/end while taking flow1"
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // Traversing flow2 (userTask -> end) appends the same three, in order.
    assert_eq!(
        recorder.snapshot(),
        vec![
            "start:flow1",
            "take:flow1",
            "end:flow1",
            "start:flow2",
            "take:flow2",
            "end:flow2",
        ],
        "seq-flow listeners fire for every taken flow, start/take/end in order"
    );
}

// ---------------------------------------------------------------------------
// 2. Process-level execution listeners
// ---------------------------------------------------------------------------

const PROCESS_LISTENER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="processListenerProcess" isExecutable="true">
        <extensionElements>
            <flowable:executionListener event="start" class="procListener" />
            <flowable:executionListener event="end" class="procListener" />
        </extensionElements>
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
        <userTask id="userTask1" name="Review" />
        <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
        <endEvent id="endEvent1" />
    </process>
</definitions>"#;

/// Java `ContinueProcessOperation.java:96-98,105-111` fires process-level
/// `start` listeners when the initial flow element is entered, and
/// `EndExecutionOperation.java:126-131` fires process-level `end` listeners
/// when the process instance completes.
#[test]
fn process_execution_listeners_fire_on_start_and_end() {
    let recorder = EventRecorder::new();
    let mut registry = LocalExecutionListenerRegistry::new();
    registry.register(
        "procListener",
        Arc::new(RecordingExecutionListener {
            recorder: recorder.clone(),
            include_activity: false,
        }),
    );
    let mut config = ProcessEngineConfiguration::default();
    config.execution_listener_registry = Some(registry);
    let engine = ProcessEngine::new_with_config("p106-process-listener".to_string(), config);

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let builder = repository_service
        .create_deployment()
        .name("Process Listener Deployment".to_string())
        .add_string("processListenerProcess.bpmn20.xml".to_string(), PROCESS_LISTENER_XML.to_string());
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    assert_eq!(
        recorder.snapshot(),
        vec!["start"],
        "process-level start executionListener must fire when the process starts"
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    assert_eq!(
        recorder.snapshot(),
        vec!["start", "end"],
        "process-level end executionListener must fire when the process completes"
    );
}

// ---------------------------------------------------------------------------
// 3. taskListener event="allEvents"
// ---------------------------------------------------------------------------

const ALL_EVENTS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="allEventsProcess" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
        <userTask id="userTask1" name="Review">
            <extensionElements>
                <flowable:taskListener event="allEvents" class="allEventsListener" />
            </extensionElements>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
        <endEvent id="endEvent1" />
    </process>
</definitions>"#;

/// Java `ListenerNotificationHelper.java:122`: a task listener configured with
/// `event="allEvents"` matches every fired event. Before P106 the Rust matcher
/// compared only by exact event name, so `allEvents` never fired.
#[test]
fn all_events_task_listener_fires_on_create_and_complete() {
    let recorder = EventRecorder::new();
    let mut registry = LocalTaskListenerRegistry::new();
    registry.register(
        "allEventsListener",
        Arc::new(RecordingTaskListener {
            recorder: recorder.clone(),
        }),
    );
    let mut config = ProcessEngineConfiguration::default();
    config.task_listener_registry = Some(registry);
    let engine = ProcessEngine::new_with_config("p106-all-events".to_string(), config);

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let builder = repository_service
        .create_deployment()
        .name("All Events Deployment".to_string())
        .add_string("allEventsProcess.bpmn20.xml".to_string(), ALL_EVENTS_XML.to_string());
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    // The allEvents listener fires for `create` as soon as the task is born.
    assert_eq!(
        recorder.snapshot(),
        vec!["create"],
        "allEvents taskListener must fire on create"
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    assert_eq!(
        recorder.snapshot(),
        vec!["create", "complete"],
        "allEvents taskListener must fire on complete too"
    );
}
