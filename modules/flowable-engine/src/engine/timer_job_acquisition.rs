use crate::engine::async_task_executor::{AsyncTask, AsyncTaskExecutor, RejectedExecutionError};
use crate::engine::job_runnable::spawn_timer_work;
use crate::engine::lock_manager::{
    ACQUIRE_TIMER_JOBS_GLOBAL_LOCK, GlobalAcquirePermit, LockManager, LockManagerConfig,
};
use crate::engine::runtime_service::RuntimeService;
use crate::engine::timer_worker::TimerWork;
use crate::engine::timer_worker::{TimerWorker, TimerWorkerConfig};
use crate::error::FlowableError;
use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct TimerJobAcquisitionConfig {
    pub timer_job_acquire_wait_ms: u64,
    pub queue_full_wait_ms: u64,
    pub max_jobs_per_acquisition: usize,
    pub coordinator_lease_timeout_ms: u64,
    pub poll_interval_ms: u64,
    pub max_jitter_ms: u64,
    pub global_acquire_lock_enabled: bool,
    pub global_acquire_lock_name: String,
    pub global_acquire_lock_wait_ms: u64,
    pub global_acquire_lock_poll_rate_ms: u64,
    pub global_acquire_lock_lease_ms: u64,
    pub lock_owner: String,
    /// Empty = acquire jobs for all tenants (default / shared mode).
    pub tenant_ids: Vec<String>,
    /// When non-empty, only acquire jobs whose category is in this list.
    /// Jobs with NULL category are excluded when filtering is active, matching
    /// Java's enabledJobCategories semantics. Empty = no category filtering.
    pub enabled_job_categories: Vec<String>,
}

impl Default for TimerJobAcquisitionConfig {
    fn default() -> Self {
        Self {
            timer_job_acquire_wait_ms: 10_000,
            queue_full_wait_ms: 5_000,
            max_jobs_per_acquisition: 512,
            coordinator_lease_timeout_ms: 300_000,
            poll_interval_ms: 100,
            max_jitter_ms: 50,
            global_acquire_lock_enabled: false,
            global_acquire_lock_name: ACQUIRE_TIMER_JOBS_GLOBAL_LOCK.to_string(),
            global_acquire_lock_wait_ms: 60_000,
            global_acquire_lock_poll_rate_ms: 500,
            global_acquire_lock_lease_ms: 600_000,
            lock_owner: "timer-acq".to_string(),
            tenant_ids: Vec::new(),
            enabled_job_categories: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimerWorkSubmissionOutcome {
    Submitted { count: usize },
    Rejected { submitted: usize, rejected: usize },
}

#[derive(Debug)]
struct TimerWorkReleaseFailure {
    work_id: String,
    error: FlowableError,
}

#[derive(Debug)]
struct TimerWorkSubmissionError {
    outcome: TimerWorkSubmissionOutcome,
    release_failures: Vec<TimerWorkReleaseFailure>,
}

fn timer_work_id(work: &TimerWork) -> &str {
    match work {
        TimerWork::RuntimeJob(job) => &job.timer_job_id,
        TimerWork::ProcessStart(subscription) => &subscription.id,
        TimerWork::EventSubprocess(subscription) => &subscription.subscription_id,
    }
}

fn submit_timer_task_or_run_inline(
    task_sender: Option<&crate::engine::async_task_executor::AsyncTaskSender>,
    task: AsyncTask,
) -> Result<(), RejectedExecutionError> {
    match task_sender {
        Some(sender) => match sender.try_send(task) {
            Ok(()) => Ok(()),
            Err(crossbeam_channel::TrySendError::Full(task)) => {
                task();
                Ok(())
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => Err(RejectedExecutionError),
        },
        None => Err(RejectedExecutionError),
    }
}

fn submit_acquired_timer_work_with(
    runtime_service: &Arc<RuntimeService>,
    works: Vec<TimerWork>,
    fencing_token: i64,
    mut submit: impl FnMut(AsyncTask) -> Result<(), RejectedExecutionError>,
) -> Result<TimerWorkSubmissionOutcome, TimerWorkSubmissionError> {
    let mut works = works.into_iter();
    let mut submitted = 0usize;

    while let Some(work) = works.next() {
        let task = spawn_timer_work(Arc::clone(runtime_service), work.clone(), fencing_token);
        if submit(task).is_ok() {
            submitted += 1;
            continue;
        }

        let rejected_works = std::iter::once(work).chain(works);
        let mut rejected = 0usize;
        let mut release_failures = Vec::new();
        for rejected_work in rejected_works {
            rejected += 1;
            if let Err(error) = runtime_service.reject_acquired_timer_work(&rejected_work) {
                release_failures.push(TimerWorkReleaseFailure {
                    work_id: timer_work_id(&rejected_work).to_string(),
                    error,
                });
            }
        }

        let outcome = TimerWorkSubmissionOutcome::Rejected {
            submitted,
            rejected,
        };
        return if release_failures.is_empty() {
            Ok(outcome)
        } else {
            Err(TimerWorkSubmissionError {
                outcome,
                release_failures,
            })
        };
    }

    Ok(TimerWorkSubmissionOutcome::Submitted { count: submitted })
}

fn timer_submission_wait_ms(
    config: &TimerJobAcquisitionConfig,
    worker_config: &TimerWorkerConfig,
    outcome: TimerWorkSubmissionOutcome,
) -> u64 {
    match outcome {
        TimerWorkSubmissionOutcome::Rejected { .. } => config.queue_full_wait_ms,
        TimerWorkSubmissionOutcome::Submitted { count }
            if count < config.max_jobs_per_acquisition =>
        {
            config.timer_job_acquire_wait_ms + worker_config.get_jitter_ms()
        }
        TimerWorkSubmissionOutcome::Submitted { .. } if config.global_acquire_lock_enabled => {
            config.global_acquire_lock_poll_rate_ms.max(1)
        }
        TimerWorkSubmissionOutcome::Submitted { .. } => 0,
    }
}

enum TimerJobAcquisitionLifecycle {
    Stopped,
    Running {
        handle: JoinHandle<()>,
        stop_tx: Sender<()>,
    },
    Stopping,
}

pub struct TimerJobAcquisition {
    is_active: Arc<AtomicBool>,
    config: TimerJobAcquisitionConfig,
    lifecycle: Mutex<TimerJobAcquisitionLifecycle>,
    lifecycle_changed: Condvar,
}

impl TimerJobAcquisition {
    pub fn new(config: TimerJobAcquisitionConfig) -> Self {
        Self {
            is_active: Arc::new(AtomicBool::new(false)),
            config,
            lifecycle: Mutex::new(TimerJobAcquisitionLifecycle::Stopped),
            lifecycle_changed: Condvar::new(),
        }
    }

    pub fn start(
        &self,
        runtime_service: Arc<RuntimeService>,
        task_executor: Arc<Mutex<Option<AsyncTaskExecutor>>>,
    ) {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while matches!(*lifecycle, TimerJobAcquisitionLifecycle::Stopping) {
            lifecycle = self
                .lifecycle_changed
                .wait(lifecycle)
                .unwrap_or_else(|error| error.into_inner());
        }
        if matches!(*lifecycle, TimerJobAcquisitionLifecycle::Running { .. }) {
            return;
        }

        self.is_active.store(true, Ordering::SeqCst);
        let is_active = Arc::clone(&self.is_active);
        let config = self.config.clone();
        let (stop_tx, stop_rx) = bounded::<()>(1);
        let task_sender: Option<crate::engine::async_task_executor::AsyncTaskSender> = {
            let guard = task_executor.lock().unwrap();
            guard.as_ref().and_then(|e| e.try_clone_sender())
        };
        let handle = thread::spawn(move || {
            let worker = TimerWorker::new(Arc::clone(&runtime_service), "async-timer-acq");
            let worker_config = TimerWorkerConfig {
                poll_interval_ms: config.poll_interval_ms,
                coordinator_lease_timeout_ms: config.coordinator_lease_timeout_ms,
                max_jitter_ms: config.max_jitter_ms,
                ..TimerWorkerConfig::default()
            };

            let lock_manager = if config.global_acquire_lock_enabled {
                Some(LockManager::new(
                    Arc::clone(&runtime_service),
                    LockManagerConfig {
                        lock_name: config.global_acquire_lock_name.clone(),
                        owner: config.lock_owner.clone(),
                        wait_ms: config.global_acquire_lock_wait_ms,
                        poll_rate_ms: config.global_acquire_lock_poll_rate_ms,
                        lease_ms: config.global_acquire_lock_lease_ms,
                    },
                ))
            } else {
                None
            };

            while is_active.load(Ordering::SeqCst) {
                let permit = match lock_manager.as_ref() {
                    Some(lock_manager) => match wait_for_permit_or_stop(
                        lock_manager,
                        &stop_rx,
                        &is_active,
                        config.global_acquire_lock_wait_ms,
                        config.global_acquire_lock_poll_rate_ms,
                    ) {
                        Ok(Some(permit)) => Some(permit),
                        Ok(None) => {
                            if !is_active.load(Ordering::SeqCst)
                                || wait_for_stop(
                                    &stop_rx,
                                    config.timer_job_acquire_wait_ms
                                        + worker_config.get_jitter_ms(),
                                )
                            {
                                break;
                            }
                            continue;
                        }
                        Err(error) => {
                            tracing::error!("failed to acquire timer global lock: {error}");
                            if wait_for_stop(
                                &stop_rx,
                                config.timer_job_acquire_wait_ms + worker_config.get_jitter_ms(),
                            ) {
                                break;
                            }
                            continue;
                        }
                    },
                    None => None,
                };

                if !is_active.load(Ordering::SeqCst) {
                    drop(permit);
                    break;
                }

                let works = worker.acquire_due_scheduled_timers_for_tenants(
                    config.coordinator_lease_timeout_ms,
                    &config.tenant_ids,
                    &config.enabled_job_categories,
                    config.max_jobs_per_acquisition,
                    permit.as_ref(),
                );
                let token = worker.get_fencing_token();

                if let Some(permit) = permit {
                    if let Err(error) = permit.finish() {
                        tracing::error!("failed to release timer global lock: {error}");
                        if wait_for_stop(
                            &stop_rx,
                            config.timer_job_acquire_wait_ms + worker_config.get_jitter_ms(),
                        ) {
                            break;
                        }
                        continue;
                    }
                }

                let works = match works {
                    Ok(works) => works,
                    Err(error) => {
                        tracing::error!("failed to acquire scheduled timer work: {error}");
                        if wait_for_stop(
                            &stop_rx,
                            config.timer_job_acquire_wait_ms + worker_config.get_jitter_ms(),
                        ) {
                            break;
                        }
                        continue;
                    }
                };

                if token <= 0 {
                    if wait_for_stop(
                        &stop_rx,
                        config.timer_job_acquire_wait_ms + worker_config.get_jitter_ms(),
                    ) {
                        break;
                    }
                    continue;
                }

                let outcome =
                    match submit_acquired_timer_work_with(&runtime_service, works, token, |task| {
                        submit_timer_task_or_run_inline(task_sender.as_ref(), task)
                    }) {
                        Ok(outcome) => outcome,
                        Err(error) => {
                            for failure in &error.release_failures {
                                tracing::error!(
                                    "failed to release rejected timer work {}: {}",
                                    failure.work_id,
                                    failure.error
                                );
                            }
                            error.outcome
                        }
                    };
                let wait_ms = timer_submission_wait_ms(&config, &worker_config, outcome);
                if wait_ms > 0 && wait_for_stop(&stop_rx, wait_ms) {
                    break;
                }
            }
            worker.graceful_shutdown();
            is_active.store(false, Ordering::SeqCst);
        });
        *lifecycle = TimerJobAcquisitionLifecycle::Running { handle, stop_tx };
    }

    pub fn stop(&self) {
        self.request_stop();
        self.await_stopped();
    }

    pub(crate) fn request_stop(&self) {
        self.is_active.store(false, Ordering::SeqCst);
        let lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let TimerJobAcquisitionLifecycle::Running { stop_tx, .. } = &*lifecycle {
            let _ = stop_tx.try_send(());
        }
    }

    pub(crate) fn await_stopped(&self) {
        let handle = {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            loop {
                match std::mem::replace(&mut *lifecycle, TimerJobAcquisitionLifecycle::Stopping) {
                    TimerJobAcquisitionLifecycle::Running { handle, stop_tx } => {
                        let _ = stop_tx.try_send(());
                        break Some(handle);
                    }
                    TimerJobAcquisitionLifecycle::Stopped => {
                        *lifecycle = TimerJobAcquisitionLifecycle::Stopped;
                        break None;
                    }
                    TimerJobAcquisitionLifecycle::Stopping => {
                        *lifecycle = TimerJobAcquisitionLifecycle::Stopping;
                        lifecycle = self
                            .lifecycle_changed
                            .wait(lifecycle)
                            .unwrap_or_else(|error| error.into_inner());
                    }
                }
            }
        };

        let Some(handle) = handle else {
            return;
        };
        let _ = handle.join();

        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *lifecycle = TimerJobAcquisitionLifecycle::Stopped;
        self.lifecycle_changed.notify_all();
    }
}

fn wait_for_stop(stop_rx: &Receiver<()>, wait_ms: u64) -> bool {
    match stop_rx.recv_timeout(Duration::from_millis(wait_ms)) {
        Ok(()) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => true,
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => false,
    }
}

fn wait_for_permit_or_stop<'manager>(
    lock_manager: &'manager LockManager,
    stop_rx: &Receiver<()>,
    is_active: &AtomicBool,
    wait_ms: u64,
    poll_rate_ms: u64,
) -> Result<Option<GlobalAcquirePermit<'manager>>, FlowableError> {
    if let Some(permit) = lock_manager.try_acquire_permit()? {
        return Ok(Some(permit));
    }

    let deadline = Instant::now().checked_add(Duration::from_millis(wait_ms));
    while is_active.load(Ordering::SeqCst)
        && deadline.map_or(true, |deadline| Instant::now() < deadline)
    {
        if wait_for_stop(stop_rx, poll_rate_ms.max(1)) {
            return Ok(None);
        }
        if let Some(permit) = lock_manager.try_acquire_permit()? {
            return Ok(Some(permit));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::async_task_executor::AsyncTaskExecutorConfig;
    use crate::engine::event_dispatcher::{
        EngineEvent, EngineEventDispatcher, EngineEventListener, EngineEventType,
    };
    use crate::engine::process_engine::ProcessEngine;
    use crate::engine::time_source::SystemTimeSource;
    use crate::persistence::runtime_store::{
        EventSubprocessTimerSubscription, ProcessTimerStartSubscription, RuntimeTimerJobState,
    };
    use crate::service::config::{AsyncExecutorConfiguration, ProcessEngineConfiguration};

    const REJECTION_OWNER: &str = "timer-rejection-owner";

    struct RejectedRuntimeJobRecorder {
        jobs: Arc<Mutex<Vec<RuntimeTimerJobState>>>,
    }

    impl EngineEventListener for RejectedRuntimeJobRecorder {
        fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
            self.jobs
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(event.job().clone());
            Ok(())
        }
    }

    fn sixty_second_wait_config() -> TimerJobAcquisitionConfig {
        TimerJobAcquisitionConfig {
            timer_job_acquire_wait_ms: 60_000,
            queue_full_wait_ms: 60_000,
            max_jitter_ms: 0,
            global_acquire_lock_enabled: false,
            ..TimerJobAcquisitionConfig::default()
        }
    }

    fn task_executor() -> Arc<Mutex<Option<AsyncTaskExecutor>>> {
        Arc::new(Mutex::new(Some(AsyncTaskExecutor::new(
            AsyncTaskExecutorConfig {
                pool_size: 1,
                queue_size: 1,
                keep_alive_ms: 60_000,
                ..AsyncTaskExecutorConfig::default()
            },
        ))))
    }

    fn wait_until_acquisition_is_waiting(runtime_service: &RuntimeService) {
        use crate::persistence::runtime_store::CoordinatorLeadershipStatus;

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let status = runtime_service.get_timer_coordinator_status();
            if status.fencing_token > 0 && status.status == CoordinatorLeadershipStatus::Active {
                thread::sleep(Duration::from_millis(25));
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timer acquisition did not enter its configured wait");
    }

    fn shutdown_task_executor(task_executor: &Arc<Mutex<Option<AsyncTaskExecutor>>>) {
        if let Some(executor) = task_executor.lock().unwrap().take() {
            executor.shutdown();
        }
    }

    fn timer_rejection_test_engine(
        rejected_runtime_jobs: Arc<Mutex<Vec<RuntimeTimerJobState>>>,
    ) -> ProcessEngine {
        let mut event_dispatcher = EngineEventDispatcher::new();
        event_dispatcher.add_typed_event_listener(
            EngineEventType::JobRejected,
            Arc::new(RejectedRuntimeJobRecorder {
                jobs: rejected_runtime_jobs,
            }),
        );
        ProcessEngine::build_with_config(
            "timer-submission-rejection-test".to_string(),
            Arc::new(SystemTimeSource),
            ProcessEngineConfiguration {
                async_executor: AsyncExecutorConfiguration {
                    lock_owner: Some(REJECTION_OWNER.to_string()),
                    ..AsyncExecutorConfiguration::default()
                },
                engine_event_dispatcher: event_dispatcher,
                ..ProcessEngineConfiguration::default()
            },
        )
        .unwrap()
    }

    /// Unique ids so shared-PG full-matrix runs do not collide on fixed keys (P73b).
    fn timer_rejection_ids() -> (String, String, String) {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        (
            format!("rejected-runtime-{suffix}"),
            format!("rejected-process-start-{suffix}"),
            format!("rejected-event-subprocess-{suffix}"),
        )
    }

    fn insert_locked_timer_works(
        engine: &ProcessEngine,
        persist_runtime_job: bool,
    ) -> Vec<TimerWork> {
        let now = 1_700_000_000_000_i64;
        let (runtime_id, process_start_id, event_sub_id) = timer_rejection_ids();
        let runtime_job = RuntimeTimerJobState {
            timer_job_id: runtime_id,
            process_instance_id: "runtime-process".to_string(),
            execution_id: "runtime-execution".to_string(),
            activity_id: "runtime-timer".to_string(),
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: Some("PT0S".to_string()),
            time_date: None,
            time_cycle: None,
            end_date: None,
            calendar_name: None,
            due_time: Some(now - 300),
            lock_owner: Some(REJECTION_OWNER.to_string()),
            lock_time: Some(now),
            lock_expiration_time: Some(now + 60_000),
            retries: Some(3),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        };
        let process_start = ProcessTimerStartSubscription {
            id: process_start_id,
            process_definition_id: "process-definition".to_string(),
            process_definition_key: "process-key".to_string(),
            start_event_id: "timer-start".to_string(),
            start_event_name: None,
            interrupting: true,
            time_duration: Some("PT0S".to_string()),
            time_date: None,
            time_cycle: None,
            end_date: None,
            calendar_name: None,
            due_time: Some(now - 200),
            lock_owner: Some(REJECTION_OWNER.to_string()),
            lock_time: Some(now),
            category: None,
        };
        let event_subprocess = EventSubprocessTimerSubscription {
            subscription_id: event_sub_id,
            process_instance_id: "event-process".to_string(),
            event_subprocess_id: "event-subprocess".to_string(),
            start_event_id: "event-timer-start".to_string(),
            interrupting: true,
            time_duration: Some("PT0S".to_string()),
            time_date: None,
            time_cycle: None,
            end_date: None,
            calendar_name: None,
            due_time: Some(now - 100),
            lock_owner: Some(REJECTION_OWNER.to_string()),
            lock_time: Some(now),
            category: None,
        };

        let store = engine.get_runtime_store();
        let deployment_manager = engine.get_command_executor().deployment_manager().clone();
        let mut session = store.create_session().unwrap();
        if persist_runtime_job {
            store.insert_timer_job_state(&runtime_job, &mut session);
        }
        deployment_manager
            .register_timer_start_subscriptions(vec![process_start.clone()], &mut session);
        store.insert_event_subprocess_timer_subscription(event_subprocess.clone(), &mut session);
        session.flush_and_commit().unwrap();

        vec![
            TimerWork::RuntimeJob(runtime_job),
            TimerWork::ProcessStart(process_start),
            TimerWork::EventSubprocess(event_subprocess),
        ]
    }

    fn assert_subscription_locks_released(engine: &ProcessEngine, works: &[TimerWork]) {
        let store = engine.get_runtime_store();
        let deployment_manager = engine.get_command_executor().deployment_manager().clone();
        let mut session = store.create_session().unwrap();
        let process_start_id = works
            .iter()
            .find_map(|w| match w {
                TimerWork::ProcessStart(s) => Some(s.id.as_str()),
                _ => None,
            })
            .expect("process start work");
        let event_sub_id = works
            .iter()
            .find_map(|w| match w {
                TimerWork::EventSubprocess(s) => Some(s.subscription_id.as_str()),
                _ => None,
            })
            .expect("event subprocess work");
        let process_start = deployment_manager
            .get_timer_start_subscriptions(&mut session)
            .into_iter()
            .find(|subscription| subscription.id == process_start_id)
            .unwrap();
        let event_subprocess = store
            .snapshot_event_subprocess_timer_subscriptions(&mut session)
            .remove(event_sub_id)
            .unwrap();
        session.rollback().unwrap();

        assert!(process_start.lock_owner.is_none());
        assert!(process_start.lock_time.is_none());
        assert_eq!(process_start.due_time, Some(1_699_999_999_800));
        assert!(event_subprocess.lock_owner.is_none());
        assert!(event_subprocess.lock_time.is_none());
        assert_eq!(event_subprocess.due_time, Some(1_699_999_999_900));
    }

    fn runtime_job_id(works: &[TimerWork]) -> String {
        works
            .iter()
            .find_map(|w| match w {
                TimerWork::RuntimeJob(j) => Some(j.timer_job_id.clone()),
                _ => None,
            })
            .expect("runtime job work")
    }

    #[test]
    fn rejected_timer_submission_releases_current_and_all_unsubmitted_work() {
        let rejected_runtime_jobs = Arc::new(Mutex::new(Vec::new()));
        let engine = timer_rejection_test_engine(Arc::clone(&rejected_runtime_jobs));
        let runtime_service = engine.get_runtime_service();
        let works = insert_locked_timer_works(&engine, true);
        let runtime_id = runtime_job_id(&works);
        let works_for_assert = works.clone();

        let outcome = submit_acquired_timer_work_with(&runtime_service, works, 1, |_task| {
            Err(RejectedExecutionError)
        })
        .unwrap();

        assert_eq!(
            outcome,
            TimerWorkSubmissionOutcome::Rejected {
                submitted: 0,
                rejected: 3,
            }
        );
        let observed_rejections = rejected_runtime_jobs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(observed_rejections.len(), 1);
        assert_eq!(observed_rejections[0].timer_job_id, runtime_id);
        assert_eq!(
            observed_rejections[0].lock_owner.as_deref(),
            Some(REJECTION_OWNER)
        );
        assert!(observed_rejections[0].lock_time.is_some());
        assert!(observed_rejections[0].lock_expiration_time.is_some());
        drop(observed_rejections);
        let runtime_job = engine
            .get_management_service()
            .find_job_by_id(&runtime_id)
            .unwrap();
        assert!(runtime_job.lock_owner.is_none());
        assert!(runtime_job.lock_time.is_none());
        assert!(runtime_job.lock_expiration_time.is_none());
        assert_eq!(runtime_job.retries, Some(3));
        assert_subscription_locks_released(&engine, &works_for_assert);
        engine.close();
    }

    #[test]
    fn submitted_timer_work_remains_locked_when_later_submission_is_rejected() {
        let rejected_runtime_jobs = Arc::new(Mutex::new(Vec::new()));
        let engine = timer_rejection_test_engine(Arc::clone(&rejected_runtime_jobs));
        let runtime_service = engine.get_runtime_service();
        let works = insert_locked_timer_works(&engine, true);
        let runtime_id = runtime_job_id(&works);
        let works_for_assert = works.clone();
        let mut attempts = 0usize;

        let outcome = submit_acquired_timer_work_with(&runtime_service, works, 1, |_task| {
            attempts += 1;
            if attempts == 1 {
                Ok(())
            } else {
                Err(RejectedExecutionError)
            }
        })
        .unwrap();

        assert_eq!(
            outcome,
            TimerWorkSubmissionOutcome::Rejected {
                submitted: 1,
                rejected: 2,
            }
        );
        assert!(
            rejected_runtime_jobs
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty(),
            "subscription rejection must not fabricate a JobRejected event"
        );
        let runtime_job = engine
            .get_management_service()
            .find_job_by_id(&runtime_id)
            .unwrap();
        assert_eq!(runtime_job.lock_owner.as_deref(), Some(REJECTION_OWNER));
        assert!(runtime_job.lock_time.is_some());
        assert!(runtime_job.lock_expiration_time.is_some());
        assert_subscription_locks_released(&engine, &works_for_assert);
        engine.close();
    }

    #[test]
    fn timer_release_failure_is_typed_and_does_not_skip_remaining_work() {
        let rejected_runtime_jobs = Arc::new(Mutex::new(Vec::new()));
        let engine = timer_rejection_test_engine(Arc::clone(&rejected_runtime_jobs));
        let runtime_service = engine.get_runtime_service();
        let works = insert_locked_timer_works(&engine, false);
        let runtime_id = runtime_job_id(&works);
        let works_for_assert = works.clone();

        let error = submit_acquired_timer_work_with(&runtime_service, works, 1, |_task| {
            Err(RejectedExecutionError)
        })
        .unwrap_err();

        assert_eq!(
            error.outcome,
            TimerWorkSubmissionOutcome::Rejected {
                submitted: 0,
                rejected: 3,
            }
        );
        assert_eq!(error.release_failures.len(), 1);
        assert_eq!(error.release_failures[0].work_id, runtime_id);
        assert!(
            error.release_failures[0]
                .error
                .to_string()
                .contains(&format!(
                    "failed to unacquire rejected timer work {runtime_id}"
                ))
        );
        let observed_rejections = rejected_runtime_jobs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(observed_rejections.len(), 1);
        assert_eq!(observed_rejections[0].timer_job_id, runtime_id);
        assert_eq!(
            observed_rejections[0].lock_owner.as_deref(),
            Some(REJECTION_OWNER)
        );
        assert!(observed_rejections[0].lock_time.is_some());
        assert!(observed_rejections[0].lock_expiration_time.is_some());
        drop(observed_rejections);
        assert_subscription_locks_released(&engine, &works_for_assert);
        engine.close();
    }

    #[test]
    fn full_timer_move_queue_runs_task_on_acquisition_thread() {
        let (sender, _receiver) = bounded::<AsyncTask>(1);
        sender.try_send(Box::new(|| {})).unwrap();
        let executed = Arc::new(AtomicBool::new(false));
        let executed_for_task = Arc::clone(&executed);

        submit_timer_task_or_run_inline(
            Some(&sender),
            Box::new(move || executed_for_task.store(true, Ordering::SeqCst)),
        )
        .unwrap();

        assert!(executed.load(Ordering::SeqCst));
    }

    #[test]
    fn timer_wait_policy_uses_configured_max_not_queue_capacity_batch() {
        let config = TimerJobAcquisitionConfig {
            timer_job_acquire_wait_ms: 777,
            max_jobs_per_acquisition: 3,
            global_acquire_lock_enabled: true,
            global_acquire_lock_poll_rate_ms: 13,
            ..Default::default()
        };
        let worker_config = TimerWorkerConfig {
            max_jitter_ms: 0,
            ..Default::default()
        };

        assert_eq!(
            timer_submission_wait_ms(
                &config,
                &worker_config,
                TimerWorkSubmissionOutcome::Submitted { count: 1 },
            ),
            config.timer_job_acquire_wait_ms
        );
        assert_eq!(
            timer_submission_wait_ms(
                &config,
                &worker_config,
                TimerWorkSubmissionOutcome::Submitted { count: 3 },
            ),
            config.global_acquire_lock_poll_rate_ms
        );
    }

    #[test]
    fn stop_interrupts_sixty_second_acquisition_wait() {
        let engine = ProcessEngine::new("interruptible-timer-acquisition".to_string());
        let runtime_service = engine.get_runtime_service();
        let task_executor = task_executor();
        let acquisition = TimerJobAcquisition::new(sixty_second_wait_config());

        acquisition.start(Arc::clone(&runtime_service), Arc::clone(&task_executor));
        wait_until_acquisition_is_waiting(&runtime_service);

        let stop_started = Instant::now();
        acquisition.stop();
        assert!(
            stop_started.elapsed() < Duration::from_secs(5),
            "stop must interrupt the 60-second acquisition wait"
        );
        assert!(!acquisition.is_active.load(Ordering::SeqCst));

        shutdown_task_executor(&task_executor);
        engine.close();
    }

    #[test]
    fn stop_is_idempotent_and_acquisition_can_restart() {
        let engine = ProcessEngine::new("restartable-timer-acquisition".to_string());
        let runtime_service = engine.get_runtime_service();
        let task_executor = task_executor();
        let acquisition = TimerJobAcquisition::new(sixty_second_wait_config());

        acquisition.start(Arc::clone(&runtime_service), Arc::clone(&task_executor));
        wait_until_acquisition_is_waiting(&runtime_service);
        acquisition.stop();

        let repeated_stop_started = Instant::now();
        acquisition.stop();
        assert!(repeated_stop_started.elapsed() < Duration::from_secs(1));

        acquisition.start(Arc::clone(&runtime_service), Arc::clone(&task_executor));
        assert!(
            matches!(
                *acquisition
                    .lifecycle
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()),
                TimerJobAcquisitionLifecycle::Running { .. }
            ),
            "restart must create a new acquisition thread"
        );
        wait_until_acquisition_is_waiting(&runtime_service);
        assert!(acquisition.is_active.load(Ordering::SeqCst));

        let restarted_stop_started = Instant::now();
        acquisition.stop();
        assert!(restarted_stop_started.elapsed() < Duration::from_secs(5));
        assert!(!acquisition.is_active.load(Ordering::SeqCst));

        shutdown_task_executor(&task_executor);
        engine.close();
    }
}
