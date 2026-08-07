//! P135 BPMN timer-reschedule constructor validation parity.
//!
//! Java authority: `RescheduleTimerJobCmd.java:43-51`.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::error::FlowableError;
use std::sync::Arc;

fn engine() -> ProcessEngine {
    let now = Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap();
    let clock = Arc::new(TestTimeSource::new(now));
    let store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap());
    ProcessEngine::build(
        "p135-bpmn-reschedule-validation".to_string(),
        clock as Arc<dyn TimeSource>,
        store,
    )
}

fn reschedule(
    engine: &ProcessEngine,
    time_date: Option<&str>,
    time_duration: Option<&str>,
    time_cycle: Option<&str>,
    end_date: Option<&str>,
) -> Result<flowable_engine::persistence::runtime_store::RuntimeTimerJobState, FlowableError> {
    engine.get_management_service().reschedule_timer_job(
        "missing-timer",
        time_date.map(str::to_string),
        time_duration.map(str::to_string),
        time_cycle.map(str::to_string),
        end_date.map(str::to_string),
        None,
    )
}

#[test]
fn reschedule_requires_exactly_one_timer_value_before_job_lookup() {
    let engine = engine();
    let none = reschedule(&engine, None, None, None, None).expect_err("one value is required");
    assert!(
        none.to_string().contains("A non-null value is required"),
        "unexpected error: {none}"
    );

    for (time_date, time_duration, time_cycle) in [
        (Some("2030-01-01T00:00:00Z"), Some("PT1H"), None),
        (Some("2030-01-01T00:00:00Z"), None, Some("R/PT1H")),
        (None, Some("PT1H"), Some("R/PT1H")),
        (
            Some("2030-01-01T00:00:00Z"),
            Some("PT1H"),
            Some("R/PT1H"),
        ),
    ] {
        let error = reschedule(&engine, time_date, time_duration, time_cycle, None)
            .expect_err("multiple timer values must fail before lookup");
        assert!(
            error.to_string().contains("At most one non-null value"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn reschedule_end_date_is_only_legal_with_time_cycle() {
    let engine = engine();
    for (time_date, time_duration) in [
        (Some("2030-01-01T00:00:00Z"), None),
        (None, Some("PT1H")),
    ] {
        let error = reschedule(
            &engine,
            time_date,
            time_duration,
            None,
            Some("2031-01-01T00:00:00Z"),
        )
        .expect_err("endDate without timeCycle must fail before lookup");
        assert!(
            error
                .to_string()
                .contains("An end date can only be provided"),
            "unexpected error: {error}"
        );
    }

    let allowed = reschedule(
        &engine,
        None,
        None,
        Some("R/PT1H"),
        Some("2031-01-01T00:00:00Z"),
    )
    .expect_err("valid parameters should proceed to the missing-job lookup");
    assert!(
        matches!(allowed, FlowableError::NotFound(_)),
        "cycle + endDate should pass validation: {allowed}"
    );
}
