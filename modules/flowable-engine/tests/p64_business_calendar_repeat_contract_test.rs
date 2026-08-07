//! P64 Task 3 — business calendars on repeat timers and management reschedule.
//!
//! Java truth:
//! - `TimerJobEntityManagerImpl.createAndCalculateNextTimer` resolves the next
//!   due through `BusinessCalendar.resolveDuedate` + `validateDuedate`.
//! - `DefaultJobManager.getBusinessCalendarName` re-evaluates the raw
//!   `calendarName` expression against the variable scope on every repeat.
//! - `RescheduleTimerJobCmd` → `TimerUtil.rescheduleTimerJob` recalculates the
//!   immediate due date through the calendar, not only persisted metadata.
//!
//! ADR-2: the raw calendar expression is persisted; repeats re-evaluate it.

use chrono::{DateTime, Duration, TimeZone, Utc};
use flowable_engine::engine::business_calendar::{BusinessCalendar, BusinessCalendarRegistry};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const ENGINE_NOW: (i32, u32, u32, u32, u32, u32) = (2026, 4, 18, 12, 0, 0);

fn now() -> DateTime<Utc> {
    let (y, mo, d, h, mi, s) = ENGINE_NOW;
    Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
}

fn expected_due(offset_minutes: i64) -> i64 {
    (now() + Duration::minutes(offset_minutes)).timestamp_millis()
}

/// Records every `resolve_due_date` call and returns `now + offset_minutes`.
/// Optionally rejects candidates past an absolute end bound so endDate tests
/// can prove `validate_due_date` is consulted on the repeat path, and can
/// hard-fail from a given call number onward to prove calendar errors are
/// never swallowed as "repeat exhausted".
#[derive(Debug)]
struct RecordingCalendar {
    offset_minutes: i64,
    resolve_count: AtomicUsize,
    reject_after: Option<DateTime<Utc>>,
    fail_from_call: Option<usize>,
}

impl RecordingCalendar {
    fn new(offset_minutes: i64) -> Self {
        Self {
            offset_minutes,
            resolve_count: AtomicUsize::new(0),
            reject_after: None,
            fail_from_call: None,
        }
    }

    fn with_end_rejection(offset_minutes: i64, reject_after: DateTime<Utc>) -> Self {
        Self {
            offset_minutes,
            resolve_count: AtomicUsize::new(0),
            reject_after: Some(reject_after),
            fail_from_call: None,
        }
    }

    /// Fails `resolve_due_date` with a hard error starting at the given
    /// 1-based call number (so creation can succeed and the repeat can fail).
    fn failing_from_call(offset_minutes: i64, fail_from_call: usize) -> Self {
        Self {
            offset_minutes,
            resolve_count: AtomicUsize::new(0),
            reject_after: None,
            fail_from_call: Some(fail_from_call),
        }
    }

    fn count(&self) -> usize {
        self.resolve_count.load(Ordering::SeqCst)
    }
}

impl BusinessCalendar for RecordingCalendar {
    fn resolve_due_date(
        &self,
        _description: &str,
        now: DateTime<Utc>,
        _max_iterations: Option<u32>,
    ) -> Result<Option<DateTime<Utc>>, FlowableError> {
        let call = self.resolve_count.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(fail_from) = self.fail_from_call
            && call >= fail_from
        {
            return Err(FlowableError::ExecutionError(
                "shift roster backend unavailable".to_string(),
            ));
        }
        Ok(Some(now + Duration::minutes(self.offset_minutes)))
    }

    fn resolve_end_date(
        &self,
        _description: &str,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, FlowableError> {
        Ok(now + Duration::days(1))
    }

    fn validate_due_date(
        &self,
        _repeat: &str,
        _max_iterations: Option<u32>,
        end_date: Option<DateTime<Utc>>,
        candidate: DateTime<Utc>,
    ) -> Result<bool, FlowableError> {
        if let Some(end) = end_date.or(self.reject_after) {
            return Ok(candidate <= end);
        }
        Ok(true)
    }
}

fn engine_with(
    name: &str,
    register: impl FnOnce(&mut BusinessCalendarRegistry),
) -> (ProcessEngine, Arc<TestTimeSource>) {
    let time_source = Arc::new(TestTimeSource::new(now()));
    let mut config = ProcessEngineConfiguration::default();
    register(&mut config.business_calendar_registry);
    let engine = ProcessEngine::build_with_config(name.to_string(), time_source.clone(), config)
        .expect("engine with custom calendars");
    (engine, time_source)
}

fn deploy(engine: &ProcessEngine, xml: &str) -> String {
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p64-repeat".to_string())
                .add_string("repeat.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

fn single_timer_job(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> flowable_engine::persistence::runtime_store::RuntimeTimerJobState {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let mut jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(process_instance_id, &mut session);
    assert_eq!(jobs.len(), 1, "expected exactly one timer job, got {jobs:?}");
    jobs.remove(0)
}

fn optional_timer_job(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Option<flowable_engine::persistence::runtime_store::RuntimeTimerJobState> {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let mut jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(process_instance_id, &mut session);
    if jobs.is_empty() {
        None
    } else {
        assert_eq!(jobs.len(), 1, "expected at most one timer job, got {jobs:?}");
        Some(jobs.remove(0))
    }
}

fn non_interrupting_cycle_xml(process_id: &str, calendar_attr: &str, cycle: &str, end: &str) -> String {
    // Java TimeCycleParser reads `flowable:endDate` from the <timeCycle>
    // element, not from <timerEventDefinition>.
    let end_attr = if end.is_empty() {
        String::new()
    } else {
        format!(r#" flowable:endDate="{end}""#)
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="{process_id}" isExecutable="true">
            <startEvent id="theStart" />
            <sequenceFlow id="flow1" sourceRef="theStart" targetRef="task" />
            <userTask id="task" name="Task with repeating timer" />
            <boundaryEvent id="boundaryTimer" cancelActivity="false" attachedToRef="task">
                <timerEventDefinition {calendar_attr}>
                    <timeCycle{end_attr}>{cycle}</timeCycle>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="task" targetRef="theEnd" />
            <sequenceFlow id="flow3" sourceRef="boundaryTimer" targetRef="sideTask" />
            <userTask id="sideTask" name="Side effect of timer" />
            <sequenceFlow id="flow4" sourceRef="sideTask" targetRef="sideEnd" />
            <endEvent id="sideEnd" />
            <endEvent id="theEnd" />
        </process>
    </definitions>"#
    )
}

#[test]
fn repeat_fires_recompute_due_through_the_custom_calendar() {
    // first due (create) + second due (after fire) must both use the recording calendar.
    let recorder = Arc::new(RecordingCalendar::new(90));
    let recorder_for_assert = Arc::clone(&recorder);
    let (engine, clock) = engine_with("p64-repeat-first-second", |registry| {
        registry
            .register("shiftCalendar", recorder as Arc<dyn BusinessCalendar>)
            .unwrap();
    });

    let def_id = deploy(
        &engine,
        &non_interrupting_cycle_xml(
            "p64RepeatFirstSecond",
            r#"flowable:businessCalendarName="shiftCalendar""#,
            "R3/PT10M",
            "",
        ),
    );
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let first = single_timer_job(&engine, &pi.id);
    assert_eq!(first.due_time, Some(expected_due(90)));
    assert_eq!(
        first.calendar_name.as_deref(),
        Some("shiftCalendar"),
        "raw calendar name must stay on the job across fires"
    );
    assert_eq!(recorder_for_assert.count(), 1, "creation resolves once");

    // Advance past the custom due, fire, and expect the calendar to recompute.
    clock.advance_time(90 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);

    let second = single_timer_job(&engine, &pi.id);
    assert_eq!(
        second.due_time,
        Some((now() + Duration::minutes(90 + 90)).timestamp_millis()),
        "second due must be fire_time + calendar offset, not ISO PT10M"
    );
    assert!(
        second
            .time_cycle
            .as_deref()
            .unwrap_or("")
            .starts_with("R2/"),
        "R3 must decrement to R2 after the first fire: {:?}",
        second.time_cycle
    );
    assert_eq!(
        recorder_for_assert.count(),
        2,
        "the calendar must be consulted again on the repeat path"
    );
}

#[test]
fn repeat_calendar_hard_error_rolls_back_the_fire_and_keeps_the_timer() {
    // P1 regression: a custom calendar hard error on the second
    // `resolve_due_date` (the repeat) must fail the fire command, not be
    // interpreted as "repeat exhausted". The original timer stays in place
    // (cycle undecremented, due unchanged) so a later run can retry.
    let recorder = Arc::new(RecordingCalendar::failing_from_call(90, 2));
    let recorder_for_assert = Arc::clone(&recorder);
    let (engine, clock) = engine_with("p64-repeat-calendar-error", |registry| {
        registry
            .register("shiftCalendar", recorder as Arc<dyn BusinessCalendar>)
            .unwrap();
    });

    let def_id = deploy(
        &engine,
        &non_interrupting_cycle_xml(
            "p64RepeatCalendarError",
            r#"flowable:businessCalendarName="shiftCalendar""#,
            "R3/PT10M",
            "",
        ),
    );
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let first = single_timer_job(&engine, &pi.id);
    assert_eq!(first.due_time, Some(expected_due(90)));
    let original_cycle = first.time_cycle.clone();
    assert!(
        original_cycle.as_deref().unwrap_or("").starts_with("R3/"),
        "unexpected initial cycle: {original_cycle:?}"
    );

    clock.advance_time(90 * 60 * 1000);
    assert_eq!(
        engine.run_due_timers().len(),
        0,
        "a calendar hard error must fail the fire command, not complete it"
    );
    assert_eq!(
        recorder_for_assert.count(),
        2,
        "creation + failing repeat resolution"
    );

    // The whole command rolled back: same job, same cycle, same due date.
    let after = single_timer_job(&engine, &pi.id);
    assert_eq!(
        after.due_time,
        Some(expected_due(90)),
        "the due date must not move on a failed fire"
    );
    assert_eq!(
        after.time_cycle, original_cycle,
        "the cycle must not be decremented on a failed fire"
    );

    // The fire's side effect must have rolled back too.
    let tasks = engine
        .get_task_service()
        .create_task_query()
        .process_instance_id(pi.id.clone())
        .list()
        .unwrap();
    assert!(
        !tasks.iter().any(|t| t.name == "Side effect of timer"),
        "the boundary path must not have executed: {tasks:?}"
    );
}

#[test]
fn end_date_rejection_stops_the_repeat() {
    // Candidate after endDate is rejected by validate_due_date → no next job.
    let end = now() + Duration::minutes(100);
    let recorder = Arc::new(RecordingCalendar::with_end_rejection(90, end));
    let (engine, clock) = engine_with("p64-repeat-end", |registry| {
        registry
            .register(
                "shiftCalendar",
                recorder as Arc<dyn BusinessCalendar>,
            )
            .unwrap();
    });

    let def_id = deploy(
        &engine,
        &non_interrupting_cycle_xml(
            "p64RepeatEnd",
            r#"flowable:businessCalendarName="shiftCalendar""#,
            "R5/PT10M",
            &end.to_rfc3339(),
        ),
    );
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    assert_eq!(single_timer_job(&engine, &pi.id).due_time, Some(expected_due(90)));

    // First fire at +90m is still within end (+100m); next candidate would be
    // +180m which the calendar rejects → no rescheduled job.
    clock.advance_time(90 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);
    assert!(
        optional_timer_job(&engine, &pi.id).is_none(),
        "endDate rejection must retire the non-interrupting boundary timer"
    );
}

#[test]
fn repeat_exhaustion_retires_the_timer() {
    // R2 → create, fire once (R1 remains conceptually for the fire itself),
    // after fire of the last remaining count there is no next job.
    let recorder = Arc::new(RecordingCalendar::new(30));
    let (engine, clock) = engine_with("p64-repeat-exhaust", |registry| {
        registry
            .register("shiftCalendar", recorder as Arc<dyn BusinessCalendar>)
            .unwrap();
    });

    let def_id = deploy(
        &engine,
        &non_interrupting_cycle_xml(
            "p64RepeatExhaust",
            r#"flowable:businessCalendarName="shiftCalendar""#,
            "R2/PT10M",
            "",
        ),
    );
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    assert_eq!(single_timer_job(&engine, &pi.id).due_time, Some(expected_due(30)));

    clock.advance_time(30 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);

    // After first fire, R1 is next. Fire the remaining one; no third job.
    let mid = single_timer_job(&engine, &pi.id);
    assert!(
        mid.time_cycle.as_deref().unwrap_or("").starts_with("R1/"),
        "expected R1 after first fire: {:?}",
        mid.time_cycle
    );

    clock.advance_time(30 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);
    assert!(
        optional_timer_job(&engine, &pi.id).is_none(),
        "R2 is exhausted after two fires"
    );
}

#[test]
fn calendar_name_expression_is_re_evaluated_after_a_variable_change() {
    // First fire uses nightCalendar (+300m); after changing the variable to
    // shiftCalendar (+90m), the next due must use the new calendar.
    let night = Arc::new(RecordingCalendar::new(300));
    let shift = Arc::new(RecordingCalendar::new(90));
    let night_count = Arc::clone(&night);
    let shift_count = Arc::clone(&shift);
    let (engine, clock) = engine_with("p64-repeat-el", |registry| {
        registry
            .register("nightCalendar", night as Arc<dyn BusinessCalendar>)
            .unwrap();
        registry
            .register("shiftCalendar", shift as Arc<dyn BusinessCalendar>)
            .unwrap();
    });

    let def_id = deploy(
        &engine,
        &non_interrupting_cycle_xml(
            "p64RepeatEl",
            r#"flowable:businessCalendarName="${calendarSelector}""#,
            "R5/PT10M",
            "",
        ),
    );
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("calendarSelector".to_string(), json!("nightCalendar")),
        )
        .unwrap();

    let first = single_timer_job(&engine, &pi.id);
    assert_eq!(first.due_time, Some(expected_due(300)));
    assert_eq!(
        first.calendar_name.as_deref(),
        Some("${calendarSelector}"),
        "ADR-2: raw expression is persisted"
    );
    assert_eq!(night_count.count(), 1);
    assert_eq!(shift_count.count(), 0);

    // Switch the selector before the first fire's reschedule runs (variables are
    // re-read from the process instance when the next schedule is computed).
    engine
        .get_runtime_service()
        .set_variable(
            pi.id.clone(),
            "calendarSelector".to_string(),
            json!("shiftCalendar"),
        )
        .unwrap();

    clock.advance_time(300 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);

    let second = single_timer_job(&engine, &pi.id);
    assert_eq!(
        second.due_time,
        Some((now() + Duration::minutes(300 + 90)).timestamp_millis()),
        "repeat must re-evaluate calendarSelector EL and pick shiftCalendar"
    );
    assert_eq!(
        second.calendar_name.as_deref(),
        Some("${calendarSelector}"),
        "the raw expression is never rewritten to the resolved name"
    );
    assert_eq!(night_count.count(), 1, "night only used at creation");
    assert_eq!(shift_count.count(), 1, "shift used on the repeat path");
}

#[test]
fn management_reschedule_uses_calendar_for_the_immediate_due_date() {
    let recorder = Arc::new(RecordingCalendar::new(42));
    let recorder_for_assert = Arc::clone(&recorder);
    let (engine, _clock) = engine_with("p64-mgmt-reschedule", |registry| {
        registry
            .register("shiftCalendar", recorder as Arc<dyn BusinessCalendar>)
            .unwrap();
    });

    // Seed a plain timer job (no process needed for management reschedule).
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let original_due = expected_due(5);
    runtime_store.insert_timer_job_state(
        &flowable_engine::persistence::runtime_store::RuntimeTimerJobState {
            timer_job_id: "timer-reschedule-cal".to_string(),
            process_instance_id: "pi-1".to_string(),
            execution_id: "exec-1".to_string(),
            activity_id: "activity-timer".to_string(),
            job_state: Some("timer".to_string()),
            time_duration: Some("PT5M".to_string()),
            due_time: Some(original_due),
            lock_owner: Some("old-worker".to_string()),
            lock_time: Some(1),
            lock_expiration_time: Some(2),
            retries: Some(3),
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let rescheduled = engine
        .get_management_service()
        .reschedule_timer_job(
            "timer-reschedule-cal",
            None,
            Some("PT5M".to_string()),
            None,
            None,
            Some("shiftCalendar".to_string()),
        )
        .expect("reschedule through custom calendar");

    assert_eq!(
        rescheduled.due_time,
        Some(expected_due(42)),
        "calendarName must change the immediate due date, not only metadata"
    );
    assert_eq!(rescheduled.calendar_name.as_deref(), Some("shiftCalendar"));
    assert!(rescheduled.lock_owner.is_none(), "locks are cleared");
    assert_eq!(
        recorder_for_assert.count(),
        1,
        "management reschedule must invoke the calendar"
    );
    assert_ne!(
        rescheduled.due_time,
        Some(original_due),
        "the due date must not stay at the pre-reschedule value"
    );
}

#[test]
fn management_reschedule_unknown_calendar_leaves_the_job_unchanged() {
    let (engine, _clock) = engine_with("p64-mgmt-unknown", |_| {});
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let original_due = expected_due(5);
    runtime_store.insert_timer_job_state(
        &flowable_engine::persistence::runtime_store::RuntimeTimerJobState {
            timer_job_id: "timer-reschedule-unknown".to_string(),
            process_instance_id: "pi-1".to_string(),
            execution_id: "exec-1".to_string(),
            activity_id: "activity-timer".to_string(),
            job_state: Some("timer".to_string()),
            time_duration: Some("PT5M".to_string()),
            due_time: Some(original_due),
            time_cycle: Some("R3/PT10M".to_string()),
            calendar_name: Some("duration".to_string()),
            lock_owner: Some("worker".to_string()),
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let err = engine
        .get_management_service()
        .reschedule_timer_job(
            "timer-reschedule-unknown",
            None,
            Some("PT5M".to_string()),
            None,
            None,
            Some("ghostCalendar".to_string()),
        )
        .expect_err("unknown calendar must hard-fail");
    assert!(
        err.to_string().contains("ghostCalendar"),
        "unexpected error: {err}"
    );

    let mut session = runtime_store.create_session().unwrap();
    let still = runtime_store
        .find_timer_job_state("timer-reschedule-unknown", &mut session)
        .expect("job must still exist");
    assert_eq!(still.due_time, Some(original_due));
    assert_eq!(still.time_cycle.as_deref(), Some("R3/PT10M"));
    assert_eq!(still.calendar_name.as_deref(), Some("duration"));
    assert_eq!(still.lock_owner.as_deref(), Some("worker"));
}

#[test]
fn management_reschedule_with_built_in_cycle_preserves_end_date_and_cycle() {
    let (engine, _clock) = engine_with("p64-mgmt-cycle", |_| {});
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store.insert_timer_job_state(
        &flowable_engine::persistence::runtime_store::RuntimeTimerJobState {
            timer_job_id: "timer-reschedule-cycle".to_string(),
            process_instance_id: "pi-1".to_string(),
            execution_id: "exec-1".to_string(),
            activity_id: "activity-timer".to_string(),
            job_state: Some("timer".to_string()),
            time_duration: Some("PT5M".to_string()),
            due_time: Some(expected_due(5)),
            lock_owner: Some("old".to_string()),
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let end = "2026-05-02T00:00:00Z".to_string();
    let rescheduled = engine
        .get_management_service()
        .reschedule_timer_job(
            "timer-reschedule-cycle",
            None,
            None,
            Some("R3/PT10M".to_string()),
            Some(end.clone()),
            None,
        )
        .expect("cycle reschedule");

    assert_eq!(rescheduled.due_time, Some(expected_due(10)));
    assert!(
        rescheduled
            .time_cycle
            .as_deref()
            .unwrap_or("")
            .starts_with("R3/"),
        "cycle is prepared/persisted: {:?}",
        rescheduled.time_cycle
    );
    assert_eq!(rescheduled.end_date.as_deref(), Some(end.as_str()));
    assert!(rescheduled.lock_owner.is_none());
}

/// Production probe for a non-instant `flowable:endDate`: records what the
/// engine hands to `resolve_end_date` / `validate_due_date` on both the
/// create and repeat paths (P2-6: previously the engine bypassed the calendar
/// with `parse_instant` and always passed `max_iterations = None`).
#[derive(Debug)]
struct EndDateProbeCalendar {
    offset_minutes: i64,
    end_offset_minutes: i64,
    end_calls: AtomicUsize,
    validate_seen: Mutex<Vec<(Option<u32>, Option<DateTime<Utc>>)>>,
}

impl BusinessCalendar for EndDateProbeCalendar {
    fn resolve_due_date(
        &self,
        _description: &str,
        now: DateTime<Utc>,
        _max_iterations: Option<u32>,
    ) -> Result<Option<DateTime<Utc>>, FlowableError> {
        Ok(Some(now + Duration::minutes(self.offset_minutes)))
    }

    fn resolve_end_date(
        &self,
        description: &str,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, FlowableError> {
        assert_eq!(description, "shift-close", "raw endDate text reaches the calendar");
        self.end_calls.fetch_add(1, Ordering::SeqCst);
        Ok(now + Duration::minutes(self.end_offset_minutes))
    }

    fn validate_due_date(
        &self,
        _repeat: &str,
        max_iterations: Option<u32>,
        end_date: Option<DateTime<Utc>>,
        candidate: DateTime<Utc>,
    ) -> Result<bool, FlowableError> {
        self.validate_seen
            .lock()
            .unwrap()
            .push((max_iterations, end_date));
        if let Some(end) = end_date {
            return Ok(candidate <= end);
        }
        Ok(true)
    }
}

#[test]
fn non_instant_end_date_and_iteration_bound_flow_through_the_production_path() {
    // `flowable:endDate="shift-close"` is not an instant: only the selected
    // calendar can resolve it, on create *and* on every repeat; the counted
    // cycle must reach the calendar as an actual iteration bound.
    let probe = Arc::new(EndDateProbeCalendar {
        offset_minutes: 90,
        end_offset_minutes: 60 * 24 * 10,
        end_calls: AtomicUsize::new(0),
        validate_seen: Mutex::new(Vec::new()),
    });
    let probe_for_assert = Arc::clone(&probe);
    let (engine, clock) = engine_with("p64-end-date-probe", |registry| {
        registry
            .register("shiftCalendar", probe as Arc<dyn BusinessCalendar>)
            .unwrap();
    });

    let def_id = deploy(
        &engine,
        &non_interrupting_cycle_xml(
            "p64EndDateProbe",
            r#"flowable:businessCalendarName="shiftCalendar""#,
            "R3/PT10M",
            "shift-close",
        ),
    );
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let first = single_timer_job(&engine, &pi.id);
    assert_eq!(first.due_time, Some(expected_due(90)));
    assert_eq!(
        first.end_date.as_deref(),
        Some("shift-close"),
        "the raw endDate text is persisted, not a resolved instant"
    );
    assert_eq!(
        probe_for_assert.end_calls.load(Ordering::SeqCst),
        1,
        "creation must resolve the endDate through the calendar"
    );
    assert_eq!(
        probe_for_assert.validate_seen.lock().unwrap()[0],
        (Some(3), Some(now() + Duration::minutes(60 * 24 * 10))),
        "creation validate must see R3's bound and the calendar-resolved end"
    );

    clock.advance_time(90 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);

    let second = single_timer_job(&engine, &pi.id);
    assert!(
        second.time_cycle.as_deref().unwrap_or("").starts_with("R2/"),
        "unexpected cycle: {:?}",
        second.time_cycle
    );
    assert_eq!(
        probe_for_assert.end_calls.load(Ordering::SeqCst),
        2,
        "the repeat must re-resolve the endDate through the same calendar"
    );
    let fire_time = now() + Duration::minutes(90);
    assert_eq!(
        probe_for_assert.validate_seen.lock().unwrap()[1],
        (
            Some(2),
            Some(fire_time + Duration::minutes(60 * 24 * 10))
        ),
        "repeat validate must see the remaining count and a freshly resolved end"
    );
}
