//! CMMN automatic history cleaning
//! (Java `DefaultCmmnHistoryCleaningManager` + `CmmnHistoryCleanupJobHandler` +
//! `HandleHistoryCleanupTimerJobCmd`).
//!
//! Deviations (P127):
//! - No Batch framework → sync per-instance delete up to batch size.
//! - No in-progress batch skip.
//! - No batch-record cleanup query.

use crate::error::CmmnError;
use crate::history::CmmnHistoryService;
use crate::job::TYPE_HISTORY_CLEANUP;
use crate::management::CmmnManagementService;
use crate::models::{CMMN_SCOPE_TYPE, CmmnJob, CmmnJobFamily};
use crate::timer_util::{next_repeat_expression, resolve_timer_due};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::time::Duration;
use uuid::Uuid;

/// Java `CmmnEngineConfiguration` history-cleaning defaults
/// (`CmmnEngineConfiguration.java:637-640`).
#[derive(Clone, Debug)]
pub struct CmmnHistoryCleaningConfiguration {
    /// `enableHistoryCleaning` default false.
    pub enable_history_cleaning: bool,
    /// `historyCleaningTimeCycleConfig` default `"0 0 1 * * ?"`.
    pub history_cleaning_time_cycle_config: String,
    /// `cleanInstancesEndedAfter` default `Duration.ofDays(365)`.
    pub clean_instances_ended_after: Duration,
    /// `cleanInstancesBatchSize` default 100.
    pub clean_instances_batch_size: u32,
}

impl Default for CmmnHistoryCleaningConfiguration {
    fn default() -> Self {
        Self {
            enable_history_cleaning: false,
            history_cleaning_time_cycle_config: "0 0 1 * * ?".to_string(),
            clean_instances_ended_after: Duration::from_secs(365 * 24 * 60 * 60),
            clean_instances_batch_size: 100,
        }
    }
}

/// Java `DefaultCmmnHistoryCleaningManager.getEndedBefore`
/// (`DefaultCmmnHistoryCleaningManager.java:49-52`).
pub fn ended_before(config: &CmmnHistoryCleaningConfiguration, now: DateTime<Utc>) -> DateTime<Utc> {
    let after = ChronoDuration::seconds(config.clean_instances_ended_after.as_secs() as i64)
        + ChronoDuration::nanoseconds(config.clean_instances_ended_after.subsec_nanos() as i64);
    now - after
}

/// Stored repeat text on the cleanup timer job (Java `TimerJobEntity.repeat`).
/// Plain string (not JSON) so ensure-job can compare with the config cron directly.
pub fn job_repeat(job: &CmmnJob) -> Option<&str> {
    job.configuration.as_deref().filter(|s| !s.trim().is_empty())
}

/// Java `HandleHistoryCleanupTimerJobCmd` for CMMN
/// (`cmmn/.../HandleHistoryCleanupTimerJobCmd.java:41-85`).
pub fn handle_history_cleanup_timer_job(
    management: &CmmnManagementService,
    config: &CmmnHistoryCleaningConfiguration,
    now: DateTime<Utc>,
) -> Result<(), CmmnError> {
    let cycle = config.history_cleaning_time_cycle_config.as_str();
    let mut cleanup_jobs: Vec<CmmnJob> = management
        .create_job_query()
        .family(CmmnJobFamily::Timer)
        .handler_type(TYPE_HISTORY_CLEANUP)
        .list()?
        .into_iter()
        .collect();
    cleanup_jobs.sort_by(|a, b| a.id.cmp(&b.id));

    if cleanup_jobs.is_empty() {
        schedule_timer_job(management, cycle, now)?;
    } else if cleanup_jobs.len() == 1 {
        let timer_job = &cleanup_jobs[0];
        if job_repeat(timer_job) != Some(cycle) {
            management.delete_job(&timer_job.id)?;
            schedule_timer_job(management, cycle, now)?;
        }
    } else {
        let first = &cleanup_jobs[0];
        if job_repeat(first) != Some(cycle) {
            management.delete_job(&first.id)?;
            schedule_timer_job(management, cycle, now)?;
        }
        for job in cleanup_jobs.iter().skip(1) {
            management.delete_job(&job.id)?;
        }
    }
    Ok(())
}

fn schedule_timer_job(
    management: &CmmnManagementService,
    cycle: &str,
    now: DateTime<Utc>,
) -> Result<(), CmmnError> {
    let due = resolve_timer_due(cycle, now).ok_or_else(|| {
        CmmnError::execution(format!(
            "CMMN history cleaning time cycle '{cycle}' could not resolve a due date"
        ))
    })?;
    let mut job = CmmnJob::new(
        format!("cmmn-history-cleanup:{}", Uuid::new_v4()),
        CmmnJobFamily::Timer,
    )
    .with_handler(TYPE_HISTORY_CLEANUP, Some(cycle.to_string()));
    // Cmmn HandleHistoryCleanupTimerJobCmd.java:78 — scopeType = CMMN
    job.scope_type = Some(CMMN_SCOPE_TYPE.to_string());
    job.due_date = Some(due);
    job.retries = 3;
    management.insert_job(job)?;
    Ok(())
}

/// Java `CmmnHistoryCleanupJobHandler.execute` without Batch
/// (`CmmnHistoryCleanupJobHandler.java:37-57`).
pub fn execute_history_cleanup(
    history: &CmmnHistoryService,
    config: &CmmnHistoryCleaningConfiguration,
    now: DateTime<Utc>,
) -> Result<usize, CmmnError> {
    let cutoff = ended_before(config, now);
    let batch_size = config.clean_instances_batch_size as usize;
    let candidates = history
        .create_historic_case_instance_query()
        .finished(true)
        .finished_before(cutoff)
        .list()?;
    // Query already filters; take first N (stable by list order).
    let to_delete: Vec<String> = candidates
        .into_iter()
        .take(batch_size)
        .map(|instance| instance.case_instance_id)
        .collect();
    let deleted = to_delete.len();
    if !to_delete.is_empty() {
        // Existing cascade path (delete_historic_case_instance_tx family).
        history.bulk_delete_historic_case_instances(&to_delete)?;
    }
    Ok(deleted)
}

/// After a successful cleanup fire, schedule the next cron occurrence
/// (Java timer reschedule with `repeat`). The fired job is deleted by
/// `CmmnEngine::execute_job` after the handler returns.
pub fn schedule_next_history_cleanup_timer(
    management: &CmmnManagementService,
    fired_job: &CmmnJob,
    now: DateTime<Utc>,
) -> Result<(), CmmnError> {
    let Some(repeat) = job_repeat(fired_job) else {
        return Ok(());
    };
    let Some(next_repeat) = next_repeat_expression(repeat) else {
        return Ok(());
    };
    let Some(next_due) = resolve_timer_due(&next_repeat, now) else {
        return Ok(());
    };
    let mut next = fired_job.clone();
    next.id = format!("cmmn-history-cleanup:{}", Uuid::new_v4());
    next.family = CmmnJobFamily::Timer;
    next.state = CmmnJobFamily::Timer.as_str().to_string();
    next.due_date = Some(next_due);
    next.configuration = Some(next_repeat);
    next.lock_owner = None;
    next.exception_message = None;
    next.exception_stacktrace = None;
    next.created_at = now;
    management.insert_job(next)?;
    Ok(())
}
