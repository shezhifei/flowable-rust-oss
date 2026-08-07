//! P92: Event Registry BPMN wait-state registration + typed trigger bridge.
//!
//! Covers four wait-state shapes that register `EventSubscriptionKind::EventRegistry`
//! from `flowable:eventType` and resume via the same commands the BPMN consumer uses:
//! intermediate catch, receive task, boundary, event subprocess (+ process start).
//!
//! Java: BpmnEventRegistryEventConsumer / EventSubscriptionManager /
//! BoundaryEventRegistryEventActivityBehavior / IntermediateCatchEventRegistry /
//! ReceiveEventTaskActivityBehavior / ProcessInstanceHelper event-registry start.

use flowable_engine::cmd::trigger_start_event_subscription_cmd::TriggerProcessStartByEventCmd;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;

const FLOWABLE_NS: &str = "http://flowable.org/bpmn";

fn deploy_xml(engine: &ProcessEngine, name: &str, xml: &str) {
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name(name.to_string())
                .add_string(format!("{name}.bpmn20.xml"), xml.to_string()),
        )
        .unwrap();
}

#[test]
fn event_registry_intermediate_catch_registers_and_triggers() {
    let engine = ProcessEngine::new("p92-er-catch".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="{FLOWABLE_NS}"
                 targetNamespace="Examples">
        <process id="erCatchProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="waitEvent" />
            <intermediateCatchEvent id="waitEvent">
                <extensionElements>
                    <flowable:eventType>orderReceived</flowable:eventType>
                </extensionElements>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="waitEvent" targetRef="afterTask" />
            <userTask id="afterTask" name="After" />
            <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );
    deploy_xml(&engine, "er-catch", &xml);

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let wait_state = runtime
        .get_event_wait_states_by_process_instance_id(process_instance.id.clone())
        .into_iter()
        .find(|state| state.activity_id.as_deref() == Some("waitEvent"))
        .expect("event-registry intermediate catch should be waiting");
    assert_eq!(wait_state.event_ref.as_deref(), Some("orderReceived"));

    runtime.trigger_event_intermediate_catch(
        EventSubscriptionKind::EventRegistry,
        "orderReceived".to_string(),
        wait_state.execution_id.clone(),
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterTask");
}

#[test]
fn event_registry_receive_task_registers_and_triggers() {
    let engine = ProcessEngine::new("p92-er-receive".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="{FLOWABLE_NS}"
                 targetNamespace="Examples">
        <process id="erReceiveProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="receiveOrder" />
            <receiveTask id="receiveOrder" name="Receive Order">
                <extensionElements>
                    <flowable:eventType>orderReceived</flowable:eventType>
                </extensionElements>
            </receiveTask>
            <sequenceFlow id="flow2" sourceRef="receiveOrder" targetRef="afterTask" />
            <userTask id="afterTask" name="After" />
            <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );
    deploy_xml(&engine, "er-receive", &xml);

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let wait_state = runtime
        .get_event_wait_states_by_process_instance_id(process_instance.id.clone())
        .into_iter()
        .find(|state| state.activity_id.as_deref() == Some("receiveOrder"))
        .expect("event-registry receive task should be waiting");
    assert_eq!(wait_state.event_ref.as_deref(), Some("orderReceived"));
    // Java ReceiveEventTaskActivityBehavior does not create a user Task.
    assert!(wait_state.task_id.is_none());
    assert!(
        task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .is_empty()
    );

    runtime.trigger_event_intermediate_catch(
        EventSubscriptionKind::EventRegistry,
        "orderReceived".to_string(),
        wait_state.execution_id.clone(),
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterTask");
}

#[test]
fn event_registry_boundary_registers_and_triggers() {
    let engine = ProcessEngine::new("p92-er-boundary".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="{FLOWABLE_NS}"
                 targetNamespace="Examples">
        <process id="erBoundaryProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="mainTask" />
            <userTask id="mainTask" name="Main" />
            <boundaryEvent id="orderBoundary" attachedToRef="mainTask" cancelActivity="true">
                <extensionElements>
                    <flowable:eventType>orderReceived</flowable:eventType>
                </extensionElements>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="mainTask" targetRef="mainEnd" />
            <sequenceFlow id="flow3" sourceRef="orderBoundary" targetRef="boundaryTask" />
            <userTask id="boundaryTask" name="Boundary" />
            <sequenceFlow id="flow4" sourceRef="boundaryTask" targetRef="boundaryEnd" />
            <endEvent id="mainEnd" />
            <endEvent id="boundaryEnd" />
        </process>
    </definitions>"#
    );
    deploy_xml(&engine, "er-boundary", &xml);

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 1);
    assert_eq!(tasks_before[0].task_definition_key, "mainTask");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let boundaries = store.find_boundary_event_states_by_process_instance_id(
        &process_instance.id,
        &mut session,
    );
    assert!(
        boundaries.iter().any(|b| {
            b.boundary_event_id == "orderBoundary"
                && b.event_subscription.kind == EventSubscriptionKind::EventRegistry
                && b.event_subscription.event_ref == "orderReceived"
        }),
        "event-registry boundary subscription should be registered"
    );
    drop(session);

    runtime.trigger_boundary_event_by_event_ref(
        EventSubscriptionKind::EventRegistry,
        "orderReceived".to_string(),
        process_instance.id.clone(),
    );

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 1);
    assert_eq!(tasks_after[0].task_definition_key, "boundaryTask");
}

#[test]
fn event_registry_event_subprocess_registers_and_triggers() {
    let engine = ProcessEngine::new("p92-er-esp".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="{FLOWABLE_NS}"
                 targetNamespace="Examples">
        <process id="erEventSubprocessProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="mainTask" />
            <userTask id="mainTask" name="Main" />
            <sequenceFlow id="flow2" sourceRef="mainTask" targetRef="mainEnd" />
            <endEvent id="mainEnd" />
            <subProcess id="orderEventSubProcess" triggeredByEvent="true">
                <startEvent id="orderEventStart" isInterrupting="false">
                    <extensionElements>
                        <flowable:eventType>orderReceived</flowable:eventType>
                    </extensionElements>
                </startEvent>
                <sequenceFlow id="eventFlow1" sourceRef="orderEventStart" targetRef="eventTask" />
                <userTask id="eventTask" name="Event Task" />
                <sequenceFlow id="eventFlow2" sourceRef="eventTask" targetRef="eventEnd" />
                <endEvent id="eventEnd" />
            </subProcess>
        </process>
    </definitions>"#
    );
    deploy_xml(&engine, "er-esp", &xml);

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let subscriptions = store.find_event_subprocess_event_subscriptions_by_process_instance_id(
        &process_instance.id,
        &mut session,
    );
    assert!(
        subscriptions.iter().any(|s| {
            s.event_kind == EventSubscriptionKind::EventRegistry
                && s.event_ref == "orderReceived"
                && !s.interrupting
        }),
        "event-registry event-subprocess subscription should be registered"
    );
    drop(session);

    let ids = {
        use flowable_engine::cmd::trigger_start_event_subscription_cmd::TriggerEventSubprocessByEventCmd;
        engine
            .get_command_executor()
            .execute(&TriggerEventSubprocessByEventCmd::new(
                EventSubscriptionKind::EventRegistry,
                "orderReceived".to_string(),
                process_instance.id.clone(),
            ))
            .unwrap()
    };
    assert!(
        !ids.is_empty(),
        "event-registry event subprocess should trigger"
    );

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();
    assert_eq!(
        task_keys,
        vec!["eventTask".to_string(), "mainTask".to_string()]
    );
}

#[test]
fn event_registry_start_event_registers_and_starts_process() {
    let engine = ProcessEngine::new("p92-er-start".to_string());
    let task_service = engine.get_task_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="{FLOWABLE_NS}"
                 targetNamespace="Examples">
        <process id="erStartProcess" isExecutable="true">
            <startEvent id="orderStart">
                <extensionElements>
                    <flowable:eventType>orderReceived</flowable:eventType>
                </extensionElements>
            </startEvent>
            <sequenceFlow id="flow1" sourceRef="orderStart" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review" />
            <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );
    deploy_xml(&engine, "er-start", &xml);

    // Java EventSubscriptionManager.insertEventRegistryEvent:224-249 at deploy time.
    let process_instance = engine
        .get_command_executor()
        .execute(&TriggerProcessStartByEventCmd::new(
            EventSubscriptionKind::EventRegistry,
            "orderReceived".to_string(),
        ))
        .expect("event-registry start subscription should start a process");

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "reviewTask");
}
