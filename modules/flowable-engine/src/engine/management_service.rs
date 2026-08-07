use crate::agenda::continue_process_operation::{
    ASYNC_CONTINUATION_JOB_STATE, ASYNC_CONTINUATION_JOB_TYPE_MARKER,
};
use crate::cmd::handle_history_cleanup_timer_job_cmd::HandleHistoryCleanupTimerJobCmd;
use crate::cmd::job_suspension::{
    SuspendedJobActivation, activate_suspended_job as activate_suspended_job_state,
};
use crate::cmd::persist_job_extra_fields_cmd::PersistJobExtraFieldsCmd;
use crate::cmd::record_failed_timer_work_cmd::RecordFailedTimerWorkCmd;
use crate::cmd::run_due_timers_cmd::ExecuteTimerWorkCmd;
use crate::engine::event_dispatcher::{EngineEvent, EngineEventType};
use crate::engine::query::{Direction, Query, QueryState};
use crate::cmd::reschedule_timer_job_cmd::RescheduleTimerJobCmd;
use crate::engine::timer_worker::{TimerCoordinationMetrics, TimerWork};
use crate::error::FlowableError;
use crate::history::async_history_job_handler::SharedHistoryJobHandler;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::db_session::DbSession;
use crate::persistence::runtime_store::{RuntimeJobType, RuntimeStore, RuntimeTimerJobState};
use std::sync::Arc;

pub use crate::engine::runtime_job_query::{
    RuntimeJobFamily, RuntimeJobQuery, RuntimeJobQueryResult,
};

pub struct TimerJobQuery {
    state: QueryState<RuntimeTimerJobState>,
}

impl TimerJobQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
        }
    }

    pub fn order_by_job_id(mut self) -> Self {
        self.state.order_by = Some("id".to_string());
        self
    }

    pub fn asc(mut self) -> Self {
        self.state.direction = Direction::Asc;
        self
    }

    pub fn desc(mut self) -> Self {
        self.state.direction = Direction::Desc;
        self
    }
}

pub struct TimerJobQueryCmd {
    query: TimerJobQuery,
}

impl TimerJobQueryCmd {
    pub fn new(query: TimerJobQuery) -> Self {
        Self { query }
    }
}

impl Command<Vec<RuntimeTimerJobState>> for TimerJobQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<RuntimeTimerJobState>, crate::error::FlowableError> {
        let mut jobs = Vec::new();

        // 1. Query timer_job_states
        let timer_jobs: Vec<RuntimeTimerJobState> = command_context
            .session()
            .find_all("timer_job_states")
            .unwrap();
        jobs.extend(timer_jobs);

        // 2. Query process_timer_start_subscriptions and map them to RuntimeTimerJobState
        let subs: Vec<crate::persistence::runtime_store::ProcessTimerStartSubscription> =
            command_context
                .session()
                .find_all("process_timer_start_subscriptions")
                .unwrap();
        for sub in subs {
            jobs.push(RuntimeTimerJobState {
                timer_job_id: sub.id.clone(),
                process_instance_id: String::new(),
                execution_id: String::new(),
                activity_id: sub.start_event_id.clone(),
                job_state: Some("timer".to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: sub.time_duration.clone(),
                time_date: sub.time_date.clone(),
                time_cycle: sub.time_cycle.clone(),
                end_date: sub.end_date.clone(),
                due_time: sub.due_time,
                lock_owner: sub.lock_owner.clone(),
                lock_time: sub.lock_time,
                lock_expiration_time: None,
                retries: None,
                error_message: None,
                error_details: None,
                category: sub.category.clone(),
                ..Default::default()
            });
        }

        jobs.retain(is_timer_job);

        // Apply ordering if requested
        if let Some(order_by) = &self.query.state.order_by
            && order_by.as_str() == "id"
        {
            jobs.sort_by(|a, b| match self.query.state.direction {
                Direction::Asc => a.timer_job_id.cmp(&b.timer_job_id),
                Direction::Desc => b.timer_job_id.cmp(&a.timer_job_id),
            });
        }

        Ok(jobs)
    }
}

impl Query<RuntimeTimerJobState, TimerJobQuery> for TimerJobQuery {
    fn list(&self) -> Result<Vec<RuntimeTimerJobState>, crate::error::FlowableError> {
        let query_clone = TimerJobQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
        };
        let cmd = TimerJobQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<RuntimeTimerJobState>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

pub struct ManagementService {
    command_executor: Arc<DefaultCommandExecutor>,
    history_job_handler: Option<SharedHistoryJobHandler>,
}

impl ManagementService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            command_executor,
            history_job_handler: None,
        }
    }

    pub fn with_history_job_handler(mut self, handler: SharedHistoryJobHandler) -> Self {
        self.history_job_handler = Some(handler);
        self
    }

    pub fn create_timer_job_query(&self) -> TimerJobQuery {
        TimerJobQuery::new(Arc::clone(&self.command_executor))
    }

    /// Java `ManagementService.handleHistoryCleanupTimerJob`
    /// (`ManagementServiceImpl.java:327-328` → `HandleHistoryCleanupTimerJobCmd`).
    ///
    /// Idempotently ensures exactly one `bpmn-history-cleanup` timer job whose
    /// `time_cycle` matches `history_cleaning_time_cycle_config`. Does not check
    /// `enable_history_cleaning` — callers (engine start) gate on that flag.
    pub fn handle_history_cleanup_timer_job(&self) -> Result<(), FlowableError> {
        self.command_executor
            .execute(&HandleHistoryCleanupTimerJobCmd)
    }

    /// Direct engine query covering all runtime job families and P65 dimensions.
    pub fn create_runtime_job_query(&self) -> RuntimeJobQuery {
        RuntimeJobQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn create_job_query(&self) -> RuntimeJobQuery {
        self.create_runtime_job_query()
            .family(RuntimeJobFamily::Executable)
    }

    pub fn create_deadletter_job_query(&self) -> RuntimeJobQuery {
        self.create_runtime_job_query()
            .family(RuntimeJobFamily::Deadletter)
    }

    pub fn create_suspended_job_query(&self) -> RuntimeJobQuery {
        self.create_runtime_job_query()
            .family(RuntimeJobFamily::Suspended)
    }

    pub fn create_history_job_query(&self) -> RuntimeJobQuery {
        self.create_runtime_job_query()
            .family(RuntimeJobFamily::History)
    }

    pub fn find_job_by_id(&self, job_id: &str) -> Option<RuntimeTimerJobState> {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        let found = self
            .command_executor
            .runtime_store()
            .find_timer_job_state(job_id, &mut session);
        let _ = session.rollback();
        found
    }

    pub fn find_timer_job_by_id(&self, job_id: &str) -> Option<RuntimeTimerJobState> {
        self.find_job_by_id(job_id).filter(is_timer_job)
    }

    /// Java `ExecuteJobCmd`: execute any executable-family job via its handler.
    ///
    /// Covers async-continuation / async-after and timer-type rows that already
    /// live in the executable family. The job body runs in its own command
    /// transaction. On failure that transaction rolls back, retry/dead-letter
    /// state is recorded in a second command (FailedJobListener semantics),
    /// and the original typed error is returned to the caller.
    pub fn execute_job(&self, job_id: &str) -> Result<(), FlowableError> {
        let job = self.find_executable_job_by_id(job_id).ok_or_else(|| {
            FlowableError::NotFound(format!("Executable job '{job_id}' not found"))
        })?;
        self.execute_runtime_job(job)
    }

    /// Execute a timer-family job by id (REST `/management/timer-jobs/{id}` execute).
    pub fn execute_timer_job(&self, job_id: &str) -> Result<(), FlowableError> {
        let job = self
            .find_timer_job_by_id(job_id)
            .ok_or_else(|| FlowableError::NotFound(format!("Timer job '{job_id}' not found")))?;
        self.execute_runtime_job(job)
    }

    /// Family-agnostic execute for a loaded runtime job row.
    ///
    /// Dispatches through `ExecuteTimerWorkCmd` (same handlers as the automatic
    /// executor). Failures always go through `RecordFailedTimerWorkCmd` so
    /// retries decrement and deadletter transition match Java
    /// FailedJobListener / JobRetryCmd.
    pub fn execute_runtime_job(&self, job: RuntimeTimerJobState) -> Result<(), FlowableError> {
        let job_id = job.timer_job_id.clone();
        if job.retries.unwrap_or(1) <= 0
            || matches!(
                job.job_state.as_deref(),
                Some("deadletter" | "history" | "suspended")
            )
        {
            return Err(FlowableError::NotFound(format!(
                "Executable job '{job_id}' not found"
            )));
        }

        let work = TimerWork::RuntimeJob(job);
        let command = ExecuteTimerWorkCmd::new_manual_job(
            work.clone(),
            Arc::new(TimerCoordinationMetrics::new()),
        );
        match self.command_executor.execute(&command) {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(FlowableError::ExecutionError(format!(
                "Executable job '{job_id}' was not executed"
            ))),
            Err(error) => {
                let retry_command = RecordFailedTimerWorkCmd::new(work, &error);
                let _ = self.command_executor.execute(&retry_command);
                Err(error)
            }
        }
    }

    pub fn list_executable_jobs(&self) -> Vec<RuntimeTimerJobState> {
        self.persisted_timer_jobs()
            .into_iter()
            .filter(is_executable_job)
            .collect()
    }

    pub fn find_executable_job_by_id(&self, job_id: &str) -> Option<RuntimeTimerJobState> {
        self.find_job_by_id(job_id).filter(is_executable_job)
    }

    pub fn list_deadletter_jobs(&self) -> Vec<RuntimeTimerJobState> {
        self.persisted_timer_jobs()
            .into_iter()
            .filter(is_deadletter_job)
            .collect()
    }

    pub fn find_deadletter_job_by_id(&self, job_id: &str) -> Option<RuntimeTimerJobState> {
        self.find_job_by_id(job_id).filter(is_deadletter_job)
    }

    pub fn list_suspended_jobs(&self) -> Vec<RuntimeTimerJobState> {
        self.persisted_timer_jobs()
            .into_iter()
            .filter(is_suspended_job)
            .collect()
    }

    pub fn find_suspended_job_by_id(&self, job_id: &str) -> Option<RuntimeTimerJobState> {
        self.find_job_by_id(job_id).filter(is_suspended_job)
    }

    pub fn list_history_jobs(&self) -> Vec<RuntimeTimerJobState> {
        self.persisted_timer_jobs()
            .into_iter()
            .filter(is_history_job)
            .collect()
    }

    pub fn find_history_job_by_id(&self, job_id: &str) -> Option<RuntimeTimerJobState> {
        self.find_job_by_id(job_id).filter(is_history_job)
    }

    pub fn job_process_definition_id(&self, job: &RuntimeTimerJobState) -> Option<String> {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        let store = self.command_executor.runtime_store();
        store
            .find_execution(&job.execution_id, &mut session)
            .and_then(|execution| execution.process_definition_id)
            .or_else(|| {
                store
                    .find_process_instance(&job.process_instance_id, &mut session)
                    .map(|process_instance| process_instance.process_definition_id)
            })
    }

    pub fn job_tenant_id(&self, job: &RuntimeTimerJobState) -> Option<String> {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        let store = self.command_executor.runtime_store();
        store
            .find_execution(&job.execution_id, &mut session)
            .and_then(|execution| execution.tenant_id)
            .or_else(|| {
                store
                    .find_process_instance(&job.process_instance_id, &mut session)
                    .and_then(|process_instance| process_instance.tenant_id)
            })
    }

    pub fn job_element_name(&self, job: &RuntimeTimerJobState) -> Option<String> {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        let store = self.command_executor.runtime_store();
        store
            .find_execution(&job.execution_id, &mut session)
            .and_then(|execution| {
                if execution.activity_id.as_deref() == Some(job.activity_id.as_str()) {
                    execution.activity_name
                } else {
                    None
                }
            })
    }

    pub fn delete_job(&self, job_id: &str) -> Result<(), FlowableError> {
        self.delete_matching_job(
            job_id,
            is_executable_job,
            "Job",
            JobDeletionLockPolicy::RejectLocked,
        )
    }

    pub fn delete_timer_job(&self, job_id: &str) -> Result<(), FlowableError> {
        self.delete_matching_job(
            job_id,
            is_timer_job,
            "Timer job",
            JobDeletionLockPolicy::RejectLocked,
        )
    }

    pub fn delete_deadletter_job(&self, job_id: &str) -> Result<(), FlowableError> {
        self.delete_matching_job(
            job_id,
            is_deadletter_job,
            "Deadletter job",
            JobDeletionLockPolicy::AllowLocked,
        )
    }

    pub fn delete_history_job(&self, job_id: &str) -> Result<(), FlowableError> {
        self.delete_matching_job(
            job_id,
            is_history_job,
            "History job",
            JobDeletionLockPolicy::AllowLocked,
        )
    }

    pub fn delete_suspended_job(&self, job_id: &str) -> Result<(), FlowableError> {
        self.delete_matching_job(
            job_id,
            is_suspended_job,
            "Suspended job",
            JobDeletionLockPolicy::AllowLocked,
        )
    }

    pub fn execute_history_job(&self, job_id: &str) -> Result<(), FlowableError> {
        self.find_history_job_by_id(job_id).ok_or_else(|| {
            FlowableError::NotFound(format!("History job '{}' not found", job_id))
        })?;
        if let Some(handler) = &self.history_job_handler {
            let cmd = ExecuteHistoryJobCmd::new(job_id.to_string(), Arc::clone(handler));
            self.command_executor.execute(&cmd)
        } else {
            let mut session = self
                .command_executor
                .runtime_store()
                .create_session()
                .unwrap();
            self.command_executor
                .runtime_store()
                .delete_timer_job_state(job_id, &mut session);
            session.flush_and_commit().unwrap();
            Ok(())
        }
    }

    /// Java `setJobRetries` / `setTimerJobRetries`: only update the retries
    /// counter on a runtime job. Does **not** move the job to deadletter when
    /// retries become zero (use [`Self::move_job_to_deadletter_job`]) and does
    /// **not** revive a deadletter job (use
    /// [`Self::move_deadletter_job_to_executable_job`]).
    pub fn set_job_retries(
        &self,
        job_id: &str,
        retries: i32,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        if retries < 0 {
            return Err(FlowableError::ExecutionError(
                "Retries must be zero or greater".to_string(),
            ));
        }
        let mut job = self
            .find_job_by_id(job_id)
            .filter(is_runtime_job_for_retry_update)
            .ok_or_else(|| FlowableError::NotFound(format!("Job '{}' not found", job_id)))?;
        job.retries = Some(retries);
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.command_executor
            .runtime_store()
            .insert_timer_job_state(&job, &mut session);
        session.flush_and_commit().unwrap();
        Ok(job)
    }

    pub fn move_timer_to_executable_job(
        &self,
        job_id: &str,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        self.command_executor.execute(&MoveTimerToExecutableJobCmd {
            job_id: job_id.to_string(),
        })
    }

    pub fn move_job_to_deadletter_job(
        &self,
        job_id: &str,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        self.move_job_to_deadletter_job_with_fields(job_id, None, None)
    }

    pub fn move_job_to_deadletter_job_with_fields(
        &self,
        job_id: &str,
        exception_message: Option<String>,
        delete_reason: Option<String>,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        self.command_executor.execute(&MoveJobToDeadletterJobCmd {
            job_id: job_id.to_string(),
            exception_message,
            delete_reason,
        })
    }

    pub fn move_deadletter_job_to_executable_job(
        &self,
        job_id: &str,
        retries: i32,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        self.command_executor.execute(&MoveDeadletterJobCmd {
            job_id: job_id.to_string(),
            retries,
            destination: DeadletterDestination::Executable,
        })
    }

    /// Java REST single deadletter `move` (JobResource): the persisted job
    /// origin decides the destination — history-origin jobs return to the
    /// history family, every other origin becomes executable.
    pub fn move_deadletter_job_by_origin(
        &self,
        job_id: &str,
        retries: i32,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        self.command_executor.execute(&MoveDeadletterJobCmd {
            job_id: job_id.to_string(),
            retries,
            destination: DeadletterDestination::ByOrigin,
        })
    }

    /// Java `bulkMoveDeadLetterJobs`: revive each deadletter job to executable
    /// (or history when the deadletter originated as a history job).
    pub fn bulk_move_deadletter_jobs(
        &self,
        job_ids: &[String],
        retries: i32,
    ) -> Result<(), FlowableError> {
        self.command_executor.execute(&BulkMoveDeadletterJobsCmd {
            job_ids: job_ids.to_vec(),
            retries,
            destination: DeadletterDestination::ByOrigin,
        })
    }

    /// Java `bulkMoveDeadLetterJobsToHistoryJobs`.
    pub fn bulk_move_deadletter_jobs_to_history_jobs(
        &self,
        job_ids: &[String],
        retries: i32,
    ) -> Result<(), FlowableError> {
        self.command_executor.execute(&BulkMoveDeadletterJobsCmd {
            job_ids: job_ids.to_vec(),
            retries,
            destination: DeadletterDestination::History,
        })
    }

    pub fn move_deadletter_job_to_history_job(
        &self,
        job_id: &str,
        retries: i32,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        self.command_executor.execute(&MoveDeadletterJobCmd {
            job_id: job_id.to_string(),
            retries,
            destination: DeadletterDestination::History,
        })
    }

    pub fn move_suspended_job_to_executable_job(
        &self,
        job_id: &str,
        retries: i32,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        if retries <= 0 {
            return Err(FlowableError::ExecutionError(
                "Retries must be greater than zero when moving a suspended job".to_string(),
            ));
        }
        self.execute_suspended_job_activation(job_id, SuspendedJobActivation::Extension { retries })
    }

    /// Activates a suspended job with Flowable Java semantics.
    ///
    /// The retry count is copied unchanged and the persisted job type decides
    /// whether the row returns to the timer, external-worker, or executable
    /// family. The existing two-argument method remains the explicit Rust
    /// extension for callers that also want to replace the retry count.
    pub fn activate_suspended_job(
        &self,
        job_id: &str,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        self.execute_suspended_job_activation(job_id, SuspendedJobActivation::Java)
    }

    fn execute_suspended_job_activation(
        &self,
        job_id: &str,
        activation: SuspendedJobActivation,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        self.command_executor.execute(&MoveSuspendedJobCmd {
            job_id: job_id.to_string(),
            activation,
        })
    }

    pub fn reschedule_timer_job(
        &self,
        job_id: &str,
        time_date: Option<String>,
        time_duration: Option<String>,
        time_cycle: Option<String>,
        end_date: Option<String>,
        calendar_name: Option<String>,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        // Java ManagementServiceImpl.rescheduleTimerJob → RescheduleTimerJobCmd.
        // Due date calculation routes through the business calendar registry so
        // calendarName changes the immediate due date (P64 Task 3).
        self.command_executor
            .execute(&RescheduleTimerJobCmd::new(
                job_id.to_string(),
                time_date,
                time_duration,
                time_cycle,
                end_date,
                calendar_name,
            ))
    }

    fn persisted_timer_jobs(&self) -> Vec<RuntimeTimerJobState> {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.command_executor
            .runtime_store()
            .snapshot_timer_job_states(&mut session)
            .into_values()
            .collect()
    }

    fn delete_matching_job(
        &self,
        job_id: &str,
        predicate: fn(&RuntimeTimerJobState) -> bool,
        job_kind: &str,
        lock_policy: JobDeletionLockPolicy,
    ) -> Result<(), FlowableError> {
        self.command_executor.execute(&DeleteMatchingJobCmd {
            job_id: job_id.to_string(),
            predicate,
            job_kind: job_kind.to_string(),
            lock_policy,
        })
    }
}

#[derive(Clone, Copy)]
enum JobDeletionLockPolicy {
    RejectLocked,
    AllowLocked,
}

struct DeleteMatchingJobCmd {
    job_id: String,
    predicate: fn(&RuntimeTimerJobState) -> bool,
    job_kind: String,
    lock_policy: JobDeletionLockPolicy,
}

struct MoveTimerToExecutableJobCmd {
    job_id: String,
}

struct MoveSuspendedJobCmd {
    job_id: String,
    activation: SuspendedJobActivation,
}

impl Command<RuntimeTimerJobState> for MoveTimerToExecutableJobCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        let store = command_context.runtime_store_handle();
        let mut job = store
            .find_timer_job_state(&self.job_id, &mut command_context.session)
            .filter(is_timer_job)
            .ok_or_else(|| {
                FlowableError::NotFound(format!("Timer job '{}' not found", self.job_id))
            })?;
        let job_type = store
            .find_timer_job_type(&job.timer_job_id, &mut command_context.session)
            .unwrap_or(RuntimeJobType::Timer);
        if job_type == RuntimeJobType::ExternalWorker {
            return Err(FlowableError::NotFound(format!(
                "Timer job '{}' not found",
                self.job_id
            )));
        }

        job.job_state = Some("executable".to_string());
        job.lock_owner = None;
        job.lock_time = None;
        job.lock_expiration_time = None;
        store.insert_timer_job_state_with_type(&job, Some(&job_type), &mut command_context.session);
        Ok(job)
    }
}

impl Command<RuntimeTimerJobState> for MoveSuspendedJobCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        let store = command_context.runtime_store_handle();
        let job = store
            .find_timer_job_state(&self.job_id, &mut command_context.session)
            .filter(is_suspended_job)
            .ok_or_else(|| {
                FlowableError::NotFound(format!("Suspended job '{}' not found", self.job_id))
            })?;

        if matches!(self.activation, SuspendedJobActivation::Java) {
            ensure_suspended_job_parent_is_active(&store, &mut command_context.session, &job)?;
        }
        activate_suspended_job_state(command_context, job, self.activation)
    }
}

struct MoveJobToDeadletterJobCmd {
    job_id: String,
    exception_message: Option<String>,
    delete_reason: Option<String>,
}

impl Command<RuntimeTimerJobState> for MoveJobToDeadletterJobCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        let store = command_context.runtime_store_handle();
        let mut job = store
            .find_timer_job_state(&self.job_id, &mut command_context.session)
            .filter(is_movable_to_deadletter_job)
            .ok_or_else(|| FlowableError::NotFound(format!("Job '{}' not found", self.job_id)))?;

        job.job_state = Some("deadletter".to_string());
        if self.exception_message.is_some() {
            job.error_message.clone_from(&self.exception_message);
        }
        job.lock_owner = None;
        job.lock_time = None;
        job.lock_expiration_time = None;
        store.insert_timer_job_state(&job, &mut command_context.session);

        PersistJobExtraFieldsCmd::new(
            job.clone(),
            vec![("deleteReason".to_string(), self.delete_reason.clone())],
        )
        .execute(command_context)?;
        Ok(job)
    }
}

#[derive(Clone, Copy)]
enum DeadletterDestination {
    Executable,
    History,
    ByOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeadletterOrigin {
    Runtime,
    History,
    ExternalWorker,
}

struct MoveDeadletterJobCmd {
    job_id: String,
    retries: i32,
    destination: DeadletterDestination,
}

impl Command<RuntimeTimerJobState> for MoveDeadletterJobCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        let store = command_context.runtime_store_handle();
        let job = store
            .find_timer_job_state(&self.job_id, &mut command_context.session)
            .filter(is_deadletter_job)
            .ok_or_else(|| {
                FlowableError::NotFound(format!("Deadletter job '{}' not found", self.job_id))
            })?;
        let origin = resolve_deadletter_origin(&store, &mut command_context.session, &job);
        let job = move_deadletter_job(job, self.retries, self.destination, origin)?;
        store.insert_timer_job_state(&job, &mut command_context.session);
        Ok(job)
    }
}

struct BulkMoveDeadletterJobsCmd {
    job_ids: Vec<String>,
    retries: i32,
    destination: DeadletterDestination,
}

impl Command<()> for BulkMoveDeadletterJobsCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        let store = command_context.runtime_store_handle();
        let mut selected_jobs: Vec<(RuntimeTimerJobState, DeadletterOrigin)> = Vec::new();

        // Java queries the deadletter table with an IN clause. Missing ids and
        // duplicate ids therefore do not create errors or duplicate moves.
        for job_id in &self.job_ids {
            if selected_jobs
                .iter()
                .any(|(job, _)| job.timer_job_id == *job_id)
            {
                continue;
            }
            if let Some(job) = store
                .find_timer_job_state(job_id, &mut command_context.session)
                .filter(is_deadletter_job)
            {
                let origin = resolve_deadletter_origin(&store, &mut command_context.session, &job);
                selected_jobs.push((job, origin));
            }
        }

        // Validate and transform the complete selection before staging writes,
        // preserving the all-or-nothing Java command transaction.
        let moved_jobs = selected_jobs
            .into_iter()
            .map(|(job, origin)| move_deadletter_job(job, self.retries, self.destination, origin))
            .collect::<Result<Vec<_>, _>>()?;

        for job in &moved_jobs {
            store.insert_timer_job_state(job, &mut command_context.session);
        }
        Ok(())
    }
}

impl Command<()> for DeleteMatchingJobCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        let store = command_context.runtime_store_handle();
        let job = store
            .find_timer_job_state(&self.job_id, &mut command_context.session)
            .filter(self.predicate)
            .ok_or_else(|| {
                FlowableError::NotFound(format!("{} '{}' not found", self.job_kind, self.job_id))
            })?;

        // Java only applies the execution-lock guard in `DeleteJobCmd` and
        // `DeleteTimerJobCmd`. Suspended, deadletter and history deletion
        // commands do not reject rows merely because lockOwner is populated.
        if matches!(self.lock_policy, JobDeletionLockPolicy::RejectLocked)
            && job.lock_owner.is_some()
        {
            return Err(FlowableError::ExecutionError(format!(
                "Cannot delete {} '{}' when the job is being executed. Try again later.",
                self.job_kind, self.job_id
            )));
        }

        let canceled = EngineEvent::Job {
            event_type: EngineEventType::JobCanceled,
            job,
        };
        let event_dispatcher = command_context.config.engine_event_dispatcher.clone();
        event_dispatcher.dispatch_in_context(&canceled, command_context)?;
        store.delete_timer_job_state(&self.job_id, &mut command_context.session);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeJobTable {
    Timer,
    Executable,
    Deadletter,
    History,
    Suspended,
    Unknown,
}

fn runtime_job_table(job: &RuntimeTimerJobState) -> RuntimeJobTable {
    match job.job_state.as_deref() {
        None | Some("timer") => RuntimeJobTable::Timer,
        Some("executable") | Some("async") | Some("async-after") => RuntimeJobTable::Executable,
        Some("deadletter") => RuntimeJobTable::Deadletter,
        Some("history") => RuntimeJobTable::History,
        Some("suspended") => RuntimeJobTable::Suspended,
        Some(_) => RuntimeJobTable::Unknown,
    }
}

fn is_deadletter_job(job: &RuntimeTimerJobState) -> bool {
    runtime_job_table(job) == RuntimeJobTable::Deadletter
}

fn is_timer_job(job: &RuntimeTimerJobState) -> bool {
    runtime_job_table(job) == RuntimeJobTable::Timer
}

fn is_executable_job(job: &RuntimeTimerJobState) -> bool {
    runtime_job_table(job) == RuntimeJobTable::Executable
}

/// Jobs that Java `setJobRetries` / `setTimerJobRetries` may update.
/// Explicit deadletter / history / suspended rows are excluded.
fn is_runtime_job_for_retry_update(job: &RuntimeTimerJobState) -> bool {
    matches!(
        runtime_job_table(job),
        RuntimeJobTable::Timer | RuntimeJobTable::Executable
    )
}

/// Java MoveJobToDeadLetterJobCmd accepts timer and executable job entities.
fn is_movable_to_deadletter_job(job: &RuntimeTimerJobState) -> bool {
    is_runtime_job_for_retry_update(job)
}

fn resolve_deadletter_origin(
    store: &RuntimeStore,
    session: &mut DbSession,
    job: &RuntimeTimerJobState,
) -> DeadletterOrigin {
    // ExternalWorker origin requires a persisted externalWorker job_type.
    // Event-wait alone must not reclassify ordinary intermediate timers.
    match store.find_timer_job_type(&job.timer_job_id, session) {
        Some(RuntimeJobType::Timer) => DeadletterOrigin::Runtime,
        Some(RuntimeJobType::History) => DeadletterOrigin::History,
        Some(RuntimeJobType::ExternalWorker) => DeadletterOrigin::ExternalWorker,
        Some(RuntimeJobType::Other(_)) => DeadletterOrigin::Runtime,
        None if job.activity_id == "async-history" => DeadletterOrigin::History,
        None => DeadletterOrigin::Runtime,
    }
}

fn ensure_suspended_job_parent_is_active(
    store: &RuntimeStore,
    session: &mut DbSession,
    job: &RuntimeTimerJobState,
) -> Result<(), FlowableError> {
    if job.process_instance_id.is_empty() {
        return Err(FlowableError::ExecutionError(format!(
            "Job {} parent is not process instance",
            job.timer_job_id
        )));
    }
    if store
        .find_process_instance(&job.process_instance_id, session)
        .is_some_and(|process_instance| process_instance.is_suspended)
    {
        return Err(FlowableError::ExecutionError(format!(
            "Can not activate job {}. Parent is suspended.",
            job.timer_job_id
        )));
    }
    Ok(())
}

fn move_deadletter_job(
    job: RuntimeTimerJobState,
    retries: i32,
    destination: DeadletterDestination,
    origin: DeadletterOrigin,
) -> Result<RuntimeTimerJobState, FlowableError> {
    match destination {
        DeadletterDestination::Executable => revive_deadletter_as_executable(job, retries, origin),
        DeadletterDestination::History => revive_deadletter_as_history(job, retries, origin),
        DeadletterDestination::ByOrigin if origin == DeadletterOrigin::History => {
            revive_deadletter_as_history(job, retries, origin)
        }
        DeadletterDestination::ByOrigin => revive_deadletter_as_executable(job, retries, origin),
    }
}

fn revive_deadletter_as_executable(
    mut job: RuntimeTimerJobState,
    retries: i32,
    origin: DeadletterOrigin,
) -> Result<RuntimeTimerJobState, FlowableError> {
    if origin == DeadletterOrigin::History {
        return Err(FlowableError::ExecutionError(
            "Cannot move a history job to an executable job".to_string(),
        ));
    }
    job.job_state = Some(executable_job_state_for_deadletter(&job, origin).to_string());
    prepare_revived_job(&mut job, retries);
    Ok(job)
}

fn revive_deadletter_as_history(
    mut job: RuntimeTimerJobState,
    retries: i32,
    origin: DeadletterOrigin,
) -> Result<RuntimeTimerJobState, FlowableError> {
    if origin != DeadletterOrigin::History {
        return Err(FlowableError::ExecutionError(
            "Can only move a history job to a history job".to_string(),
        ));
    }
    job.job_state = Some("history".to_string());
    prepare_revived_job(&mut job, retries);
    Ok(job)
}

fn prepare_revived_job(job: &mut RuntimeTimerJobState, retries: i32) {
    job.retries = Some(retries);
    job.lock_owner = None;
    job.lock_time = None;
    job.lock_expiration_time = None;
}

fn is_suspended_job(job: &RuntimeTimerJobState) -> bool {
    runtime_job_table(job) == RuntimeJobTable::Suspended
}

fn is_history_job(job: &RuntimeTimerJobState) -> bool {
    runtime_job_table(job) == RuntimeJobTable::History
}

fn executable_job_state_for_deadletter(
    job: &RuntimeTimerJobState,
    origin: DeadletterOrigin,
) -> &'static str {
    if origin == DeadletterOrigin::ExternalWorker {
        "timer"
    } else if job.time_duration.as_deref() == Some(ASYNC_CONTINUATION_JOB_TYPE_MARKER) {
        ASYNC_CONTINUATION_JOB_STATE
    } else {
        "executable"
    }
}

struct ExecuteHistoryJobCmd {
    job_id: String,
    handler: SharedHistoryJobHandler,
}

impl ExecuteHistoryJobCmd {
    fn new(job_id: String, handler: SharedHistoryJobHandler) -> Self {
        Self { job_id, handler }
    }
}

impl Command<()> for ExecuteHistoryJobCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        let job = {
            let (store, session) = command_context.store_and_session();
            store
                .find_timer_job_state(&self.job_id, session)
                .ok_or_else(|| {
                    FlowableError::NotFound(format!("History job '{}' not found", self.job_id))
                })?
        };
        if !is_history_job(&job) {
            return Err(FlowableError::ExecutionError(format!(
                "Job '{}' is not a history job",
                self.job_id
            )));
        }
        self.handler.execute(&job, command_context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::process_engine::ProcessEngine;
    use crate::engine::time_source::TestTimeSource;
    use crate::persistence::db_store::DbStore;
    use crate::runtime::process_instance::ProcessInstance;
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    #[test]
    fn management_service_filters_persisted_job_families() {
        let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap();
        let engine = ProcessEngine::build(
            "management-service-job-filter-test".to_string(),
            Arc::new(TestTimeSource::new(now)),
            Arc::new(DbStore::new_in_memory().unwrap()),
        );
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state(
            &timer_job_state("timer-job", Some("timer"), Some(1)),
            &mut session,
        );
        store.insert_timer_job_state(
            &timer_job_state("zero-retry-timer-job", Some("timer"), Some(0)),
            &mut session,
        );
        store.insert_timer_job_state(
            &timer_job_state("deadletter-job", Some("deadletter"), Some(0)),
            &mut session,
        );
        store.insert_timer_job_state(
            &timer_job_state("history-job", Some("history"), Some(1)),
            &mut session,
        );
        store.insert_timer_job_state(
            &timer_job_state("suspended-job", Some("suspended"), Some(1)),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        let management_service = engine.get_management_service();
        assert_eq!(
            management_service
                .find_timer_job_by_id("timer-job")
                .unwrap()
                .timer_job_id,
            "timer-job"
        );
        assert!(
            management_service
                .find_timer_job_by_id("history-job")
                .is_none()
        );
        assert_eq!(management_service.list_deadletter_jobs().len(), 1);
        assert!(
            management_service
                .find_timer_job_by_id("zero-retry-timer-job")
                .is_some()
        );
        assert!(
            management_service
                .find_deadletter_job_by_id("zero-retry-timer-job")
                .is_none()
        );
        assert_eq!(
            management_service
                .find_history_job_by_id("history-job")
                .unwrap()
                .timer_job_id,
            "history-job"
        );
        assert_eq!(
            management_service
                .find_suspended_job_by_id("suspended-job")
                .unwrap()
                .timer_job_id,
            "suspended-job"
        );
        assert!(
            management_service
                .find_suspended_job_by_id("timer-job")
                .is_none()
        );
    }

    #[test]
    fn persisted_job_type_drives_deadletter_origin_without_activity_id_classification() {
        let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap();
        let engine = ProcessEngine::build(
            "management-service-typed-job-origin-test".to_string(),
            Arc::new(TestTimeSource::new(now)),
            Arc::new(DbStore::new_in_memory().unwrap()),
        );
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();

        let mut typed_history = timer_job_state("typed-history", Some("deadletter"), Some(0));
        typed_history.activity_id = "custom-history-handler".to_string();
        store.insert_timer_job_state_with_type(
            &typed_history,
            Some(&RuntimeJobType::History),
            &mut session,
        );

        let mut misleading_runtime = timer_job_state("typed-runtime", Some("deadletter"), Some(0));
        misleading_runtime.activity_id = "async-history".to_string();
        store.insert_timer_job_state_with_type(
            &misleading_runtime,
            Some(&RuntimeJobType::Other("message".to_string())),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        let management_service = engine.get_management_service();
        let history = management_service
            .move_deadletter_job_to_history_job("typed-history", 2)
            .expect("persisted history job type must allow history restoration");
        assert_eq!(history.job_state.as_deref(), Some("history"));

        let runtime = management_service
            .move_deadletter_job_to_executable_job("typed-runtime", 2)
            .expect("persisted non-history job type must override legacy activity id");
        assert_eq!(runtime.job_state.as_deref(), Some("executable"));
    }

    #[test]
    fn java_suspended_activation_preserves_retries_and_routes_by_job_type() {
        let engine = management_test_engine("management-java-suspended-activation");
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", false), &mut session);

        for (id, job_type, retries) in [
            ("suspended-timer", Some(RuntimeJobType::Timer), 0),
            (
                "suspended-external-worker",
                Some(RuntimeJobType::ExternalWorker),
                -3,
            ),
            (
                "suspended-message",
                Some(RuntimeJobType::Other("message".to_string())),
                4,
            ),
            (
                "suspended-custom",
                Some(RuntimeJobType::Other("customType".to_string())),
                2,
            ),
            ("suspended-history-marker", Some(RuntimeJobType::History), 1),
            ("suspended-untyped", None, 0),
        ] {
            let mut job = timer_job_state(id, Some("suspended"), Some(retries));
            job.due_time = Some(1_777_777_777_777);
            job.lock_owner = Some("stale-owner".to_string());
            job.lock_time = Some(10);
            job.lock_expiration_time = Some(20);
            job.error_message = Some("preserved failure".to_string());
            job.error_details = Some("preserved stacktrace".to_string());
            job.category = Some("preserved-category".to_string());
            store.insert_timer_job_state_with_type(&job, job_type.as_ref(), &mut session);
        }
        session.flush_and_commit().unwrap();

        let management_service = engine.get_management_service();
        for (id, expected_state, expected_retries) in [
            ("suspended-timer", "timer", 0),
            ("suspended-external-worker", "timer", -3),
            ("suspended-message", "executable", 4),
            ("suspended-custom", "executable", 2),
            ("suspended-history-marker", "executable", 1),
            ("suspended-untyped", "executable", 0),
        ] {
            let activated = management_service
                .activate_suspended_job(id)
                .expect("Java-compatible activation should succeed");
            assert_eq!(activated.job_state.as_deref(), Some(expected_state));
            assert_eq!(activated.retries, Some(expected_retries));
            assert_eq!(activated.due_time, Some(1_777_777_777_777));
            assert_eq!(
                activated.error_message.as_deref(),
                Some("preserved failure")
            );
            assert_eq!(
                activated.error_details.as_deref(),
                Some("preserved stacktrace")
            );
            assert_eq!(activated.category.as_deref(), Some("preserved-category"));
            assert!(activated.lock_owner.is_none());
            assert!(activated.lock_time.is_none());
            assert!(activated.lock_expiration_time.is_none());
            assert!(management_service.find_suspended_job_by_id(id).is_none());
        }

        let mut session = store.create_session().unwrap();
        assert_eq!(
            store
                .find_timer_job_type("suspended-external-worker", &mut session)
                .as_ref(),
            Some(&RuntimeJobType::ExternalWorker)
        );
        assert_eq!(
            store
                .find_timer_job_type("suspended-custom", &mut session)
                .as_ref(),
            Some(&RuntimeJobType::Other("customType".to_string()))
        );
        session.rollback().unwrap();
    }

    #[test]
    fn suspended_activation_rejects_suspended_parent_without_mutating_job() {
        let engine = management_test_engine("management-suspended-parent-rollback");
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", true), &mut session);
        let mut job = timer_job_state("blocked-suspended-job", Some("suspended"), Some(0));
        job.lock_owner = Some("existing-owner".to_string());
        store.insert_timer_job_state_with_type(&job, Some(&RuntimeJobType::Timer), &mut session);
        session.flush_and_commit().unwrap();

        let error = engine
            .get_management_service()
            .activate_suspended_job("blocked-suspended-job")
            .expect_err("a suspended parent must reject activation");
        assert!(matches!(error, FlowableError::ExecutionError(_)));
        assert_eq!(
            error.to_string(),
            "Execution error: Can not activate job blocked-suspended-job. Parent is suspended."
        );

        let persisted = engine
            .get_management_service()
            .find_suspended_job_by_id("blocked-suspended-job")
            .expect("failed activation must leave the suspended job intact");
        assert_eq!(persisted.retries, Some(0));
        assert_eq!(persisted.lock_owner.as_deref(), Some("existing-owner"));
    }

    #[test]
    fn java_suspended_activation_rejects_missing_process_parent() {
        let engine = management_test_engine("management-suspended-missing-parent");
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let mut job = timer_job_state("missing-parent-job", Some("suspended"), Some(-2));
        job.process_instance_id.clear();
        store.insert_timer_job_state_with_type(&job, Some(&RuntimeJobType::Timer), &mut session);
        session.flush_and_commit().unwrap();

        let error = engine
            .get_management_service()
            .activate_suspended_job("missing-parent-job")
            .expect_err("Java-compatible activation requires a process parent");
        assert_eq!(
            error.to_string(),
            "Execution error: Job missing-parent-job parent is not process instance"
        );
        let persisted = engine
            .get_management_service()
            .find_suspended_job_by_id("missing-parent-job")
            .expect("parent validation failure must leave the job suspended");
        assert_eq!(persisted.retries, Some(-2));
    }

    #[test]
    fn suspended_activation_is_rolled_back_with_the_enclosing_command() {
        struct ActivateThenFailCmd;

        impl Command<()> for ActivateThenFailCmd {
            fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
                MoveSuspendedJobCmd {
                    job_id: "rollback-job".to_string(),
                    activation: SuspendedJobActivation::Java,
                }
                .execute(command_context)?;
                Err(FlowableError::ExecutionError(
                    "forced rollback after activation".to_string(),
                ))
            }
        }

        let engine = management_test_engine("management-suspended-command-rollback");
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", false), &mut session);
        let mut original = timer_job_state("rollback-job", Some("suspended"), Some(0));
        original.lock_owner = Some("preserved-owner".to_string());
        original.lock_time = Some(11);
        original.lock_expiration_time = Some(22);
        original.error_message = Some("preserved error".to_string());
        store.insert_timer_job_state_with_type(
            &original,
            Some(&RuntimeJobType::Timer),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        let error = engine
            .get_command_executor()
            .execute(&ActivateThenFailCmd)
            .expect_err("the enclosing command should force rollback");
        assert_eq!(
            error.to_string(),
            "Execution error: forced rollback after activation"
        );

        let persisted = engine
            .get_management_service()
            .find_suspended_job_by_id("rollback-job")
            .expect("rollback must restore suspended table membership");
        assert_eq!(persisted.retries, original.retries);
        assert_eq!(persisted.lock_owner, original.lock_owner);
        assert_eq!(persisted.lock_time, original.lock_time);
        assert_eq!(
            persisted.lock_expiration_time,
            original.lock_expiration_time
        );
        assert_eq!(persisted.error_message, original.error_message);
        let mut session = store.create_session().unwrap();
        assert_eq!(
            store.find_timer_job_type("rollback-job", &mut session),
            Some(RuntimeJobType::Timer)
        );
        session.rollback().unwrap();
    }

    #[test]
    fn suspended_activation_extension_keeps_retry_override_contract() {
        let engine = management_test_engine("management-suspended-extension");
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", false), &mut session);
        for id in [
            "extension-success",
            "extension-invalid",
            "extension-parent-suspended",
        ] {
            store.insert_timer_job_state_with_type(
                &timer_job_state(id, Some("suspended"), Some(0)),
                Some(&RuntimeJobType::Timer),
                &mut session,
            );
        }
        session.flush_and_commit().unwrap();

        let management_service = engine.get_management_service();
        let activated = management_service
            .move_suspended_job_to_executable_job("extension-success", 5)
            .expect("existing extension should still override retries");
        assert_eq!(activated.job_state.as_deref(), Some("executable"));
        assert_eq!(activated.retries, Some(5));

        let error = management_service
            .move_suspended_job_to_executable_job("extension-invalid", 0)
            .expect_err("existing extension should still reject non-positive retries");
        assert!(matches!(error, FlowableError::ExecutionError(_)));
        let persisted = management_service
            .find_suspended_job_by_id("extension-invalid")
            .expect("invalid extension request must not move the job");
        assert_eq!(persisted.retries, Some(0));

        let missing_error = management_service
            .move_suspended_job_to_executable_job("missing", 0)
            .expect_err("retry validation must keep its pre-lookup precedence");
        assert_eq!(
            missing_error.to_string(),
            "Execution error: Retries must be greater than zero when moving a suspended job"
        );

        let negative_error = management_service
            .move_suspended_job_to_executable_job("missing", -1)
            .expect_err("negative retries must retain the extension validation contract");
        assert_eq!(negative_error.to_string(), missing_error.to_string());

        let mut session = store.create_session().unwrap();
        store.update_process_instance(&process_instance("process-1", true), &mut session);
        session.flush_and_commit().unwrap();
        let activated = management_service
            .move_suspended_job_to_executable_job("extension-parent-suspended", 3)
            .expect("the existing extension did not validate parent suspension");
        assert_eq!(activated.job_state.as_deref(), Some("executable"));
        assert_eq!(activated.retries, Some(3));
    }

    #[test]
    fn java_suspended_activation_only_accepts_suspended_table_membership() {
        let engine = management_test_engine("management-suspended-source-validation");
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", false), &mut session);
        for (id, state) in [
            ("source-timer", "timer"),
            ("source-executable", "executable"),
            ("source-deadletter", "deadletter"),
            ("source-history", "history"),
        ] {
            store.insert_timer_job_state(&timer_job_state(id, Some(state), Some(1)), &mut session);
        }
        session.flush_and_commit().unwrap();

        let management_service = engine.get_management_service();
        for id in [
            "missing",
            "source-timer",
            "source-executable",
            "source-deadletter",
            "source-history",
        ] {
            let error = management_service
                .activate_suspended_job(id)
                .expect_err("only suspended table membership may be activated");
            assert!(matches!(error, FlowableError::NotFound(_)));
        }
        assert!(
            management_service
                .find_timer_job_by_id("source-timer")
                .is_some()
        );
        assert!(
            management_service
                .find_executable_job_by_id("source-executable")
                .is_some()
        );
        assert!(
            management_service
                .find_deadletter_job_by_id("source-deadletter")
                .is_some()
        );
        assert!(
            management_service
                .find_history_job_by_id("source-history")
                .is_some()
        );
    }

    /// Install a submit handle on the engine's activation coordinator that
    /// records the jobs offered to it (instead of touching a real executor) and
    /// returns whichever [`HintSubmitOutcome`] the test configures. Returns the
    /// shared record so the test can inspect what was hinted after commit.
    fn record_hints(
        engine: &ProcessEngine,
        outcome: crate::engine::activation_coordinator::HintSubmitOutcome,
    ) -> Arc<std::sync::Mutex<Vec<RuntimeTimerJobState>>> {
        let recorded: Arc<std::sync::Mutex<Vec<RuntimeTimerJobState>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&recorded);
        engine
            .get_config()
            .activation_coordinator
            .set_submit_handle(Arc::new(move |job: RuntimeTimerJobState| {
                sink.lock().unwrap().push(job);
                outcome.clone()
            }));
        recorded
    }

    #[test]
    fn active_executor_prelocks_row_and_hints_on_commit() {
        use crate::engine::activation_coordinator::HintSubmitOutcome;

        let engine = management_test_engine("management-activation-active-prelock");
        let coordinator = engine.get_config().activation_coordinator.clone();
        let owner = coordinator.lock_owner();
        assert!(!owner.is_empty(), "engine must resolve an executor owner");
        let lock_ms = coordinator.async_job_lock_ms();
        // 2026-04-21 12:00:00 UTC as configured by management_test_engine.
        let now_ms = Utc
            .with_ymd_and_hms(2026, 4, 21, 12, 0, 0)
            .unwrap()
            .timestamp_millis();

        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", false), &mut session);
        // A message job activates to the executable family — async-eligible.
        store.insert_timer_job_state_with_type(
            &timer_job_state("active-message", Some("suspended"), Some(4)),
            Some(&RuntimeJobType::Other("message".to_string())),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        // Executor is live and its submit handle records offered jobs.
        coordinator.active_flag().store(true, Ordering::SeqCst);
        let recorded = record_hints(&engine, HintSubmitOutcome::Submitted);

        let activated = engine
            .get_management_service()
            .activate_suspended_job("active-message")
            .expect("activation should succeed while executor is live");

        assert_eq!(activated.job_state.as_deref(), Some("executable"));
        // The persisted row carries the executor pre-lock (owner + expiration).
        let persisted = engine
            .get_management_service()
            .find_executable_job_by_id("active-message")
            .expect("pre-locked executable job must be persisted");
        assert_eq!(persisted.lock_owner.as_deref(), Some(owner.as_str()));
        assert_eq!(persisted.lock_time, Some(now_ms));
        assert_eq!(persisted.lock_expiration_time, Some(now_ms + lock_ms));
        assert_eq!(persisted.retries, Some(4));

        // The committed hint fired exactly once, carrying the pre-locked row.
        let hints = recorded.lock().unwrap();
        assert_eq!(hints.len(), 1, "exactly one committed hint per activation");
        assert_eq!(hints[0].timer_job_id, "active-message");
        assert_eq!(hints[0].lock_owner.as_deref(), Some(owner.as_str()));
        assert_eq!(hints[0].lock_expiration_time, Some(now_ms + lock_ms));
    }

    #[test]
    fn inactive_executor_neither_prelocks_nor_hints() {
        use crate::engine::activation_coordinator::HintSubmitOutcome;

        let engine = management_test_engine("management-activation-inactive");
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", false), &mut session);
        store.insert_timer_job_state_with_type(
            &timer_job_state("inactive-message", Some("suspended"), Some(2)),
            Some(&RuntimeJobType::Other("message".to_string())),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        // Executor is NOT active (default). Record a handle anyway to prove it is
        // never called.
        let recorded = record_hints(&engine, HintSubmitOutcome::Submitted);

        let activated = engine
            .get_management_service()
            .activate_suspended_job("inactive-message")
            .expect("activation should succeed while executor is inactive");

        assert_eq!(activated.job_state.as_deref(), Some("executable"));
        // No pre-lock: the polling acquisition owns this job.
        assert!(activated.lock_owner.is_none());
        assert!(activated.lock_time.is_none());
        assert!(activated.lock_expiration_time.is_none());
        let persisted = engine
            .get_management_service()
            .find_executable_job_by_id("inactive-message")
            .expect("activated job must be persisted and unlocked");
        assert!(persisted.lock_owner.is_none());
        assert!(persisted.lock_expiration_time.is_none());

        assert!(
            recorded.lock().unwrap().is_empty(),
            "an inactive executor must never be hinted"
        );
    }

    #[test]
    fn active_executor_with_category_mismatch_prelocks_but_does_not_hint() {
        use crate::engine::activation_coordinator::HintSubmitOutcome;

        let engine = management_test_engine("management-activation-category-mismatch");
        let coordinator = engine.get_config().activation_coordinator.clone();
        let owner = coordinator.lock_owner();
        // Only "urgent" jobs are hinted by this node.
        coordinator.configure(
            owner.clone(),
            coordinator.async_job_lock_ms(),
            vec!["urgent".to_string()],
            Vec::new(),
        );
        coordinator.active_flag().store(true, Ordering::SeqCst);

        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", false), &mut session);
        // Job carries a category NOT in the enabled list.
        let mut job = timer_job_state("bulk-message", Some("suspended"), Some(1));
        job.category = Some("bulk".to_string());
        store.insert_timer_job_state_with_type(
            &job,
            Some(&RuntimeJobType::Other("message".to_string())),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        let recorded = record_hints(&engine, HintSubmitOutcome::Submitted);

        engine
            .get_management_service()
            .activate_suspended_job("bulk-message")
            .expect("activation should succeed");

        // Java pre-locks regardless of category (isAsyncExecutorActive alone).
        let persisted = engine
            .get_management_service()
            .find_executable_job_by_id("bulk-message")
            .expect("category-mismatched job must still be pre-locked");
        assert_eq!(persisted.lock_owner.as_deref(), Some(owner.as_str()));
        assert!(persisted.lock_expiration_time.is_some());

        // ...but the hint is left to another node (category not enabled here).
        assert!(
            recorded.lock().unwrap().is_empty(),
            "a category-mismatched job must be pre-locked but not hinted"
        );
    }

    #[test]
    fn rollback_discards_the_pending_activation_hint() {
        use crate::engine::activation_coordinator::HintSubmitOutcome;

        struct ActivateThenFailCmd;
        impl Command<()> for ActivateThenFailCmd {
            fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
                MoveSuspendedJobCmd {
                    job_id: "rollback-hint-job".to_string(),
                    activation: SuspendedJobActivation::Java,
                }
                .execute(command_context)?;
                Err(FlowableError::ExecutionError(
                    "forced rollback after activation".to_string(),
                ))
            }
        }

        let engine = management_test_engine("management-activation-rollback-hint");
        let coordinator = engine.get_config().activation_coordinator.clone();
        coordinator.active_flag().store(true, Ordering::SeqCst);
        let recorded = record_hints(&engine, HintSubmitOutcome::Submitted);

        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", false), &mut session);
        store.insert_timer_job_state_with_type(
            &timer_job_state("rollback-hint-job", Some("suspended"), Some(3)),
            Some(&RuntimeJobType::Other("message".to_string())),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        let error = engine
            .get_command_executor()
            .execute(&ActivateThenFailCmd)
            .expect_err("the enclosing command should force rollback");
        assert_eq!(
            error.to_string(),
            "Execution error: forced rollback after activation"
        );

        // The row is still suspended (rolled back) and NO hint was enqueued: the
        // command executor only drains hints after a successful commit.
        let persisted = engine
            .get_management_service()
            .find_suspended_job_by_id("rollback-hint-job")
            .expect("rollback must restore suspended membership");
        assert_eq!(persisted.job_state.as_deref(), Some("suspended"));
        assert!(
            recorded.lock().unwrap().is_empty(),
            "a rolled-back activation must never enqueue a committed hint"
        );
    }

    #[test]
    fn fatal_committed_activation_hint_error_is_propagated_after_commit() {
        use crate::engine::activation_coordinator::HintSubmitOutcome;

        let engine = management_test_engine("management-activation-fatal-hint");
        let coordinator = engine.get_config().activation_coordinator.clone();
        coordinator.active_flag().store(true, Ordering::SeqCst);
        record_hints(
            &engine,
            HintSubmitOutcome::Fatal(FlowableError::ExecutionError(
                "fatal committed hint listener".to_string(),
            )),
        );

        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance("process-1", false), &mut session);
        store.insert_timer_job_state_with_type(
            &timer_job_state("fatal-hint-job", Some("suspended"), Some(3)),
            Some(&RuntimeJobType::Other("message".to_string())),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        let error = engine
            .get_management_service()
            .activate_suspended_job("fatal-hint-job")
            .expect_err("fatal committed hint errors must reach the caller");
        assert_eq!(
            error.to_string(),
            "Execution error: fatal committed hint listener"
        );

        let persisted = engine
            .get_management_service()
            .find_executable_job_by_id("fatal-hint-job")
            .expect("the database transaction was already committed");
        assert_eq!(
            persisted.lock_owner.as_deref(),
            Some(coordinator.lock_owner().as_str())
        );
        assert!(persisted.lock_expiration_time.is_some());
    }

    fn management_test_engine(name: &str) -> ProcessEngine {
        let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap();
        ProcessEngine::build(
            name.to_string(),
            Arc::new(TestTimeSource::new(now)),
            Arc::new(DbStore::new_in_memory().unwrap()),
        )
    }

    fn process_instance(id: &str, is_suspended: bool) -> ProcessInstance {
        ProcessInstance {
            id: id.to_string(),
            name: None,
            process_definition_id: "definition-1".to_string(),
            process_definition_key: "definition".to_string(),
            process_definition_name: None,
            process_definition_version: 1,
            business_key: None,
            business_status: None,
            is_suspended,
            tenant_id: None,
            start_time: None,
            start_user_id: None,
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
            is_ended: false,
            super_execution_id: None,
            root_process_instance_id: Some(id.to_string()),
        }
    }

    fn timer_job_state(
        id: &str,
        job_state: Option<&str>,
        retries: Option<i32>,
    ) -> RuntimeTimerJobState {
        RuntimeTimerJobState {
            timer_job_id: id.to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-1".to_string(),
            activity_id: "activity-1".to_string(),
            job_state: job_state.map(str::to_string),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(1_775_000_000_000),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries,
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        }
    }
}
