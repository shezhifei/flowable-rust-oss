use crate::agenda::continue_process_operation::ASYNC_CONTINUATION_JOB_TYPE_MARKER;
use crate::engine::event_dispatcher::{EngineEvent, EngineEventType};
use crate::engine::job_retry::resolve_failed_job_retry_cycle;
use crate::engine::timer_worker::TimerWork;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailedJobExecutionOrigin {
    ManualManagementService,
    AutomaticExecutor,
}

pub struct RecordFailedTimerWorkCmd {
    work: TimerWork,
    error: crate::error::FlowableError,
    error_message: String,
    error_details: String,
    unrecoverable: bool,
    origin: FailedJobExecutionOrigin,
}

impl RecordFailedTimerWorkCmd {
    pub fn new(work: TimerWork, error: &crate::error::FlowableError) -> Self {
        Self::new_with_origin(
            work,
            error,
            FailedJobExecutionOrigin::ManualManagementService,
        )
    }

    pub fn new_with_origin(
        work: TimerWork,
        error: &crate::error::FlowableError,
        origin: FailedJobExecutionOrigin,
    ) -> Self {
        Self {
            work,
            error: error.clone(),
            error_message: error.raw_primary_message().into_owned(),
            error_details: format!("{:?}", error),
            unrecoverable: error.is_unrecoverable_job_failure(),
            origin,
        }
    }

    fn dispatch_execution_failure(
        &self,
        job: crate::persistence::runtime_store::RuntimeTimerJobState,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let execution_failure = EngineEvent::JobExecutionFailure {
            job,
            error: self.error.clone(),
        };
        let event_dispatcher = command_context.config.engine_event_dispatcher.clone();
        match self.origin {
            FailedJobExecutionOrigin::ManualManagementService => {
                event_dispatcher.dispatch_in_context(&execution_failure, command_context)
            }
            FailedJobExecutionOrigin::AutomaticExecutor => {
                // Flowable Java protects the automatic executor's original
                // failure from JOB_EXECUTION_FAILURE listener exceptions.
                let _ = event_dispatcher.dispatch_in_context(&execution_failure, command_context);
                Ok(())
            }
        }
    }
}

impl Command<()> for RecordFailedTimerWorkCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let TimerWork::RuntimeJob(timer_job) = &self.work else {
            return Ok(());
        };

        let mut persisted_job = command_context
            .runtime_store
            .find_timer_job_state(&timer_job.timer_job_id, &mut command_context.session)
            .unwrap_or_else(|| timer_job.clone());

        let failed_job = persisted_job.clone();
        if self.origin == FailedJobExecutionOrigin::ManualManagementService {
            self.dispatch_execution_failure(failed_job.clone(), command_context)?;
        }
        let event_dispatcher = command_context.config.engine_event_dispatcher.clone();

        let first_failure = persisted_job.error_message.is_none();
        let retry_cycle =
            if persisted_job.time_duration.as_deref() == Some(ASYNC_CONTINUATION_JOB_TYPE_MARKER) {
                match persisted_job.time_cycle.as_deref() {
                    Some(raw_cycle) => {
                        let execution = command_context
                            .execution_entity_manager
                            .find_by_id(&persisted_job.execution_id, &mut command_context.session)
                            .ok_or_else(|| {
                                crate::error::FlowableError::NotFound(format!(
                                    "Execution '{}' for failed async job '{}' was not found",
                                    persisted_job.execution_id, persisted_job.timer_job_id
                                ))
                            })?;
                        // P6-B: failedJobRetryTimeCycle expression must walk the
                        // parent scope chain — the failing execution may be a
                        // forked child whose variable maps were emptied by P4-7b.
                        let evaluation_execution =
                            crate::engine::variable_service::evaluation_execution(
                                command_context,
                                &execution,
                            );
                        Some(resolve_failed_job_retry_cycle(
                            raw_cycle,
                            &evaluation_execution,
                        )?)
                    }
                    None => None,
                }
            } else {
                None
            };
        let retries_before_failure = if first_failure {
            retry_cycle
                .as_ref()
                .map(|cycle| cycle.repetitions)
                .unwrap_or_else(|| {
                    persisted_job.retries.unwrap_or(
                        command_context
                            .config
                            .async_executor
                            .number_of_retries
                            .max(0),
                    )
                })
        } else {
            persisted_job.retries.unwrap_or(0)
        };
        let retries = if self.unrecoverable {
            0
        } else {
            retries_before_failure.saturating_sub(1)
        };
        persisted_job.retries = Some(retries);
        persisted_job.error_message = Some(self.error_message.clone());
        persisted_job.error_details = Some(self.error_details.clone());
        persisted_job.lock_owner = None;
        persisted_job.lock_time = None;
        persisted_job.lock_expiration_time = None;

        let moved_to_deadletter =
            retries <= 0 && persisted_job.job_state.as_deref() != Some("deadletter");
        if retries <= 0 {
            persisted_job.job_state = Some("deadletter".to_string());
        } else if persisted_job.time_duration.as_deref() == Some(ASYNC_CONTINUATION_JOB_TYPE_MARKER)
        {
            // Flowable Java moves a failed executable async job into the
            // timer-job family while it waits for the retry due date.
            persisted_job.job_state = Some("timer".to_string());
        } else if persisted_job.job_state.is_none() {
            persisted_job.job_state = Some("timer".to_string());
        }
        if retries > 0
            && persisted_job.time_duration.as_deref() == Some(ASYNC_CONTINUATION_JOB_TYPE_MARKER)
        {
            let delay_ms = retry_cycle
                .map(|cycle| cycle.delay_ms.max(0) as u64)
                .unwrap_or(
                    command_context
                        .config
                        .async_executor
                        .async_failed_job_wait_time_ms,
                );
            persisted_job.due_time = Some(
                command_context
                    .runtime_store
                    .time_source()
                    .now()
                    .timestamp_millis()
                    + delay_ms.min(i64::MAX as u64) as i64,
            );
        }

        command_context
            .runtime_store
            .insert_timer_job_state(&persisted_job, &mut command_context.session);
        if moved_to_deadletter {
            let moved_to_deadletter = EngineEvent::Job {
                event_type: EngineEventType::JobMovedToDeadLetter,
                job: persisted_job.clone(),
            };
            event_dispatcher.dispatch_in_context(&moved_to_deadletter, command_context)?;
        }
        let entity_updated = EngineEvent::Job {
            event_type: EngineEventType::EntityUpdated,
            job: persisted_job.clone(),
        };
        event_dispatcher.dispatch_in_context(&entity_updated, command_context)?;
        let retries_decremented = EngineEvent::Job {
            event_type: EngineEventType::JobRetriesDecremented,
            job: persisted_job,
        };
        event_dispatcher.dispatch_in_context(&retries_decremented, command_context)?;
        if self.origin == FailedJobExecutionOrigin::AutomaticExecutor {
            self.dispatch_execution_failure(failed_job.clone(), command_context)?;
        }
        // Java ExecuteAsyncRunnable.java:275-306: after handleFailedJob the
        // executor calls unlockJobIfNeeded — the exclusive PI scope lock is
        // cleared in a separate transaction so the retry/dead-letter row does
        // not keep the process instance locked. Manual management execution
        // never took the scope lock, so only the automatic origin clears it.
        if self.origin == FailedJobExecutionOrigin::AutomaticExecutor
            && failed_job.exclusive
            && !failed_job.process_instance_id.is_empty()
        {
            command_context.runtime_store.clear_process_instance_lock(
                &failed_job.process_instance_id,
                &mut command_context.session,
            );
        }
        Ok(())
    }
}
