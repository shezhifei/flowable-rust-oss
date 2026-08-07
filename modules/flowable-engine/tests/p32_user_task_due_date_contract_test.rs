//! Java parity for user-task due-date evaluation.
//!
//! Reference implementation:
//! - `UserTaskActivityBehavior.java:241-272` evaluates the due-date expression
//!   and accepts Date, Instant, LocalDate, LocalDateTime, or String values.
//! - `DueDateBusinessCalendar.java:36-57` resolves String values as either an
//!   absolute date/time or an ISO-8601 duration relative to the engine clock.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::time_source::TestTimeSource;
use serde_json::{Value, json};
use std::sync::Arc;

const NOW_MILLIS: i64 = 1_767_268_800_000; // 2026-01-01T12:00:00Z

fn engine_with_fixed_clock(name: &str) -> ProcessEngine {
    let now = Utc.timestamp_millis_opt(NOW_MILLIS).single().unwrap();
    ProcessEngine::with_time_source(name.to_string(), Arc::new(TestTimeSource::new(now)))
}

fn user_task_xml(process_key: &str, due_date: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="{process_key}" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toTask" sourceRef="start" targetRef="task" />
    <userTask id="task" name="Review" flowable:dueDate="{due_date}" />
    <sequenceFlow id="toEnd" sourceRef="task" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#
    )
}

fn deploy_and_start(
    engine: &ProcessEngine,
    process_key: &str,
    due_date: &str,
    variable: Option<Value>,
) -> Result<String, flowable_engine::error::FlowableError> {
    let repository_service = engine.get_repository_service();
    repository_service.deploy(
        repository_service
            .create_deployment()
            .name(process_key.to_string())
            .add_string(
                format!("{process_key}.bpmn20.xml"),
                user_task_xml(process_key, due_date),
            ),
    )?;
    let process_definition_id = repository_service
        .get_process_definition_ids()?
        .into_iter()
        .find(|id| id.contains(process_key))
        .unwrap();
    let mut start = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(process_definition_id);
    if let Some(value) = variable {
        start = start.variable("dueDate".to_string(), value);
    }
    engine
        .get_runtime_service()
        .start_process_instance(start)
        .map(|instance| instance.id)
}

fn assert_task_and_history_due_date(
    engine: &ProcessEngine,
    process_instance_id: &str,
    expected_millis: i64,
) {
    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
        .into_iter()
        .next()
        .expect("user task must be created");
    assert_eq!(
        task.due_date.map(|due_date| due_date.timestamp_millis()),
        Some(expected_millis)
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let historic = runtime_store
        .get_historic_task_instance(&task.id, &mut session)
        .expect("historic task must be created");
    assert_eq!(
        historic
            .due_date
            .map(|due_date| due_date.timestamp_millis()),
        Some(expected_millis)
    );
}

#[test]
fn expression_string_iso_duration_is_relative_to_engine_clock() {
    let engine = engine_with_fixed_clock("p32-duration-expression");
    let process_instance_id = deploy_and_start(
        &engine,
        "durationExpression",
        "${dueDate}",
        Some(json!("P2DT5H40M")),
    )
    .unwrap();

    assert_task_and_history_due_date(
        &engine,
        &process_instance_id,
        NOW_MILLIS + ((2 * 24 + 5) * 60 + 40) * 60 * 1_000,
    );
}

#[test]
fn literal_iso_duration_uses_the_same_calendar_resolution() {
    let engine = engine_with_fixed_clock("p32-duration-literal");
    let process_instance_id = deploy_and_start(&engine, "durationLiteral", "PT5M", None).unwrap();

    assert_task_and_history_due_date(&engine, &process_instance_id, NOW_MILLIS + 5 * 60 * 1_000);
}

#[test]
fn expression_accepts_epoch_instant_and_iso_local_date_time_values() {
    let cases = [
        (
            "epochMillis",
            json!(521_035_800_000_i64),
            521_035_800_000_i64,
        ),
        (
            "instantString",
            json!("1986-07-06T12:10:00Z"),
            521_035_800_000_i64,
        ),
        (
            "localDateTimeString",
            json!("1986-07-06T12:10:00"),
            521_035_800_000_i64,
        ),
        ("localDateString", json!("1986-07-06"), 520_992_000_000_i64),
    ];

    for (process_key, value, expected_millis) in cases {
        let engine = engine_with_fixed_clock(&format!("p32-{process_key}"));
        let process_instance_id =
            deploy_and_start(&engine, process_key, "${dueDate}", Some(value)).unwrap();
        assert_task_and_history_due_date(&engine, &process_instance_id, expected_millis);
    }
}

#[test]
fn unsupported_expression_value_aborts_task_creation() {
    let engine = engine_with_fixed_clock("p32-invalid-type");

    let error = deploy_and_start(
        &engine,
        "invalidDueDateType",
        "${dueDate}",
        Some(json!(false)),
    )
    .expect_err("boolean due date must be rejected like Java");

    assert!(
        error
            .to_string()
            .contains("Due date expression does not resolve"),
        "unexpected error: {error}"
    );
    assert!(
        engine
            .get_task_service()
            .create_task_query()
            .list()
            .unwrap()
            .is_empty()
    );
}
