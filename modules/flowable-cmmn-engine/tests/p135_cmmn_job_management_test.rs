//! P135 CMMN management job contracts.
//!
//! Java references:
//! - `RescheduleTimerJobCmd.java:56-88` resolves the timer's plan-item instance.
//! - `RescheduleTimerJobCmd.java:90-122` resolves date, duration, repetition and cron values.
//! - `RescheduleTimerJobCmd.java:127-149` deletes the old timer and schedules a new row/id.

use chrono::{Duration, Timelike, Utc};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnEventListener, CmmnHumanTask, CmmnJob, CmmnJobFamily, CmmnModel, CmmnPlanItem,
    TYPE_SET_ASYNC_VARIABLES,
};

fn timer_case_model(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("timer-listener", CmmnEventListener::EVENT_TYPE_TIMER)
                .with_timer_expression("PT1H"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-timer", "timer-listener"))
        .with_human_task(CmmnHumanTask::new("keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new("plan-item-keepalive", "keepalive"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("definition-{case_key}"),
        case_key,
        "P135 timer management case",
        plan_model,
    )])
}

fn scheduled_timer(engine: &CmmnEngine, case_key: &str) -> CmmnJob {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("deployment-{case_key}"))
                .with_resource(format!("{case_key}.cmmn"), timer_case_model(case_key)),
        )
        .expect("deployment");
    let case_id = engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id;
    engine
        .management_service()
        .create_job_query()
        .family(CmmnJobFamily::Timer)
        .scope_id(case_id)
        .single_result()
        .expect("timer query")
        .expect("scheduled timer")
}

fn assert_rebuilt(engine: &CmmnEngine, old: &CmmnJob, new: &CmmnJob) {
    assert_ne!(new.id, old.id, "Java rebuilds the timer with a fresh id");
    assert!(engine.management_service().get_job(&old.id).is_err());
    assert_eq!(new.family, CmmnJobFamily::Timer);
    assert_eq!(new.scope_id, old.scope_id);
    assert_eq!(new.sub_scope_id, old.sub_scope_id);
    assert_eq!(new.scope_definition_id, old.scope_definition_id);
    assert_eq!(new.element_id, old.element_id);
    assert_eq!(new.handler_type, old.handler_type);
    assert_eq!(new.retries, 3);
    assert!(new.lock_owner.is_none());
    assert!(new.exception_message.is_none());
    assert!(new.exception_stacktrace.is_none());
}

#[test]
fn reschedule_direct_date_and_iso_date_rebuild_timer() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let old = scheduled_timer(&engine, "p135DirectDate");
    let direct_due = Utc::now() + Duration::hours(3);
    let direct = engine
        .management_service()
        .reschedule_time_date_job(&old.id, direct_due)
        .expect("direct date reschedule");
    assert_rebuilt(&engine, &old, &direct);
    assert_eq!(direct.due_date, Some(direct_due));

    let iso_due = Utc::now() + Duration::hours(5);
    let iso = engine
        .management_service()
        .reschedule_time_date_value_job(&direct.id, &iso_due.to_rfc3339())
        .expect("ISO date reschedule");
    assert_rebuilt(&engine, &direct, &iso);
    assert_eq!(iso.due_date, Some(iso_due));
    assert!(iso.configuration.is_none());
}

#[test]
fn reschedule_duration_uses_due_date_calendar_semantics() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let old = scheduled_timer(&engine, "p135Duration");
    let before = Utc::now() + Duration::minutes(90);
    let rebuilt = engine
        .management_service()
        .reschedule_time_date_value_job(&old.id, "PT90M")
        .expect("duration reschedule");
    let after = Utc::now() + Duration::minutes(90);
    assert_rebuilt(&engine, &old, &rebuilt);
    assert!(
        rebuilt
            .due_date
            .is_some_and(|due| due >= before && due <= after)
    );
    assert!(rebuilt.configuration.is_none());
}

#[test]
fn reschedule_repetition_prepares_repeat_configuration() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let old = scheduled_timer(&engine, "p135Repetition");
    let rebuilt = engine
        .management_service()
        .reschedule_time_date_value_job(&old.id, "R3/PT15M")
        .expect("repetition reschedule");
    assert_rebuilt(&engine, &old, &rebuilt);
    let configuration: serde_json::Value =
        serde_json::from_str(rebuilt.configuration.as_deref().expect("configuration"))
            .expect("configuration JSON");
    let repeat = configuration["repeat"].as_str().expect("repeat");
    assert!(repeat.starts_with("R3/"), "prepared repeat: {repeat}");
    assert!(repeat.ends_with("/PT15M"), "prepared repeat: {repeat}");
}

#[test]
fn reschedule_quartz_cron_uses_cycle_calendar_semantics() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let old = scheduled_timer(&engine, "p135Cron");
    let rebuilt = engine
        .management_service()
        .reschedule_time_date_value_job(&old.id, "0 0 * * * ?")
        .expect("cron reschedule");
    assert_rebuilt(&engine, &old, &rebuilt);
    let due = rebuilt.due_date.expect("cron due date");
    assert_eq!(due.minute(), 0);
    assert_eq!(due.second(), 0);
    let configuration: serde_json::Value =
        serde_json::from_str(rebuilt.configuration.as_deref().expect("configuration"))
            .expect("configuration JSON");
    assert_eq!(configuration["repeat"], "0 0 * * * ?");
}

#[test]
fn reschedule_rejects_invalid_family_missing_plan_item_and_unknown_expression() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .management_service()
        .insert_job(CmmnJob::new("not-timer", CmmnJobFamily::Executable))
        .expect("executable fixture");
    let wrong_family = engine
        .management_service()
        .reschedule_time_date_value_job("not-timer", "PT1M")
        .expect_err("family mismatch");
    assert!(wrong_family.to_string().contains("timer job"));

    let mut orphan = CmmnJob::new("orphan-timer", CmmnJobFamily::Timer);
    orphan.sub_scope_id = Some("missing-plan-item".to_string());
    engine
        .management_service()
        .insert_job(orphan)
        .expect("orphan fixture");
    let missing_plan_item = engine
        .management_service()
        .reschedule_time_date_value_job("orphan-timer", "PT1M")
        .expect_err("missing plan item");
    assert!(missing_plan_item.to_string().contains("Plan item instance"));

    let timer = scheduled_timer(&engine, "p135InvalidExpression");
    let invalid = engine
        .management_service()
        .reschedule_time_date_value_job(&timer.id, "not-a-timer")
        .expect_err("invalid timer expression");
    assert!(invalid.to_string().contains("did not resolve"));
    assert!(engine.management_service().get_job(&timer.id).is_ok());
}

#[test]
fn deadletter_move_by_job_type_selects_executable_or_history() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let mut runtime = CmmnJob::new("deadletter-runtime", CmmnJobFamily::Deadletter);
    runtime.job_type = Some("message".to_string());
    runtime.retries = 0;
    runtime.lock_owner = Some("failed-worker".to_string());
    runtime.exception_message = Some("boom".to_string());
    engine
        .management_service()
        .insert_job(runtime.clone())
        .expect("runtime deadletter");

    let executable = engine
        .management_service()
        .move_deadletter_job_by_type(&runtime.id, 3)
        .expect("runtime deadletter to executable");
    assert_eq!(executable.id, runtime.id);
    assert_eq!(executable.family, CmmnJobFamily::Executable);
    assert_eq!(executable.state, "executable");
    assert_eq!(executable.job_type.as_deref(), Some("message"));
    assert_eq!(executable.retries, 3);
    assert!(executable.lock_owner.is_none());
    assert_eq!(executable.exception_message.as_deref(), Some("boom"));

    let mut history = CmmnJob::new("deadletter-history", CmmnJobFamily::Deadletter);
    history.job_type = Some("history".to_string());
    history.retries = 0;
    engine
        .management_service()
        .insert_job(history.clone())
        .expect("history deadletter");
    let revived_history = engine
        .management_service()
        .move_deadletter_job_by_type(&history.id, 7)
        .expect("history deadletter to history");
    assert_eq!(revived_history.family, CmmnJobFamily::History);
    assert_eq!(revived_history.state, "history");
    assert_eq!(revived_history.job_type.as_deref(), Some("history"));
    assert_eq!(revived_history.retries, 7);
}

#[test]
fn deadletter_move_rejects_wrong_destination_family_and_unknown_id() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let mut runtime = CmmnJob::new("deadletter-wrong-history", CmmnJobFamily::Deadletter);
    runtime.job_type = Some("message".to_string());
    engine
        .management_service()
        .insert_job(runtime)
        .expect("runtime deadletter");
    let wrong_history = engine
        .management_service()
        .move_deadletter_job_to_history_job("deadletter-wrong-history", 3)
        .expect_err("non-history origin cannot become history");
    assert!(
        wrong_history
            .to_string()
            .contains("Can only move a history job to a history job")
    );

    let mut history = CmmnJob::new("deadletter-wrong-runtime", CmmnJobFamily::Deadletter);
    history.job_type = Some("history".to_string());
    engine
        .management_service()
        .insert_job(history)
        .expect("history deadletter");
    let wrong_executable = engine
        .management_service()
        .move_deadletter_job_to_executable_job("deadletter-wrong-runtime", 3)
        .expect_err("history origin cannot become executable");
    assert!(
        wrong_executable
            .to_string()
            .contains("Cannot move a history job to an executable job")
    );

    let wrong_family = engine
        .management_service()
        .move_deadletter_job_by_type("deadletter-wrong-history", 3)
        .expect("failed forced move left row in deadletter family");
    assert_eq!(wrong_family.family, CmmnJobFamily::Executable);
    let not_deadletter = engine
        .management_service()
        .move_deadletter_job_by_type("deadletter-wrong-history", 3)
        .expect_err("executable is not a deadletter job");
    assert!(not_deadletter.to_string().contains("deadletter job"));

    let missing = engine
        .management_service()
        .move_deadletter_job_by_type("missing-deadletter", 3)
        .expect_err("unknown id");
    assert!(missing.to_string().contains("missing-deadletter"));
}

#[test]
fn history_job_execute_runs_direct_fixture_and_is_family_typed() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let timer = scheduled_timer(&engine, "p135HistoryExecute");
    let case_id = timer.scope_id.expect("case id");

    let mut history = CmmnJob::new("history-execute", CmmnJobFamily::History).with_handler(
        TYPE_SET_ASYNC_VARIABLES,
        Some(r#"{"historyExecuted":true}"#.to_string()),
    );
    history.scope_id = Some(case_id.clone());
    engine
        .management_service()
        .insert_job(history)
        .expect("direct history fixture");
    engine
        .execute_history_job("history-execute")
        .expect("execute history job");
    assert!(engine.management_service().get_job("history-execute").is_err());
    assert_eq!(
        engine
            .runtime_service()
            .get_case_instance(&case_id)
            .expect("case")
            .variables["historyExecuted"],
        true
    );

    let executable = CmmnJob::new("not-history", CmmnJobFamily::Executable).with_handler(
        TYPE_SET_ASYNC_VARIABLES,
        Some(r#"{"shouldNotRun":true}"#.to_string()),
    );
    engine
        .management_service()
        .insert_job(executable)
        .expect("executable fixture");
    let mismatch = engine
        .execute_history_job("not-history")
        .expect_err("family mismatch");
    assert!(mismatch.to_string().contains("history job"));
    assert!(engine.management_service().get_job("not-history").is_ok());

    let missing = engine
        .execute_history_job("missing-history")
        .expect_err("unknown history job");
    assert!(missing.to_string().contains("missing-history"));
}
