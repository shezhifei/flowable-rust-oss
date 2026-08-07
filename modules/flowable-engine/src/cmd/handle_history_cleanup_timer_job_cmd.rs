//! Java `HandleHistoryCleanupTimerJobCmd` parity
//! (`modules/flowable-engine/.../HandleHistoryCleanupTimerJobCmd.java`).
//!
//! Ensures exactly one `bpmn-history-cleanup` timer job:
//! - none → schedule
//! - one with different cron → delete + schedule
//! - many → keep/replace first, delete the rest

use crate::engine::business_calendar::CYCLE_CALENDAR_NAME;
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    RuntimeTimerJobState, job_handler_types, stamp_new_job_metadata,
};
use uuid::Uuid;

/// Java `HandleHistoryCleanupTimerJobCmd` (HandleHistoryCleanupTimerJobCmd.java:33-83).
pub struct HandleHistoryCleanupTimerJobCmd;

impl Command<()> for HandleHistoryCleanupTimerJobCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        let cycle = command_context
            .config
            .history_cleaning_time_cycle_config
            .clone();
        let (store, session) = command_context.store_and_session();
        let mut cleanup_jobs: Vec<RuntimeTimerJobState> = store
            .snapshot_timer_job_states(session)
            .into_values()
            .filter(|job| {
                job.handler_type.as_deref() == Some(job_handler_types::BPMN_HISTORY_CLEANUP)
                    && is_timer_family(job)
            })
            .collect();
        // Stable order so "first" is deterministic across runs.
        cleanup_jobs.sort_by(|a, b| a.timer_job_id.cmp(&b.timer_job_id));

        if cleanup_jobs.is_empty() {
            // HandleHistoryCleanupTimerJobCmd.java:43-44
            schedule_timer_job(command_context, &cycle)?;
        } else if cleanup_jobs.len() == 1 {
            // HandleHistoryCleanupTimerJobCmd.java:46-52
            let timer_job = &cleanup_jobs[0];
            if timer_job.time_cycle.as_deref() != Some(cycle.as_str()) {
                let (store, session) = command_context.store_and_session();
                store.delete_timer_job_state(&timer_job.timer_job_id, session);
                schedule_timer_job(command_context, &cycle)?;
            }
        } else {
            // HandleHistoryCleanupTimerJobCmd.java:53-63
            let first = &cleanup_jobs[0];
            if first.time_cycle.as_deref() != Some(cycle.as_str()) {
                let id = first.timer_job_id.clone();
                let (store, session) = command_context.store_and_session();
                store.delete_timer_job_state(&id, session);
                schedule_timer_job(command_context, &cycle)?;
            }
            for job in cleanup_jobs.iter().skip(1) {
                let (store, session) = command_context.store_and_session();
                store.delete_timer_job_state(&job.timer_job_id, session);
            }
        }
        Ok(())
    }
}

/// Java `HandleHistoryCleanupTimerJobCmd.scheduleTimerJob` (:69-80).
fn schedule_timer_job(
    command_context: &mut CommandContext,
    cycle: &str,
) -> Result<(), FlowableError> {
    let now = command_context.runtime_store.time_source().now();
    let calendars = &command_context.config.business_calendar_registry;
    let calendar = calendars.require(CYCLE_CALENDAR_NAME)?;
    // CycleBusinessCalendar.resolveDuedate(cron) — HandleHistoryCleanupTimerJobCmd.java:76-77
    let due = calendar
        .resolve_due_date(cycle, now, None)?
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "History cleaning time cycle '{cycle}' could not resolve a due date"
            ))
        })?;

    let now_ms = now.timestamp_millis();
    let mut job = RuntimeTimerJobState {
        timer_job_id: format!("bpmn-history-cleanup:{}", Uuid::new_v4()),
        process_instance_id: String::new(),
        execution_id: String::new(),
        activity_id: job_handler_types::BPMN_HISTORY_CLEANUP.to_string(),
        job_state: Some("timer".to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: None,
        time_date: None,
        // Java TimerJobEntity.repeat = historyCleaningTimeCycleConfig
        time_cycle: Some(cycle.to_string()),
        end_date: None,
        calendar_name: Some(CYCLE_CALENDAR_NAME.to_string()),
        due_time: Some(due.timestamp_millis()),
        retries: crate::bpmn::timer_util::default_timer_retries(command_context),
        exclusive: true,
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
    // Ensure handler_type is the cleanup type even if stamp kept a prior value.
    job.handler_type = Some(job_handler_types::BPMN_HISTORY_CLEANUP.to_string());

    let (store, session) = command_context.store_and_session();
    store.insert_timer_job_state(&job, session);
    Ok(())
}

fn is_timer_family(job: &RuntimeTimerJobState) -> bool {
    matches!(
        job.job_state.as_deref(),
        None | Some("timer") | Some("")
    ) || (job.job_state.is_none() && job.due_time.is_some())
}
