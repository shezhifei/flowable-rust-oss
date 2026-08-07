//! BPMN automatic history cleaning
//! (Java `DefaultHistoryCleaningManager` + `BpmnHistoryCleanupJobHandler`).
//!
//! Deviations from Java (documented for P127):
//! - Java `deleteSequentiallyUsingBatch` uses the Batch framework; Rust has no
//!   Batch framework → synchronous per-instance delete up to batch size.
//! - Java skips when an in-progress delete batch exists; Rust has no batch
//!   records → skip logic omitted.
//! - Java `createBatchCleaningQuery().delete()` omitted (no batch records).

use crate::error::FlowableError;
use crate::history::historic_entities::HistoricProcessInstance;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{RuntimeTimerJobState, job_handler_types};
use chrono::{DateTime, Duration as ChronoDuration, Utc};

/// Java `BpmnHistoryCleanupJobHandler.TYPE` (`BpmnHistoryCleanupJobHandler.java:27`).
pub const BPMN_HISTORY_CLEANUP_TYPE: &str = job_handler_types::BPMN_HISTORY_CLEANUP;

/// Java `DefaultHistoryCleaningManager.getEndedBefore`
/// (`DefaultHistoryCleaningManager.java:46-49`).
pub fn ended_before(command_context: &CommandContext) -> DateTime<Utc> {
    let now = command_context.runtime_store.time_source().now();
    let after = command_context.config.clean_instances_ended_after;
    // std::time::Duration → chrono via seconds (day-level config; sub-second unused).
    let chrono_after = ChronoDuration::seconds(after.as_secs() as i64)
        + ChronoDuration::nanoseconds(after.subsec_nanos() as i64);
    now - chrono_after
}

/// Java `DefaultHistoryCleaningManager.createHistoricProcessInstanceCleaningQuery`
/// (`DefaultHistoryCleaningManager.java:33-36`): finishedBefore(endedBefore).
pub fn select_historic_process_instances_for_cleaning(
    command_context: &mut CommandContext,
) -> Result<Vec<HistoricProcessInstance>, FlowableError> {
    let cutoff = ended_before(command_context);
    let (store, session) = command_context.store_and_session();
    let all = store.list_historic_process_instances(session);
    // finishedBefore: end_time is set AND end_time < cutoff
    // (Java HistoricProcessInstanceQuery.finishedBefore → END_TIME_ < ?)
    let mut candidates: Vec<HistoricProcessInstance> = all
        .into_iter()
        .filter(|instance| {
            instance
                .end_time
                .is_some_and(|end| end < cutoff)
        })
        .collect();
    // Deterministic order for batch truncation (oldest finished first).
    candidates.sort_by(|a, b| {
        a.end_time
            .cmp(&b.end_time)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(candidates)
}

/// Java `BpmnHistoryCleanupJobHandler.execute` body without Batch
/// (`BpmnHistoryCleanupJobHandler.java:37-57`).
///
/// Sync-deletes up to `clean_instances_batch_size` historic process instances
/// via the existing cascade path (`delete_historic_process_instance_cascade`).
/// Returns the number of instances deleted.
pub fn execute_history_cleanup(
    command_context: &mut CommandContext,
) -> Result<usize, FlowableError> {
    let batch_size = command_context.config.clean_instances_batch_size as usize;
    let candidates = select_historic_process_instances_for_cleaning(command_context)?;
    let to_delete: Vec<String> = candidates
        .into_iter()
        .take(batch_size)
        .map(|instance| instance.id)
        .collect();

    let deleted = to_delete.len();
    let (store, session) = command_context.store_and_session();
    for id in &to_delete {
        store.delete_historic_process_instance_cascade(id, session);
    }
    Ok(deleted)
}

/// After a successful cleanup fire: reschedule the timer to the next cron fire
/// (Java `TimerJobSchedulerImpl.rescheduleTimerJobAfterExecution` for jobs with
/// `repeat`). Cron expressions are unbounded — the same cycle text is kept.
pub fn reschedule_history_cleanup_timer(
    command_context: &mut CommandContext,
    job: &RuntimeTimerJobState,
) -> Result<(), FlowableError> {
    let Some(cycle) = job.time_cycle.as_deref().filter(|s| !s.trim().is_empty()) else {
        // One-shot (no repeat): delete like a non-repeating timer.
        let (store, session) = command_context.store_and_session();
        store.delete_timer_job_state(&job.timer_job_id, session);
        return Ok(());
    };

    let now = command_context.runtime_store.time_source().now();
    let calendars = &command_context.config.business_calendar_registry;
    let calendar_name = job
        .calendar_name
        .as_deref()
        .unwrap_or(crate::engine::business_calendar::CYCLE_CALENDAR_NAME);
    let calendar = calendars.require(calendar_name)?;
    let Some(due) = calendar.resolve_due_date(cycle, now, None)? else {
        let (store, session) = command_context.store_and_session();
        store.delete_timer_job_state(&job.timer_job_id, session);
        return Ok(());
    };

    let mut next = job.clone();
    next.due_time = Some(due.timestamp_millis());
    next.lock_owner = None;
    next.lock_time = None;
    next.lock_expiration_time = None;
    next.error_message = None;
    next.error_details = None;
    next.job_state = Some("timer".to_string());
    let (store, session) = command_context.store_and_session();
    store.insert_timer_job_state(&next, session);
    Ok(())
}

/// True when the job is the BPMN history cleanup timer.
pub fn is_bpmn_history_cleanup_job(job: &RuntimeTimerJobState) -> bool {
    job.handler_type.as_deref() == Some(BPMN_HISTORY_CLEANUP_TYPE)
}
