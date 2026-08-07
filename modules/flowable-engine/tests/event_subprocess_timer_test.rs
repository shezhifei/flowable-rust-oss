use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use serde_json::json;
use std::sync::Arc;

#[test]
fn event_subprocess_timer_subscription_category_literal_and_expression() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap(),
    ));
    let engine =
        ProcessEngine::with_time_source("event-subprocess-timer-category".to_string(), time_source);

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="eventSubprocessTimerCategory" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="hostTask" />
            <userTask id="hostTask" name="Host" />
            <sequenceFlow id="flow2" sourceRef="hostTask" targetRef="end" />
            <endEvent id="end" />

            <subProcess id="timerEventSubProcess" triggeredByEvent="true">
                <startEvent id="eventTimerStart" isInterrupting="true">
                    <extensionElements>
                        <flowable:jobCategory>${categoryValue}</flowable:jobCategory>
                    </extensionElements>
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

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("event-subprocess-timer-category".to_string())
                .add_string(
                    "eventSubprocessTimerCategory.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("categoryValue".to_string(), json!("event-orders")),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subs = runtime_store.find_event_subprocess_timer_subscriptions_by_process_instance_id(
        &process_instance.id,
        &mut session,
    );
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].start_event_id, "eventTimerStart");
    assert_eq!(
        subs[0].category.as_deref(),
        Some("event-orders"),
        "expression category must resolve from process execution variables"
    );
}

#[test]
fn non_interrupting_event_subprocess_timer_cycle_repeats() {
    // Java TimerEventSubprocessTest.testNonInterruptingMultipleInstances — R3/P1D
    // keeps the subscription and re-activates on each cycle fire.
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::with_time_source(
        "event-subprocess-timer-repeat".to_string(),
        time_source.clone(),
    );

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="espRepeat" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="hostTask" />
            <userTask id="hostTask" name="Host" />
            <sequenceFlow id="flow2" sourceRef="hostTask" targetRef="end" />
            <endEvent id="end" />

            <subProcess id="timerEventSubProcess" triggeredByEvent="true">
                <startEvent id="eventTimerStart" isInterrupting="false">
                    <timerEventDefinition>
                        <timeCycle>R3/P1D</timeCycle>
                    </timerEventDefinition>
                </startEvent>
                <sequenceFlow id="espFlow" sourceRef="eventTimerStart" targetRef="espTask" />
                <userTask id="espTask" name="ESP Task" />
                <sequenceFlow id="espEndFlow" sourceRef="espTask" targetRef="espEnd" />
                <endEvent id="espEnd" />
            </subProcess>
        </process>
    </definitions>"#;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("esp-repeat".to_string())
                .add_string("esp.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    // Fire 1
    time_source.advance_time(24 * 60 * 60 * 1000);
    let fired1 = engine.run_due_timers();
    assert!(
        fired1
            .iter()
            .any(|id| id.contains("event_subprocess_timer:")),
        "first R3 fire"
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subs = runtime_store.find_event_subprocess_timer_subscriptions_by_process_instance_id(
        &pi.id,
        &mut session,
    );
    assert_eq!(subs.len(), 1, "non-interrupting cycle must keep subscription");
    assert!(subs[0].due_time.is_some());
    assert!(
        subs[0]
            .time_cycle
            .as_deref()
            .unwrap_or("")
            .starts_with("R2/"),
        "remaining count after first fire: {:?}",
        subs[0].time_cycle
    );
    session.rollback().unwrap();
    drop(session);

    // Fire 2
    time_source.advance_time(24 * 60 * 60 * 1000);
    assert!(
        engine
            .run_due_timers()
            .iter()
            .any(|id| id.contains("event_subprocess_timer:"))
    );

    // Fire 3 (last)
    time_source.advance_time(24 * 60 * 60 * 1000);
    assert!(
        engine
            .run_due_timers()
            .iter()
            .any(|id| id.contains("event_subprocess_timer:"))
    );

    let mut session = runtime_store.create_session().unwrap();
    let subs_after = runtime_store
        .find_event_subprocess_timer_subscriptions_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(subs_after.len(), 1, "exhausted cycle retires in place");
    assert!(
        subs_after[0].due_time.is_none(),
        "R3 exhausted → due_time cleared (no further fires)"
    );
    session.rollback().unwrap();
    drop(session);

    // Extra advance must not fire again
    time_source.advance_time(24 * 60 * 60 * 1000);
    assert!(
        !engine
            .run_due_timers()
            .iter()
            .any(|id| id.contains("event_subprocess_timer:")),
        "exhausted cycle must not fire again"
    );

    let esp_tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap()
        .into_iter()
        .filter(|t| t.task_definition_key == "espTask")
        .count();
    assert_eq!(esp_tasks, 3, "R3 should activate event subprocess three times");
}
