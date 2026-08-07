// P30: interrupting boundary events must NOT delete process-instance-level
// event-subprocess timer subscriptions.
//
// Java parity: `BoundaryEventActivityBehavior.java:63-112` (executeInterruptingBehavior)
// and `:157-164` (deleteChildExecutions) only delete the host activity's child
// execution subtree. Event-subprocess timer subscriptions registered at the
// process-instance level survive until the process instance itself ends.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use std::sync::Arc;

/// Interrupting message boundary fires on the host task; the top-level timer
/// event subprocess subscription must survive and still fire on schedule.
#[test]
fn interrupting_message_boundary_keeps_event_subprocess_timer_subscription() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 28, 8, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::with_time_source(
        "p30-msg-boundary-es-timer".to_string(),
        time_source.clone(),
    );
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p30MsgBoundaryEsTimer" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="hostTask" />
            <userTask id="hostTask" name="Host" />
            <boundaryEvent id="msgBoundary" attachedToRef="hostTask" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="hostTask" targetRef="end" />
            <sequenceFlow id="flow3" sourceRef="msgBoundary" targetRef="afterBoundaryTask" />
            <userTask id="afterBoundaryTask" name="After Boundary" />
            <sequenceFlow id="flow4" sourceRef="afterBoundaryTask" targetRef="end" />
            <endEvent id="end" />

            <subProcess id="timerEventSubProcess" triggeredByEvent="true">
                <startEvent id="eventTimerStart" isInterrupting="false">
                    <timerEventDefinition>
                        <timeDuration>PT5M</timeDuration>
                    </timerEventDefinition>
                </startEvent>
                <sequenceFlow id="espFlow" sourceRef="eventTimerStart" targetRef="espTask" />
                <userTask id="espTask" name="ESP Task" />
                <sequenceFlow id="espEndFlow" sourceRef="espTask" targetRef="espEnd" />
                <endEvent id="espEnd" />
            </subProcess>
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p30-msg-boundary-es-timer".to_string())
                .add_string("p30MsgBoundaryEsTimer.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subs_before = runtime_store
        .find_event_subprocess_timer_subscriptions_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(subs_before.len(), 1, "ES timer subscription registered at start");
    drop(session);

    // Fire the interrupting message boundary: host task cancelled, flow moves on.
    runtime_service
        .trigger_boundary_event_by_message_ref("cancelMessage".to_string(), pi.id.clone());

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert!(
        tasks.iter().any(|t| t.task_definition_key == "afterBoundaryTask"),
        "boundary path must have progressed to afterBoundaryTask"
    );
    assert!(
        !tasks.iter().any(|t| t.task_definition_key == "hostTask"),
        "interrupting boundary must cancel the host task"
    );

    // Java parity (`BoundaryEventActivityBehavior.java:63-112,157-164`): the
    // PI-level ES timer subscription must survive the interrupting boundary.
    let mut session = runtime_store.create_session().unwrap();
    let subs_after = runtime_store
        .find_event_subprocess_timer_subscriptions_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(
        subs_after.len(),
        1,
        "interrupting message boundary must NOT delete the PI-level ES timer subscription"
    );
    drop(session);

    // The subscription must still fire on schedule.
    time_source.advance_time(5 * 60 * 1000);
    let fired = engine.run_due_timers();
    assert!(
        fired.iter().any(|id| id.contains("event_subprocess_timer:")),
        "ES timer must still fire after the boundary interrupt: {fired:?}"
    );

    let esp_tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap()
        .into_iter()
        .filter(|t| t.task_definition_key == "espTask")
        .count();
    assert_eq!(esp_tasks, 1, "event subprocess must have activated after the interrupt");
}

/// Interrupting timer boundary fires on the host task; the top-level timer
/// event subprocess subscription must survive and still fire on schedule.
#[test]
fn interrupting_timer_boundary_keeps_event_subprocess_timer_subscription() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 28, 9, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::with_time_source(
        "p30-timer-boundary-es-timer".to_string(),
        time_source.clone(),
    );
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p30TimerBoundaryEsTimer" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="hostTask" />
            <userTask id="hostTask" name="Host" />
            <boundaryEvent id="timerBoundary" attachedToRef="hostTask" cancelActivity="true">
                <timerEventDefinition>
                    <timeDuration>PT1M</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="hostTask" targetRef="end" />
            <sequenceFlow id="flow3" sourceRef="timerBoundary" targetRef="afterBoundaryTask" />
            <userTask id="afterBoundaryTask" name="After Boundary" />
            <sequenceFlow id="flow4" sourceRef="afterBoundaryTask" targetRef="end" />
            <endEvent id="end" />

            <subProcess id="timerEventSubProcess" triggeredByEvent="true">
                <startEvent id="eventTimerStart" isInterrupting="false">
                    <timerEventDefinition>
                        <timeDuration>PT5M</timeDuration>
                    </timerEventDefinition>
                </startEvent>
                <sequenceFlow id="espFlow" sourceRef="eventTimerStart" targetRef="espTask" />
                <userTask id="espTask" name="ESP Task" />
                <sequenceFlow id="espEndFlow" sourceRef="espTask" targetRef="espEnd" />
                <endEvent id="espEnd" />
            </subProcess>
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p30-timer-boundary-es-timer".to_string())
                .add_string(
                    "p30TimerBoundaryEsTimer.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();

    // Fire the interrupting timer boundary at +1m (ES timer due at +5m).
    time_source.advance_time(60 * 1000);
    engine.run_due_timers();

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert!(
        tasks.iter().any(|t| t.task_definition_key == "afterBoundaryTask"),
        "timer boundary path must have progressed to afterBoundaryTask"
    );
    assert!(
        !tasks.iter().any(|t| t.task_definition_key == "hostTask"),
        "interrupting timer boundary must cancel the host task"
    );

    // Java parity (`BoundaryEventActivityBehavior.java:63-112,157-164`): the
    // PI-level ES timer subscription must survive the interrupting boundary.
    let mut session = runtime_store.create_session().unwrap();
    let subs_after = runtime_store
        .find_event_subprocess_timer_subscriptions_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(
        subs_after.len(),
        1,
        "interrupting timer boundary must NOT delete the PI-level ES timer subscription"
    );
    drop(session);

    // The subscription must still fire on schedule (+4m more → +5m total).
    time_source.advance_time(4 * 60 * 1000);
    let fired = engine.run_due_timers();
    assert!(
        fired.iter().any(|id| id.contains("event_subprocess_timer:")),
        "ES timer must still fire after the boundary interrupt: {fired:?}"
    );

    let esp_tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap()
        .into_iter()
        .filter(|t| t.task_definition_key == "espTask")
        .count();
    assert_eq!(esp_tasks, 1, "event subprocess must have activated after the interrupt");
}
