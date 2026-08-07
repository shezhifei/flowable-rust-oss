//! P16 timer cycle / duration parse parity probes and Java-aligned contracts.
//!
//! Java refs:
//! - `DurationHelper` (3-segment ISO8601, weeks, anchored next-due)
//! - `CycleBusinessCalendar` (cron fallback for non-R)
//! - `TimerUtil.prepareRepeat` (schedule-time anchor)
//! - `DefaultJobManager.executeTimerJob` + endDate validation

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{
    parse_iso8601_duration, prepare_repeat, reschedule_cycle_after_fire, schedule_cycle,
    TestTimeSource,
};
use std::sync::Arc;

#[test]
fn weeks_duration_is_not_immediate_zero() {
    // Old bug: `P2W` fell into `_ => {}` → Some(0) → immediate fire.
    let millis = parse_iso8601_duration("P2W").expect("P2W must parse");
    assert_eq!(millis, 14 * 24 * 60 * 60 * 1000);
}

#[test]
fn three_segment_r_start_period_fires() {
    let start = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(start));
    let engine = ProcessEngine::with_time_source("three-seg".to_string(), time_source.clone());

    let cycle = format!(
        "R2/{}/PT1H",
        start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="threeSeg" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="catch1" />
            <intermediateCatchEvent id="catch1">
                <timerEventDefinition>
                    <timeCycle>{cycle}</timeCycle>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="catch1" targetRef="end1" />
            <endEvent id="end1" />
        </process>
    </definitions>"#
    );

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("three-seg".to_string())
                .add_string("p.bpmn20.xml".to_string(), xml),
        )
        .unwrap();
    let pd = engine
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
                .process_definition_id(pd),
        )
        .unwrap();

    // Intermediate catch with cycle still fires once when due (no repeat for intermediate).
    assert_eq!(engine.run_due_timers().len(), 0);
    time_source.advance_time(60 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_process_instance(&pi.id, &mut session)
            .unwrap()
            .is_ended
    );
}

#[test]
fn boundary_repeat_is_anchored_when_late() {
    let start = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    // Prepared form as stored after schedule
    let cycle = prepare_repeat("R3/PT1H", start);
    // Fire 30 minutes late after first due
    let late = start + chrono::Duration::minutes(90);
    let next = reschedule_cycle_after_fire(&cycle, None, late).expect("next");
    // Anchored next is start+2h, not late+1h
    assert_eq!(
        next.due_time_millis,
        (start + chrono::Duration::hours(2)).timestamp_millis()
    );
}

#[test]
fn cron_time_cycle_schedules_next() {
    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let s = schedule_cycle("0 0 * * * ?", None, now).expect("cron schedule");
    assert_eq!(
        s.due_time_millis,
        Utc.with_ymd_and_hms(2026, 4, 18, 13, 0, 0)
            .unwrap()
            .timestamp_millis()
    );
}

#[test]
fn end_date_stops_boundary_cycle_reschedule() {
    // Java BoundaryTimerEventRepeatWithEndTest / StartTimerEventRepeatWithEndTest
    let start = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(start));
    let engine = ProcessEngine::with_time_source("end-date".to_string(), time_source.clone());

    // endDate is 90 minutes from start → first fire at +1h ok, reschedule to +2h blocked
    let end = (start + chrono::Duration::minutes(90))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="endDateBoundary" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" />
            <boundaryEvent id="timer" attachedToRef="task1" cancelActivity="false">
                <timerEventDefinition>
                    <timeCycle flowable:endDate="{end}">R10/PT1H</timeCycle>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end1" />
            <endEvent id="end1" />
            <sequenceFlow id="f3" sourceRef="timer" targetRef="esc" />
            <userTask id="esc" />
            <sequenceFlow id="f4" sourceRef="esc" targetRef="end2" />
            <endEvent id="end2" />
        </process>
    </definitions>"#
    );

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("end-date".to_string())
                .add_string("p.bpmn20.xml".to_string(), xml),
        )
        .unwrap();
    let pd = engine
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
                .process_definition_id(pd),
        )
        .unwrap();

    // Confirm endDate was parsed onto the job
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let timers =
        runtime_store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(timers.len(), 1);
    assert!(
        timers[0].end_date.as_deref().is_some(),
        "endDate must be stored on timer job"
    );
    session.rollback().unwrap();
    drop(session);

    time_source.advance_time(60 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);

    let mut session = runtime_store.create_session().unwrap();
    let timers_after =
        runtime_store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert!(
        timers_after.is_empty(),
        "next due (+2h) is past endDate (+90m) → no reschedule"
    );

    let esc = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id)
        .unwrap()
        .into_iter()
        .filter(|t| t.task_definition_key == "esc")
        .count();
    assert_eq!(esc, 1);
}

#[test]
fn start_timer_with_end_date_stops_repeating() {
    let start = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(start));
    let engine =
        ProcessEngine::with_time_source("start-end-date".to_string(), time_source.clone());

    let end = (start + chrono::Duration::seconds(12))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="startEnd" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition>
                    <timeCycle flowable:endDate="{end}">R10/PT5S</timeCycle>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="timerStart" targetRef="task1" />
            <userTask id="task1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end1" />
            <endEvent id="end1" />
        </process>
    </definitions>"#
    );

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("start-end".to_string())
                .add_string("p.bpmn20.xml".to_string(), xml),
        )
        .unwrap();

    // First fire at +5s
    time_source.advance_time(5000);
    assert!(
        engine
            .run_due_timers()
            .iter()
            .any(|id| id.contains("timer_start:"))
    );
    // Second would be +10s — still before end (+12s)
    time_source.advance_time(5000);
    assert!(
        engine
            .run_due_timers()
            .iter()
            .any(|id| id.contains("timer_start:"))
    );
    // Third would be +15s — past endDate → no more
    time_source.advance_time(5000);
    let third = engine.run_due_timers();
    assert!(
        !third.iter().any(|id| id.contains("timer_start:")),
        "endDate must stop further start-timer fires"
    );

    let instances = {
        let store = engine.get_runtime_store();
        let mut s = store.create_session().unwrap();
        store.snapshot_process_instances(&mut s).len()
    };
    assert_eq!(instances, 2);
}
