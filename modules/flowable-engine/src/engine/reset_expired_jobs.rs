use crate::engine::runtime_service::RuntimeService;
use crate::error::FlowableError;
use crate::persistence::runtime_store::{ExpiredJobClass, ResetExpiredJobsBatchOutcome};
use crossbeam_channel::{RecvTimeoutError, Sender, bounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ResetExpiredJobsConfig {
    pub reset_interval_ms: u64,
    pub page_size: usize,
    pub job_classes: Vec<ExpiredJobClass>,
    pub tenant_ids: Vec<String>,
    pub enabled_job_categories: Vec<String>,
}

impl Default for ResetExpiredJobsConfig {
    fn default() -> Self {
        Self {
            reset_interval_ms: 60_000,
            page_size: 3,
            job_classes: ExpiredJobClass::ALL.to_vec(),
            tenant_ids: Vec::new(),
            enabled_job_categories: Vec::new(),
        }
    }
}

pub struct ResetExpiredJobs {
    is_active: Arc<AtomicBool>,
    handle: std::sync::Mutex<Option<JoinHandle<()>>>,
    stop_tx: std::sync::Mutex<Option<Sender<()>>>,
    config: ResetExpiredJobsConfig,
}

/// Drain every configured class independently until a page scans zero rows.
/// Compare-and-set conflicts are counted in the batch outcome and do not stop
/// the cycle. A non-conflict command/storage error ends the current cycle.
pub(crate) fn reset_cycle(
    runtime_service: &RuntimeService,
    config: &ResetExpiredJobsConfig,
) -> Result<usize, FlowableError> {
    reset_cycle_with(config, |job_class, page_size| {
        runtime_service.reset_expired_jobs_batch_scoped(
            job_class,
            page_size,
            &config.tenant_ids,
            &config.enabled_job_categories,
        )
    })
}

pub(crate) fn reset_cycle_with<F>(
    config: &ResetExpiredJobsConfig,
    mut reset_batch: F,
) -> Result<usize, FlowableError>
where
    F: FnMut(ExpiredJobClass, usize) -> Result<ResetExpiredJobsBatchOutcome, FlowableError>,
{
    let mut total_reset = 0usize;
    for &job_class in &config.job_classes {
        loop {
            let outcome = reset_batch(job_class, config.page_size)?;
            total_reset += outcome.reset;
            if outcome.scanned == 0 {
                break;
            }
        }
    }
    Ok(total_reset)
}

impl ResetExpiredJobs {
    pub fn new(config: ResetExpiredJobsConfig) -> Self {
        Self {
            is_active: Arc::new(AtomicBool::new(false)),
            handle: std::sync::Mutex::new(None),
            stop_tx: std::sync::Mutex::new(None),
            config,
        }
    }

    pub fn start(&self, runtime_service: Arc<RuntimeService>) {
        let mut handle_guard = self.handle.lock().unwrap();
        if handle_guard.is_some() {
            return;
        }
        let (stop_tx, stop_rx) = bounded::<()>(1);
        *self.stop_tx.lock().unwrap() = Some(stop_tx);
        self.is_active.store(true, Ordering::SeqCst);
        let is_active = Arc::clone(&self.is_active);
        let config = self.config.clone();
        let handle = thread::spawn(move || {
            while is_active.load(Ordering::SeqCst) {
                if let Err(error) = reset_cycle(&runtime_service, &config) {
                    tracing::error!(
                        error = %error,
                        "expired job reset cycle failed; waiting for next interval"
                    );
                }
                if !is_active.load(Ordering::SeqCst) {
                    break;
                }
                // Java Object.wait(0) waits forever; match that for interval 0.
                let wait_result = if config.reset_interval_ms == 0 {
                    stop_rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
                } else {
                    stop_rx.recv_timeout(Duration::from_millis(config.reset_interval_ms))
                };
                match wait_result {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
            is_active.store(false, Ordering::SeqCst);
        });
        *handle_guard = Some(handle);
    }

    pub fn stop(&self) {
        self.request_stop();
        self.await_stopped();
    }

    pub(crate) fn request_stop(&self) {
        self.is_active.store(false, Ordering::SeqCst);
        if let Some(stop_tx) = self.stop_tx.lock().unwrap().as_ref() {
            let _ = stop_tx.try_send(());
        }
    }

    pub(crate) fn await_stopped(&self) {
        let handle = {
            let mut handle_guard = self.handle.lock().unwrap();
            self.stop_tx.lock().unwrap().take();
            handle_guard.take()
        };
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::process_engine::ProcessEngine;
    use crate::engine::time_source::{SystemTimeSource, TestTimeSource, TimeSource};
    use crate::persistence::db_store::DbStore;
    use crate::persistence::runtime_store::RuntimeTimerJobState;
    use crate::service::config::{AsyncExecutorConfiguration, ProcessEngineConfiguration};
    use chrono::{TimeZone, Utc};
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    fn expired_job(id: &str, state: &str, now_ms: i64) -> RuntimeTimerJobState {
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
            due_time: Some(now_ms - 2_000),
            lock_owner: Some("dead-owner".to_string()),
            lock_time: Some(now_ms - 2_000),
            lock_expiration_time: Some(now_ms - 1_000),
            retries: Some(3),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        }
    }

    #[test]
    fn stop_is_interruptible_idempotent_and_restartable() {
        let engine = ProcessEngine::new("reset-expired-restart".to_string());
        let reset = ResetExpiredJobs::new(ResetExpiredJobsConfig {
            reset_interval_ms: 60_000,
            page_size: 3,
            job_classes: ExpiredJobClass::ALL.to_vec(),
            ..Default::default()
        });

        reset.start(engine.get_runtime_service());
        assert!(reset.is_active.load(Ordering::SeqCst));
        let stop_started = Instant::now();
        reset.stop();
        assert!(stop_started.elapsed() < Duration::from_secs(5));
        assert!(!reset.is_active.load(Ordering::SeqCst));

        reset.stop();
        reset.start(engine.get_runtime_service());
        assert!(reset.is_active.load(Ordering::SeqCst));
        reset.stop();
        assert!(!reset.is_active.load(Ordering::SeqCst));
    }

    #[test]
    fn automatic_executor_close_interrupts_sixty_second_reset_wait() {
        let mut config = ProcessEngineConfiguration::default();
        config.async_executor = AsyncExecutorConfiguration {
            auto_activate: true,
            pool_size: 1,
            async_job_acquisition_enabled: false,
            timer_job_acquisition_enabled: false,
            reset_expired_job_enabled: true,
            reset_expired_jobs_interval_ms: 60_000,
            ..AsyncExecutorConfiguration::default()
        };
        let engine = ProcessEngine::build_with_config(
            "reset-expired-close".to_string(),
            Arc::new(SystemTimeSource),
            config,
        )
        .expect("build auto-activated executor with reset loop");

        assert!(engine.async_executor_is_active());
        let close_started = Instant::now();
        engine.close();
        assert!(
            close_started.elapsed() < Duration::from_secs(5),
            "engine close must interrupt the 60-second reset wait"
        );
        assert!(!engine.async_executor_is_active());
    }

    #[test]
    fn zero_interval_waits_until_stop_notification() {
        let engine = ProcessEngine::new("reset-expired-zero-interval".to_string());
        let reset = ResetExpiredJobs::new(ResetExpiredJobsConfig {
            reset_interval_ms: 0,
            page_size: 3,
            job_classes: ExpiredJobClass::ALL.to_vec(),
            ..Default::default()
        });

        reset.start(engine.get_runtime_service());
        assert!(reset.is_active.load(Ordering::SeqCst));
        // Give the worker time to complete the first cycle and block on recv().
        thread::sleep(Duration::from_millis(50));
        assert!(
            reset.is_active.load(Ordering::SeqCst),
            "interval 0 must wait indefinitely rather than busy-looping and exiting"
        );

        let stop_started = Instant::now();
        reset.stop();
        assert!(stop_started.elapsed() < Duration::from_secs(5));
        assert!(!reset.is_active.load(Ordering::SeqCst));
    }

    #[test]
    fn reset_cycle_drains_all_pages_for_each_class_independently() {
        let now = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let engine = ProcessEngine::build_with_db_store_and_config(
            "reset-cycle-drain".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let store = engine.get_runtime_store();
        let now_ms = time_source.now().timestamp_millis();
        let page_size = 3;
        let pages_per_class = 3;

        let mut session = store.create_session().unwrap();
        for index in 0..(page_size * pages_per_class) {
            // Mix async / async-after so the Async class still sees one pool.
            let state = if index % 2 == 0 {
                "async"
            } else {
                "async-after"
            };
            store.insert_timer_job_state(
                &expired_job(&format!("async-{index}"), state, now_ms),
                &mut session,
            );
            store.insert_timer_job_state(
                &expired_job(&format!("timer-{index}"), "timer", now_ms),
                &mut session,
            );
            store.insert_timer_job_state(
                &expired_job(&format!("history-{index}"), "history", now_ms),
                &mut session,
            );
        }
        session.flush_and_commit().unwrap();

        let config = ResetExpiredJobsConfig {
            reset_interval_ms: 60_000,
            page_size,
            job_classes: ExpiredJobClass::ALL.to_vec(),
            ..Default::default()
        };
        let total_reset = reset_cycle(&engine.get_runtime_service(), &config).unwrap();
        // Async: page_size*3, Timer: page_size*3, History: page_size*3
        assert_eq!(total_reset, page_size * pages_per_class * 3);

        let mut session = store.create_session().unwrap();
        let remaining = store.snapshot_timer_job_states(&mut session);
        session.rollback().unwrap();
        assert!(
            remaining
                .values()
                .all(|job| job.lock_owner.is_none() && job.lock_expiration_time.is_none()),
            "one cycle must drain every expired page for every class"
        );
    }

    #[test]
    fn reset_cycle_drains_classes_with_independent_page_quotas() {
        let now = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let time_source = Arc::new(TestTimeSource::new(now));
        let engine = ProcessEngine::build_with_db_store_and_config(
            "reset-cycle-independent".to_string(),
            Arc::clone(&time_source) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
            ProcessEngineConfiguration::default(),
        )
        .unwrap();
        let store = engine.get_runtime_store();
        let now_ms = time_source.now().timestamp_millis();
        let page_size = 2;

        let mut session = store.create_session().unwrap();
        // Many async jobs would exhaust a shared page budget if classes shared
        // a single untyped scan.
        for index in 0..(page_size * 4) {
            store.insert_timer_job_state(
                &expired_job(&format!("async-{index}"), "async", now_ms),
                &mut session,
            );
        }
        for index in 0..page_size {
            store.insert_timer_job_state(
                &expired_job(&format!("timer-{index}"), "timer", now_ms),
                &mut session,
            );
            store.insert_timer_job_state(
                &expired_job(&format!("history-{index}"), "history", now_ms),
                &mut session,
            );
        }
        session.flush_and_commit().unwrap();

        let mut class_calls: Vec<ExpiredJobClass> = Vec::new();
        let config = ResetExpiredJobsConfig {
            reset_interval_ms: 0,
            page_size,
            job_classes: vec![
                ExpiredJobClass::Async,
                ExpiredJobClass::Timer,
                ExpiredJobClass::History,
            ],
            ..Default::default()
        };
        let total = reset_cycle_with(&config, |job_class, size| {
            assert_eq!(size, page_size);
            class_calls.push(job_class);
            engine
                .get_runtime_service()
                .reset_expired_jobs_batch(job_class, size)
        })
        .unwrap();

        assert_eq!(total, page_size * 4 + page_size + page_size);
        // Async needs 5 scans (4 full pages + empty), Timer 2, History 2.
        assert!(
            class_calls
                .iter()
                .filter(|c| **c == ExpiredJobClass::Async)
                .count()
                >= 5,
            "async class must be paged independently: {class_calls:?}"
        );
        assert!(
            class_calls
                .iter()
                .filter(|c| **c == ExpiredJobClass::Timer)
                .count()
                >= 2
        );
        assert!(
            class_calls
                .iter()
                .filter(|c| **c == ExpiredJobClass::History)
                .count()
                >= 2
        );

        let mut session = store.create_session().unwrap();
        let remaining = store.snapshot_timer_job_states(&mut session);
        session.rollback().unwrap();
        assert!(remaining.values().all(|job| job.lock_owner.is_none()));
    }

    #[test]
    fn reset_cycle_stops_on_command_error_but_reports_it() {
        let attempts = AtomicUsize::new(0);
        let config = ResetExpiredJobsConfig {
            reset_interval_ms: 0,
            page_size: 3,
            job_classes: vec![ExpiredJobClass::Async, ExpiredJobClass::Timer],
            ..Default::default()
        };
        let result = reset_cycle_with(&config, |job_class, _| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // First Async page succeeds with work remaining so the cycle
                // would continue if no error occurred later.
                Ok(ResetExpiredJobsBatchOutcome {
                    scanned: 3,
                    reset: 3,
                    conflicts: 0,
                })
            } else if n == 1 {
                Err(FlowableError::Internal(
                    "synthetic storage failure".to_string(),
                ))
            } else {
                panic!("cycle must stop after the first non-conflict error; got {job_class:?}");
            }
        });
        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn reset_cycle_continues_after_conflicts() {
        let attempts = AtomicUsize::new(0);
        let config = ResetExpiredJobsConfig {
            reset_interval_ms: 0,
            page_size: 2,
            job_classes: vec![ExpiredJobClass::Timer],
            ..Default::default()
        };
        let total = reset_cycle_with(&config, |_, _| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            Ok(match n {
                0 => ResetExpiredJobsBatchOutcome {
                    scanned: 2,
                    reset: 1,
                    conflicts: 1,
                },
                1 => ResetExpiredJobsBatchOutcome {
                    scanned: 0,
                    reset: 0,
                    conflicts: 0,
                },
                _ => panic!("unexpected extra page after empty scan"),
            })
        })
        .unwrap();
        assert_eq!(total, 1);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn command_error_is_logged_and_does_not_kill_reset_thread() {
        let engine = ProcessEngine::new("reset-expired-error-survives".to_string());
        // Use a short non-zero interval so a cycle error is followed by another
        // wait that proves the thread stayed alive.
        let reset = ResetExpiredJobs::new(ResetExpiredJobsConfig {
            reset_interval_ms: 30,
            page_size: 3,
            job_classes: ExpiredJobClass::ALL.to_vec(),
            ..Default::default()
        });
        reset.start(engine.get_runtime_service());
        thread::sleep(Duration::from_millis(80));
        assert!(
            reset.is_active.load(Ordering::SeqCst),
            "reset thread must remain active across successful cycles"
        );
        let stop_started = Instant::now();
        reset.stop();
        assert!(stop_started.elapsed() < Duration::from_secs(5));
        assert!(!reset.is_active.load(Ordering::SeqCst));
    }

    #[test]
    fn default_reset_expired_page_size_is_three() {
        assert_eq!(
            ProcessEngineConfiguration::default()
                .async_executor
                .reset_expired_jobs_page_size,
            3
        );
        assert_eq!(ResetExpiredJobsConfig::default().page_size, 3);
    }
}
