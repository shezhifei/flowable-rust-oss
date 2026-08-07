//! Management reschedule of a timer job (Java `RescheduleTimerJobCmd`).
//!
//! Rebuilds the due date through the engine-local business calendar registry
//! so a supplied `calendarName` changes the *immediate* due date, not only
//! persisted metadata (P64 Task 3).

use crate::bpmn::timer_util::resolve_timer_schedule;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::RuntimeTimerJobState;
use crate::runtime::execution::Execution;

pub struct RescheduleTimerJobCmd {
    pub job_id: String,
    pub time_date: Option<String>,
    pub time_duration: Option<String>,
    pub time_cycle: Option<String>,
    pub end_date: Option<String>,
    pub calendar_name: Option<String>,
}

impl RescheduleTimerJobCmd {
    pub fn new(
        job_id: String,
        time_date: Option<String>,
        time_duration: Option<String>,
        time_cycle: Option<String>,
        end_date: Option<String>,
        calendar_name: Option<String>,
    ) -> Self {
        Self {
            job_id,
            time_date,
            time_duration,
            time_cycle,
            end_date,
            calendar_name,
        }
    }
}

impl Command<RuntimeTimerJobState> for RescheduleTimerJobCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RuntimeTimerJobState, crate::error::FlowableError> {
        // Java RescheduleTimerJobCmd.java:43-48 requires exactly one of the three
        // timer values. Rust validates at command entry (before lookup/write) because
        // this command's established constructor returns `Self`, not `Result`.
        let time_value_count = [
            self.time_date.as_ref(),
            self.time_duration.as_ref(),
            self.time_cycle.as_ref(),
        ]
        .into_iter()
        .flatten()
        .count();
        if time_value_count == 0 {
            return Err(crate::error::FlowableError::ExecutionError(
                "A non-null value is required for one of timeDate, timeDuration, or timeCycle"
                    .to_string(),
            ));
        }
        if time_value_count > 1 {
            return Err(crate::error::FlowableError::ExecutionError(
                "At most one non-null value can be provided for timeDate, timeDuration, or timeCycle"
                    .to_string(),
            ));
        }

        // Java RescheduleTimerJobCmd.java:50-51 checks timeCycle despite the upstream
        // exception text saying timeDuration; preserve both the actual condition and
        // Java's externally visible message.
        if self.end_date.is_some() && self.time_cycle.is_none() {
            return Err(crate::error::FlowableError::ExecutionError(
                "An end date can only be provided when rescheduling a timer using timeDuration."
                    .to_string(),
            ));
        }

        let Some(mut job) = command_context
            .runtime_store
            .find_timer_job_state(&self.job_id, &mut command_context.session)
        else {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Timer job '{}' not found",
                self.job_id
            )));
        };

        // Snapshot used only if resolution fails before any write — callers that
        // catch the error still see the original row (single command session).
        let _original_due = job.due_time;
        let _original_cycle = job.time_cycle.clone();

        // Variable scope for calendarName / timer field EL: host execution → PI → empty.
        let execution = command_context
            .runtime_store
            .find_execution(&job.execution_id, &mut command_context.session)
            .or_else(|| {
                if job.process_instance_id.is_empty() {
                    None
                } else {
                    command_context.runtime_store.find_execution(
                        &job.process_instance_id,
                        &mut command_context.session,
                    )
                }
            })
            .unwrap_or_else(Execution::default);

        let now = command_context.runtime_store.time_source().now();
        let calendars = command_context.config.business_calendar_registry.clone();

        // Java TimerUtil.rescheduleTimerJob → createTimerEntityForTimerEventDefinition:
        // calculate due through the (possibly custom) business calendar.
        let schedule = resolve_timer_schedule(
            self.time_date.as_ref(),
            self.time_duration.as_ref(),
            self.time_cycle.as_ref(),
            self.end_date.as_ref(),
            self.calendar_name.as_ref(),
            &execution,
            &calendars,
            now,
        )?;

        job.time_date = schedule.time_date;
        job.time_duration = schedule.time_duration;
        job.time_cycle = schedule.time_cycle;
        job.end_date = schedule.end_date.or_else(|| self.end_date.clone());
        job.calendar_name = schedule
            .calendar_name
            .or_else(|| self.calendar_name.clone());
        job.due_time = schedule.due_time;
        job.lock_owner = None;
        job.lock_time = None;
        job.lock_expiration_time = None;
        job.error_message = None;
        job.error_details = None;

        command_context
            .runtime_store
            .insert_timer_job_state(&job, &mut command_context.session);

        // P125: Java TimerUtil.rescheduleTimerJob (277-282) — JOB_RESCHEDULED
        // first, then TIMER_SCHEDULED for the (re)inserted job.
        crate::engine::event_dispatcher::dispatch_job_rescheduled(command_context, &job);
        crate::engine::event_dispatcher::dispatch_timer_scheduled(command_context, &job);

        Ok(job)
    }
}
