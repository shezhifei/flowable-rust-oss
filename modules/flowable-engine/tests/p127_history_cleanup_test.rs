//! P127 — BPMN automatic history cleaning
//! (Java HandleHistoryCleanupTimerJobCmd / BpmnHistoryCleanupJobHandler /
//! DefaultHistoryCleaningManager).

use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::history::historic_entities::{
    HistoricActivityInstance, HistoricProcessInstance, HistoricTaskInstance,
    HistoricVariableInstance,
};
use flowable_engine::persistence::runtime_store::{
    RuntimeTimerJobState, job_handler_types, stamp_new_job_metadata,
};
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_CRON: &str = "0 0 1 * * ?";
const ALT_CRON: &str = "0 0 2 * * ?";

fn fixed_now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap()
}

fn engine_with(config: ProcessEngineConfiguration) -> ProcessEngine {
    let time = Arc::new(TestTimeSource::new(fixed_now()));
    ProcessEngine::build_with_config("p127-history-cleanup".to_string(), time, config)
        .expect("engine")
}

fn list_cleanup_timers(engine: &ProcessEngine) -> Vec<RuntimeTimerJobState> {
    engine
        .get_management_service()
        .create_runtime_job_query()
        .handler_type(job_handler_types::BPMN_HISTORY_CLEANUP)
        .list()
        .expect("query")
}

fn insert_historic_pi(
    engine: &ProcessEngine,
    id: &str,
    end_time: Option<chrono::DateTime<Utc>>,
) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let start = fixed_now() - ChronoDuration::days(400);
    store.insert_historic_process_instance(
        &HistoricProcessInstance {
            id: id.to_string(),
            process_definition_id: "pd:1".to_string(),
            business_key: None,
            start_time: start,
            end_time,
            duration_ms: end_time.map(|e| (e - start).num_milliseconds()),
            start_user_id: None,
            delete_reason: None,
        },
        &mut session,
    );
    // Associated history rows that cascade-delete must vanish with the PI.
    store.insert_historic_activity_instance(
        HistoricActivityInstance {
            id: format!("{id}-act"),
            activity_id: "task1".to_string(),
            activity_name: Some("Task".to_string()),
            activity_type: "userTask".to_string(),
            process_instance_id: id.to_string(),
            execution_id: format!("{id}-exec"),
            start_time: start,
            end_time,
            duration_ms: None,
            assignee: None,
            delete_reason: None,
        },
        &mut session,
    );
    store.insert_historic_task_instance(
        HistoricTaskInstance {
            id: format!("{id}-task"),
            process_instance_id: id.to_string(),
            process_definition_id: Some("pd:1".to_string()),
            execution_id: format!("{id}-exec"),
            task_definition_key: Some("task1".to_string()),
            name: Some("Task".to_string()),
            description: None,
            assignee: None,
            owner: None,
            claim_time: None,
            tenant_id: None,
            category: None,
            form_key: None,
            parent_task_id: None,
            priority: None,
            due_date: None,
            start_time: start,
            end_time,
            duration_ms: None,
            delete_reason: None,
        },
        &mut session,
    );
    store.insert_historic_variable_instance(
        &HistoricVariableInstance {
            id: format!("{id}-var"),
            process_instance_id: id.to_string(),
            execution_id: Some(format!("{id}-exec")),
            task_id: None,
            name: "v".to_string(),
            variable_type: "string".to_string(),
            value: serde_json::json!("x"),
            create_time: start,
            last_updated_time: start,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
}

#[test]
fn config_defaults_align_with_java_bpmn() {
    // ProcessEngineConfiguration.java:149-152
    let config = ProcessEngineConfiguration::default();
    assert!(!config.enable_history_cleaning);
    assert_eq!(config.history_cleaning_time_cycle_config, DEFAULT_CRON);
    assert_eq!(
        config.clean_instances_ended_after,
        Duration::from_secs(365 * 24 * 60 * 60)
    );
    assert_eq!(config.clean_instances_batch_size, 100);
}

#[test]
fn ensure_job_idempotent_single_timer() {
    let mut config = ProcessEngineConfiguration::default();
    config.enable_history_cleaning = false; // do not auto-ensure on build
    let engine = engine_with(config);

    engine
        .get_management_service()
        .handle_history_cleanup_timer_job()
        .expect("ensure 1");
    engine
        .get_management_service()
        .handle_history_cleanup_timer_job()
        .expect("ensure 2");

    let jobs = list_cleanup_timers(&engine);
    assert_eq!(jobs.len(), 1, "repeated ensure must not create a second job");
    assert_eq!(
        jobs[0].handler_type.as_deref(),
        Some(job_handler_types::BPMN_HISTORY_CLEANUP)
    );
    assert_eq!(jobs[0].time_cycle.as_deref(), Some(DEFAULT_CRON));
    assert!(jobs[0].due_time.is_some());
}

#[test]
fn ensure_job_replaces_when_cron_changes() {
    let mut config = ProcessEngineConfiguration::default();
    config.enable_history_cleaning = false;
    let engine = engine_with(config);

    engine
        .get_management_service()
        .handle_history_cleanup_timer_job()
        .expect("ensure");
    let first_id = list_cleanup_timers(&engine)[0].timer_job_id.clone();

    // Mutate config cron (shared Arc is cloned into command via engine config).
    // ProcessEngine holds Arc config — we need a path that sees the new cron.
    // handle cmd reads command_context.config which is the engine Arc.
    // Rebuild with new cron and same store is heavy; instead mutate via
    // engine.get_config() is Arc — config is not interior-mutable.
    // So: delete path by constructing a second ensure after manual time_cycle
    // mismatch is tested via re-insert of a job with wrong cycle + ensure.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut stale = list_cleanup_timers(&engine)[0].clone();
    stale.time_cycle = Some(ALT_CRON.to_string());
    store.insert_timer_job_state(&stale, &mut session);
    session.flush_and_commit().unwrap();

    // Config still DEFAULT_CRON → ensure must replace.
    engine
        .get_management_service()
        .handle_history_cleanup_timer_job()
        .expect("replace");
    let jobs = list_cleanup_timers(&engine);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].time_cycle.as_deref(), Some(DEFAULT_CRON));
    assert_ne!(
        jobs[0].timer_job_id, first_id,
        "cron mismatch must schedule a new job id"
    );
}

#[test]
fn ensure_job_deletes_redundant_duplicates() {
    let mut config = ProcessEngineConfiguration::default();
    config.enable_history_cleaning = false;
    let engine = engine_with(config);

    engine
        .get_management_service()
        .handle_history_cleanup_timer_job()
        .expect("ensure");

    // Manually insert two extra cleanup timers.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let now_ms = fixed_now().timestamp_millis();
    for i in 0..2 {
        let mut job = RuntimeTimerJobState {
            timer_job_id: format!("extra-cleanup-{i}"),
            job_state: Some("timer".to_string()),
            time_cycle: Some(DEFAULT_CRON.to_string()),
            due_time: Some(now_ms + 1_000),
            ..Default::default()
        };
        stamp_new_job_metadata(
            &mut job,
            now_ms,
            job_handler_types::BPMN_HISTORY_CLEANUP,
            None,
            None,
            None,
        );
        job.handler_type = Some(job_handler_types::BPMN_HISTORY_CLEANUP.to_string());
        store.insert_timer_job_state(&job, &mut session);
    }
    session.flush_and_commit().unwrap();
    assert_eq!(list_cleanup_timers(&engine).len(), 3);

    engine
        .get_management_service()
        .handle_history_cleanup_timer_job()
        .expect("dedupe");
    assert_eq!(
        list_cleanup_timers(&engine).len(),
        1,
        "ensure must leave exactly one cleanup timer"
    );
}

#[test]
fn enable_false_does_not_auto_create_job_on_engine_start() {
    let config = ProcessEngineConfiguration::default();
    assert!(!config.enable_history_cleaning);
    let engine = engine_with(config);
    assert!(
        list_cleanup_timers(&engine).is_empty(),
        "enableHistoryCleaning=false must not create a cleanup timer at start"
    );
}

#[test]
fn enable_true_auto_creates_job_on_engine_start() {
    let mut config = ProcessEngineConfiguration::default();
    config.enable_history_cleaning = true;
    let engine = engine_with(config);
    assert_eq!(list_cleanup_timers(&engine).len(), 1);
}

#[test]
fn handler_deletes_only_old_finished_instances_and_cascade() {
    let mut config = ProcessEngineConfiguration::default();
    config.enable_history_cleaning = false;
    // 365 days retention; now is 2026-08-05 → cutoff = 2025-08-05
    config.clean_instances_ended_after = Duration::from_secs(365 * 24 * 60 * 60);
    let engine = engine_with(config);

    let old_end = fixed_now() - ChronoDuration::days(400);
    let recent_end = fixed_now() - ChronoDuration::days(10);
    insert_historic_pi(&engine, "pi-old", Some(old_end));
    insert_historic_pi(&engine, "pi-recent", Some(recent_end));
    insert_historic_pi(&engine, "pi-running", None);

    engine
        .get_management_service()
        .handle_history_cleanup_timer_job()
        .expect("ensure");
    let job_id = list_cleanup_timers(&engine)[0].timer_job_id.clone();

    // Force due so execute path runs (manual execute does not check due).
    engine
        .get_management_service()
        .execute_timer_job(&job_id)
        .expect("execute cleanup");

    let remaining: Vec<String> = engine
        .get_history_service()
        .create_historic_process_instance_query()
        .list()
        .expect("list")
        .into_iter()
        .map(|pi| pi.id)
        .collect();
    assert!(
        !remaining.iter().any(|id| id == "pi-old"),
        "old finished PI must be deleted"
    );
    assert!(
        remaining.iter().any(|id| id == "pi-recent"),
        "recent finished PI must remain"
    );
    assert!(
        remaining.iter().any(|id| id == "pi-running"),
        "unfinished PI must remain"
    );

    // Cascade: activity / task / variable for pi-old gone.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store
            .find_historic_activity_instances_by_process_instance_id("pi-old", &mut session)
            .is_empty()
    );
    assert!(
        store
            .get_historic_task_instance("pi-old-task", &mut session)
            .is_none()
    );
    assert!(
        store
            .get_historic_variable_instance("pi-old-var", &mut session)
            .is_none()
    );
    // Cascade kept for remaining instances.
    assert!(
        !store
            .find_historic_activity_instances_by_process_instance_id("pi-recent", &mut session)
            .is_empty()
    );
    let _ = session.rollback();

    // Timer rescheduled (still exactly one cleanup job).
    assert_eq!(list_cleanup_timers(&engine).len(), 1);
}

#[test]
fn handler_respects_batch_size() {
    let mut config = ProcessEngineConfiguration::default();
    config.enable_history_cleaning = false;
    config.clean_instances_ended_after = Duration::from_secs(1); // everything finished is old
    config.clean_instances_batch_size = 2;
    let engine = engine_with(config);

    let old_end = fixed_now() - ChronoDuration::days(2);
    insert_historic_pi(&engine, "pi-a", Some(old_end));
    insert_historic_pi(&engine, "pi-b", Some(old_end));
    insert_historic_pi(&engine, "pi-c", Some(old_end));

    engine
        .get_management_service()
        .handle_history_cleanup_timer_job()
        .expect("ensure");
    let job_id = list_cleanup_timers(&engine)[0].timer_job_id.clone();
    engine
        .get_management_service()
        .execute_timer_job(&job_id)
        .expect("execute");

    let remaining = engine
        .get_history_service()
        .create_historic_process_instance_query()
        .list()
        .expect("list");
    assert_eq!(
        remaining.len(),
        1,
        "batch size 2 must leave 1 of 3 old instances"
    );
}
