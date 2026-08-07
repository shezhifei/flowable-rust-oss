use crate::engine::async_task_executor::{
    AsyncTask, AsyncTaskExecutor, AsyncTaskSender, RejectedExecutionError,
};
use crate::engine::job_runnable::spawn_timer_work;
use crate::engine::lock_manager::{ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK, LockManager, LockManagerConfig};
use crate::engine::runtime_service::{AsyncJobRejectOutcome, RuntimeService};
use crate::engine::timer_worker::TimerWork;
use crate::error::FlowableError;
use crate::persistence::runtime_store::RuntimeTimerJobState;
use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct AsyncJobAcquisitionConfig {
    pub async_job_acquire_wait_ms: u64,
    pub queue_full_wait_ms: u64,
    pub max_jobs_per_acquisition: usize,
    pub async_job_lock_time_ms: i64,
    pub coordinator_lease_timeout_ms: u64,
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

impl Default for AsyncJobAcquisitionConfig {
    fn default() -> Self {
        Self {
            async_job_acquire_wait_ms: 10_000,
            queue_full_wait_ms: 5_000,
            max_jobs_per_acquisition: 512,
            async_job_lock_time_ms: 300_000,
            coordinator_lease_timeout_ms: 300_000,
            global_acquire_lock_enabled: false,
            global_acquire_lock_name: ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK.to_string(),
            global_acquire_lock_wait_ms: 60_000,
            global_acquire_lock_poll_rate_ms: 500,
            global_acquire_lock_lease_ms: 600_000,
            lock_owner: "async-acq".to_string(),
            tenant_ids: Vec::new(),
            enabled_job_categories: Vec::new(),
        }
    }
}

pub struct AsyncJobAcquisition {
    is_active: Arc<AtomicBool>,
    handle: std::sync::Mutex<Option<JoinHandle<()>>>,
    config: AsyncJobAcquisitionConfig,
    stop_tx: std::sync::Mutex<Option<Sender<()>>>,
}

impl AsyncJobAcquisition {
    pub fn new(config: AsyncJobAcquisitionConfig) -> Self {
        Self {
            is_active: Arc::new(AtomicBool::new(false)),
            handle: std::sync::Mutex::new(None),
            config,
            stop_tx: std::sync::Mutex::new(None),
        }
    }

    pub fn start(
        &self,
        runtime_service: Arc<RuntimeService>,
        task_executor: Arc<Mutex<Option<AsyncTaskExecutor>>>,
    ) {
        {
            let guard = self.handle.lock().unwrap();
            if guard.is_some() {
                return;
            }
        }
        self.is_active.store(true, Ordering::SeqCst);
        let is_active = Arc::clone(&self.is_active);
        let config = self.config.clone();
        let (stop_tx, stop_rx) = bounded::<()>(1);
        *self.stop_tx.lock().unwrap() = Some(stop_tx);
        let task_sender: Option<crate::engine::async_task_executor::AsyncTaskSender> = {
            let guard = task_executor.lock().unwrap();
            guard.as_ref().and_then(|e| e.try_clone_sender())
        };
        let handle = thread::spawn(move || {
            acquisition_loop(runtime_service, task_sender, is_active, config, stop_rx);
        });
        *self.handle.lock().unwrap() = Some(handle);
    }

    pub fn stop(&self) {
        self.request_stop();
        self.await_stopped();
    }

    pub(crate) fn request_stop(&self) {
        self.is_active.store(false, Ordering::SeqCst);
        if let Some(tx) = self.stop_tx.lock().unwrap().as_ref() {
            let _ = tx.try_send(());
        }
    }

    pub(crate) fn await_stopped(&self) {
        let mut guard = self.handle.lock().unwrap();
        if let Some(handle) = guard.take() {
            if handle.join().is_err() {
                tracing::error!("async acquisition thread panicked");
            }
        }
        *self.stop_tx.lock().unwrap() = None;
    }
}

fn submit_async_task_or_run_inline(
    task_sender: Option<&AsyncTaskSender>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AsyncJobSubmissionOutcome {
    Submitted { count: usize },
    Rejected { submitted: usize, rejected: usize },
}

/// A single release infrastructure failure recorded during bulk rejection.
/// The job's lock was NOT released.
#[derive(Debug)]
struct AsyncJobReleaseFailure {
    job_id: String,
    error: FlowableError,
}

/// Aggregated submission error that preserves both the submission outcome
/// (so the acquisition loop can choose the right wait policy) and the
/// collected failures.
///
/// `fatal_listener_error`, when present, means a `JOB_REJECTED` listener
/// returned a fatal error and the batch was short-circuited (matching Java
/// `offerJobs` semantics). Remaining jobs were NOT processed and stay locked.
///
/// `release_failures` aggregates release infrastructure failures across all
/// processed rejected jobs. Each entry corresponds to a job whose lock was
/// NOT released.
#[derive(Debug)]
struct AsyncJobSubmissionError {
    outcome: AsyncJobSubmissionOutcome,
    fatal_listener_error: Option<FlowableError>,
    release_failures: Vec<AsyncJobReleaseFailure>,
}

fn submit_acquired_jobs_with(
    runtime_service: &Arc<RuntimeService>,
    jobs: Vec<RuntimeTimerJobState>,
    mut submit: impl FnMut(AsyncTask) -> Result<(), RejectedExecutionError>,
) -> Result<AsyncJobSubmissionOutcome, AsyncJobSubmissionError> {
    let mut submitted = 0usize;
    let mut rejected = 0usize;
    let mut release_failures = Vec::new();
    let mut fatal_listener_error: Option<FlowableError> = None;

    for job in jobs {
        // A fatal listener error short-circuits the batch (matching Java
        // `offerJobs` semantics). Remaining jobs stay locked and rely on
        // reset-expired recovery.
        if fatal_listener_error.is_some() {
            break;
        }

        let work = TimerWork::RuntimeJob(job.clone());
        let task = spawn_timer_work(Arc::clone(runtime_service), work, 0);
        match submit(task) {
            Ok(()) => submitted += 1,
            Err(_) => {
                rejected += 1;
                match runtime_service.try_reject_acquired_async_job(&job) {
                    AsyncJobRejectOutcome::Released => {}
                    AsyncJobRejectOutcome::ListenerFatal(error) => {
                        fatal_listener_error = Some(error);
                    }
                    AsyncJobRejectOutcome::ReleaseFailure(error) => {
                        // Release infrastructure failure: aggregate and
                        // continue so remaining rejected jobs are still
                        // released.
                        release_failures.push(AsyncJobReleaseFailure {
                            job_id: job.timer_job_id.clone(),
                            error,
                        });
                    }
                }
            }
        }
    }

    let outcome = if rejected == 0 {
        AsyncJobSubmissionOutcome::Submitted { count: submitted }
    } else {
        AsyncJobSubmissionOutcome::Rejected {
            submitted,
            rejected,
        }
    };

    if fatal_listener_error.is_some() || !release_failures.is_empty() {
        Err(AsyncJobSubmissionError {
            outcome,
            fatal_listener_error,
            release_failures,
        })
    } else {
        Ok(outcome)
    }
}

fn normal_acquire_wait_ms(config: &AsyncJobAcquisitionConfig) -> u64 {
    config.async_job_acquire_wait_ms
}

fn acquire_cycle_wait_ms(
    config: &AsyncJobAcquisitionConfig,
    acquired_count: usize,
    outcome: AsyncJobSubmissionOutcome,
) -> u64 {
    match outcome {
        AsyncJobSubmissionOutcome::Rejected { .. } => config.queue_full_wait_ms,
        AsyncJobSubmissionOutcome::Submitted { .. }
            if acquired_count < config.max_jobs_per_acquisition =>
        {
            normal_acquire_wait_ms(config)
        }
        AsyncJobSubmissionOutcome::Submitted { .. } if config.global_acquire_lock_enabled => {
            config.global_acquire_lock_poll_rate_ms.max(1)
        }
        AsyncJobSubmissionOutcome::Submitted { .. } => 0,
    }
}

enum AsyncAcquisitionCycleOutcome {
    Stop,
    Wait(u64),
}

fn acquisition_loop(
    runtime_service: Arc<RuntimeService>,
    task_sender: Option<crate::engine::async_task_executor::AsyncTaskSender>,
    is_active: Arc<AtomicBool>,
    config: AsyncJobAcquisitionConfig,
    stop_rx: Receiver<()>,
) {
    let sleep_with_stop = |ms: u64| {
        let _ = stop_rx.recv_timeout(Duration::from_millis(ms));
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
            Some(lock_manager) => match lock_manager.try_wait_for_permit() {
                Ok(Some(permit)) => Some(permit),
                Ok(None) => {
                    sleep_with_stop(config.async_job_acquire_wait_ms);
                    continue;
                }
                Err(error) => {
                    tracing::error!("failed to acquire async global lock: {error}");
                    sleep_with_stop(config.async_job_acquire_wait_ms);
                    continue;
                }
            },
            None => None,
        };

        let cycle_result = (|| -> Result<AsyncAcquisitionCycleOutcome, FlowableError> {
            if !is_active.load(Ordering::SeqCst) {
                return Ok(AsyncAcquisitionCycleOutcome::Stop);
            }

            // Java global acquisition reads capacity only after entering the
            // global critical section, so the value cannot become stale while
            // waiting for the property lock.
            let remaining = match task_sender.as_ref() {
                Some(s) => s.capacity().unwrap_or(0).saturating_sub(s.len()),
                None => 0,
            };
            if remaining == 0 {
                return Ok(AsyncAcquisitionCycleOutcome::Wait(
                    config.queue_full_wait_ms,
                ));
            }

            let batch = remaining.min(config.max_jobs_per_acquisition);
            let jobs = match permit.as_ref() {
                Some(permit) => runtime_service.acquire_async_jobs_global_for_tenants(
                    permit,
                    config.async_job_lock_time_ms,
                    batch,
                    &config.tenant_ids,
                    &config.enabled_job_categories,
                )?,
                None => runtime_service.try_acquire_async_jobs_for_tenants(
                    config.async_job_lock_time_ms,
                    batch,
                    &config.tenant_ids,
                    &config.enabled_job_categories,
                )?,
            };
            let acquired_count = jobs.len();
            if jobs.is_empty() {
                return Ok(AsyncAcquisitionCycleOutcome::Wait(
                    config.async_job_acquire_wait_ms,
                ));
            }

            let outcome = match submit_acquired_jobs_with(&runtime_service, jobs, |task| {
                submit_async_task_or_run_inline(task_sender.as_ref(), task)
            }) {
                Ok(outcome) => outcome,
                Err(error) => {
                    if let Some(fatal) = error.fatal_listener_error {
                        tracing::error!("async job acquisition fatal listener error: {fatal}");
                    }
                    for failure in &error.release_failures {
                        tracing::error!(
                            "failed to release rejected async job {}: {}",
                            failure.job_id,
                            failure.error
                        );
                    }
                    error.outcome
                }
            };
            Ok(AsyncAcquisitionCycleOutcome::Wait(acquire_cycle_wait_ms(
                &config,
                acquired_count,
                outcome,
            )))
        })();

        // Async global acquisition covers the complete acquire-and-offer
        // cycle, including JOB_REJECTED dispatch and unacquire. Explicit
        // finish propagates release failures; Drop remains the fallback for
        // unwinding and unforeseen early returns.
        if let Some(permit) = permit {
            if let Err(error) = permit.finish() {
                tracing::error!("failed to release async global lock: {error}");
                sleep_with_stop(normal_acquire_wait_ms(&config));
                continue;
            }
        }

        match cycle_result {
            Ok(AsyncAcquisitionCycleOutcome::Stop) => break,
            Ok(AsyncAcquisitionCycleOutcome::Wait(wait_ms)) if wait_ms > 0 => {
                sleep_with_stop(wait_ms);
            }
            Ok(AsyncAcquisitionCycleOutcome::Wait(_)) => {}
            Err(error) => {
                tracing::error!("async job acquisition cycle failed: {error}");
                sleep_with_stop(normal_acquire_wait_ms(&config));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::event_dispatcher::{
        EngineEvent, EngineEventDispatcher, EngineEventListener, EngineEventType,
    };
    use crate::engine::process_engine::ProcessEngine;
    use crate::engine::time_source::{TestTimeSource, TimeSource};
    use crate::persistence::runtime_store::RuntimeTimerJobState;
    use crate::service::config::ProcessEngineConfiguration;
    use chrono::{TimeZone, Utc};
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    /// Unique job-id namespace so shared-PG full-matrix runs do not steal
    /// acquire slots from leftover rows written by earlier tests (P73b).
    fn ns(base: &str) -> String {
        format!("{base}-{}", Uuid::new_v4().simple())
    }

    struct RejectingJobEventListener {
        observed_jobs: Arc<Mutex<Vec<RuntimeTimerJobState>>>,
        fatal: bool,
    }

    impl EngineEventListener for RejectingJobEventListener {
        fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
            self.observed_jobs.lock().unwrap().push(event.job().clone());
            Err(FlowableError::ExecutionError(
                "rejected job listener failed".to_string(),
            ))
        }

        fn is_fail_on_exception(&self) -> bool {
            self.fatal
        }
    }

    fn rejection_test_engine(
        fatal_listener: bool,
        observed_jobs: Arc<Mutex<Vec<RuntimeTimerJobState>>>,
    ) -> (ProcessEngine, Arc<TestTimeSource>) {
        let time_source = Arc::new(TestTimeSource::new(
            Utc.with_ymd_and_hms(2026, 7, 19, 10, 0, 0).unwrap(),
        ));
        let mut event_dispatcher = EngineEventDispatcher::new();
        event_dispatcher.add_typed_event_listener(
            EngineEventType::JobRejected,
            Arc::new(RejectingJobEventListener {
                observed_jobs,
                fatal: fatal_listener,
            }),
        );
        let engine = ProcessEngine::build_with_config(
            "async-job-rejection-test".to_string(),
            time_source.clone(),
            ProcessEngineConfiguration {
                engine_event_dispatcher: event_dispatcher,
                ..Default::default()
            },
        )
        .unwrap();
        (engine, time_source)
    }

    fn insert_async_jobs(engine: &ProcessEngine, time_source: &TestTimeSource, job_ids: &[&str]) {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let now = time_source.now().timestamp_millis();
        for job_id in job_ids {
            store.insert_timer_job_state(
                &RuntimeTimerJobState {
                    timer_job_id: (*job_id).to_string(),
                    process_instance_id: format!("process-{job_id}"),
                    execution_id: format!("execution-{job_id}"),
                    activity_id: "asyncTask".to_string(),
                    job_state: Some("async".to_string()),
                    is_boundary: false,
                    attached_activity_id: None,
                    cancel_activity: false,
                    time_duration: None,
                    time_date: None,
                    time_cycle: None,
                    end_date: None,
                    due_time: Some(now),
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
        }
        session.flush_and_commit().unwrap();
    }

    fn persisted_job(engine: &ProcessEngine, job_id: &str) -> RuntimeTimerJobState {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let job = store
            .find_timer_job_state(job_id, &mut session)
            .expect("async job must remain persisted");
        session.rollback().unwrap();
        job
    }

    #[test]
    fn nonfatal_rejection_listener_observes_locked_jobs_and_entire_batch_is_unacquired() {
        let observed_jobs = Arc::new(Mutex::new(Vec::new()));
        let (engine, time_source) = rejection_test_engine(false, Arc::clone(&observed_jobs));
        let job_ids = [ns("rejected-1"), ns("rejected-2"), ns("rejected-3")];
        let job_id_refs: Vec<&str> = job_ids.iter().map(|s| s.as_str()).collect();
        insert_async_jobs(&engine, time_source.as_ref(), &job_id_refs);
        let runtime_service = engine.get_runtime_service();
        let our_ids: std::collections::HashSet<&str> =
            job_ids.iter().map(|s| s.as_str()).collect();
        let acquired: Vec<_> = runtime_service
            .acquire_async_jobs(300_000, 64)
            .into_iter()
            .filter(|j| our_ids.contains(j.timer_job_id.as_str()))
            .collect();
        assert_eq!(acquired.len(), job_ids.len());
        assert!(acquired.iter().all(|job| job.lock_owner.is_some()));
        assert!(
            acquired
                .iter()
                .all(|job| job.lock_expiration_time.is_some())
        );

        let outcome = submit_acquired_jobs_with(&runtime_service, acquired, |_task| {
            Err(RejectedExecutionError)
        })
        .unwrap();

        assert_eq!(
            outcome,
            AsyncJobSubmissionOutcome::Rejected {
                submitted: 0,
                rejected: job_ids.len(),
            }
        );
        let observed = observed_jobs.lock().unwrap();
        assert_eq!(observed.len(), job_ids.len());
        assert!(observed.iter().all(|job| job.lock_owner.is_some()));
        assert!(
            observed
                .iter()
                .all(|job| job.lock_expiration_time.is_some())
        );
        drop(observed);

        for job_id in &job_ids {
            let job = persisted_job(&engine, job_id);
            assert!(job.lock_owner.is_none());
            assert!(job.lock_time.is_none());
            assert!(job.lock_expiration_time.is_none());
            assert_eq!(job.retries, Some(3));
        }
    }

    #[test]
    fn fatal_rejection_listener_returns_error_and_preserves_job_lock() {
        let observed_jobs = Arc::new(Mutex::new(Vec::new()));
        let (engine, time_source) = rejection_test_engine(true, Arc::clone(&observed_jobs));
        let fatal_id = ns("fatal-rejected");
        insert_async_jobs(&engine, time_source.as_ref(), &[fatal_id.as_str()]);
        let runtime_service = engine.get_runtime_service();
        // Acquire only our job: scan enough to outrun leftover shared-PG rows, then
        // keep the matching id (shared full-matrix isolation).
        let acquired_all = runtime_service.acquire_async_jobs(300_000, 64);
        let acquired: Vec<_> = acquired_all
            .into_iter()
            .filter(|j| j.timer_job_id == fatal_id)
            .collect();
        assert_eq!(acquired.len(), 1, "expected to acquire our fatal-rejected job");
        let locked_job = acquired[0].clone();

        let error = submit_acquired_jobs_with(&runtime_service, acquired, |_task| {
            Err(RejectedExecutionError)
        })
        .unwrap_err();

        let fatal = error
            .fatal_listener_error
            .as_ref()
            .expect("fatal listener error should be present");
        assert!(fatal.to_string().contains("rejected job listener failed"));
        let observed = observed_jobs.lock().unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].lock_owner, locked_job.lock_owner);
        assert_eq!(
            observed[0].lock_expiration_time,
            locked_job.lock_expiration_time
        );
        drop(observed);

        let persisted = persisted_job(&engine, &fatal_id);
        assert_eq!(persisted.lock_owner, locked_job.lock_owner);
        assert_eq!(persisted.lock_time, locked_job.lock_time);
        assert_eq!(
            persisted.lock_expiration_time,
            locked_job.lock_expiration_time
        );
    }

    #[test]
    fn rejected_submission_uses_queue_full_wait_policy() {
        let config = AsyncJobAcquisitionConfig {
            async_job_acquire_wait_ms: 777,
            queue_full_wait_ms: 4_321,
            global_acquire_lock_enabled: true,
            global_acquire_lock_poll_rate_ms: 13,
            ..Default::default()
        };

        assert_eq!(
            acquire_cycle_wait_ms(
                &config,
                3,
                AsyncJobSubmissionOutcome::Rejected {
                    submitted: 1,
                    rejected: 2,
                },
            ),
            config.queue_full_wait_ms
        );
        assert_eq!(
            acquire_cycle_wait_ms(
                &config,
                3,
                AsyncJobSubmissionOutcome::Submitted { count: 2 },
            ),
            normal_acquire_wait_ms(&config)
        );
    }

    #[test]
    fn global_wait_policy_compares_acquired_count_with_configured_max() {
        let config = AsyncJobAcquisitionConfig {
            async_job_acquire_wait_ms: 777,
            max_jobs_per_acquisition: 512,
            global_acquire_lock_enabled: true,
            global_acquire_lock_poll_rate_ms: 13,
            ..Default::default()
        };

        assert_eq!(
            acquire_cycle_wait_ms(
                &config,
                1,
                AsyncJobSubmissionOutcome::Submitted { count: 1 },
            ),
            config.async_job_acquire_wait_ms
        );
        assert_eq!(
            acquire_cycle_wait_ms(
                &config,
                512,
                AsyncJobSubmissionOutcome::Submitted { count: 512 },
            ),
            config.global_acquire_lock_poll_rate_ms
        );
    }

    #[test]
    fn fatal_listener_error_short_circuits_batch_and_remaining_jobs_stay_locked() {
        let observed_jobs = Arc::new(Mutex::new(Vec::new()));
        let (engine, time_source) = rejection_test_engine(true, Arc::clone(&observed_jobs));
        // Insert 3 jobs; the first rejection will trigger a fatal listener
        // error, so the remaining 2 jobs must NOT be submitted or rejected.
        let job_ids = [ns("fatal-1"), ns("fatal-2"), ns("fatal-3")];
        let job_id_refs: Vec<&str> = job_ids.iter().map(|s| s.as_str()).collect();
        insert_async_jobs(&engine, time_source.as_ref(), &job_id_refs);
        let runtime_service = engine.get_runtime_service();
        let our_ids: std::collections::HashSet<&str> =
            job_ids.iter().map(|s| s.as_str()).collect();
        let acquired: Vec<_> = runtime_service
            .acquire_async_jobs(300_000, 64)
            .into_iter()
            .filter(|j| our_ids.contains(j.timer_job_id.as_str()))
            .collect();
        assert_eq!(acquired.len(), job_ids.len());
        let locked_by_id: std::collections::HashMap<String, Option<String>> = acquired
            .iter()
            .map(|j| (j.timer_job_id.clone(), j.lock_owner.clone()))
            .collect();

        let error = submit_acquired_jobs_with(&runtime_service, acquired, |_task| {
            Err(RejectedExecutionError)
        })
        .unwrap_err();

        // Fatal listener error is present and short-circuited the batch.
        let fatal = error
            .fatal_listener_error
            .as_ref()
            .expect("fatal listener error should short-circuit the batch");
        assert!(fatal.to_string().contains("rejected job listener failed"));

        // Only the first job was observed by the listener before short-circuit.
        let observed = observed_jobs.lock().unwrap();
        assert_eq!(
            observed.len(),
            1,
            "fatal listener should short-circuit after first job"
        );
        drop(observed);

        // The first job's lock is NOT released (fatal listener error occurs
        // before release). Remaining jobs were never processed and stay locked.
        for job_id in &job_ids {
            let persisted = persisted_job(&engine, job_id);
            assert_eq!(
                persisted.lock_owner,
                locked_by_id.get(job_id).cloned().flatten(),
                "job {job_id} should remain locked after fatal listener short-circuit"
            );
        }

        // Outcome reflects only the first rejected job.
        assert_eq!(
            error.outcome,
            AsyncJobSubmissionOutcome::Rejected {
                submitted: 0,
                rejected: 1,
            }
        );
        assert!(
            error.release_failures.is_empty(),
            "no release infrastructure failures expected in fatal listener scenario"
        );
    }

    #[test]
    fn release_infrastructure_failure_is_aggregated_and_does_not_skip_remaining_jobs() {
        // Use a non-fatal listener so dispatch succeeds, then simulate a
        // release infrastructure failure by deleting the first job's
        // persisted state before rejection (find returns None → CAS false →
        // ReleaseFailure).
        let observed_jobs = Arc::new(Mutex::new(Vec::new()));
        let (engine, time_source) = rejection_test_engine(false, Arc::clone(&observed_jobs));
        let job_ids = [
            ns("release-fail-1"),
            ns("release-fail-2"),
            ns("release-fail-3"),
        ];
        let job_id_refs: Vec<&str> = job_ids.iter().map(|s| s.as_str()).collect();
        insert_async_jobs(&engine, time_source.as_ref(), &job_id_refs);
        let runtime_service = engine.get_runtime_service();
        let our_ids: std::collections::HashSet<&str> =
            job_ids.iter().map(|s| s.as_str()).collect();
        let acquired: Vec<_> = runtime_service
            .acquire_async_jobs(300_000, 64)
            .into_iter()
            .filter(|j| our_ids.contains(j.timer_job_id.as_str()))
            .collect();
        assert_eq!(acquired.len(), job_ids.len());

        // Delete the first job's persisted state so its release fails
        // (find_timer_job_state returns None → release_timer_job_lock returns
        // false → ReleaseFailure). The remaining two jobs keep their state
        // and should release OK.
        {
            let store = engine.get_runtime_store();
            let mut session = store.create_session().unwrap();
            store.delete_timer_job_state(&job_ids[0], &mut session);
            session.flush_and_commit().unwrap();
        }

        let error = submit_acquired_jobs_with(&runtime_service, acquired, |_task| {
            Err(RejectedExecutionError)
        })
        .unwrap_err();

        // No fatal listener error — all jobs were dispatched.
        assert!(
            error.fatal_listener_error.is_none(),
            "non-fatal listener should not short-circuit"
        );

        // Exactly one release failure (the first job whose state was deleted).
        assert_eq!(error.release_failures.len(), 1);
        assert_eq!(error.release_failures[0].job_id, job_ids[0]);
        assert!(
            error.release_failures[0]
                .error
                .to_string()
                .contains(&format!(
                    "failed to unacquire rejected async job {}",
                    job_ids[0]
                )),
            "release failure error should mention the job id"
        );

        // All 3 jobs were observed by the listener (no short-circuit).
        let observed = observed_jobs.lock().unwrap();
        assert_eq!(
            observed.len(),
            3,
            "all jobs should be dispatched to listener"
        );
        drop(observed);

        // The remaining two jobs ARE released (their release was not skipped
        // despite the earlier release failure).
        for job_id in &job_ids[1..] {
            let persisted = persisted_job(&engine, job_id);
            assert!(
                persisted.lock_owner.is_none(),
                "job {job_id} lock should be released despite earlier release failure"
            );
            assert!(persisted.lock_time.is_none());
            assert!(persisted.lock_expiration_time.is_none());
        }

        // Outcome reflects all 3 rejected jobs.
        assert_eq!(
            error.outcome,
            AsyncJobSubmissionOutcome::Rejected {
                submitted: 0,
                rejected: 3,
            }
        );
    }

    fn insert_async_jobs_with_categories(
        engine: &ProcessEngine,
        time_source: &TestTimeSource,
        jobs: &[(&str, Option<&str>)],
    ) {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let now = time_source.now().timestamp_millis();
        for (job_id, category) in jobs {
            store.insert_timer_job_state(
                &RuntimeTimerJobState {
                    timer_job_id: (*job_id).to_string(),
                    process_instance_id: format!("process-{job_id}"),
                    execution_id: format!("execution-{job_id}"),
                    activity_id: "asyncTask".to_string(),
                    job_state: Some("async".to_string()),
                    is_boundary: false,
                    attached_activity_id: None,
                    cancel_activity: false,
                    time_duration: None,
                    time_date: None,
                    time_cycle: None,
                    end_date: None,
                    due_time: Some(now),
                    lock_owner: None,
                    lock_time: None,
                    lock_expiration_time: None,
                    retries: Some(3),
                    error_message: None,
                    error_details: None,
                    category: category.map(|c| c.to_string()),
                    ..Default::default()
                },
                &mut session,
            );
        }
        session.flush_and_commit().unwrap();
    }

    #[test]
    fn category_filter_returns_only_matching_jobs_and_excludes_null_category() {
        let time_source = Arc::new(TestTimeSource::new(
            Utc.with_ymd_and_hms(2026, 7, 19, 10, 0, 0).unwrap(),
        ));
        let engine = ProcessEngine::build_with_config(
            "category-filter-test".to_string(),
            time_source.clone(),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let runtime_service = engine.get_runtime_service();

        let jobs = [
            ("cat-a-1", Some("A")),
            ("cat-a-2", Some("A")),
            ("cat-b-1", Some("B")),
            ("no-cat-1", None),
            ("no-cat-2", None),
        ];
        insert_async_jobs_with_categories(&engine, time_source.as_ref(), &jobs);

        let empty_categories: Vec<String> = Vec::new();
        let all_jobs =
            runtime_service.acquire_async_jobs_for_tenants(300_000, 10, &[], &empty_categories);
        assert_eq!(
            all_jobs.len(),
            5,
            "empty category filter should acquire all jobs"
        );

        let engine2 = ProcessEngine::build_with_config(
            "category-filter-test-2".to_string(),
            Arc::new(TestTimeSource::new(
                Utc.with_ymd_and_hms(2026, 7, 19, 10, 0, 0).unwrap(),
            )),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let jobs2 = [
            ("cat-a-1", Some("A")),
            ("cat-a-2", Some("A")),
            ("cat-b-1", Some("B")),
            ("no-cat-1", None),
        ];
        let ts2 = Arc::new(TestTimeSource::new(
            Utc.with_ymd_and_hms(2026, 7, 19, 10, 0, 0).unwrap(),
        ));
        insert_async_jobs_with_categories(&engine2, ts2.as_ref(), &jobs2);
        let rs2 = engine2.get_runtime_service();

        let category_a = vec!["A".to_string()];
        let a_jobs = rs2.acquire_async_jobs_for_tenants(300_000, 10, &[], &category_a);
        assert_eq!(
            a_jobs.len(),
            2,
            "filter=[A] should return 2 category-A jobs"
        );
        for job in &a_jobs {
            assert_eq!(job.category.as_deref(), Some("A"));
        }

        let engine3 = ProcessEngine::build_with_config(
            "category-filter-test-3".to_string(),
            Arc::new(TestTimeSource::new(
                Utc.with_ymd_and_hms(2026, 7, 19, 10, 0, 0).unwrap(),
            )),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let jobs3 = [
            ("cat-a-1", Some("A")),
            ("cat-b-1", Some("B")),
            ("no-cat-1", None),
        ];
        let ts3 = Arc::new(TestTimeSource::new(
            Utc.with_ymd_and_hms(2026, 7, 19, 10, 0, 0).unwrap(),
        ));
        insert_async_jobs_with_categories(&engine3, ts3.as_ref(), &jobs3);
        let rs3 = engine3.get_runtime_service();

        let category_ab = vec!["A".to_string(), "B".to_string()];
        let ab_jobs = rs3.acquire_async_jobs_for_tenants(300_000, 10, &[], &category_ab);
        assert_eq!(
            ab_jobs.len(),
            2,
            "filter=[A,B] should return 2 jobs (A and B), excluding NULL category"
        );
        for job in &ab_jobs {
            assert!(job.category.as_deref() == Some("A") || job.category.as_deref() == Some("B"));
        }

        let engine4 = ProcessEngine::build_with_config(
            "category-filter-test-4".to_string(),
            Arc::new(TestTimeSource::new(
                Utc.with_ymd_and_hms(2026, 7, 19, 10, 0, 0).unwrap(),
            )),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let jobs4 = [("no-cat-1", None), ("no-cat-2", None)];
        let ts4 = Arc::new(TestTimeSource::new(
            Utc.with_ymd_and_hms(2026, 7, 19, 10, 0, 0).unwrap(),
        ));
        insert_async_jobs_with_categories(&engine4, ts4.as_ref(), &jobs4);
        let rs4 = engine4.get_runtime_service();

        let category_c = vec!["C".to_string()];
        let c_jobs = rs4.acquire_async_jobs_for_tenants(300_000, 10, &[], &category_c);
        assert!(
            c_jobs.is_empty(),
            "filter=[C] with no matching jobs and only NULL-category jobs should return empty"
        );
    }
}
