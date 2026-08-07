use crate::engine::async_job_acquisition::{AsyncJobAcquisition, AsyncJobAcquisitionConfig};
use crate::engine::async_task_executor::{AsyncTaskExecutor, AsyncTaskExecutorConfig};
use crate::engine::history_job_dispatcher::HistoryJobDispatcher;
use crate::engine::job_runnable::{spawn_direct_hint_work, spawn_timer_work};
use crate::engine::lock_manager::{ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK, ACQUIRE_TIMER_JOBS_GLOBAL_LOCK};
use crate::engine::reset_expired_jobs::{ResetExpiredJobs, ResetExpiredJobsConfig};
use crate::engine::runtime_service::RuntimeService;
use crate::engine::timer_job_acquisition::{TimerJobAcquisition, TimerJobAcquisitionConfig};
use crate::engine::timer_worker::TimerWork;
use crate::error::FlowableError;
use crate::persistence::runtime_store::{ExpiredJobClass, RuntimeStore};
use crate::service::config::{
    AsyncExecutorConfiguration, AsyncExecutorTenantScope, AsyncExecutorTopology,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct TemporaryRuntimeJob {
    runtime_service: Arc<RuntimeService>,
    job: crate::persistence::runtime_store::RuntimeTimerJobState,
    fencing_token: i64,
    /// When set, the job is a pre-locked activation hint and must be replayed
    /// through the direct-hint path once the executor becomes active.
    direct_hint: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncExecutorStartOutcome {
    Started,
    AlreadyActive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncExecutorShutdownOutcome {
    Shutdown,
    AlreadyInactive,
}

pub struct AsyncExecutor {
    task_executor: Arc<Mutex<Option<AsyncTaskExecutor>>>,
    async_acquisition: Option<AsyncJobAcquisition>,
    timer_acquisition: Option<TimerJobAcquisition>,
    reset_expired: Option<ResetExpiredJobs>,
    config: AsyncExecutorConfiguration,
    lifecycle_lock: Mutex<()>,
    runtime_service: Mutex<Option<Arc<RuntimeService>>>,
    temporary_jobs: Mutex<VecDeque<TemporaryRuntimeJob>>,
    /// Live active flag. `Arc`-backed so it can be *shared* with the
    /// [`ActivationCoordinator`](crate::engine::activation_coordinator::ActivationCoordinator):
    /// a command executing on another thread can then observe the true runtime
    /// state of this executor (Java parity for `isAsyncExecutorActive`).
    is_active: Arc<AtomicBool>,
}

impl AsyncExecutor {
    fn create_task_executor(config: &AsyncExecutorConfiguration) -> AsyncTaskExecutor {
        AsyncTaskExecutor::new(AsyncTaskExecutorConfig {
            pool_size: config.pool_size,
            queue_size: config.queue_size,
            keep_alive_ms: 5000,
            thread_name_prefix: "flowable-async".to_string(),
            ..AsyncTaskExecutorConfig::default()
        })
    }

    pub fn new(config: AsyncExecutorConfiguration) -> Self {
        let lock_owner = config
            .lock_owner
            .clone()
            .unwrap_or_else(|| format!("async-executor:{}", uuid::Uuid::new_v4()));
        Self::new_with_lock_owner(config, lock_owner)
    }

    pub fn new_with_lock_owner(
        mut config: AsyncExecutorConfiguration,
        lock_owner: impl Into<String>,
    ) -> Self {
        let lock_owner = lock_owner.into();
        config.lock_owner = Some(lock_owner.clone());
        let max_async_jobs_due_per_acquisition =
            config.effective_max_async_jobs_due_per_acquisition();
        let max_timer_jobs_per_acquisition = config.effective_max_timer_jobs_per_acquisition();
        Self {
            task_executor: Arc::new(Mutex::new(None)),
            async_acquisition: if config.async_job_acquisition_enabled {
                Some(AsyncJobAcquisition::new(AsyncJobAcquisitionConfig {
                    async_job_acquire_wait_ms: config.async_job_acquire_wait_ms,
                    queue_full_wait_ms: config.queue_full_wait_ms,
                    max_jobs_per_acquisition: max_async_jobs_due_per_acquisition,
                    async_job_lock_time_ms: config.async_job_lock_time_ms as i64,
                    coordinator_lease_timeout_ms: 300_000,
                    global_acquire_lock_enabled: config.global_acquire_lock_enabled,
                    global_acquire_lock_name: config
                        .global_lock_name_for(ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK),
                    global_acquire_lock_wait_ms: config.effective_async_jobs_global_lock_wait_ms(),
                    global_acquire_lock_poll_rate_ms: config
                        .effective_async_jobs_global_lock_poll_rate_ms(),
                    global_acquire_lock_lease_ms: config
                        .effective_async_jobs_global_lock_force_acquire_after_ms(),
                    lock_owner: lock_owner.clone(),
                    tenant_ids: config.tenant_ids.clone(),
                    enabled_job_categories: config.enabled_job_categories.clone(),
                }))
            } else {
                None
            },
            timer_acquisition: if config.timer_job_acquisition_enabled {
                Some(TimerJobAcquisition::new(TimerJobAcquisitionConfig {
                    timer_job_acquire_wait_ms: config.timer_job_acquire_wait_ms,
                    queue_full_wait_ms: config.queue_full_wait_ms,
                    max_jobs_per_acquisition: max_timer_jobs_per_acquisition,
                    coordinator_lease_timeout_ms: 300_000,
                    poll_interval_ms: 100,
                    max_jitter_ms: 50,
                    global_acquire_lock_enabled: config.global_acquire_lock_enabled,
                    global_acquire_lock_name: config
                        .global_lock_name_for(ACQUIRE_TIMER_JOBS_GLOBAL_LOCK),
                    global_acquire_lock_wait_ms: config.effective_timer_global_lock_wait_ms(),
                    global_acquire_lock_poll_rate_ms: config
                        .effective_timer_global_lock_poll_rate_ms(),
                    global_acquire_lock_lease_ms: config
                        .effective_timer_global_lock_force_acquire_after_ms(),
                    lock_owner,
                    tenant_ids: config.tenant_ids.clone(),
                    enabled_job_categories: config.enabled_job_categories.clone(),
                }))
            } else {
                None
            },
            reset_expired: if config.reset_expired_job_enabled {
                Some(ResetExpiredJobs::new(ResetExpiredJobsConfig {
                    reset_interval_ms: config.reset_expired_jobs_interval_ms,
                    page_size: config.reset_expired_jobs_page_size,
                    job_classes: ExpiredJobClass::ALL.to_vec(),
                    tenant_ids: config.tenant_ids.clone(),
                    enabled_job_categories: config.enabled_job_categories.clone(),
                }))
            } else {
                None
            },
            config,
            lifecycle_lock: Mutex::new(()),
            runtime_service: Mutex::new(None),
            temporary_jobs: Mutex::new(VecDeque::new()),
            is_active: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Adopt an externally-owned active flag (the
    /// [`ActivationCoordinator`](crate::engine::activation_coordinator::ActivationCoordinator)'s).
    /// Must be called before the executor is started so the coordinator and the
    /// executor share the *same* `Arc<AtomicBool>`. The flag is reset to
    /// inactive to match a freshly-constructed executor.
    pub fn with_shared_active_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        flag.store(false, Ordering::SeqCst);
        self.is_active = flag;
        self
    }

    /// Convenience constructor scoped to a single tenant.
    /// Equivalent to setting `config.tenant_ids = vec![tenant_id]`.
    pub fn for_tenant(mut config: AsyncExecutorConfiguration, tenant_id: String) -> Self {
        config.tenant_ids = vec![tenant_id];
        Self::new(config)
    }

    pub fn start(&self, runtime_service: Arc<RuntimeService>) {
        if let Err(error) = self.try_start(runtime_service) {
            tracing::error!("failed to start async executor: {error}");
        }
    }

    pub fn try_start(
        &self,
        runtime_service: Arc<RuntimeService>,
    ) -> Result<AsyncExecutorStartOutcome, FlowableError> {
        let _lifecycle_guard = self
            .lifecycle_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.is_active.load(Ordering::SeqCst) {
            return Ok(AsyncExecutorStartOutcome::AlreadyActive);
        }

        *self
            .runtime_service
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Arc::clone(&runtime_service));
        self.is_active.store(true, Ordering::SeqCst);
        if self.config.unlocks_owned_jobs_on_start() {
            runtime_service.unlock_owned_jobs(&self.config.tenant_ids)?;
        }

        {
            let mut task_executor = self
                .task_executor
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if task_executor.is_none() {
                *task_executor = Some(Self::create_task_executor(&self.config));
            }
        }
        if let Some(acq) = &self.async_acquisition {
            acq.start(
                Arc::clone(&runtime_service),
                Arc::clone(&self.task_executor),
            );
        }
        if let Some(acq) = &self.timer_acquisition {
            acq.start(
                Arc::clone(&runtime_service),
                Arc::clone(&self.task_executor),
            );
        }
        if let Some(reset) = &self.reset_expired {
            reset.start(Arc::clone(&runtime_service));
        }
        self.execute_temporary_jobs()?;
        Ok(AsyncExecutorStartOutcome::Started)
    }

    pub fn shutdown(&self) {
        if let Err(error) = self.try_shutdown() {
            tracing::error!("failed to shut down async executor: {error}");
        }
    }

    pub fn try_shutdown(&self) -> Result<AsyncExecutorShutdownOutcome, FlowableError> {
        let _lifecycle_guard = self
            .lifecycle_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let was_active = self.is_active.load(Ordering::SeqCst);
        if !was_active {
            return Ok(AsyncExecutorShutdownOutcome::AlreadyInactive);
        }
        if let Some(acq) = &self.async_acquisition {
            acq.request_stop();
        }
        if let Some(acq) = &self.timer_acquisition {
            acq.request_stop();
        }
        if let Some(reset) = &self.reset_expired {
            reset.request_stop();
        }
        if let Some(reset) = &self.reset_expired {
            reset.await_stopped();
        }
        if let Some(acq) = &self.timer_acquisition {
            acq.await_stopped();
        }
        if let Some(acq) = &self.async_acquisition {
            acq.await_stopped();
        }
        let pool = self
            .task_executor
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(pool) = pool {
            pool.shutdown();
        }
        let runtime_service = if was_active {
            self.runtime_service
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned()
        } else {
            None
        };
        if was_active && self.config.unlocks_owned_jobs_on_shutdown() {
            if let Some(runtime_service) = runtime_service.as_ref() {
                runtime_service.unlock_owned_jobs(&self.config.tenant_ids)?;
            }
        }
        self.runtime_service
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        self.is_active.store(false, Ordering::SeqCst);
        Ok(AsyncExecutorShutdownOutcome::Shutdown)
    }

    fn execute_temporary_jobs(&self) -> Result<(), FlowableError> {
        loop {
            let temporary_job = self
                .temporary_jobs
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front();
            let Some(temporary_job) = temporary_job else {
                return Ok(());
            };
            let work = TimerWork::RuntimeJob(temporary_job.job.clone());
            let task = if temporary_job.direct_hint {
                spawn_direct_hint_work(Arc::clone(&temporary_job.runtime_service), work)
            } else {
                spawn_timer_work(
                    Arc::clone(&temporary_job.runtime_service),
                    work,
                    temporary_job.fencing_token,
                )
            };
            let submitted = self
                .task_executor
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .map(|pool| pool.execute(task).is_ok())
                .unwrap_or(false);
            if !submitted {
                temporary_job
                    .runtime_service
                    .reject_acquired_async_job(&temporary_job.job)?;
            }
        }
    }

    pub fn start_history_dispatcher(
        &self,
        runtime_service: Arc<RuntimeService>,
        dispatcher: &mut HistoryJobDispatcher,
        runtime_store: RuntimeStore,
    ) {
        dispatcher.start(
            runtime_service,
            Arc::clone(&self.task_executor),
            runtime_store,
        );
    }

    pub fn execute_async_job(
        &self,
        runtime_service: Arc<RuntimeService>,
        work: TimerWork,
        fencing_token: i64,
    ) -> bool {
        let _lifecycle_guard = self
            .lifecycle_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.is_active.load(Ordering::SeqCst) {
            if let TimerWork::RuntimeJob(job) = work {
                self.temporary_jobs
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push_back(TemporaryRuntimeJob {
                        runtime_service,
                        job,
                        fencing_token,
                        direct_hint: false,
                    });
                return true;
            }
            return false;
        }
        let task = spawn_timer_work(runtime_service, work, fencing_token);
        let guard = self.task_executor.lock().unwrap();
        guard
            .as_ref()
            .map(|pool| pool.execute(task).is_ok())
            .unwrap_or(false)
    }

    /// Offer a pre-locked activation hint to the executor pool.
    ///
    /// Unlike [`execute_async_job`](Self::execute_async_job), the job carries a
    /// valid executor row lock (owner + expiration set inside the activating
    /// transaction), so it executes through the direct-hint path: the timer
    /// coordinator lease is skipped and the row lock is re-verified before the
    /// body runs. Returns `false` when the pool rejected the job (queue full /
    /// shut down); the caller then dispatches `JOB_REJECTED` and CAS-releases
    /// the pre-lock.
    pub fn submit_direct_hint_job(
        &self,
        runtime_service: Arc<RuntimeService>,
        job: crate::persistence::runtime_store::RuntimeTimerJobState,
    ) -> bool {
        let _lifecycle_guard = self
            .lifecycle_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.is_active.load(Ordering::SeqCst) {
            // An inactive activation coordinator persists an unlocked job: it
            // cannot hand us the executor-row lease required by the direct-hint
            // path. Do not enqueue that unlocked row alongside the polling
            // acquisition thread started later, or both paths can submit the
            // same job. The durable row is the queue in this case and startup
            // acquisition will pick it up normally.
            //
            // A job already leased by this executor can occur during a lifecycle
            // hand-off. Preserve that hint so its lease is not left waiting for
            // expiry, but only when all lease fields needed for fencing exist.
            let owns_lease = job.lock_owner.as_deref() == Some(self.lock_owner())
                && job.lock_time.is_some()
                && job.lock_expiration_time.is_some();
            if owns_lease {
                self.temporary_jobs
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push_back(TemporaryRuntimeJob {
                        runtime_service,
                        job,
                        fencing_token: 0,
                        direct_hint: true,
                    });
            }
            return true;
        }
        let work = TimerWork::RuntimeJob(job);
        let task = spawn_direct_hint_work(runtime_service, work);
        let guard = self.task_executor.lock().unwrap();
        guard
            .as_ref()
            .map(|pool| pool.execute(task).is_ok())
            .unwrap_or(false)
    }

    pub fn remaining_capacity(&self) -> usize {
        self.task_executor
            .lock()
            .unwrap()
            .as_ref()
            .map(|pool| pool.remaining_capacity())
            .unwrap_or(0)
    }

    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::SeqCst)
    }

    pub fn configuration(&self) -> &AsyncExecutorConfiguration {
        &self.config
    }

    pub fn topology(&self) -> AsyncExecutorTopology {
        self.config.topology
    }

    pub fn tenant_scope(&self) -> AsyncExecutorTenantScope {
        self.config.tenant_scope()
    }

    pub fn unlocks_owned_jobs_on_start(&self) -> bool {
        self.config.unlocks_owned_jobs_on_start()
    }

    pub fn unlocks_owned_jobs_on_shutdown(&self) -> bool {
        self.config.unlocks_owned_jobs_on_shutdown()
    }

    /// Effective owner used by this executor for runtime and acquisition locks.
    pub fn lock_owner(&self) -> &str {
        self.config
            .lock_owner
            .as_deref()
            .expect("AsyncExecutor always resolves a lock owner during construction")
    }

    /// Tenant filter applied during job acquisition. Empty means all tenants.
    pub fn tenant_ids(&self) -> &[String] {
        &self.config.tenant_ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::deployment_manager::DeploymentManager;
    use crate::interceptor::command_executor::DefaultCommandExecutor;
    use crate::persistence::db_store::DbStore;
    use crate::persistence::runtime_store::{RuntimeStore, RuntimeTimerJobState};
    use crate::service::config::ProcessEngineConfiguration;
    use flowable_http_service::DeterministicHttpRuntime;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    fn runtime_service_for_test() -> Arc<RuntimeService> {
        let db_store = Arc::new(DbStore::new_in_memory().unwrap());
        let deployment_manager =
            DeploymentManager::new_with_memory_backend_for_test(Arc::clone(&db_store));
        let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
        let command_executor = Arc::new(DefaultCommandExecutor::new(
            deployment_manager,
            runtime_store,
            Arc::new(ProcessEngineConfiguration::default()),
            Arc::new(DeterministicHttpRuntime::default()),
        ));
        Arc::new(RuntimeService::new(
            command_executor,
            Arc::from("async-executor-restart-test-owner"),
        ))
    }

    #[test]
    fn executor_is_inactive_until_started() {
        let executor = AsyncExecutor::new(AsyncExecutorConfiguration {
            pool_size: 1,
            async_job_acquisition_enabled: false,
            timer_job_acquisition_enabled: false,
            reset_expired_job_enabled: false,
            ..AsyncExecutorConfiguration::default()
        });

        assert!(!executor.is_active());
        assert_eq!(executor.remaining_capacity(), 0);
        executor.shutdown();
        assert!(!executor.is_active());
        assert_eq!(executor.remaining_capacity(), 0);
    }

    #[test]
    fn executor_recreates_task_pool_when_started_after_shutdown() {
        let executor = AsyncExecutor::new(AsyncExecutorConfiguration {
            pool_size: 1,
            queue_size: 1,
            async_job_acquisition_enabled: false,
            timer_job_acquisition_enabled: false,
            reset_expired_job_enabled: false,
            ..AsyncExecutorConfiguration::default()
        });

        let runtime_service = runtime_service_for_test();
        executor.start(Arc::clone(&runtime_service));
        assert!(executor.is_active());
        assert_eq!(executor.remaining_capacity(), 1);
        executor.shutdown();
        assert!(!executor.is_active());
        assert_eq!(executor.remaining_capacity(), 0);

        executor.start(Arc::clone(&runtime_service));
        executor.start(runtime_service);
        assert!(executor.is_active());

        let executed = Arc::new(AtomicBool::new(false));
        let executed_for_task = Arc::clone(&executed);
        let (completed_tx, completed_rx) = crossbeam_channel::bounded::<()>(1);
        let task_executor = executor
            .task_executor
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        task_executor
            .as_ref()
            .expect("start should recreate the task executor")
            .execute(Box::new(move || {
                executed_for_task.store(true, Ordering::SeqCst);
                completed_tx.send(()).unwrap();
            }))
            .unwrap();
        drop(task_executor);

        completed_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("recreated task executor should execute submitted work");
        assert!(executed.load(Ordering::SeqCst));

        executor.shutdown();
        executor.shutdown();
        assert!(!executor.is_active());
        assert_eq!(executor.remaining_capacity(), 0);
    }

    #[test]
    fn executor_retains_runtime_service_until_shutdown_unlock_finishes() {
        let executor = AsyncExecutor::new(AsyncExecutorConfiguration {
            pool_size: 1,
            unlock_owned_jobs: false,
            async_job_acquisition_enabled: false,
            timer_job_acquisition_enabled: false,
            reset_expired_job_enabled: false,
            ..AsyncExecutorConfiguration::default()
        });
        let runtime_service = runtime_service_for_test();
        let runtime_service_weak = Arc::downgrade(&runtime_service);

        executor.start(runtime_service);
        assert!(runtime_service_weak.upgrade().is_some());

        executor.shutdown();
        assert!(runtime_service_weak.upgrade().is_none());
    }

    #[test]
    fn typed_lifecycle_outcomes_preserve_idempotent_public_behavior() {
        let executor = AsyncExecutor::new(AsyncExecutorConfiguration {
            pool_size: 1,
            unlock_owned_jobs: false,
            async_job_acquisition_enabled: false,
            timer_job_acquisition_enabled: false,
            reset_expired_job_enabled: false,
            ..AsyncExecutorConfiguration::default()
        });
        let runtime_service = runtime_service_for_test();

        assert_eq!(
            executor.try_shutdown().unwrap(),
            AsyncExecutorShutdownOutcome::AlreadyInactive
        );
        assert_eq!(
            executor.try_start(Arc::clone(&runtime_service)).unwrap(),
            AsyncExecutorStartOutcome::Started
        );
        assert_eq!(
            executor.try_start(runtime_service).unwrap(),
            AsyncExecutorStartOutcome::AlreadyActive
        );
        assert_eq!(
            executor.try_shutdown().unwrap(),
            AsyncExecutorShutdownOutcome::Shutdown
        );
        assert_eq!(
            executor.try_shutdown().unwrap(),
            AsyncExecutorShutdownOutcome::AlreadyInactive
        );
    }

    #[test]
    fn inactive_runtime_job_is_deferred_and_drained_when_started() {
        let executor = AsyncExecutor::new(AsyncExecutorConfiguration {
            pool_size: 1,
            queue_size: 2,
            unlock_owned_jobs: false,
            async_job_acquisition_enabled: false,
            timer_job_acquisition_enabled: false,
            reset_expired_job_enabled: false,
            ..AsyncExecutorConfiguration::default()
        });
        let runtime_service = runtime_service_for_test();
        let job = RuntimeTimerJobState {
            timer_job_id: "temporary-job".to_string(),
            process_instance_id: "temporary-process".to_string(),
            execution_id: "temporary-execution".to_string(),
            activity_id: "temporary-activity".to_string(),
            job_state: Some("async".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(0),
            lock_owner: Some(executor.lock_owner().to_string()),
            lock_time: Some(0),
            lock_expiration_time: Some(1),
            retries: Some(3),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        };

        assert!(executor.execute_async_job(
            Arc::clone(&runtime_service),
            TimerWork::RuntimeJob(job),
            1,
        ));
        assert_eq!(executor.temporary_jobs.lock().unwrap().len(), 1);

        assert_eq!(
            executor.try_start(runtime_service).unwrap(),
            AsyncExecutorStartOutcome::Started
        );
        assert!(executor.temporary_jobs.lock().unwrap().is_empty());
        executor.shutdown();
    }

    #[test]
    fn inactive_direct_hint_defers_unlocked_job_to_persistent_acquisition() {
        let executor = AsyncExecutor::new(AsyncExecutorConfiguration {
            pool_size: 1,
            async_job_acquisition_enabled: false,
            timer_job_acquisition_enabled: false,
            reset_expired_job_enabled: false,
            ..AsyncExecutorConfiguration::default()
        });
        let runtime_service = runtime_service_for_test();
        let unlocked = RuntimeTimerJobState {
            timer_job_id: "persistent-unlocked-job".to_string(),
            job_state: Some("async".to_string()),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            ..Default::default()
        };

        assert!(executor.submit_direct_hint_job(runtime_service, unlocked));
        assert!(
            executor.temporary_jobs.lock().unwrap().is_empty(),
            "an unlocked durable job must be acquired by the startup poller, not submitted twice"
        );
    }

    #[test]
    fn inactive_direct_hint_preserves_job_prelocked_by_this_executor() {
        let executor = AsyncExecutor::new(AsyncExecutorConfiguration {
            pool_size: 1,
            async_job_acquisition_enabled: false,
            timer_job_acquisition_enabled: false,
            reset_expired_job_enabled: false,
            ..AsyncExecutorConfiguration::default()
        });
        let runtime_service = runtime_service_for_test();
        let prelocked = RuntimeTimerJobState {
            timer_job_id: "persistent-prelocked-job".to_string(),
            job_state: Some("async".to_string()),
            lock_owner: Some(executor.lock_owner().to_string()),
            lock_time: Some(10),
            lock_expiration_time: Some(20),
            ..Default::default()
        };

        assert!(executor.submit_direct_hint_job(runtime_service, prelocked));
        let queued = executor.temporary_jobs.lock().unwrap();
        assert_eq!(queued.len(), 1);
        assert!(queued.front().unwrap().direct_hint);
    }
}
