use crate::engine::async_task_executor::{
    AsyncTask, AsyncTaskExecutor, AsyncTaskExecutorConfig, AsyncTaskSender, RejectedExecutionError,
};
use crate::engine::job_runnable::spawn_timer_work;
use crate::engine::reset_expired_jobs::{ResetExpiredJobs, ResetExpiredJobsConfig};
use crate::engine::runtime_service::RuntimeService;
use crate::engine::timer_worker::TimerWork;
use crate::persistence::runtime_store::ExpiredJobClass;
use crate::service::config::AsyncHistoryConfiguration;
use crossbeam_channel::TrySendError;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// An independent executor for async history jobs with its own thread pool
/// and acquisition cycle. Used when `AsyncHistoryConfiguration::use_shared_executor = false`.
///
/// Java equivalent: `DefaultAsyncHistoryJobExecutor` which has its own
/// thread pool and acquisition thread separate from the main async executor.
pub struct AsyncHistoryExecutor {
    task_executor: Arc<Mutex<Option<AsyncTaskExecutor>>>,
    acquisition_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
    reset_expired: Option<ResetExpiredJobs>,
    is_active: Arc<AtomicBool>,
    config: AsyncHistoryConfiguration,
}

impl AsyncHistoryExecutor {
    pub fn new(config: AsyncHistoryConfiguration) -> Self {
        let reset_config = history_reset_config(&config);
        Self::with_reset_config(config, reset_config)
    }

    /// Additive constructor that accepts a pre-resolved history-only reset policy.
    pub fn with_reset_config(
        config: AsyncHistoryConfiguration,
        reset_config: Option<ResetExpiredJobsConfig>,
    ) -> Self {
        let pool = AsyncTaskExecutor::new(AsyncTaskExecutorConfig {
            pool_size: config.pool_size,
            queue_size: config.queue_size,
            keep_alive_ms: 5000,
            thread_name_prefix: "flowable-async-history".to_string(),
            ..AsyncTaskExecutorConfig::default()
        });
        Self {
            task_executor: Arc::new(Mutex::new(Some(pool))),
            acquisition_handle: std::sync::Mutex::new(None),
            reset_expired: reset_config.map(ResetExpiredJobs::new),
            is_active: Arc::new(AtomicBool::new(false)),
            config,
        }
    }

    pub fn start(&self, runtime_service: Arc<RuntimeService>) {
        if self.is_active.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(reset) = &self.reset_expired {
            reset.start(Arc::clone(&runtime_service));
        }
        let is_active = Arc::clone(&self.is_active);
        let runtime_service_clone = Arc::clone(&runtime_service);
        let task_executor_clone = Arc::clone(&self.task_executor);
        let acquire_interval = self.config.acquire_interval_ms;
        let lock_duration_ms = 300_000i64;

        let handle = thread::spawn(move || {
            history_acquisition_loop(
                runtime_service_clone,
                task_executor_clone,
                is_active,
                acquire_interval,
                lock_duration_ms,
            );
        });
        *self
            .acquisition_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }

    pub fn shutdown(&self) {
        if !self.is_active.swap(false, Ordering::SeqCst) {
            // Still stop reset if a partial start left it running.
            if let Some(reset) = &self.reset_expired {
                reset.stop();
            }
            return;
        }
        if let Some(reset) = &self.reset_expired {
            reset.request_stop();
        }
        let mut guard = self
            .acquisition_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = guard.take() {
            let _ = handle.join();
        }
        if let Some(reset) = &self.reset_expired {
            reset.await_stopped();
        }
        if let Ok(mut executor_guard) = self.task_executor.lock()
            && let Some(pool) = executor_guard.take()
        {
            pool.shutdown();
        }
    }

    pub fn is_active(&self) -> bool {
        self.is_active.load(Ordering::SeqCst)
    }
}

fn history_reset_config(config: &AsyncHistoryConfiguration) -> Option<ResetExpiredJobsConfig> {
    if !config.resolved_reset_expired_job_enabled() {
        return None;
    }
    Some(ResetExpiredJobsConfig {
        reset_interval_ms: config.resolved_reset_expired_jobs_interval_ms(),
        page_size: config.resolved_reset_expired_jobs_page_size(),
        job_classes: vec![ExpiredJobClass::History],
        tenant_ids: Vec::new(),
        enabled_job_categories: Vec::new(),
    })
}

fn submit_history_task_or_run_inline(
    task_sender: Option<&AsyncTaskSender>,
    task: AsyncTask,
) -> Result<(), RejectedExecutionError> {
    match task_sender {
        Some(sender) => match sender.try_send(task) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(task)) => {
                task();
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => Err(RejectedExecutionError),
        },
        None => Err(RejectedExecutionError),
    }
}

fn history_acquisition_loop(
    runtime_service: Arc<RuntimeService>,
    task_executor: Arc<Mutex<Option<AsyncTaskExecutor>>>,
    is_active: Arc<AtomicBool>,
    acquire_interval_ms: u64,
    lock_duration_ms: i64,
) {
    while is_active.load(Ordering::SeqCst) {
        let remaining = task_executor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|pool| pool.remaining_capacity())
            .unwrap_or(0);

        if remaining == 0 {
            thread::sleep(Duration::from_millis(acquire_interval_ms));
            continue;
        }

        let batch = remaining.min(512);
        let jobs = runtime_service.acquire_history_jobs(lock_duration_ms, batch);
        if jobs.is_empty() {
            thread::sleep(Duration::from_millis(acquire_interval_ms));
            continue;
        }

        let task_sender: Option<AsyncTaskSender> = task_executor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|pool| pool.try_clone_sender());

        for job in &jobs {
            let work = TimerWork::RuntimeJob(job.clone());
            let task = spawn_timer_work(Arc::clone(&runtime_service), work, 0);
            if submit_history_task_or_run_inline(task_sender.as_ref(), task).is_err() {
                runtime_service.release_timer_job_lock(&job.timer_job_id);
            }
        }
    }
}
