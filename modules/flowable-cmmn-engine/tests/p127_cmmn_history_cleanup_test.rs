//! P127 — CMMN automatic history cleaning
//! (Java HandleHistoryCleanupTimerJobCmd / CmmnHistoryCleanupJobHandler /
//! DefaultCmmnHistoryCleaningManager).

use chrono::{Duration as ChronoDuration, Utc};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnHistoryCleaningConfiguration, CmmnHumanTask, CmmnHumanTaskCompletionRequest, CmmnJobFamily,
    CmmnModel, CmmnPlanItem, TYPE_HISTORY_CLEANUP,
};
use std::time::Duration;

const DEFAULT_CRON: &str = "0 0 1 * * ?";
const ALT_CRON: &str = "0 0 2 * * ?";

fn simple_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));
    CmmnModel::new(vec![CmmnCase::new(
        "case-cleanup",
        "cleanupCase",
        "Cleanup case",
        plan_model,
    )])
}

fn deploy_and_complete(engine: &CmmnEngine, count: usize) -> Vec<String> {
    engine
        .deploy(
            CmmnDeploymentRequest::new("cleanup")
                .with_resource("cleanup.cmmn", simple_model()),
        )
        .expect("deploy");
    let mut ids = Vec::new();
    for i in 0..count {
        let case_instance = engine
            .start_case_instance_by_key(
                "cleanupCase",
                CmmnCaseInstanceStartRequest::new().with_business_key(format!("bk-{i}")),
            )
            .expect("start");
        let task = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .single_result()
            .expect("q")
            .expect("task");
        engine
            .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
            .expect("complete");
        ids.push(case_instance.id);
    }
    ids
}

fn list_cleanup_timers(engine: &CmmnEngine) -> Vec<flowable_cmmn_engine::CmmnJob> {
    engine
        .management_service()
        .create_job_query()
        .family(CmmnJobFamily::Timer)
        .handler_type(TYPE_HISTORY_CLEANUP)
        .list()
        .expect("list")
}

#[test]
fn config_defaults_align_with_java_cmmn() {
    // CmmnEngineConfiguration.java:637-640
    let config = CmmnHistoryCleaningConfiguration::default();
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
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine.handle_history_cleanup_timer_job().expect("ensure 1");
    engine.handle_history_cleanup_timer_job().expect("ensure 2");
    let jobs = list_cleanup_timers(&engine);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].handler_type.as_deref(), Some(TYPE_HISTORY_CLEANUP));
    assert_eq!(jobs[0].configuration.as_deref(), Some(DEFAULT_CRON));
    assert!(jobs[0].due_date.is_some());
}

#[test]
fn ensure_job_replaces_when_cron_changes() {
    let mut engine = CmmnEngine::new_in_memory().expect("engine");
    engine.handle_history_cleanup_timer_job().expect("ensure");
    let first_id = list_cleanup_timers(&engine)[0].id.clone();

    // Point config at ALT cron and re-ensure.
    let mut cfg = engine.history_cleaning_config().clone();
    cfg.history_cleaning_time_cycle_config = ALT_CRON.to_string();
    engine.set_history_cleaning_config(cfg);
    engine.handle_history_cleanup_timer_job().expect("replace");

    let jobs = list_cleanup_timers(&engine);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].configuration.as_deref(), Some(ALT_CRON));
    assert_ne!(jobs[0].id, first_id);
}

#[test]
fn ensure_job_deletes_redundant_duplicates() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine.handle_history_cleanup_timer_job().expect("ensure");

    for i in 0..2 {
        let mut job = flowable_cmmn_engine::CmmnJob::new(
            format!("extra-cmmn-cleanup-{i}"),
            CmmnJobFamily::Timer,
        )
        .with_handler(TYPE_HISTORY_CLEANUP, Some(DEFAULT_CRON.to_string()));
        job.due_date = Some(Utc::now() + ChronoDuration::hours(1));
        engine
            .management_service()
            .insert_job(job)
            .expect("insert extra");
    }
    assert_eq!(list_cleanup_timers(&engine).len(), 3);

    engine.handle_history_cleanup_timer_job().expect("dedupe");
    assert_eq!(list_cleanup_timers(&engine).len(), 1);
}

#[test]
fn enable_false_does_not_auto_create_job_on_engine_start() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    assert!(
        list_cleanup_timers(&engine).is_empty(),
        "default enableHistoryCleaning=false must not create a cleanup timer"
    );
}

#[test]
fn enable_true_auto_creates_job_on_engine_start() {
    let mut cfg = CmmnHistoryCleaningConfiguration::default();
    cfg.enable_history_cleaning = true;
    let engine = CmmnEngine::new_in_memory_with_history_cleaning(cfg).expect("engine");
    assert_eq!(list_cleanup_timers(&engine).len(), 1);
}

#[test]
fn handler_deletes_only_old_finished_cases_and_cascade() {
    // Retention 365 days: just-finished cases stay; zero retention would delete all.
    // To get an "old" finished case we complete, then re-ensure with 0 retention for
    // a second case set... Simpler: two engines paths —
    // (1) ended_after=0 deletes finished; unfinished kept via separate unfinished case.
    let mut cfg = CmmnHistoryCleaningConfiguration::default();
    cfg.enable_history_cleaning = false;
    cfg.clean_instances_ended_after = Duration::from_secs(0);
    cfg.clean_instances_batch_size = 100;
    let engine = CmmnEngine::new_in_memory_with_history_cleaning(cfg).expect("engine");

    let finished_ids = deploy_and_complete(&engine, 1);
    let unfinished = engine
        .start_case_instance_by_key("cleanupCase", CmmnCaseInstanceStartRequest::new())
        .expect("start unfinished");

    // "Recent finished under long retention" check: use a second finished case with
    // large retention by reconfiguring and only deleting zero-retention ones.
    // With ended_after=0 every finished is eligible; unfinished is not.
    engine.handle_history_cleanup_timer_job().expect("ensure");
    let job_id = list_cleanup_timers(&engine)[0].id.clone();
    // Force due so run_due path / execute works.
    let mut job = engine.management_service().get_job(&job_id).expect("job");
    job.due_date = Some(Utc::now() - ChronoDuration::seconds(1));
    engine.management_service().update_job(&job).expect("due");

    engine.execute_job(&job_id).expect("execute cleanup");

    assert!(
        engine
            .history_service()
            .get_historic_case_instance(&finished_ids[0])
            .is_err(),
        "finished case must be deleted under zero retention"
    );
    assert!(
        engine
            .history_service()
            .get_historic_case_instance(&unfinished.id)
            .is_ok(),
        "unfinished historic case must remain"
    );
    // Human-task history for the finished case cascade-deleted.
    let tasks = engine
        .history_service()
        .create_historic_human_task_query()
        .case_instance_id(&finished_ids[0])
        .list()
        .expect("task hist");
    assert!(tasks.is_empty());

    // Rescheduled: still exactly one cleanup timer (new id after fire).
    assert_eq!(list_cleanup_timers(&engine).len(), 1);
}

#[test]
fn handler_keeps_finished_within_retention_window() {
    let mut cfg = CmmnHistoryCleaningConfiguration::default();
    cfg.clean_instances_ended_after = Duration::from_secs(365 * 24 * 60 * 60);
    let engine = CmmnEngine::new_in_memory_with_history_cleaning(cfg).expect("engine");
    let finished = deploy_and_complete(&engine, 1);

    engine.handle_history_cleanup_timer_job().expect("ensure");
    let job_id = list_cleanup_timers(&engine)[0].id.clone();
    let mut job = engine.management_service().get_job(&job_id).expect("job");
    job.due_date = Some(Utc::now() - ChronoDuration::seconds(1));
    engine.management_service().update_job(&job).expect("due");
    engine.execute_job(&job_id).expect("execute");

    assert!(
        engine
            .history_service()
            .get_historic_case_instance(&finished[0])
            .is_ok(),
        "just-finished case must remain under 365-day retention"
    );
}

#[test]
fn handler_respects_batch_size() {
    let mut cfg = CmmnHistoryCleaningConfiguration::default();
    cfg.clean_instances_ended_after = Duration::from_secs(0);
    cfg.clean_instances_batch_size = 2;
    let engine = CmmnEngine::new_in_memory_with_history_cleaning(cfg).expect("engine");
    let ids = deploy_and_complete(&engine, 3);

    engine.handle_history_cleanup_timer_job().expect("ensure");
    let job_id = list_cleanup_timers(&engine)[0].id.clone();
    let mut job = engine.management_service().get_job(&job_id).expect("job");
    job.due_date = Some(Utc::now() - ChronoDuration::seconds(1));
    engine.management_service().update_job(&job).expect("due");
    engine.execute_job(&job_id).expect("execute");

    let remaining: usize = ids
        .iter()
        .filter(|id| {
            engine
                .history_service()
                .get_historic_case_instance(id)
                .is_ok()
        })
        .count();
    assert_eq!(remaining, 1, "batch size 2 must leave 1 of 3 finished cases");
}
