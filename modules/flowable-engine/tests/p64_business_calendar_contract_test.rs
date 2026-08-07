//! P64 Task 1 — engine-local BusinessCalendar registry contract.
//!
//! Java truth:
//! - `BusinessCalendar.java:20-28` — resolveDuedate / validateDuedate / resolveEndDate.
//! - `MapBusinessCalendarManager.java:39-47` — unknown name throws listing allowed names.
//! - `ProcessEngineConfigurationImpl` seeds `dueDate`, `duration`, `cycle`.
//!
//! ADR-1: registries are engine-local, explicit, and never global.

use chrono::{DateTime, Duration, TimeZone, Utc};
use flowable_engine::engine::business_calendar::{
    BusinessCalendar, BusinessCalendarRegistry, CYCLE_CALENDAR_NAME, DUE_DATE_CALENDAR_NAME,
    DURATION_CALENDAR_NAME,
};
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap()
}

/// Test calendar that always resolves to a fixed offset, regardless of description.
#[derive(Debug)]
struct FixedOffsetCalendar {
    offset_minutes: i64,
}

impl BusinessCalendar for FixedOffsetCalendar {
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
        _description: &str,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, FlowableError> {
        Ok(now + Duration::days(1))
    }

    fn validate_due_date(
        &self,
        _repeat: &str,
        _max_iterations: Option<u32>,
        _end_date: Option<DateTime<Utc>>,
        _candidate: DateTime<Utc>,
    ) -> Result<bool, FlowableError> {
        Ok(true)
    }
}

#[test]
fn default_registry_exposes_the_three_java_calendars() {
    let registry = BusinessCalendarRegistry::default();
    let mut names = registry.names();
    names.sort();
    assert_eq!(
        names,
        vec![CYCLE_CALENDAR_NAME, DUE_DATE_CALENDAR_NAME, DURATION_CALENDAR_NAME],
        "default registry must seed exactly Java's dueDate/duration/cycle calendars"
    );
    assert_eq!(DUE_DATE_CALENDAR_NAME, "dueDate");
    assert_eq!(DURATION_CALENDAR_NAME, "duration");
    assert_eq!(CYCLE_CALENDAR_NAME, "cycle");
}

#[test]
fn names_are_deterministic() {
    let registry = BusinessCalendarRegistry::default();
    assert_eq!(registry.names(), registry.names());
    let sorted = {
        let mut copy = registry.names();
        copy.sort();
        copy
    };
    assert_eq!(registry.names(), sorted, "names() must be sorted");
}

#[test]
fn duration_calendar_resolves_iso_duration() {
    let registry = BusinessCalendarRegistry::default();
    let calendar = registry.get(DURATION_CALENDAR_NAME).expect("duration calendar");
    let due = calendar
        .resolve_due_date("PT10M", now(), None)
        .expect("resolve PT10M")
        .expect("PT10M has a due date");
    assert_eq!(due, now() + Duration::minutes(10));
}

#[test]
fn due_date_calendar_resolves_instant() {
    let registry = BusinessCalendarRegistry::default();
    let calendar = registry.get(DUE_DATE_CALENDAR_NAME).expect("dueDate calendar");
    let due = calendar
        .resolve_due_date("2030-01-01T00:00:00Z", now(), None)
        .expect("resolve instant")
        .expect("instant has a due date");
    assert_eq!(due, Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap());
}

#[test]
fn cycle_calendar_resolves_repeating_expression() {
    let registry = BusinessCalendarRegistry::default();
    let calendar = registry.get(CYCLE_CALENDAR_NAME).expect("cycle calendar");
    let due = calendar
        .resolve_due_date("R5/PT1H", now(), None)
        .expect("resolve R5/PT1H")
        .expect("R5/PT1H has a next fire");
    assert_eq!(due, now() + Duration::hours(1));
}

#[test]
fn unknown_name_fails_with_allowed_names() {
    // MapBusinessCalendarManager.java:41-46 — no silent fallback; the message
    // names the requested calendar and the allowed set.
    let registry = BusinessCalendarRegistry::default();
    let err = registry.require("nope").unwrap_err();
    let text = err.to_string();
    assert!(text.contains("nope"), "message must name the request: {text}");
    assert!(
        text.contains("dueDate") && text.contains("duration") && text.contains("cycle"),
        "message must list allowed calendars: {text}"
    );
    assert!(registry.get("nope").is_none());
}

#[test]
fn registering_a_new_name_succeeds_and_duplicate_registration_is_rejected() {
    let mut registry = BusinessCalendarRegistry::default();
    registry
        .register(
            "custom",
            Arc::new(FixedOffsetCalendar { offset_minutes: 7 }),
        )
        .expect("first registration");

    let err = registry
        .register(
            "custom",
            Arc::new(FixedOffsetCalendar { offset_minutes: 9 }),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("custom"),
        "duplicate registration must name the calendar: {err}"
    );

    // The first implementation is still the one in effect.
    let due = registry
        .require("custom")
        .unwrap()
        .resolve_due_date("ignored", now(), None)
        .unwrap()
        .unwrap();
    assert_eq!(due, now() + Duration::minutes(7));
}

#[test]
fn replace_is_explicit_and_can_override_a_built_in() {
    let mut registry = BusinessCalendarRegistry::default();
    registry.replace(
        DURATION_CALENDAR_NAME,
        Arc::new(FixedOffsetCalendar { offset_minutes: 1 }),
    );
    let due = registry
        .require(DURATION_CALENDAR_NAME)
        .unwrap()
        .resolve_due_date("PT10M", now(), None)
        .unwrap()
        .unwrap();
    assert_eq!(
        due,
        now() + Duration::minutes(1),
        "explicit replace must take effect"
    );
    assert_eq!(
        registry.names().len(),
        3,
        "replacing a built-in must not add a name"
    );
}

#[test]
fn two_registries_hold_different_implementations_under_the_same_name() {
    // ADR-1 isolation: no process-global map.
    let mut first = BusinessCalendarRegistry::default();
    let mut second = BusinessCalendarRegistry::default();
    first
        .register("shared", Arc::new(FixedOffsetCalendar { offset_minutes: 3 }))
        .unwrap();
    second
        .register(
            "shared",
            Arc::new(FixedOffsetCalendar { offset_minutes: 300 }),
        )
        .unwrap();

    assert_eq!(
        first
            .require("shared")
            .unwrap()
            .resolve_due_date("x", now(), None)
            .unwrap()
            .unwrap(),
        now() + Duration::minutes(3)
    );
    assert_eq!(
        second
            .require("shared")
            .unwrap()
            .resolve_due_date("x", now(), None)
            .unwrap()
            .unwrap(),
        now() + Duration::minutes(300)
    );
}

#[test]
fn registry_is_not_serialized_into_configuration_json() {
    let mut config = ProcessEngineConfiguration::default();
    config
        .business_calendar_registry
        .register(
            "custom",
            Arc::new(FixedOffsetCalendar { offset_minutes: 5 }),
        )
        .unwrap();

    let json = serde_json::to_string(&config).expect("serialize configuration");
    assert!(
        !json.contains("business_calendar_registry") && !json.contains("custom"),
        "runtime registries must be #[serde(skip)]"
    );

    let restored: ProcessEngineConfiguration =
        serde_json::from_str(&json).expect("deserialize configuration");
    let mut names = restored.business_calendar_registry.names();
    names.sort();
    assert_eq!(
        names,
        vec![CYCLE_CALENDAR_NAME, DUE_DATE_CALENDAR_NAME, DURATION_CALENDAR_NAME],
        "a deserialized configuration falls back to the seeded defaults"
    );
}

#[test]
fn configuration_default_seeds_the_registry() {
    let config = ProcessEngineConfiguration::default();
    assert!(config
        .business_calendar_registry
        .get(DUE_DATE_CALENDAR_NAME)
        .is_some());
    assert!(config
        .business_calendar_registry
        .get(DURATION_CALENDAR_NAME)
        .is_some());
    assert!(config
        .business_calendar_registry
        .get(CYCLE_CALENDAR_NAME)
        .is_some());
}

#[test]
fn cycle_calendar_validates_end_date_rejection() {
    let registry = BusinessCalendarRegistry::default();
    let calendar = registry.get(CYCLE_CALENDAR_NAME).unwrap();
    let end = now() + Duration::minutes(30);
    // Candidate after endDate is invalid (Java isValidDate / validateDuedate).
    assert!(!calendar
        .validate_due_date("R5/PT1H", None, Some(end), now() + Duration::hours(1))
        .unwrap());
    assert!(calendar
        .validate_due_date("R5/PT1H", None, Some(end), now() + Duration::minutes(10))
        .unwrap());
    // No endDate → always valid.
    assert!(calendar
        .validate_due_date("R5/PT1H", None, None, now() + Duration::days(365))
        .unwrap());
}

#[test]
fn duration_calendar_rejects_unparsable_description() {
    let registry = BusinessCalendarRegistry::default();
    let err = registry
        .require(DURATION_CALENDAR_NAME)
        .unwrap()
        .resolve_due_date("not-a-duration", now(), None)
        .unwrap_err();
    assert!(
        err.to_string().contains("not-a-duration"),
        "error must quote the offending description: {err}"
    );
}

#[test]
fn resolve_end_date_parses_instants() {
    let registry = BusinessCalendarRegistry::default();
    for name in [DUE_DATE_CALENDAR_NAME, DURATION_CALENDAR_NAME, CYCLE_CALENDAR_NAME] {
        let calendar = registry.require(name).unwrap();
        let end = calendar
            .resolve_end_date("2030-01-01T00:00:00Z", now())
            .unwrap_or_else(|e| panic!("{name} resolve_end_date: {e}"));
        assert_eq!(end, Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap());
    }
}

// ── Task 2: engine-level routing of timer creation through the registry ─────
//
// Java truth: `TimerUtil.createTimerEntityForTimerEventDefinition` picks the
// kind default calendar (`dueDate` / `cycle` / `duration`), then overrides it
// with the evaluated `businessCalendarName`, and calls
// `businessCalendar.resolveDuedate(dueDateString)`. A name that is not in the
// manager is a hard failure (`MapBusinessCalendarManager.getBusinessCalendar`),
// never a silent fallback.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use serde_json::json;

const ENGINE_NOW: (i32, u32, u32, u32, u32, u32) = (2026, 4, 18, 12, 0, 0);

/// Engine whose registry additionally holds `shiftCalendar` (+90m) and
/// `nightCalendar` (+300m). Both ignore the timer description entirely, so any
/// due date they produce proves the calendar — not the ISO parser — computed it.
fn engine_with_custom_calendars(name: &str) -> (ProcessEngine, Arc<TestTimeSource>) {
    let (y, mo, d, h, mi, s) = ENGINE_NOW;
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap(),
    ));
    let mut config = ProcessEngineConfiguration::default();
    config
        .business_calendar_registry
        .register(
            "shiftCalendar",
            Arc::new(FixedOffsetCalendar { offset_minutes: 90 }),
        )
        .unwrap();
    config
        .business_calendar_registry
        .register(
            "nightCalendar",
            Arc::new(FixedOffsetCalendar {
                offset_minutes: 300,
            }),
        )
        .unwrap();
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
                .name("p64-calendar".to_string())
                .add_string("calendar.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

fn expected_due(offset_minutes: i64) -> i64 {
    let (y, mo, d, h, mi, s) = ENGINE_NOW;
    (Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap() + Duration::minutes(offset_minutes))
        .timestamp_millis()
}

fn boundary_timer_xml(process_id: &str, calendar_attr: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="{process_id}" isExecutable="true">
            <startEvent id="theStart" />
            <sequenceFlow id="flow1" sourceRef="theStart" targetRef="task" />
            <userTask id="task" name="Task with timer" />
            <boundaryEvent id="boundaryTimer" cancelActivity="true" attachedToRef="task">
                <timerEventDefinition {calendar_attr}>
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="task" targetRef="theEnd" />
            <sequenceFlow id="flow3" sourceRef="boundaryTimer" targetRef="theEnd" />
            <endEvent id="theEnd" />
        </process>
    </definitions>"#
    )
}

fn single_timer_job(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> flowable_engine::persistence::runtime_store::RuntimeTimerJobState {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let mut jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(process_instance_id, &mut session);
    assert_eq!(jobs.len(), 1, "expected exactly one timer job");
    jobs.remove(0)
}

#[test]
fn boundary_timer_uses_the_literal_custom_calendar() {
    let (engine, _clock) = engine_with_custom_calendars("p64-boundary-literal");
    let def_id = deploy(
        &engine,
        &boundary_timer_xml(
            "p64BoundaryLiteral",
            r#"flowable:businessCalendarName="shiftCalendar""#,
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

    let job = single_timer_job(&engine, &pi.id);
    assert_eq!(
        job.due_time,
        Some(expected_due(90)),
        "shiftCalendar (+90m) must compute the due date, not the PT10M ISO parser"
    );
    assert_eq!(
        job.calendar_name.as_deref(),
        Some("shiftCalendar"),
        "the modelled calendar name is persisted on the job"
    );
}

#[test]
fn boundary_timer_evaluates_a_calendar_name_expression_and_persists_it_raw() {
    let (engine, _clock) = engine_with_custom_calendars("p64-boundary-el");
    let def_id = deploy(
        &engine,
        &boundary_timer_xml(
            "p64BoundaryEl",
            r#"flowable:businessCalendarName="${calendarSelector}""#,
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

    let job = single_timer_job(&engine, &pi.id);
    assert_eq!(
        job.due_time,
        Some(expected_due(300)),
        "the expression must select nightCalendar (+300m)"
    );
    assert_eq!(
        job.calendar_name.as_deref(),
        Some("${calendarSelector}"),
        "ADR-2: the raw expression is persisted, never the name it resolved to"
    );
}

#[test]
fn unknown_calendar_name_fails_the_command_and_creates_no_timer_job() {
    let (engine, _clock) = engine_with_custom_calendars("p64-boundary-unknown");
    let def_id = deploy(
        &engine,
        &boundary_timer_xml(
            "p64BoundaryUnknown",
            r#"flowable:businessCalendarName="ghostCalendar""#,
        ),
    );

    let error = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .expect_err("an unmodelled calendar must not fall back to a default");
    let message = error.to_string();
    assert!(
        message.contains("ghostCalendar") && message.contains("does not exist"),
        "Java MapBusinessCalendarManager reports the missing name: {message}"
    );
    assert!(
        message.contains("shiftCalendar"),
        "the error must list the allowed calendars: {message}"
    );
}

#[test]
fn intermediate_catch_timer_uses_the_custom_calendar() {
    let (engine, clock) = engine_with_custom_calendars("p64-catch");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p64Catch" isExecutable="true">
            <startEvent id="theStart" />
            <sequenceFlow id="flow1" sourceRef="theStart" targetRef="catch1" />
            <intermediateCatchEvent id="catch1">
                <timerEventDefinition flowable:businessCalendarName="shiftCalendar">
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="catch1" targetRef="theEnd" />
            <endEvent id="theEnd" />
        </process>
    </definitions>"#;
    let def_id = deploy(&engine, xml);
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    assert_eq!(
        single_timer_job(&engine, &pi.id).due_time,
        Some(expected_due(90))
    );

    // The ISO duration would have fired after 10 minutes; the calendar's 90 must win.
    clock.advance_time(11 * 60 * 1000);
    assert_eq!(
        engine.run_due_timers().len(),
        0,
        "PT10M must not drive the schedule when a custom calendar is modelled"
    );
    clock.advance_time(80 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);
}

#[test]
fn timer_start_event_resolves_its_calendar_at_deploy_time() {
    let (engine, _clock) = engine_with_custom_calendars("p64-start");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p64Start" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition flowable:businessCalendarName="nightCalendar">
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="flow1" sourceRef="timerStart" targetRef="theEnd" />
            <endEvent id="theEnd" />
        </process>
    </definitions>"#;
    deploy(&engine, xml);

    let deployment_manager = engine.get_command_executor().deployment_manager().clone();
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subscriptions = deployment_manager.get_timer_start_subscriptions(&mut session);
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(
        subscriptions[0].due_time,
        Some(expected_due(300)),
        "deploy-time timer start scheduling routes through the registry too"
    );
    assert_eq!(
        subscriptions[0].calendar_name.as_deref(),
        Some("nightCalendar")
    );
}

#[test]
fn deploying_a_timer_start_with_an_unknown_calendar_fails() {
    let (engine, _clock) = engine_with_custom_calendars("p64-start-unknown");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p64StartUnknown" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition flowable:businessCalendarName="ghostCalendar">
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="flow1" sourceRef="timerStart" targetRef="theEnd" />
            <endEvent id="theEnd" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();
    let error = repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p64-calendar".to_string())
                .add_string("calendar.bpmn20.xml".to_string(), xml.to_string()),
        )
        .expect_err("deploy must fail rather than schedule against a default calendar");
    assert!(
        error.to_string().contains("ghostCalendar"),
        "unexpected error: {error}"
    );
}

#[test]
fn event_subprocess_timer_uses_the_custom_calendar() {
    let (engine, _clock) = engine_with_custom_calendars("p64-event-subprocess");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="p64EventSub" isExecutable="true">
            <startEvent id="theStart" />
            <sequenceFlow id="flow1" sourceRef="theStart" targetRef="task" />
            <userTask id="task" name="Waiting task" />
            <sequenceFlow id="flow2" sourceRef="task" targetRef="theEnd" />
            <endEvent id="theEnd" />
            <subProcess id="eventSub" triggeredByEvent="true">
                <startEvent id="eventTimerStart" isInterrupting="false">
                    <timerEventDefinition flowable:businessCalendarName="shiftCalendar">
                        <timeDuration>PT10M</timeDuration>
                    </timerEventDefinition>
                </startEvent>
                <sequenceFlow id="esFlow" sourceRef="eventTimerStart" targetRef="esEnd" />
                <endEvent id="esEnd" />
            </subProcess>
        </process>
    </definitions>"#;
    let def_id = deploy(&engine, xml);
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subscriptions = runtime_store
        .find_event_subprocess_timer_subscriptions_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].due_time, Some(expected_due(90)));
    assert_eq!(
        subscriptions[0].calendar_name.as_deref(),
        Some("shiftCalendar")
    );
}
