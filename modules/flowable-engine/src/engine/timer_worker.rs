use crate::engine::runtime_service::RuntimeService;
use crate::persistence::runtime_store::{
    EventSubprocessTimerSubscription, ProcessTimerStartSubscription, RuntimeTimerJobState,
};
use std::sync::Arc;

use std::sync::atomic::AtomicUsize;

/// In-process counters for async/timer acquisition and execution.
///
/// Surface aligns with Java `AcquireAsyncJobsDueLifecycleListener` lifecycle
/// hooks (batch size, conflicts, execute outcome) plus coordinator lease stats.
/// Exported via REST `/metrics` (Prometheus) and management timer ledgers.
#[derive(Default, Debug)]
pub struct TimerCoordinationMetrics {
    /// Candidates considered during acquire (acquired + conflicts).
    pub acquire_attempts: AtomicUsize,
    /// Optimistic/global-lock acquire races lost.
    pub acquire_conflicts: AtomicUsize,
    /// Jobs successfully locked for this owner in acquire commands.
    pub jobs_acquired: AtomicUsize,
    /// Size of the most recent acquire batch (last-write-wins).
    pub last_acquire_batch_size: AtomicUsize,
    pub renew_successes: AtomicUsize,
    pub renew_misses: AtomicUsize,
    pub expired_lease_recoveries: AtomicUsize,
    /// Successful executes by work kind (Java jobs.async/timer type=executed).
    pub execute_count_runtime_job: AtomicUsize,
    pub execute_count_process_start: AtomicUsize,
    pub execute_count_event_subprocess: AtomicUsize,
    /// Automatic executor path failures that entered the failed-job handler.
    pub execute_failures: AtomicUsize,
}

impl TimerCoordinationMetrics {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimerWork {
    RuntimeJob(RuntimeTimerJobState),
    ProcessStart(ProcessTimerStartSubscription),
    EventSubprocess(EventSubprocessTimerSubscription),
}

impl TimerWork {
    pub fn due_time(&self) -> Option<i64> {
        match self {
            TimerWork::RuntimeJob(j) => j.due_time,
            TimerWork::ProcessStart(s) => s.due_time,
            TimerWork::EventSubprocess(s) => s.due_time,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TimerWorkerConfig {
    pub poll_interval_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub max_jitter_ms: u64,
    pub coordinator_lease_timeout_ms: u64,
}

impl Default for TimerWorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 100,
            heartbeat_interval_ms: 60_000,
            max_jitter_ms: 50,
            coordinator_lease_timeout_ms: 300_000,
        }
    }
}

impl TimerWorkerConfig {
    pub fn get_jitter_ms(&self) -> u64 {
        if self.max_jitter_ms == 0 {
            return 0;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        (nanos as u64) % self.max_jitter_ms
    }
}

pub struct TimerWorker {
    pub runtime_service: Arc<RuntimeService>,
    worker_type: String,
    fencing_token: std::sync::atomic::AtomicI64,
}

impl TimerWorker {
    pub fn new(runtime_service: Arc<RuntimeService>, worker_type: &str) -> Self {
        Self {
            runtime_service,
            worker_type: worker_type.to_string(),
            fencing_token: std::sync::atomic::AtomicI64::new(0),
        }
    }

    pub fn get_fencing_token(&self) -> i64 {
        self.fencing_token
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_fencing_token(&self, token: i64) {
        self.fencing_token
            .store(token, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn heartbeat(&self) {
        // Java parity: node liveness writes funnel through the MyBatis session
        // (`DbSqlSession.flushInserts`, flowable-engine-common/.../impl/db/DbSqlSession.java:495),
        // which throws on a SQL failure. The worker loop must keep running, so the failure
        // is logged rather than dropped.
        if let Err(error) = self
            .runtime_service
            .heartbeat_timer_node(&self.worker_type)
        {
            tracing::error!("failed to record timer node heartbeat: {error}");
        }
    }

    pub fn acquire_due_timers(
        &self,
        coordinator_lease_timeout_ms: u64,
    ) -> Result<Vec<TimerWork>, crate::error::FlowableError> {
        self.acquire_due_timers_for_tenants(coordinator_lease_timeout_ms, &[], &[])
    }

    /// Acquire due timers, optionally restricted by process-instance tenant.
    /// Empty `tenant_ids` means all tenants (shared mode).
    ///
    /// Java parity: `AcquireTimerJobsRunnable` retries a failed acquisition by catching the
    /// exception and waiting (`AcquireTimerJobsRunnable.java:169-175` for lock errors,
    /// `:212-216` for other throwables) — it never reports "no work due" for a failure.
    /// Only `Ok(None)` from the lease query ("a peer holds the coordinator lease") is a
    /// legitimate empty batch.
    pub fn acquire_due_timers_for_tenants(
        &self,
        coordinator_lease_timeout_ms: u64,
        tenant_ids: &[String],
        enabled_job_categories: &[String],
    ) -> Result<Vec<TimerWork>, crate::error::FlowableError> {
        self.heartbeat();
        let Some(token) = self
            .runtime_service
            .acquire_coordinator_lease(coordinator_lease_timeout_ms)?
        else {
            // A peer holds the coordinator lease: genuinely nothing to acquire here.
            self.fencing_token
                .store(0, std::sync::atomic::Ordering::Relaxed);
            return Ok(Vec::new());
        };
        self.fencing_token
            .store(token, std::sync::atomic::Ordering::Relaxed);
        self.runtime_service
            .acquire_timer_work_for_tenants(token, tenant_ids, enabled_job_categories)
    }

    pub(crate) fn acquire_due_scheduled_timers_for_tenants(
        &self,
        coordinator_lease_timeout_ms: u64,
        tenant_ids: &[String],
        enabled_job_categories: &[String],
        max_jobs: usize,
        global_acquire_permit: Option<&crate::engine::lock_manager::GlobalAcquirePermit<'_>>,
    ) -> Result<Vec<TimerWork>, crate::error::FlowableError> {
        self.heartbeat();
        let Some(token) = self
            .runtime_service
            .acquire_coordinator_lease(coordinator_lease_timeout_ms)?
        else {
            self.fencing_token
                .store(0, std::sync::atomic::Ordering::Relaxed);
            return Ok(Vec::new());
        };
        self.fencing_token
            .store(token, std::sync::atomic::Ordering::Relaxed);
        if let Some(permit) = global_acquire_permit {
            self.runtime_service
                .acquire_scheduled_timer_work_global_for_tenants(
                    permit,
                    token,
                    tenant_ids,
                    enabled_job_categories,
                    max_jobs,
                )
        } else {
            self.runtime_service.acquire_scheduled_timer_work_for_tenants(
                token,
                tenant_ids,
                enabled_job_categories,
                max_jobs,
            )
        }
    }

    pub fn execute_timer(&self, work: &TimerWork) {
        let token = self
            .fencing_token
            .load(std::sync::atomic::Ordering::Relaxed);
        if token > 0 {
            let _ = self.runtime_service.execute_timer_work(work, token);
        }
    }

    pub fn renew_timer_lease(&self, work: &TimerWork) {
        let token = self
            .fencing_token
            .load(std::sync::atomic::Ordering::Relaxed);
        if token > 0 {
            if let Err(error) = self.runtime_service.renew_timer_lease(work, token) {
                tracing::error!("failed to renew timer lease: {error}");
            }
        }
    }

    pub fn release_leadership(&self) {
        let token = self
            .fencing_token
            .load(std::sync::atomic::Ordering::Relaxed);
        if token > 0 {
            // Java parity: `DbSqlSession.flushDeletes` (DbSqlSession.java:636) throws on a
            // SQL failure. A failed release degrades to lease expiry, which must be
            // observable rather than silent.
            if let Err(error) = self.runtime_service.release_coordinator_lease(token) {
                tracing::error!("failed to release coordinator lease: {error}");
            }
            self.fencing_token
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Deregisters this worker from the node registry.
    /// Called during graceful shutdown so that the node is no longer
    /// listed by the control surface.
    pub fn deregister(&self) {
        // Java parity: the node-registry delete goes through the session, whose flush
        // throws on a SQL failure; report it instead of leaving a stale "Active" node.
        if let Err(error) = self
            .runtime_service
            .deregister_timer_node(self.runtime_service.timer_owner_id())
        {
            tracing::error!("failed to deregister timer node: {error}");
        }
    }

    /// Combined graceful shutdown: release leadership + deregister node.
    pub fn graceful_shutdown(&self) {
        self.release_leadership();
        self.deregister();
    }

    pub fn worker_type(&self) -> &str {
        &self.worker_type
    }
}
