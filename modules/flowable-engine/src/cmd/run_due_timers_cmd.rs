use crate::agenda::FlowableEngineAgenda;
use crate::agenda::continue_process_operation::{
    ASYNC_AFTER_JOB_STATE, ASYNC_AFTER_JOB_TYPE_MARKER, ASYNC_AFTER_RESUME_FLAG,
    ASYNC_CONTINUATION_JOB_STATE, ASYNC_CONTINUATION_JOB_TYPE_MARKER,
    ASYNC_CONTINUATION_RESUME_FLAG,
};
use crate::cmd::start_process_instance_cmd::StartProcessInstanceCmd;
use crate::cmd::trigger_boundary_event_cmd::TriggerTimerBoundaryEventCmd;
use crate::cmd::trigger_intermediate_catch_event_cmd::TriggerTimerIntermediateCatchEventCmd;
use crate::engine::event_dispatcher::{EngineEvent, EngineEventType};
use crate::engine::timer_worker::TimerWork;
use crate::history::async_history_job_handler::HistoryJobHandler;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    AcquisitionWritePolicy, ExpiredJobClass, JobLockEligibility, ResetExpiredJobsBatchOutcome,
    RuntimeTimerJobState,
};
use crate::runtime::execution::Execution;
use crate::runtime::process_instance_builder::ProcessInstanceBuilder;
use serde_json::Value;
use uuid::Uuid;

use crate::engine::timer_worker::TimerCoordinationMetrics;
use crate::persistence::runtime_store::TimerWorkerNode;
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub struct HeartbeatTimerNodeCmd {
    node_id: Arc<str>,
    worker_type: String,
}

impl HeartbeatTimerNodeCmd {
    pub fn new(node_id: Arc<str>, worker_type: String) -> Self {
        Self {
            node_id,
            worker_type,
        }
    }
}

impl Command<()> for HeartbeatTimerNodeCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let _dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let now = store.time_source().now().timestamp_millis();
        let node = TimerWorkerNode {
            node_id: self.node_id.to_string(),
            last_heartbeat: now,
            worker_type: self.worker_type.clone(),
        };
        store.insert_timer_worker_node(node, session);
        Ok(())
    }
}

pub struct AcquireCoordinatorLeaseCmd {
    node_id: Arc<str>,
    timeout_ms: i64,
}

impl AcquireCoordinatorLeaseCmd {
    pub fn new(node_id: Arc<str>, timeout_ms: u64) -> Self {
        Self {
            node_id,
            timeout_ms: timeout_ms as i64,
        }
    }
}

impl Command<Option<i64>> for AcquireCoordinatorLeaseCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<i64>, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let _dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let now = store.time_source().now().timestamp_millis();
        Ok(store.acquire_coordinator_lease(
            "timer-coordinator",
            self.node_id.as_ref(),
            now,
            self.timeout_ms,
            session,
        ))
    }
}

pub struct ReleaseCoordinatorLeaseCmd {
    node_id: Arc<str>,
    fencing_token: i64,
}

impl ReleaseCoordinatorLeaseCmd {
    pub fn new(node_id: Arc<str>, fencing_token: i64) -> Self {
        Self {
            node_id,
            fencing_token,
        }
    }
}

impl Command<bool> for ReleaseCoordinatorLeaseCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<bool, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let _dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        Ok(store.release_coordinator_lease(
            "timer-coordinator",
            self.node_id.as_ref(),
            self.fencing_token,
            session,
        ))
    }
}

pub struct AcquireGlobalLockCmd {
    lock_name: String,
    owner: String,
    lease_ms: i64,
    now_ms: Option<i64>,
}

impl AcquireGlobalLockCmd {
    pub fn new(lock_name: String, owner: String, lease_ms: i64) -> Self {
        Self {
            lock_name,
            owner,
            lease_ms,
            now_ms: None,
        }
    }

    pub fn new_at(lock_name: String, owner: String, lease_ms: i64, now_ms: i64) -> Self {
        Self {
            lock_name,
            owner,
            lease_ms,
            now_ms: Some(now_ms),
        }
    }
}

impl Command<bool> for AcquireGlobalLockCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<bool, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let session = command_context.session();
        let now = self
            .now_ms
            .unwrap_or_else(|| store.time_source().now().timestamp_millis());
        store
            .try_acquire_property_lock(&self.lock_name, &self.owner, now, self.lease_ms, session)
            .map_err(crate::error::FlowableError::from)
    }
}

pub struct ReleaseGlobalLockCmd {
    lock_name: String,
    owner: String,
}

impl ReleaseGlobalLockCmd {
    pub fn new(lock_name: String, owner: String) -> Self {
        Self { lock_name, owner }
    }
}

impl Command<bool> for ReleaseGlobalLockCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<bool, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let session = command_context.session();
        store
            .release_property_lock(&self.lock_name, &self.owner, session)
            .map_err(crate::error::FlowableError::from)
    }
}

pub struct AcquireTimerWorkCmd {
    owner_id: Arc<str>,
    fencing_token: i64,
    metrics: Arc<TimerCoordinationMetrics>,
    /// Empty = all tenants.
    tenant_ids: Vec<String>,
    /// Empty = no category filtering. Non-empty = only jobs with category in this list are acquired.
    enabled_job_categories: Vec<String>,
    write_policy: AcquisitionWritePolicy,
    scheduled_timers_only: bool,
    max_jobs: Option<usize>,
}

impl AcquireTimerWorkCmd {
    pub fn new(
        owner_id: Arc<str>,
        fencing_token: i64,
        metrics: Arc<TimerCoordinationMetrics>,
    ) -> Self {
        Self {
            owner_id,
            fencing_token,
            metrics,
            tenant_ids: Vec::new(),
            enabled_job_categories: Vec::new(),
            write_policy: AcquisitionWritePolicy::Optimistic,
            scheduled_timers_only: false,
            max_jobs: None,
        }
    }

    pub fn with_tenant_ids(mut self, tenant_ids: Vec<String>) -> Self {
        self.tenant_ids = tenant_ids;
        self
    }

    pub fn with_enabled_job_categories(mut self, categories: Vec<String>) -> Self {
        self.enabled_job_categories = categories;
        self
    }

    pub(crate) fn serialized_by_global_lock(mut self) -> Self {
        self.write_policy = AcquisitionWritePolicy::SerializedByGlobalLock;
        self
    }

    pub fn scheduled_timers_only(mut self) -> Self {
        self.scheduled_timers_only = true;
        self
    }

    pub fn with_max_jobs(mut self, max_jobs: usize) -> Self {
        self.max_jobs = Some(max_jobs);
        self
    }
}

#[derive(Clone, Debug)]
enum ScheduledTimerCandidateKind {
    RuntimeTimer(String),
    ProcessStart(String),
    EventSubprocess(String),
}

#[derive(Clone, Debug)]
struct ScheduledTimerCandidate {
    due_time: i64,
    stable_id: String,
    kind: ScheduledTimerCandidateKind,
}

impl ScheduledTimerCandidateKind {
    fn rank(&self) -> u8 {
        match self {
            Self::RuntimeTimer(_) => 0,
            Self::ProcessStart(_) => 1,
            Self::EventSubprocess(_) => 2,
        }
    }
}

fn select_scheduled_timer_candidates(
    store: &crate::persistence::runtime_store::RuntimeStore,
    deployment_manager: &crate::engine::deployment_manager::DeploymentManager,
    now: i64,
    lock_timeout_ms: i64,
    tenant_filter: Option<&[String]>,
    category_filter: Option<&[String]>,
    max_jobs: usize,
    eligibility: JobLockEligibility,
    session: &mut crate::persistence::db_session::DbSession,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut candidates = Vec::new();

    candidates.extend(
        store
            .find_due_scheduled_timer_job_candidates_filtered(
                now,
                lock_timeout_ms,
                tenant_filter,
                category_filter,
                session,
            )
            .into_iter()
            .filter(|job| {
                !matches!(eligibility, JobLockEligibility::UnlockedOnly) || job.lock_owner.is_none()
            })
            .filter_map(|job| {
                job.due_time.map(|due_time| ScheduledTimerCandidate {
                    due_time,
                    stable_id: job.timer_job_id.clone(),
                    kind: ScheduledTimerCandidateKind::RuntimeTimer(job.timer_job_id),
                })
            }),
    );
    candidates.extend(
        deployment_manager
            .find_due_process_timer_start_subscription_candidates(
                now,
                lock_timeout_ms,
                category_filter,
                session,
            )
            .into_iter()
            .filter(|subscription| {
                !matches!(eligibility, JobLockEligibility::UnlockedOnly)
                    || subscription.lock_owner.is_none()
            })
            .filter_map(|subscription| {
                subscription
                    .due_time
                    .map(|due_time| ScheduledTimerCandidate {
                        due_time,
                        stable_id: subscription.id.clone(),
                        kind: ScheduledTimerCandidateKind::ProcessStart(subscription.id),
                    })
            }),
    );
    candidates.extend(
        store
            .find_due_event_subprocess_timer_subscription_candidates(
                now,
                lock_timeout_ms,
                category_filter,
                session,
            )
            .into_iter()
            .filter(|subscription| {
                !matches!(eligibility, JobLockEligibility::UnlockedOnly)
                    || subscription.lock_owner.is_none()
            })
            .filter_map(|subscription| {
                subscription
                    .due_time
                    .map(|due_time| ScheduledTimerCandidate {
                        due_time,
                        stable_id: subscription.subscription_id.clone(),
                        kind: ScheduledTimerCandidateKind::EventSubprocess(
                            subscription.subscription_id,
                        ),
                    })
            }),
    );

    candidates.sort_by(|left, right| {
        left.due_time
            .cmp(&right.due_time)
            .then_with(|| left.stable_id.cmp(&right.stable_id))
            .then_with(|| left.kind.rank().cmp(&right.kind.rank()))
    });

    let mut runtime_timer_ids = Vec::new();
    let mut process_start_ids = Vec::new();
    let mut event_subprocess_ids = Vec::new();
    for candidate in candidates.into_iter().take(max_jobs) {
        match candidate.kind {
            ScheduledTimerCandidateKind::RuntimeTimer(id) => runtime_timer_ids.push(id),
            ScheduledTimerCandidateKind::ProcessStart(id) => process_start_ids.push(id),
            ScheduledTimerCandidateKind::EventSubprocess(id) => event_subprocess_ids.push(id),
        }
    }

    (runtime_timer_ids, process_start_ids, event_subprocess_ids)
}

impl Command<Vec<TimerWork>> for AcquireTimerWorkCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<TimerWork>, crate::error::FlowableError> {
        let lock_timeout_ms = command_context
            .config
            .async_executor
            .timer_lock_time_ms
            .min(i64::MAX as u64) as i64;
        let store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let lease_opt = store.find_timer_coordinator_lease("timer-coordinator", session);
        if let Some(lease) = lease_opt {
            if lease.fencing_token != self.fencing_token
                || lease.owner_node_id != self.owner_id.as_ref()
            {
                return Ok(Vec::new());
            }
        } else {
            return Ok(Vec::new());
        }

        let now = store.time_source().now().timestamp_millis();

        let mut works = Vec::new();

        let tenant_filter = if self.tenant_ids.is_empty() {
            None
        } else {
            Some(self.tenant_ids.as_slice())
        };
        let category_filter = if self.enabled_job_categories.is_empty() {
            None
        } else {
            Some(self.enabled_job_categories.as_slice())
        };
        let selected_candidate_ids = if self.scheduled_timers_only {
            self.max_jobs.map(|max_jobs| {
                select_scheduled_timer_candidates(
                    &store,
                    &dm,
                    now,
                    lock_timeout_ms,
                    tenant_filter,
                    category_filter,
                    max_jobs,
                    // Expired locks require reset first for both optimistic and
                    // serialized acquisition paths.
                    JobLockEligibility::UnlockedOnly,
                    session,
                )
            })
        } else {
            None
        };

        let (due_timers, r1, c1) = match selected_candidate_ids.as_ref() {
            Some((runtime_timer_ids, _, _)) if runtime_timer_ids.is_empty() => (Vec::new(), 0, 0),
            Some((runtime_timer_ids, _, _)) => match self.write_policy {
                AcquisitionWritePolicy::Optimistic => store
                    .acquire_selected_scheduled_timer_jobs_filtered(
                        self.owner_id.as_ref(),
                        now,
                        lock_timeout_ms,
                        runtime_timer_ids,
                        tenant_filter,
                        category_filter,
                        session,
                    )?,
                AcquisitionWritePolicy::SerializedByGlobalLock => store
                    .acquire_selected_scheduled_timer_jobs_global_filtered(
                        self.owner_id.as_ref(),
                        now,
                        lock_timeout_ms,
                        runtime_timer_ids,
                        tenant_filter,
                        category_filter,
                        session,
                    )?,
            },
            None if self.scheduled_timers_only => store.acquire_due_scheduled_timer_jobs_filtered(
                self.owner_id.as_ref(),
                now,
                lock_timeout_ms,
                100,
                tenant_filter,
                category_filter,
                session,
            )?,
            None => store.try_acquire_due_timer_jobs_filtered(
                self.owner_id.as_ref(),
                now,
                lock_timeout_ms,
                tenant_filter,
                category_filter,
                session,
            )?,
        };
        for timer in due_timers {
            works.push(TimerWork::RuntimeJob(timer));
        }

        let (timer_start_subscriptions, r2, c2) = match selected_candidate_ids.as_ref() {
            Some((_, process_start_ids, _)) if process_start_ids.is_empty() => (Vec::new(), 0, 0),
            Some((_, process_start_ids, _)) => match self.write_policy {
                AcquisitionWritePolicy::Optimistic => dm
                    .acquire_selected_process_timer_start_subscriptions(
                        self.owner_id.as_ref(),
                        now,
                        lock_timeout_ms,
                        process_start_ids,
                        category_filter,
                        session,
                    ),
                AcquisitionWritePolicy::SerializedByGlobalLock => dm
                    .acquire_selected_process_timer_start_subscriptions_global(
                        self.owner_id.as_ref(),
                        now,
                        lock_timeout_ms,
                        process_start_ids,
                        category_filter,
                        session,
                    )?,
            },
            None => dm.acquire_due_process_timer_start_subscriptions_filtered(
                self.owner_id.as_ref(),
                now,
                lock_timeout_ms,
                category_filter,
                session,
            ),
        };
        for sub in timer_start_subscriptions {
            works.push(TimerWork::ProcessStart(sub));
        }

        let (due_event_sub_timer_subs, r3, c3) = match selected_candidate_ids.as_ref() {
            Some((_, _, event_subprocess_ids)) if event_subprocess_ids.is_empty() => {
                (Vec::new(), 0, 0)
            }
            Some((_, _, event_subprocess_ids)) => match self.write_policy {
                AcquisitionWritePolicy::Optimistic => store
                    .acquire_selected_event_subprocess_timer_subscriptions(
                        self.owner_id.as_ref(),
                        now,
                        lock_timeout_ms,
                        event_subprocess_ids,
                        category_filter,
                        session,
                    ),
                AcquisitionWritePolicy::SerializedByGlobalLock => store
                    .acquire_selected_event_subprocess_timer_subscriptions_global(
                        self.owner_id.as_ref(),
                        now,
                        lock_timeout_ms,
                        event_subprocess_ids,
                        category_filter,
                        session,
                    )?,
            },
            None => store.acquire_due_event_subprocess_timer_subscriptions_filtered(
                self.owner_id.as_ref(),
                now,
                lock_timeout_ms,
                category_filter,
                session,
            ),
        };
        for sub in due_event_sub_timer_subs {
            works.push(TimerWork::EventSubprocess(sub));
        }

        let total_recovered = r1 + r2 + r3;
        let total_conflicts = c1 + c2 + c3;
        let total_acquired = works.len();
        debug_assert_eq!(
            total_recovered, 0,
            "timer acquisition must not recover expired leases; reset owns that path"
        );
        let _ = total_recovered;

        self.metrics
            .acquire_conflicts
            .fetch_add(total_conflicts, Ordering::Relaxed);
        self.metrics
            .acquire_attempts
            .fetch_add(total_acquired + total_conflicts, Ordering::Relaxed);
        self.metrics
            .jobs_acquired
            .fetch_add(total_acquired, Ordering::Relaxed);
        self.metrics
            .last_acquire_batch_size
            .store(total_acquired, Ordering::Relaxed);

        works.sort_by_key(|w| w.due_time().unwrap_or(0));

        Ok(works)
    }
}

pub struct AcquireAsyncJobsCmd {
    owner_id: Arc<str>,
    lock_duration_ms: i64,
    max_jobs: usize,
    metrics: Arc<TimerCoordinationMetrics>,
    /// Empty = all tenants.
    tenant_ids: Vec<String>,
    /// Empty = no category filtering. Non-empty = only jobs with category in this list are acquired.
    enabled_job_categories: Vec<String>,
    write_policy: AcquisitionWritePolicy,
}

impl AcquireAsyncJobsCmd {
    pub fn new(
        owner_id: Arc<str>,
        lock_duration_ms: i64,
        max_jobs: usize,
        metrics: Arc<TimerCoordinationMetrics>,
    ) -> Self {
        Self {
            owner_id,
            lock_duration_ms,
            max_jobs,
            metrics,
            tenant_ids: Vec::new(),
            enabled_job_categories: Vec::new(),
            write_policy: AcquisitionWritePolicy::Optimistic,
        }
    }

    pub fn with_tenant_ids(mut self, tenant_ids: Vec<String>) -> Self {
        self.tenant_ids = tenant_ids;
        self
    }

    pub fn with_enabled_job_categories(mut self, categories: Vec<String>) -> Self {
        self.enabled_job_categories = categories;
        self
    }

    pub(crate) fn serialized_by_global_lock(mut self) -> Self {
        self.write_policy = AcquisitionWritePolicy::SerializedByGlobalLock;
        self
    }
}

impl Command<Vec<RuntimeTimerJobState>> for AcquireAsyncJobsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<RuntimeTimerJobState>, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let _dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let now = store.time_source().now().timestamp_millis();
        let tenant_filter = if self.tenant_ids.is_empty() {
            None
        } else {
            Some(self.tenant_ids.as_slice())
        };
        let category_filter = if self.enabled_job_categories.is_empty() {
            None
        } else {
            Some(self.enabled_job_categories.as_slice())
        };
        let (jobs, recovered, conflicts) = match self.write_policy {
            AcquisitionWritePolicy::Optimistic => store.acquire_due_async_timer_jobs_filtered(
                self.owner_id.as_ref(),
                now,
                self.lock_duration_ms,
                self.max_jobs,
                tenant_filter,
                category_filter,
                session,
            )?,
            AcquisitionWritePolicy::SerializedByGlobalLock => store
                .acquire_due_async_timer_jobs_global_filtered(
                    self.owner_id.as_ref(),
                    now,
                    self.lock_duration_ms,
                    self.max_jobs,
                    tenant_filter,
                    category_filter,
                    session,
                )?,
        };
        debug_assert_eq!(
            recovered, 0,
            "acquisition must not recover expired leases; reset owns that path"
        );
        let _ = recovered;
        self.metrics
            .acquire_conflicts
            .fetch_add(conflicts, Ordering::Relaxed);
        self.metrics
            .acquire_attempts
            .fetch_add(jobs.len() + conflicts, Ordering::Relaxed);
        self.metrics
            .jobs_acquired
            .fetch_add(jobs.len(), Ordering::Relaxed);
        self.metrics
            .last_acquire_batch_size
            .store(jobs.len(), Ordering::Relaxed);
        Ok(jobs)
    }
}

pub struct AcquireHistoryJobsCmd {
    owner_id: Arc<str>,
    lock_duration_ms: i64,
    max_jobs: usize,
    metrics: Arc<TimerCoordinationMetrics>,
}

impl AcquireHistoryJobsCmd {
    pub fn new(
        owner_id: Arc<str>,
        lock_duration_ms: i64,
        max_jobs: usize,
        metrics: Arc<TimerCoordinationMetrics>,
    ) -> Self {
        Self {
            owner_id,
            lock_duration_ms,
            max_jobs,
            metrics,
        }
    }
}

impl Command<Vec<RuntimeTimerJobState>> for AcquireHistoryJobsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<RuntimeTimerJobState>, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let _dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let now = store.time_source().now().timestamp_millis();
        let (jobs, recovered, conflicts) = store.acquire_due_history_jobs(
            self.owner_id.as_ref(),
            now,
            self.lock_duration_ms,
            self.max_jobs,
            session,
        )?;
        debug_assert_eq!(
            recovered, 0,
            "history acquisition must not recover expired leases; reset owns that path"
        );
        let _ = recovered;
        self.metrics
            .acquire_conflicts
            .fetch_add(conflicts, Ordering::Relaxed);
        self.metrics
            .acquire_attempts
            .fetch_add(jobs.len() + conflicts, Ordering::Relaxed);
        self.metrics
            .jobs_acquired
            .fetch_add(jobs.len(), Ordering::Relaxed);
        self.metrics
            .last_acquire_batch_size
            .store(jobs.len(), Ordering::Relaxed);
        Ok(jobs)
    }
}

pub struct ReleaseTimerJobLockCmd {
    timer_job_id: String,
    owner_id: Arc<str>,
}

impl ReleaseTimerJobLockCmd {
    pub fn new(timer_job_id: String, owner_id: Arc<str>) -> Self {
        Self {
            timer_job_id,
            owner_id,
        }
    }
}

impl Command<bool> for ReleaseTimerJobLockCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<bool, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let _dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        store
            .release_timer_job_lock(&self.timer_job_id, self.owner_id.as_ref(), session)
            .map_err(crate::error::FlowableError::from)
    }
}

/// Java `LockExclusiveJobCmd.java:55-62` — a separate command (own transaction)
/// that takes the exclusive process-instance scope lock before an exclusive job
/// is executed. Lock owner / expiration mirror
/// `DefaultInternalJobManager.lockJobScopeInternal` (:184-215): prefer the
/// values already stamped on the acquired job row, else fall back to the
/// executor's lock owner and `now + asyncJobLockTimeInMillis`.
pub struct LockExclusiveJobScopeCmd {
    job: RuntimeTimerJobState,
    fallback_owner: Arc<str>,
}

impl LockExclusiveJobScopeCmd {
    pub fn new(job: RuntimeTimerJobState, fallback_owner: Arc<str>) -> Self {
        Self {
            job,
            fallback_owner,
        }
    }
}

impl Command<bool> for LockExclusiveJobScopeCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<bool, crate::error::FlowableError> {
        let async_job_lock_time_ms = command_context
            .config
            .async_executor
            .async_job_lock_time_ms
            .min(i64::MAX as u64) as i64;
        let store = command_context.runtime_store_handle();
        let session = command_context.session();

        let now = store.time_source().now().timestamp_millis();
        // DefaultInternalJobManager.java:189-199: job's own lock owner/expiration
        // (written by the acquire) win over the executor fallback.
        let lock_owner = self
            .job
            .lock_owner
            .clone()
            .unwrap_or_else(|| self.fallback_owner.as_ref().to_string());
        let lock_expiration = self
            .job
            .lock_expiration_time
            .unwrap_or(now + async_job_lock_time_ms);
        Ok(store.lock_process_instance(
            &self.job.process_instance_id,
            &lock_owner,
            lock_expiration,
            now,
            session,
        ))
    }
}

pub struct ReleaseAcquiredTimerWorkLockCmd {
    work: TimerWork,
    owner_id: Arc<str>,
}

impl ReleaseAcquiredTimerWorkLockCmd {
    pub fn new(work: TimerWork, owner_id: Arc<str>) -> Self {
        Self { work, owner_id }
    }
}

impl Command<bool> for ReleaseAcquiredTimerWorkLockCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<bool, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let deployment_manager = command_context.deployment_manager_handle();
        let session = command_context.session();

        match &self.work {
            TimerWork::RuntimeJob(job) => store
                .release_timer_job_lock(&job.timer_job_id, self.owner_id.as_ref(), session)
                .map_err(crate::error::FlowableError::from),
            TimerWork::ProcessStart(subscription) => deployment_manager
                .release_process_timer_start_subscription_lock(
                    subscription,
                    self.owner_id.as_ref(),
                    session,
                )
                .map_err(crate::error::FlowableError::from),
            TimerWork::EventSubprocess(subscription) => store
                .release_event_subprocess_timer_subscription_lock(
                    subscription,
                    self.owner_id.as_ref(),
                    session,
                )
                .map_err(crate::error::FlowableError::from),
        }
    }
}

pub struct ResetExpiredJobsBatchCmd {
    job_class: ExpiredJobClass,
    page_size: usize,
    tenant_ids: Vec<String>,
    enabled_job_categories: Vec<String>,
}

impl ResetExpiredJobsBatchCmd {
    pub fn new(job_class: ExpiredJobClass, page_size: usize) -> Self {
        Self {
            job_class,
            page_size,
            tenant_ids: Vec::new(),
            enabled_job_categories: Vec::new(),
        }
    }

    pub fn with_tenant_ids(mut self, tenant_ids: Vec<String>) -> Self {
        self.tenant_ids = tenant_ids;
        self
    }

    pub fn with_enabled_job_categories(mut self, enabled_job_categories: Vec<String>) -> Self {
        self.enabled_job_categories = enabled_job_categories;
        self
    }
}

impl Command<ResetExpiredJobsBatchOutcome> for ResetExpiredJobsBatchCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ResetExpiredJobsBatchOutcome, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let session = command_context.session();
        let now = store.time_source().now().timestamp_millis();
        let tenant_filter = if self.tenant_ids.is_empty() {
            None
        } else {
            Some(self.tenant_ids.as_slice())
        };
        let category_filter = if self.enabled_job_categories.is_empty() {
            None
        } else {
            Some(self.enabled_job_categories.as_slice())
        };
        store
            .reset_expired_job_locks_batch(
                now,
                self.job_class,
                self.page_size,
                tenant_filter,
                category_filter,
                session,
            )
            .map_err(crate::error::FlowableError::from)
    }
}

pub struct ResetExpiredTimerJobLocksCmd {
    page_size: usize,
}

impl ResetExpiredTimerJobLocksCmd {
    pub fn new(page_size: usize) -> Self {
        Self { page_size }
    }
}

impl Command<usize> for ResetExpiredTimerJobLocksCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<usize, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let _dm = command_context.deployment_manager_handle();
        let session = command_context.session();

        let now = store.time_source().now().timestamp_millis();
        Ok(store.reset_expired_timer_job_locks(now, self.page_size, session))
    }
}

/// Manual `RuntimeService::execute_timer_job_by_id` path: dispatch `TIMER_FIRED`
/// then trigger the boundary/intermediate timer (Java
/// `TriggerTimerEventJobHandler.java:44-46` before `planTriggerExecutionOperation`).
pub struct ExecuteTimerJobWithFiredEventCmd {
    job: RuntimeTimerJobState,
}

impl ExecuteTimerJobWithFiredEventCmd {
    pub fn new(job: RuntimeTimerJobState) -> Self {
        Self { job }
    }
}

impl Command<()> for ExecuteTimerJobWithFiredEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // P119: TIMER_FIRED — Java TriggerTimerEventJobHandler.java:44-46.
        crate::engine::event_dispatcher::dispatch_timer_fired(command_context, &self.job);
        if self.job.is_boundary {
            let cmd = TriggerTimerBoundaryEventCmd::new(
                self.job.activity_id.clone(),
                self.job.process_instance_id.clone(),
            );
            cmd.execute(command_context)
        } else {
            let cmd =
                TriggerTimerIntermediateCatchEventCmd::new(self.job.execution_id.clone());
            cmd.execute(command_context)
        }
    }
}

pub struct ExecuteTimerWorkCmd {
    work: TimerWork,
    owner_id: Arc<str>,
    fencing_token: i64,
    metrics: Arc<TimerCoordinationMetrics>,
    require_acquired_lock: bool,
    /// Set when the job was pre-locked by the active async executor and handed
    /// over by a post-commit hint (Java `JobAddedTransactionListener`). Such a
    /// job carries a valid executor *row* lock (owner + expiration), so the
    /// timer-coordinator lease is not consulted; instead the row owner and lock
    /// expiration are re-verified before execution.
    direct_hint: bool,
}

impl ExecuteTimerWorkCmd {
    pub fn new(
        work: TimerWork,
        owner_id: Arc<str>,
        fencing_token: i64,
        metrics: Arc<TimerCoordinationMetrics>,
    ) -> Self {
        Self {
            work,
            owner_id,
            fencing_token,
            metrics,
            require_acquired_lock: true,
            direct_hint: false,
        }
    }

    pub fn new_manual_async_job(work: TimerWork, metrics: Arc<TimerCoordinationMetrics>) -> Self {
        Self::new_manual_job(work, metrics)
    }

    /// Manual management-service execution of any runtime job (async, timer, …).
    /// Skips coordinator lease and acquired-lock ownership checks; still runs
    /// the same handler dispatch path as the automatic executor.
    pub fn new_manual_job(work: TimerWork, metrics: Arc<TimerCoordinationMetrics>) -> Self {
        Self {
            work,
            owner_id: Arc::from("manual-job-execution"),
            fencing_token: 0,
            metrics,
            require_acquired_lock: false,
            direct_hint: false,
        }
    }

    /// Execute a job pre-locked by the active async executor and delivered via a
    /// post-commit hint. `owner_id` is the executor lock owner that pre-locked
    /// the row. The coordinator lease is skipped only after the re-read row is
    /// confirmed to still be owned by `owner_id` with a non-expired lock.
    pub fn new_direct_hint(
        work: TimerWork,
        owner_id: Arc<str>,
        metrics: Arc<TimerCoordinationMetrics>,
    ) -> Self {
        Self {
            work,
            owner_id,
            fencing_token: 0,
            metrics,
            require_acquired_lock: true,
            direct_hint: true,
        }
    }
}

fn timer_work_is_async_job(work: &TimerWork) -> bool {
    let TimerWork::RuntimeJob(job) = work else {
        return false;
    };
    is_async_continuation_job(job) || is_async_after_job(job) || is_history_job(job)
}

fn is_history_job(job: &RuntimeTimerJobState) -> bool {
    job.job_state.as_deref() == Some("history")
}

impl Command<Option<String>> for ExecuteTimerWorkCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<String>, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();

        // A direct-hint job was pre-locked by the active async executor and
        // holds a valid executor *row* lock (owner + expiration); it never went
        // through the timer coordinator, so the lease is not consulted (the row
        // lock is re-verified below instead). Async jobs offered with token 0
        // likewise use row locks, not the coordinator lease.
        // Manual management execute (`require_acquired_lock = false`) and
        // pre-locked direct hints do not hold a timer-coordinator lease.
        let skip_coordinator_lease = self.direct_hint
            || !self.require_acquired_lock
            || (self.fencing_token == 0 && timer_work_is_async_job(&self.work));
        if !skip_coordinator_lease {
            let lease_opt = store
                .find_timer_coordinator_lease("timer-coordinator", &mut command_context.session);
            if let Some(lease) = lease_opt {
                if lease.fencing_token != self.fencing_token
                    || lease.owner_node_id != self.owner_id.as_ref()
                {
                    self.metrics
                        .acquire_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                }
            } else {
                self.metrics
                    .acquire_conflicts
                    .fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
        }

        match &self.work {
            TimerWork::RuntimeJob(timer) => {
                // A concurrent executor (or a prior successful execute) may have
                // already deleted the row between acquire/offer and this command.
                // Panic here used to surface as an intermittent test failure /
                // executor-thread crash under full-suite load; treat as a benign
                // acquire conflict (no double-execution).
                let Some(current_job) =
                    store.find_timer_job_state(&timer.timer_job_id, &mut command_context.session)
                else {
                    if self.require_acquired_lock
                        && timer.exclusive
                        && !timer.process_instance_id.is_empty()
                    {
                        store.clear_process_instance_lock(
                            &timer.process_instance_id,
                            &mut command_context.session,
                        );
                    }
                    self.metrics
                        .acquire_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                };
                // Direct-hint jobs bypass the coordinator lease, so the executor
                // row lock is the only guard. Re-verify the re-read row still
                // carries this executor's owner and a non-expired lock before
                // executing; a concurrent reset-expired or re-acquisition may have
                // taken the row over since the hint was queued. On mismatch the
                // job is left for whoever now owns it (no double-execution).
                if self.direct_hint {
                    let now = store.time_source().now().timestamp_millis();
                    let same_lease = current_job.lock_owner.as_deref()
                        == Some(self.owner_id.as_ref())
                        && current_job.lock_owner == timer.lock_owner
                        && current_job.lock_time == timer.lock_time
                        && current_job.lock_expiration_time == timer.lock_expiration_time;
                    let lock_live = current_job
                        .lock_expiration_time
                        .map(|expiration| expiration > now)
                        .unwrap_or(false);
                    if !same_lease || !lock_live {
                        if current_job.exclusive && !current_job.process_instance_id.is_empty() {
                            store.clear_process_instance_lock(
                                &current_job.process_instance_id,
                                &mut command_context.session,
                            );
                        }
                        self.metrics
                            .acquire_conflicts
                            .fetch_add(1, Ordering::Relaxed);
                        return Ok(None);
                    }
                }
                // Shared HistoryJobDispatcher submits post-commit history jobs with
                // fencing_token == 0 and no prior acquire lock. Allow that path so
                // shared async history can drain without a polling acquire.
                let unlocked_shared_history =
                    self.fencing_token == 0 && is_history_job(&current_job);
                // Manual management execute does not pre-lock the row.
                let unlocked_manual = !self.require_acquired_lock;
                if !unlocked_shared_history
                    && !unlocked_manual
                    && current_job.lock_owner.as_deref() != Some(self.owner_id.as_ref())
                {
                    if current_job.exclusive && !current_job.process_instance_id.is_empty() {
                        store.clear_process_instance_lock(
                            &current_job.process_instance_id,
                            &mut command_context.session,
                        );
                    }
                    self.metrics
                        .acquire_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                }

                // Scheduled process-definition suspend/activate timers carry the
                // definition id in `execution_id` and are dispatched through the
                // shared command so the real timer worker and the manual
                // `RuntimeService::execute_timer_job_by_id` path share exactly one
                // transactional implementation (definition + instances + timer
                // roll back together). Without this branch the worker would treat
                // the definition id as an intermediate execution id.
                if let Some(suspended) =
                    crate::cmd::process_definition_suspension::scheduled_process_definition_suspended(
                        &current_job,
                    )
                {
                    let cmd = crate::cmd::process_definition_suspension::ExecuteScheduledProcessDefinitionActionCmd::new(
                        current_job.clone(),
                        suspended,
                    );
                    cmd.execute(command_context)?;
                    let execution_success = EngineEvent::Job {
                        event_type: EngineEventType::JobExecutionSuccess,
                        job: current_job,
                    };
                    command_context.add_post_agenda_event(execution_success);
                    self.metrics
                        .execute_count_runtime_job
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(Some(timer.timer_job_id.clone()));
                }

                // Java DefaultJobManager.executeTimerJob: if dueDate is after endDate,
                // delete the timer without firing.
                if current_job.due_time.is_some()
                    && !crate::engine::time_source::is_valid_due_millis(
                        current_job.due_time.unwrap_or_default(),
                        current_job.end_date.as_deref(),
                    )
                    && !is_async_continuation_job(&current_job)
                    && !is_async_after_job(&current_job)
                    && !is_history_job(&current_job)
                    && !is_set_async_variables_job(&current_job)
                    && !is_async_complete_call_activity_job(&current_job)
                {
                    store.delete_timer_job_state(
                        &current_job.timer_job_id,
                        &mut command_context.session,
                    );
                    return Ok(None);
                }

                if is_set_async_variables_job(&current_job) {
                    // Dispatched by handler type (Java `jobHandlerType`), ahead of the
                    // job-state checks: these jobs share the "async" job state with
                    // async continuations but carry their own payload.
                    crate::engine::variable_service::execute_set_async_variables_job(
                        command_context,
                        &current_job,
                    )?;
                } else if crate::engine::history_cleaning::is_bpmn_history_cleanup_job(&current_job)
                {
                    // Java BpmnHistoryCleanupJobHandler.execute (BpmnHistoryCleanupJobHandler.java:37-57).
                    // No process instance / execution; sync-deletes old historic PIs
                    // then reschedules the repeating cleanup timer (cron/R cycle).
                    crate::engine::history_cleaning::execute_history_cleanup(command_context)?;
                    crate::engine::history_cleaning::reschedule_history_cleanup_timer(
                        command_context,
                        &current_job,
                    )?;
                } else if is_async_complete_call_activity_job(&current_job) {
                    // Java AsyncCompleteCallActivityJobHandler.execute (44-47):
                    // dispatched by jobHandlerType, ends the child PI synchronously.
                    crate::bpmn::behavior::end_event_activity_behavior::
                        execute_async_complete_call_activity_job(command_context, &current_job)?;
                } else if is_async_continuation_job(&current_job) {
                    if self.require_acquired_lock {
                        command_context.set_automatic_job_for_future_success(current_job.clone());
                    }
                    execute_async_continuation_job(command_context, &current_job, &store)?;
                } else if is_async_after_job(&current_job) {
                    execute_async_after_job(command_context, &current_job, &store)?;
                } else if is_history_job(&current_job) {
                    let handler = crate::history::async_history_job_handler::AsyncHistoryJobHandler;
                    handler.execute(&current_job, command_context)?;
                } else if timer.is_boundary {
                    // P119: TIMER_FIRED before boundary trigger —
                    // Java TriggerTimerEventJobHandler.java:44-46.
                    crate::engine::event_dispatcher::dispatch_timer_fired(
                        command_context,
                        &current_job,
                    );
                    // Propagate trigger failures as retriable job errors
                    // (do not panic the worker — Java retries / dead-letters).
                    let cmd = TriggerTimerBoundaryEventCmd::new(
                        timer.activity_id.clone(),
                        timer.process_instance_id.clone(),
                    );
                    cmd.execute(command_context)?;
                } else {
                    // P119: TIMER_FIRED for intermediate / catch timers —
                    // Java TriggerTimerEventJobHandler.java:44-46.
                    crate::engine::event_dispatcher::dispatch_timer_fired(
                        command_context,
                        &current_job,
                    );
                    let cmd =
                        TriggerTimerIntermediateCatchEventCmd::new(timer.execution_id.clone());
                    cmd.execute(command_context)?;
                }
                let execution_success = EngineEvent::Job {
                    event_type: EngineEventType::JobExecutionSuccess,
                    job: current_job.clone(),
                };
                // Java ExecuteAsyncRunnable.java:199-204: unlock == true for the
                // executor path — the exclusive PI scope lock is released in the
                // *same transaction* as the successful job execution. Manual
                // management execute (require_acquired_lock = false) never took
                // the scope lock (Java ExecuteJobCmd has no exclusive handling).
                if self.require_acquired_lock
                    && current_job.exclusive
                    && !current_job.process_instance_id.is_empty()
                {
                    store.clear_process_instance_lock(
                        &current_job.process_instance_id,
                        &mut command_context.session,
                    );
                }
                command_context.add_post_agenda_event(execution_success);
                self.metrics
                    .execute_count_runtime_job
                    .fetch_add(1, Ordering::Relaxed);
                Ok(Some(timer.timer_job_id.clone()))
            }
            TimerWork::ProcessStart(sub) => {
                let Some(current_sub) = dm
                    .get_timer_start_subscriptions(&mut command_context.session)
                    .into_iter()
                    .find(|s| s.id == sub.id)
                else {
                    self.metrics
                        .acquire_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                };
                if current_sub.lock_owner.as_deref() != Some(self.owner_id.as_ref()) {
                    self.metrics
                        .acquire_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                }

                // Java DefaultJobManager.executeTimerJob: skip fire when due is past endDate.
                if let Some(due) = current_sub.due_time
                    && !crate::engine::time_source::is_valid_due_millis(
                        due,
                        current_sub.end_date.as_deref(),
                    )
                {
                    dm.reschedule_or_release_process_timer_start_subscription(
                        &current_sub,
                        None,
                        &mut command_context.session,
                    );
                    return Ok(None);
                }

                // Java TimerStartEventJobHandler: silently skip when the process
                // definition is suspended; cycle subscriptions still reschedule.
                let definition_suspended = dm
                    .get_process_definitions(&mut command_context.session)
                    .get(&sub.process_definition_id)
                    .map(|d| d.is_suspended)
                    .unwrap_or(false);

                let started_instance_id = if definition_suspended {
                    tracing::debug!(
                        process_definition_id = %sub.process_definition_id,
                        "ignoring timer of suspended process definition"
                    );
                    None
                } else {
                    let builder = ProcessInstanceBuilder::new()
                        .process_definition_id(sub.process_definition_id.clone());

                    let started = StartProcessInstanceCmd::with_start_event_id(
                        builder,
                        sub.start_event_id.clone(),
                    )
                    .execute(command_context)?;
                    Some(started.id)
                };

                // timeCycle start events reschedule (Java TimerJobSchedulerImpl).
                // Duration/date one-shots retire the subscription (due_time=None).
                // Suspended definitions still reschedule so the next fire can retry.
                // P64: route through the business calendar registry (and re-evaluate
                // raw calendarName). Deploy-time subscriptions have no process
                // variable scope, so EL calendar names use an empty execution.
                let next = if let Some(cycle) = current_sub.time_cycle.as_deref() {
                    let now = command_context.runtime_store.time_source().now();
                    let calendars = command_context.config.business_calendar_registry.clone();
                    crate::bpmn::timer_util::resolve_next_timer_schedule(
                        cycle,
                        current_sub.end_date.as_deref(),
                        current_sub.calendar_name.as_ref(),
                        &crate::runtime::execution::Execution::default(),
                        &calendars,
                        now,
                    )?
                } else {
                    None
                };
                dm.reschedule_or_release_process_timer_start_subscription(
                    &current_sub,
                    next,
                    &mut command_context.session,
                );

                self.metrics
                    .execute_count_process_start
                    .fetch_add(1, Ordering::Relaxed);

                Ok(Some(match started_instance_id {
                    Some(id) => format!(
                        "timer_start:{}:{}:{}",
                        id, sub.process_definition_key, sub.start_event_id
                    ),
                    None => format!(
                        "timer_start_skipped_suspended:{}:{}",
                        sub.process_definition_key, sub.start_event_id
                    ),
                }))
            }
            TimerWork::EventSubprocess(sub) => {
                let current_subs = store
                    .find_event_subprocess_timer_subscriptions_by_process_instance_id(
                        &sub.process_instance_id,
                        &mut command_context.session,
                    );
                let Some(current_sub) = current_subs
                    .iter()
                    .find(|s| s.subscription_id == sub.subscription_id)
                    .cloned()
                else {
                    self.metrics
                        .acquire_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                };
                if current_sub.lock_owner.as_deref() != Some(self.owner_id.as_ref()) {
                    self.metrics
                        .acquire_conflicts
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                }

                // endDate past due → drop without activating (Java parity).
                if let Some(due) = current_sub.due_time
                    && !crate::engine::time_source::is_valid_due_millis(
                        due,
                        current_sub.end_date.as_deref(),
                    )
                {
                    store.delete_event_subprocess_timer_subscription(
                        &sub.subscription_id,
                        &mut command_context.session,
                    );
                    return Ok(None);
                }

                if sub.interrupting {
                    activate_interrupting_event_subprocess(command_context, sub, &store);
                } else {
                    activate_non_interrupting_event_subprocess(command_context, sub, &store);
                }

                // Non-interrupting + timeCycle: keep and reschedule
                // (Java TimerUtil.repeat = !isInterruptingTimer; TimerEventSubprocessTest
                // testNonInterruptingMultipleInstances with R3/P1D).
                // On exhaust, retire with due_time=None (do not delete): the ESP user-task
                // path re-runs event-subprocess registration and would otherwise recreate
                // a fresh R-cycle from the BPMN model.
                let should_reschedule = !sub.interrupting && current_sub.time_cycle.is_some();
                if should_reschedule {
                    let now = command_context.runtime_store.time_source().now();
                    let mut next_sub = current_sub.clone();
                    next_sub.lock_owner = None;
                    next_sub.lock_time = None;
                    // Re-evaluate calendarName against the process instance scope
                    // so a changed variable can select a different calendar (ADR-2).
                    let execution = store
                        .find_execution(&sub.process_instance_id, &mut command_context.session)
                        .unwrap_or_default();
                    let calendars = command_context.config.business_calendar_registry.clone();
                    if let Some(schedule) = current_sub
                        .time_cycle
                        .as_deref()
                        .map(|cycle| {
                            crate::bpmn::timer_util::resolve_next_timer_schedule(
                                cycle,
                                current_sub.end_date.as_deref(),
                                current_sub.calendar_name.as_ref(),
                                &execution,
                                &calendars,
                                now,
                            )
                        })
                        .transpose()?
                        .flatten()
                    {
                        next_sub.time_cycle = Some(schedule.cycle);
                        next_sub.due_time = Some(schedule.due_time_millis);
                    } else {
                        next_sub.due_time = None;
                    }
                    store.insert_event_subprocess_timer_subscription(
                        next_sub,
                        &mut command_context.session,
                    );
                } else {
                    store.delete_event_subprocess_timer_subscription(
                        &sub.subscription_id,
                        &mut command_context.session,
                    );
                }

                self.metrics
                    .execute_count_event_subprocess
                    .fetch_add(1, Ordering::Relaxed);

                Ok(Some(format!(
                    "event_subprocess_timer:{}:{}",
                    sub.process_instance_id, sub.start_event_id
                )))
            }
        }
    }
}

fn is_set_async_variables_job(job: &RuntimeTimerJobState) -> bool {
    job.handler_type.as_deref()
        == Some(crate::persistence::runtime_store::job_handler_types::SET_ASYNC_VARIABLES)
}

// Java AsyncCompleteCallActivityJobHandler.TYPE = "async-complete-call-actiivty"
// (original Java misspelling is part of the wire contract).
fn is_async_complete_call_activity_job(job: &RuntimeTimerJobState) -> bool {
    job.handler_type.as_deref()
        == Some(crate::persistence::runtime_store::job_handler_types::ASYNC_COMPLETE_CALL_ACTIVITY)
}

fn is_async_continuation_job(job: &RuntimeTimerJobState) -> bool {
    job.job_state.as_deref() == Some(ASYNC_CONTINUATION_JOB_STATE)
        || job.time_duration.as_deref() == Some(ASYNC_CONTINUATION_JOB_TYPE_MARKER)
}

fn is_async_after_job(job: &RuntimeTimerJobState) -> bool {
    job.job_state.as_deref() == Some(ASYNC_AFTER_JOB_STATE)
        || job.time_duration.as_deref() == Some(ASYNC_AFTER_JOB_TYPE_MARKER)
}

fn execute_async_continuation_job(
    command_context: &mut CommandContext,
    job: &RuntimeTimerJobState,
    store: &crate::persistence::runtime_store::RuntimeStore,
) -> Result<(), crate::error::FlowableError> {
    let mut execution = (*command_context.execution_entity_manager)
        .find_by_id(&job.execution_id, &mut command_context.session)
        .ok_or_else(|| {
            // ExecutionError (not NotFound) so REST job execute maps to 500 like Java.
            crate::error::FlowableError::ExecutionError(format!(
                "Execution '{}' for async continuation job '{}' not found",
                job.execution_id, job.timer_job_id
            ))
        })?;

    store.delete_timer_job_state(&job.timer_job_id, &mut command_context.session);

    execution.is_active = true;
    execution.is_ended = false;
    execution.transient_variables.insert(
        ASYNC_CONTINUATION_RESUME_FLAG.to_string(),
        Value::Bool(true),
    );
    command_context
        .agenda
        .plan_continue_process_operation(execution);

    Ok(())
}

fn execute_async_after_job(
    command_context: &mut CommandContext,
    job: &RuntimeTimerJobState,
    store: &crate::persistence::runtime_store::RuntimeStore,
) -> Result<(), crate::error::FlowableError> {
    let mut execution = (*command_context.execution_entity_manager)
        .find_by_id(&job.execution_id, &mut command_context.session)
        .ok_or_else(|| {
            crate::error::FlowableError::ExecutionError(format!(
                "Execution '{}' for async-after job '{}' not found",
                job.execution_id, job.timer_job_id
            ))
        })?;

    store.delete_timer_job_state(&job.timer_job_id, &mut command_context.session);

    execution.is_active = true;
    execution.is_ended = false;
    execution
        .transient_variables
        .insert(ASYNC_AFTER_RESUME_FLAG.to_string(), Value::Bool(true));
    command_context
        .agenda
        .plan_take_outgoing_sequence_flows_operation(execution);

    Ok(())
}

fn activate_interrupting_event_subprocess(
    command_context: &mut CommandContext,
    sub: &crate::persistence::runtime_store::EventSubprocessTimerSubscription,
    store: &crate::persistence::runtime_store::RuntimeStore,
) {
    store.delete_event_wait_states_by_process_instance_id(
        &sub.process_instance_id,
        &mut command_context.session,
    );

    let host_executions: Vec<_> = store
        .snapshot_executions(&mut command_context.session)
        .into_values()
        .filter(|e| e.process_instance_id.as_deref() == Some(&sub.process_instance_id))
        .filter(|e| e.is_active)
        .collect();

    for exec in &host_executions {
        (*command_context.execution_entity_manager).delete(&exec.id, &mut command_context.session);
    }

    store.delete_timer_job_states_by_process_instance_id(
        &sub.process_instance_id,
        &mut command_context.session,
    );

    store.delete_boundary_event_states_by_process_instance_id(
        &sub.process_instance_id,
        &mut command_context.session,
    );

    inject_event_subprocess_execution(command_context, sub, store);
}

fn activate_non_interrupting_event_subprocess(
    command_context: &mut CommandContext,
    sub: &crate::persistence::runtime_store::EventSubprocessTimerSubscription,
    store: &crate::persistence::runtime_store::RuntimeStore,
) {
    inject_event_subprocess_execution(command_context, sub, store);
}

fn inject_event_subprocess_execution(
    command_context: &mut CommandContext,
    sub: &crate::persistence::runtime_store::EventSubprocessTimerSubscription,
    store: &crate::persistence::runtime_store::RuntimeStore,
) {
    let process_instance =
        match store.find_process_instance(&sub.process_instance_id, &mut command_context.session) {
            Some(pi) => pi,
            None => {
                tracing::error!(
                    "Process instance {} not found for event subprocess timer activation",
                    sub.process_instance_id
                );
                return;
            }
        };

    let process_definition_id = process_instance.process_definition_id.clone();
    // Seed from the process-instance scope execution row: it is the single
    // process-level variable store.
    let process_variables = command_context
        .runtime_store
        .find_execution(&process_instance.id, &mut command_context.session)
        .map(|root_execution| root_execution.variables)
        .unwrap_or_default();

    let start_event_execution = Execution {
        id: Uuid::new_v4().to_string(),
        parent_id: Some(process_instance.id.clone()),
        super_execution_id: None,
        root_process_instance_id: Some(process_instance.id.clone()),
        process_instance_id: Some(process_instance.id.clone()),
        process_definition_id: Some(process_definition_id),
        process_definition_key: Some(process_instance.process_definition_key.clone()),
        process_definition_name: None,
        process_definition_version: Some(process_instance.process_definition_version),
        activity_id: Some(sub.start_event_id.clone()),
        activity_name: None,
        name: None,
        description: None,
        is_suspended: false,
        is_ended: false,
        is_active: true,
        is_concurrent: false,
        is_scope: true,
        is_multi_instance_root: false,
        tenant_id: process_instance.tenant_id.clone(),
        variables: process_variables,
        ..Default::default()
    };

    (*command_context.execution_entity_manager)
        .insert(&start_event_execution, &mut command_context.session);

    command_context
        .agenda
        .plan_continue_process_operation(start_event_execution);
}

#[cfg(test)]
mod acquisition_limit_tests {
    use super::*;
    use crate::engine::process_engine::ProcessEngine;
    use crate::engine::time_source::{TestTimeSource, TimeSource};
    use crate::interceptor::command_executor::CommandExecutor;
    use crate::persistence::db_store::DbStore;
    use crate::persistence::runtime_store::{
        EventSubprocessTimerSubscription, ProcessTimerStartSubscription, RuntimeTimerJobState,
    };
    use crate::service::config::ProcessEngineConfiguration;
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;

    #[test]
    fn scheduled_timer_max_is_global_across_all_candidate_types_before_locking() {
        let now = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let configured_owner = "timer-max-owner";
        let timer_lock_time_ms = 5_678_i64;
        let mut config = ProcessEngineConfiguration::default();
        config.async_executor.lock_owner = Some(configured_owner.to_string());
        config.async_executor.timer_lock_time_ms = timer_lock_time_ms as u64;

        let engine = ProcessEngine::build_with_db_store_and_config(
            "scheduled-timer-max".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            config,
        )
        .unwrap();
        let store = engine.get_runtime_store();
        let deployment_manager = engine.get_command_executor().deployment_manager().clone();
        let now_ms = time_source.now().timestamp_millis();

        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: "runtime-earliest".to_string(),
                process_instance_id: "runtime-pi".to_string(),
                execution_id: "runtime-execution".to_string(),
                activity_id: "runtime-timer".to_string(),
                job_state: Some("timer".to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                calendar_name: None,
                due_time: Some(now_ms - 300),
                lock_owner: None,
                lock_time: None,
                lock_expiration_time: None,
                retries: Some(3),
                error_message: None,
                error_details: None,
                category: None,
                ..Default::default()
            },
            &mut session,
        );
        deployment_manager.register_timer_start_subscriptions(
            vec![ProcessTimerStartSubscription {
                id: "process-start-middle".to_string(),
                process_definition_id: "definition-id".to_string(),
                process_definition_key: "definition-key".to_string(),
                start_event_id: "timer-start".to_string(),
                start_event_name: None,
                interrupting: true,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                calendar_name: None,
                due_time: Some(now_ms - 200),
                lock_owner: None,
                lock_time: None,
                category: None,
            }],
            &mut session,
        );
        store.insert_event_subprocess_timer_subscription(
            EventSubprocessTimerSubscription {
                subscription_id: "event-subprocess-latest".to_string(),
                process_instance_id: "event-pi".to_string(),
                event_subprocess_id: "event-subprocess".to_string(),
                start_event_id: "event-timer-start".to_string(),
                interrupting: true,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                calendar_name: None,
                due_time: Some(now_ms - 100),
                lock_owner: None,
                lock_time: None,
                category: None,
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();

        let runtime_service = engine.get_runtime_service();
        let fencing_token = runtime_service
            .acquire_coordinator_lease(300_000)
            .unwrap()
            .expect("acquire timer coordinator lease");
        let works =
            runtime_service.acquire_scheduled_timer_work_for_tenants(fencing_token, &[], &[], 1);

        assert_eq!(works.len(), 1);
        assert!(matches!(
            &works[0],
            TimerWork::RuntimeJob(job) if job.timer_job_id == "runtime-earliest"
        ));

        let mut verification_session = store.create_session().unwrap();
        let runtime_jobs = store.snapshot_timer_job_states(&mut verification_session);
        let process_starts =
            deployment_manager.get_timer_start_subscriptions(&mut verification_session);
        let event_subprocesses =
            store.snapshot_event_subprocess_timer_subscriptions(&mut verification_session);
        let locked_count = runtime_jobs
            .values()
            .filter(|job| job.lock_owner.is_some())
            .count()
            + process_starts
                .iter()
                .filter(|subscription| subscription.lock_owner.is_some())
                .count()
            + event_subprocesses
                .values()
                .filter(|subscription| subscription.lock_owner.is_some())
                .count();
        assert_eq!(locked_count, 1, "max=1 must persist exactly one lock");

        let locked_runtime_job = runtime_jobs.get("runtime-earliest").unwrap();
        assert_eq!(
            locked_runtime_job.lock_owner.as_deref(),
            Some(configured_owner)
        );
        assert_eq!(locked_runtime_job.lock_time, Some(now_ms));
        assert_eq!(
            locked_runtime_job.lock_expiration_time,
            Some(now_ms + timer_lock_time_ms)
        );
        assert!(
            process_starts
                .iter()
                .all(|subscription| subscription.lock_owner.is_none())
        );
        assert!(
            event_subprocesses
                .values()
                .all(|subscription| subscription.lock_owner.is_none())
        );
        verification_session.rollback().unwrap();

        let global_command = AcquireTimerWorkCmd::new(
            Arc::from(configured_owner),
            fencing_token,
            Arc::new(TimerCoordinationMetrics::new()),
        )
        .scheduled_timers_only()
        .with_max_jobs(2)
        .serialized_by_global_lock();
        let global_works = engine
            .get_command_executor()
            .execute(&global_command)
            .unwrap();
        assert_eq!(global_works.len(), 2);
        assert!(global_works.iter().any(
            |work| matches!(work, TimerWork::ProcessStart(subscription) if subscription.id == "process-start-middle")
        ));
        assert!(global_works.iter().any(
            |work| matches!(work, TimerWork::EventSubprocess(subscription) if subscription.subscription_id == "event-subprocess-latest")
        ));

        let mut verification_session = store.create_session().unwrap();
        let process_starts =
            deployment_manager.get_timer_start_subscriptions(&mut verification_session);
        let event_subprocesses =
            store.snapshot_event_subprocess_timer_subscriptions(&mut verification_session);
        verification_session.rollback().unwrap();
        assert_eq!(
            process_starts[0].lock_owner.as_deref(),
            Some(configured_owner)
        );
        assert_eq!(process_starts[0].lock_time, Some(now_ms));
        assert_eq!(
            event_subprocesses
                .get("event-subprocess-latest")
                .unwrap()
                .lock_owner
                .as_deref(),
            Some(configured_owner)
        );
        assert_eq!(
            event_subprocesses
                .get("event-subprocess-latest")
                .unwrap()
                .lock_time,
            Some(now_ms)
        );

        runtime_service
            .release_coordinator_lease(fencing_token)
            .unwrap();
        engine.close();
    }

    fn seed_timer_subscription_category_fixture(
        engine: &ProcessEngine,
        now_ms: i64,
    ) -> (
        crate::persistence::runtime_store::RuntimeStore,
        crate::engine::deployment_manager::DeploymentManager,
    ) {
        let store = engine.get_runtime_store();
        let deployment_manager = engine.get_command_executor().deployment_manager().clone();
        let mut session = store.create_session().unwrap();
        deployment_manager.register_timer_start_subscriptions(
            vec![
                ProcessTimerStartSubscription {
                    id: "start-orders".to_string(),
                    process_definition_id: "def-orders".to_string(),
                    process_definition_key: "key-orders".to_string(),
                    start_event_id: "timerStartOrders".to_string(),
                    start_event_name: None,
                    interrupting: true,
                    time_duration: Some("PT1S".to_string()),
                    time_date: None,
                    time_cycle: None,
                    end_date: None,
                    calendar_name: None,
                    due_time: Some(now_ms - 300),
                    lock_owner: None,
                    lock_time: None,
                    category: Some("orders".to_string()),
                },
                ProcessTimerStartSubscription {
                    id: "start-null".to_string(),
                    process_definition_id: "def-null".to_string(),
                    process_definition_key: "key-null".to_string(),
                    start_event_id: "timerStartNull".to_string(),
                    start_event_name: None,
                    interrupting: true,
                    time_duration: Some("PT1S".to_string()),
                    time_date: None,
                    time_cycle: None,
                    end_date: None,
                    calendar_name: None,
                    due_time: Some(now_ms - 200),
                    lock_owner: None,
                    lock_time: None,
                    category: None,
                },
            ],
            &mut session,
        );
        store.insert_event_subprocess_timer_subscription(
            EventSubprocessTimerSubscription {
                subscription_id: "event-billing".to_string(),
                process_instance_id: "pi-billing".to_string(),
                event_subprocess_id: "eventSubBilling".to_string(),
                start_event_id: "eventTimerStart".to_string(),
                interrupting: true,
                time_duration: Some("PT1S".to_string()),
                time_date: None,
                time_cycle: None,
                end_date: None,
                calendar_name: None,
                due_time: Some(now_ms - 100),
                lock_owner: None,
                lock_time: None,
                category: Some("billing".to_string()),
            },
            &mut session,
        );
        store.insert_event_subprocess_timer_subscription(
            EventSubprocessTimerSubscription {
                subscription_id: "event-orders".to_string(),
                process_instance_id: "pi-orders".to_string(),
                event_subprocess_id: "eventSubOrders".to_string(),
                start_event_id: "eventTimerStartOrders".to_string(),
                interrupting: true,
                time_duration: Some("PT1S".to_string()),
                time_date: None,
                time_cycle: None,
                end_date: None,
                calendar_name: None,
                due_time: Some(now_ms - 50),
                lock_owner: None,
                lock_time: None,
                category: Some("orders".to_string()),
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
        (store, deployment_manager)
    }

    #[test]
    fn timer_subscription_empty_category_filter_acquires_all() {
        let now = Utc.timestamp_millis_opt(1_700_000_100_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let mut config = ProcessEngineConfiguration::default();
        config.async_executor.lock_owner = Some("sub-cat-empty".to_string());
        let engine = ProcessEngine::build_with_db_store_and_config(
            "timer-sub-category-empty".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            config,
        )
        .unwrap();
        seed_timer_subscription_category_fixture(&engine, time_source.now().timestamp_millis());
        let runtime_service = engine.get_runtime_service();
        let fencing_token = runtime_service
            .acquire_coordinator_lease(300_000)
            .unwrap()
            .expect("lease");
        let works = runtime_service.acquire_timer_work_for_tenants(fencing_token, &[], &[]);
        assert_eq!(
            works.len(),
            4,
            "empty enabled categories keeps categorized and uncategorized eligible"
        );
        runtime_service
            .release_coordinator_lease(fencing_token)
            .unwrap();
        engine.close();
    }

    #[test]
    fn timer_subscription_matching_category_is_acquired_mismatch_and_null_remain_unlocked() {
        let now = Utc.timestamp_millis_opt(1_700_000_100_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let mut config = ProcessEngineConfiguration::default();
        config.async_executor.lock_owner = Some("sub-cat-match".to_string());
        let engine = ProcessEngine::build_with_db_store_and_config(
            "timer-sub-category-match".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            config,
        )
        .unwrap();
        let (store, deployment_manager) =
            seed_timer_subscription_category_fixture(&engine, time_source.now().timestamp_millis());
        let runtime_service = engine.get_runtime_service();
        let fencing_token = runtime_service
            .acquire_coordinator_lease(300_000)
            .unwrap()
            .expect("lease");

        let orders = vec!["orders".to_string()];
        let matching = runtime_service.acquire_timer_work_for_tenants(fencing_token, &[], &orders);
        assert_eq!(matching.len(), 2);
        for work in &matching {
            match work {
                TimerWork::ProcessStart(sub) => {
                    assert_eq!(sub.id, "start-orders");
                    assert_eq!(sub.category.as_deref(), Some("orders"));
                }
                TimerWork::EventSubprocess(sub) => {
                    assert_eq!(sub.subscription_id, "event-orders");
                    assert_eq!(sub.category.as_deref(), Some("orders"));
                }
                other => panic!("unexpected work item: {other:?}"),
            }
        }

        let mut verification = store.create_session().unwrap();
        let process_starts = deployment_manager.get_timer_start_subscriptions(&mut verification);
        let null_start = process_starts
            .iter()
            .find(|s| s.id == "start-null")
            .expect("null category start should still exist");
        assert!(
            null_start.lock_owner.is_none(),
            "category == None must be excluded and remain unlocked"
        );
        let event_subs = store.snapshot_event_subprocess_timer_subscriptions(&mut verification);
        let billing = event_subs
            .get("event-billing")
            .expect("billing event sub should remain");
        assert!(
            billing.lock_owner.is_none(),
            "non-matching category must remain unlocked"
        );
        verification.rollback().unwrap();

        runtime_service
            .release_coordinator_lease(fencing_token)
            .unwrap();
        engine.close();
    }

    #[test]
    fn timer_subscription_multi_category_filter_excludes_null_only() {
        let now = Utc.timestamp_millis_opt(1_700_000_100_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let mut config = ProcessEngineConfiguration::default();
        config.async_executor.lock_owner = Some("sub-cat-multi".to_string());
        let engine = ProcessEngine::build_with_db_store_and_config(
            "timer-sub-category-multi".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            config,
        )
        .unwrap();
        seed_timer_subscription_category_fixture(&engine, time_source.now().timestamp_millis());
        let runtime_service = engine.get_runtime_service();
        let fencing_token = runtime_service
            .acquire_coordinator_lease(300_000)
            .unwrap()
            .expect("lease");

        let multi = vec!["orders".to_string(), "billing".to_string()];
        let multi_works =
            runtime_service.acquire_timer_work_for_tenants(fencing_token, &[], &multi);
        assert_eq!(
            multi_works.len(),
            3,
            "multi-category filter excludes null only"
        );
        let mut categories: Vec<_> = multi_works
            .iter()
            .map(|work| match work {
                TimerWork::ProcessStart(sub) => sub.category.clone(),
                TimerWork::EventSubprocess(sub) => sub.category.clone(),
                _ => None,
            })
            .collect();
        categories.sort();
        assert_eq!(
            categories,
            vec![
                Some("billing".to_string()),
                Some("orders".to_string()),
                Some("orders".to_string())
            ]
        );

        runtime_service
            .release_coordinator_lease(fencing_token)
            .unwrap();
        engine.close();
    }

    #[test]
    fn global_async_acquisition_bulk_updates_selected_unlocked_jobs_only() {
        let now = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let configured_owner = "global-async-owner";
        let lock_duration_ms = 4_321_i64;
        let mut config = ProcessEngineConfiguration::default();
        config.async_executor.lock_owner = Some(configured_owner.to_string());
        let engine = ProcessEngine::build_with_db_store_and_config(
            "global-async-bulk".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            config,
        )
        .unwrap();
        let store = engine.get_runtime_store();
        let now_ms = time_source.now().timestamp_millis();
        let mut session = store.create_session().unwrap();
        for (job_id, lock_owner, lock_time, lock_expiration_time) in [
            ("global-unlocked-a", None, None, None),
            ("global-unlocked-b", None, None, None),
            (
                "global-expired-but-not-reset",
                Some("old-owner".to_string()),
                Some(now_ms - 10_000),
                Some(now_ms - 5_000),
            ),
        ] {
            store.insert_timer_job_state(
                &RuntimeTimerJobState {
                    timer_job_id: job_id.to_string(),
                    process_instance_id: format!("process-{job_id}"),
                    execution_id: format!("execution-{job_id}"),
                    activity_id: "async-activity".to_string(),
                    job_state: Some("async".to_string()),
                    is_boundary: false,
                    attached_activity_id: None,
                    cancel_activity: false,
                    time_duration: None,
                    time_date: None,
                    time_cycle: None,
                    end_date: None,
                    calendar_name: None,
                    due_time: Some(now_ms - 100),
                    lock_owner,
                    lock_time,
                    lock_expiration_time,
                    retries: Some(3),
                    error_message: None,
                    error_details: None,
                    category: None,
                    ..Default::default()
                },
                &mut session,
            );
        }
        session.flush_and_commit().unwrap();

        let metrics = Arc::new(TimerCoordinationMetrics::new());
        let command = AcquireAsyncJobsCmd::new(
            Arc::from(configured_owner),
            lock_duration_ms,
            3,
            Arc::clone(&metrics),
        )
        .serialized_by_global_lock();
        let acquired = engine.get_command_executor().execute(&command).unwrap();

        assert_eq!(acquired.len(), 2);
        assert!(acquired.iter().all(|job| {
            job.lock_owner.as_deref() == Some(configured_owner)
                && job.lock_time == Some(now_ms)
                && job.lock_expiration_time == Some(now_ms + lock_duration_ms)
        }));
        assert_eq!(
            metrics.acquire_conflicts.load(Ordering::Relaxed),
            0,
            "serialized global acquisition must not report optimistic conflicts"
        );

        let mut session = store.create_session().unwrap();
        let persisted_a = store
            .find_timer_job_state("global-unlocked-a", &mut session)
            .unwrap();
        let persisted_b = store
            .find_timer_job_state("global-unlocked-b", &mut session)
            .unwrap();
        let persisted_expired = store
            .find_timer_job_state("global-expired-but-not-reset", &mut session)
            .unwrap();
        session.rollback().unwrap();
        for job in [persisted_a, persisted_b] {
            assert_eq!(job.lock_owner.as_deref(), Some(configured_owner));
            assert_eq!(job.lock_time, Some(now_ms));
            assert_eq!(job.lock_expiration_time, Some(now_ms + lock_duration_ms));
        }
        assert_eq!(persisted_expired.lock_owner.as_deref(), Some("old-owner"));
        assert_eq!(persisted_expired.lock_time, Some(now_ms - 10_000));
    }

    #[test]
    fn expired_lock_requires_reset_before_optimistic_async_reacquire() {
        let now = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let engine = ProcessEngine::build_with_db_store_and_config(
            "expired-lock-requires-reset".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let store = engine.get_runtime_store();
        let now_ms = time_source.now().timestamp_millis();
        let mut session = store.create_session().unwrap();
        for (id, state) in [
            ("async-expired", "async"),
            ("timer-expired", "timer"),
            ("history-expired", "history"),
        ] {
            store.insert_timer_job_state(
                &RuntimeTimerJobState {
                    timer_job_id: id.to_string(),
                    process_instance_id: format!("process-{id}"),
                    execution_id: format!("execution-{id}"),
                    activity_id: "activity".to_string(),
                    job_state: Some(state.to_string()),
                    is_boundary: false,
                    attached_activity_id: None,
                    cancel_activity: false,
                    time_duration: None,
                    time_date: None,
                    time_cycle: None,
                    end_date: None,
                    calendar_name: None,
                    due_time: Some(now_ms - 100),
                    lock_owner: Some("dead-owner".to_string()),
                    lock_time: Some(now_ms - 10_000),
                    lock_expiration_time: Some(now_ms - 5_000),
                    retries: Some(3),
                    error_message: None,
                    error_details: None,
                    category: None,
                    ..Default::default()
                },
                &mut session,
            );
        }
        session.flush_and_commit().unwrap();

        // While expired but not reset, acquisition must not reclaim.
        let metrics = Arc::new(TimerCoordinationMetrics::new());
        let async_acquired = engine
            .get_command_executor()
            .execute(&AcquireAsyncJobsCmd::new(
                Arc::from("new-owner"),
                1_000, // short duration must not re-derive expiry from lock_time
                10,
                Arc::clone(&metrics),
            ))
            .unwrap();
        assert!(
            async_acquired.is_empty(),
            "expired async job must remain unavailable until reset"
        );
        assert_eq!(
            metrics.expired_lease_recoveries.load(Ordering::Relaxed),
            0,
            "acquisition must not record expired recovery"
        );

        let timer_acquired = engine.get_runtime_service().acquire_timer_work(0);
        assert!(
            !timer_acquired
                .iter()
                .any(|work| matches!(work, TimerWork::RuntimeJob(job) if job.timer_job_id == "timer-expired")),
            "expired timer job must remain unavailable until reset"
        );

        let history_acquired = engine.get_runtime_service().acquire_history_jobs(1_000, 10);
        assert!(
            history_acquired.is_empty(),
            "expired history job must remain unavailable until reset"
        );

        // Changing configured lock duration cannot make the existing lease expire.
        let still_blocked = engine
            .get_command_executor()
            .execute(&AcquireAsyncJobsCmd::new(
                Arc::from("new-owner"),
                1, // even tinier duration
                10,
                Arc::new(TimerCoordinationMetrics::new()),
            ))
            .unwrap();
        assert!(still_blocked.is_empty());

        // After typed reset, jobs become acquirable.
        for class in [
            ExpiredJobClass::Async,
            ExpiredJobClass::Timer,
            ExpiredJobClass::History,
        ] {
            let outcome = engine
                .get_runtime_service()
                .reset_expired_jobs_batch(class, 10)
                .unwrap();
            assert_eq!(outcome.reset, 1);
        }

        let async_after = engine.get_runtime_service().acquire_async_jobs(5_000, 10);
        assert!(
            async_after
                .iter()
                .any(|job| job.timer_job_id == "async-expired")
        );
        let history_after = engine.get_runtime_service().acquire_history_jobs(5_000, 10);
        assert!(
            history_after
                .iter()
                .any(|job| job.timer_job_id == "history-expired")
        );
    }

    #[test]
    fn optimistic_and_global_async_acquisition_share_unlocked_only_eligibility() {
        let now = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let engine = ProcessEngine::build_with_db_store_and_config(
            "unlocked-only-parity".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let store = engine.get_runtime_store();
        let now_ms = time_source.now().timestamp_millis();
        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: "expired-parity".to_string(),
                process_instance_id: "process-parity".to_string(),
                execution_id: "execution-parity".to_string(),
                activity_id: "async".to_string(),
                job_state: Some("async".to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                calendar_name: None,
                due_time: Some(now_ms - 100),
                lock_owner: Some("old".to_string()),
                lock_time: Some(now_ms - 10_000),
                lock_expiration_time: Some(now_ms - 1),
                retries: Some(3),
                error_message: None,
                error_details: None,
                category: None,
                ..Default::default()
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();

        let optimistic = engine
            .get_command_executor()
            .execute(&AcquireAsyncJobsCmd::new(
                Arc::from("opt-owner"),
                60_000,
                10,
                Arc::new(TimerCoordinationMetrics::new()),
            ))
            .unwrap();
        let global = engine
            .get_command_executor()
            .execute(
                &AcquireAsyncJobsCmd::new(
                    Arc::from("global-owner"),
                    60_000,
                    10,
                    Arc::new(TimerCoordinationMetrics::new()),
                )
                .serialized_by_global_lock(),
            )
            .unwrap();
        assert!(optimistic.is_empty());
        assert!(global.is_empty());
    }
}

#[cfg(test)]
mod reset_expired_job_batch_tests {
    use super::*;
    use crate::engine::process_engine::ProcessEngine;
    use crate::engine::time_source::{TestTimeSource, TimeSource};
    use crate::interceptor::command_executor::CommandExecutor;
    use crate::persistence::db_store::DbStore;
    use crate::persistence::runtime_store::{
        ExpiredJobClass, ResetExpiredJobsBatchOutcome, RuntimeTimerJobState,
    };
    use crate::service::config::ProcessEngineConfiguration;
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;

    fn job(
        id: &str,
        state: &str,
        lock_owner: Option<&str>,
        lock_time: Option<i64>,
        lock_expiration_time: Option<i64>,
    ) -> RuntimeTimerJobState {
        RuntimeTimerJobState {
            timer_job_id: id.to_string(),
            process_instance_id: format!("process-{id}"),
            execution_id: format!("execution-{id}"),
            activity_id: "activity".to_string(),
            job_state: Some(state.to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            calendar_name: None,
            due_time: Some(lock_time.unwrap_or_default()),
            lock_owner: lock_owner.map(str::to_string),
            lock_time,
            lock_expiration_time,
            retries: Some(7),
            error_message: Some("preserved-message".to_string()),
            error_details: Some("preserved-details".to_string()),
            category: None,
            ..Default::default()
        }
    }

    #[test]
    fn reset_expired_job_batch_is_state_aware_and_preserves_job_data() {
        let now = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let engine = ProcessEngine::build_with_db_store_and_config(
            "typed-expired-job-batch".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let store = engine.get_runtime_store();
        let now_ms = time_source.now().timestamp_millis();
        let mut session = store.create_session().unwrap();
        for (id, state) in [
            ("async", "async"),
            ("async-after", "async-after"),
            ("timer", "timer"),
            ("history", "history"),
            ("deadletter", "deadletter"),
            ("suspended", "suspended"),
        ] {
            store.insert_timer_job_state(
                &job(
                    id,
                    state,
                    Some("dead-owner"),
                    Some(now_ms - 2_000),
                    Some(now_ms - 1_000),
                ),
                &mut session,
            );
        }
        session.flush_and_commit().unwrap();

        let outcome = engine
            .get_command_executor()
            .execute(&ResetExpiredJobsBatchCmd::new(ExpiredJobClass::Async, 10))
            .unwrap();
        assert_eq!(
            outcome,
            ResetExpiredJobsBatchOutcome {
                scanned: 2,
                reset: 2,
                conflicts: 0,
            }
        );

        let mut session = store.create_session().unwrap();
        for (id, state) in [("async", "async"), ("async-after", "async-after")] {
            let persisted = store.find_timer_job_state(id, &mut session).unwrap();
            assert!(persisted.lock_owner.is_none());
            assert!(persisted.lock_time.is_none());
            assert!(persisted.lock_expiration_time.is_none());
            assert_eq!(persisted.job_state.as_deref(), Some(state));
            assert_eq!(persisted.retries, Some(7));
            assert_eq!(
                persisted.error_message.as_deref(),
                Some("preserved-message")
            );
            assert_eq!(
                persisted.error_details.as_deref(),
                Some("preserved-details")
            );
        }
        for id in ["timer", "history", "deadletter", "suspended"] {
            let persisted = store.find_timer_job_state(id, &mut session).unwrap();
            assert_eq!(persisted.lock_owner.as_deref(), Some("dead-owner"));
            assert!(persisted.lock_expiration_time.is_some());
        }
        session.rollback().unwrap();
    }

    #[test]
    fn reset_expired_job_batch_uses_strict_expiration_and_repairs_missing_owner() {
        let now = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let engine = ProcessEngine::build_with_db_store_and_config(
            "expired-job-boundary".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let store = engine.get_runtime_store();
        let now_ms = time_source.now().timestamp_millis();
        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state(
            &job(
                "equal-boundary",
                "timer",
                Some("owner"),
                Some(now_ms - 1_000),
                Some(now_ms),
            ),
            &mut session,
        );
        store.insert_timer_job_state(
            &job(
                "missing-owner",
                "timer",
                None,
                Some(now_ms - 2_000),
                Some(now_ms - 1),
            ),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        let outcome = engine
            .get_command_executor()
            .execute(&ResetExpiredJobsBatchCmd::new(ExpiredJobClass::Timer, 10))
            .unwrap();
        assert_eq!(outcome.scanned, 1);
        assert_eq!(outcome.reset, 1);
        assert_eq!(outcome.conflicts, 0);

        let mut session = store.create_session().unwrap();
        let boundary = store
            .find_timer_job_state("equal-boundary", &mut session)
            .unwrap();
        assert_eq!(boundary.lock_owner.as_deref(), Some("owner"));
        assert_eq!(boundary.lock_expiration_time, Some(now_ms));
        let repaired = store
            .find_timer_job_state("missing-owner", &mut session)
            .unwrap();
        assert!(repaired.lock_owner.is_none());
        assert!(repaired.lock_time.is_none());
        assert!(repaired.lock_expiration_time.is_none());
        session.rollback().unwrap();
    }

    #[test]
    fn reset_expired_job_batch_reports_conflict_when_lease_renewed_between_select_and_cas() {
        let now = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let engine = ProcessEngine::build_with_db_store_and_config(
            "expired-job-cas-conflict".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let store = engine.get_runtime_store();
        let now_ms = time_source.now().timestamp_millis();
        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state(
            &job(
                "history-expired",
                "history",
                Some("dead-owner"),
                Some(now_ms - 2_000),
                Some(now_ms - 1_000),
            ),
            &mut session,
        );
        session.flush_and_commit().unwrap();

        // Select the expired candidate first (as the batch would).
        let mut select_session = store.create_session().unwrap();
        let candidates = store
            .find_expired_job_lock_candidates(
                now_ms,
                ExpiredJobClass::History,
                10,
                None,
                None,
                &mut select_session,
            )
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].lock_owner.as_deref(), Some("dead-owner"));

        // Renew the lease before the selected snapshot is applied.
        let mut renew_session = store.create_session().unwrap();
        let mut renewed = store
            .find_timer_job_state("history-expired", &mut renew_session)
            .unwrap();
        let old_expiration = renewed.lock_expiration_time.unwrap();
        renewed.lock_owner = Some("renewed-owner".to_string());
        renewed.lock_time = Some(now_ms);
        renewed.lock_expiration_time = Some(now_ms + 60_000);
        assert!(store.replace_timer_job_state_if_locked(
            &renewed,
            "dead-owner",
            old_expiration,
            &mut renew_session,
        ));
        renew_session.flush_and_commit().unwrap();

        // CAS against the pre-renewal snapshot must conflict and leave the lease.
        let outcome = store
            .compare_and_reset_expired_job_locks(candidates, &mut select_session)
            .unwrap();
        assert_eq!(
            outcome,
            ResetExpiredJobsBatchOutcome {
                scanned: 1,
                reset: 0,
                conflicts: 1,
            }
        );
        select_session.flush_and_commit().unwrap();

        let mut session = store.create_session().unwrap();
        let persisted = store
            .find_timer_job_state("history-expired", &mut session)
            .unwrap();
        assert_eq!(persisted.lock_owner.as_deref(), Some("renewed-owner"));
        assert_eq!(persisted.lock_time, Some(now_ms));
        assert_eq!(persisted.lock_expiration_time, Some(now_ms + 60_000));
        assert_eq!(persisted.retries, Some(7));
        session.rollback().unwrap();

        // A subsequent typed batch must not select the renewed (non-expired) lease.
        let follow_up = engine
            .get_command_executor()
            .execute(&ResetExpiredJobsBatchCmd::new(ExpiredJobClass::History, 10))
            .unwrap();
        assert_eq!(
            follow_up,
            ResetExpiredJobsBatchOutcome {
                scanned: 0,
                reset: 0,
                conflicts: 0,
            }
        );
    }
}
