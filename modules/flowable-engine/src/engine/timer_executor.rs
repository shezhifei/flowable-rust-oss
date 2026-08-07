use crate::engine::runtime_service::RuntimeService;
use crate::engine::timer_worker::TimerWorkerConfig;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

type BeforeExecuteHook = Arc<dyn Fn() + Send + Sync>;

/// Tracks the lifecycle of the embedded timer executor.
///
/// States:
///   - Idle: not started
///   - Running: acquiring and executing timer work
///   - Draining: stopped acquiring, waiting for in-flight work to finish
///   - Stopped: fully stopped
pub struct TimerExecutor {
    /// When true, the poll loop keeps acquiring new work.
    is_acquiring: Arc<AtomicBool>,
    /// Counter of in-flight work items (acquired and currently being executed).
    in_flight_count: Arc<AtomicUsize>,
    /// Condvar used to signal when in-flight count drops to zero.
    drain_pair: Arc<(Mutex<()>, Condvar)>,
    /// The poll-loop thread handle.
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    /// Configuration for polling, heartbeat, and jitter.
    config: Arc<Mutex<TimerWorkerConfig>>,
    /// Optional hook used by tests to block right before execution begins.
    before_execute_hook: Arc<Mutex<Option<BeforeExecuteHook>>>,
}

impl Default for TimerExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerExecutor {
    pub fn new() -> Self {
        Self {
            is_acquiring: Arc::new(AtomicBool::new(false)),
            in_flight_count: Arc::new(AtomicUsize::new(0)),
            drain_pair: Arc::new((Mutex::new(()), Condvar::new())),
            handle: Mutex::new(None),
            config: Arc::new(Mutex::new(TimerWorkerConfig::default())),
            before_execute_hook: Arc::new(Mutex::new(None)),
        }
    }

    /// Update the configuration of the executor
    pub fn set_config(&self, config: TimerWorkerConfig) {
        *self.config.lock().unwrap() = config;
    }

    /// Returns the current number of in-flight work items.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight_count.load(Ordering::SeqCst)
    }

    /// Returns true if the executor is currently acquiring new work.
    pub fn is_acquiring(&self) -> bool {
        self.is_acquiring.load(Ordering::SeqCst)
    }

    /// Installs a callback that runs after a lease is acquired but before the
    /// work is executed. Used by tests to hold a genuine in-flight lease.
    pub fn set_before_execute_hook(&self, hook: Option<BeforeExecuteHook>) {
        *self.before_execute_hook.lock().unwrap() = hook;
    }

    pub fn start(&self, runtime_service: Arc<RuntimeService>) {
        // Prevent double-start
        {
            let handle_guard = self.handle.lock().unwrap();
            if handle_guard.is_some() {
                return;
            }
        }

        self.is_acquiring.store(true, Ordering::SeqCst);

        let is_acquiring = Arc::clone(&self.is_acquiring);
        let in_flight_count = Arc::clone(&self.in_flight_count);
        let drain_pair = Arc::clone(&self.drain_pair);
        let worker_config = self.config.lock().unwrap().clone();
        let before_execute_hook = Arc::clone(&self.before_execute_hook);

        let handle = thread::spawn(move || {
            let worker = crate::engine::timer_worker::TimerWorker::new(
                Arc::clone(&runtime_service),
                "embedded",
            );

            while is_acquiring.load(Ordering::SeqCst) {
                let current_timeout = worker_config.coordinator_lease_timeout_ms;
                let works = worker.acquire_due_timers(current_timeout);

                for work in works {
                    // Increment in-flight counter before execution.
                    in_flight_count.fetch_add(1, Ordering::SeqCst);

                    // Spawn a heartbeat thread that renews the lease
                    // periodically until the work is completed or the
                    // executor is draining.
                    let hb_work = work.clone();
                    let hb_runtime = Arc::clone(&runtime_service);
                    let hb_done = Arc::new(AtomicBool::new(false));
                    let hb_done_clone = Arc::clone(&hb_done);
                    let hb_interval = worker_config.heartbeat_interval_ms;

                    let hb_token = worker.get_fencing_token();
                    let hb_handle = thread::spawn(move || {
                        let hb_worker = crate::engine::timer_worker::TimerWorker::new(
                            hb_runtime,
                            "embedded_hb",
                        );
                        hb_worker.set_fencing_token(hb_token);
                        let tick_ms: u64 = 100; // check done flag every 100ms
                        let mut elapsed_ms: u64 = 0;
                        while !hb_done_clone.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(tick_ms));
                            elapsed_ms += tick_ms;
                            if hb_done_clone.load(Ordering::SeqCst) {
                                break;
                            }
                            if elapsed_ms >= hb_interval {
                                hb_worker.renew_timer_lease(&hb_work);
                                elapsed_ms = 0;
                            }
                        }
                    });

                    if let Some(hook) = before_execute_hook.lock().unwrap().clone() {
                        hook();
                    }

                    // Execute the timer work synchronously.
                    worker.execute_timer(&work);

                    // Signal the heartbeat thread to stop.
                    hb_done.store(true, Ordering::SeqCst);
                    let _ = hb_handle.join();

                    // Decrement in-flight counter and notify drain waiters.
                    let prev = in_flight_count.fetch_sub(1, Ordering::SeqCst);
                    if prev == 1 {
                        // Last in-flight item completed - notify drain.
                        let (lock, cvar) = &*drain_pair;
                        let _guard = lock.lock().unwrap();
                        cvar.notify_all();
                    }
                }

                let current_poll = worker_config.poll_interval_ms + worker_config.get_jitter_ms();
                thread::sleep(Duration::from_millis(current_poll));
            }

            worker.graceful_shutdown();
        });

        *self.handle.lock().unwrap() = Some(handle);
    }

    /// Stops acquiring new work, waits for in-flight work to finish (drain),
    /// then joins the poll-loop thread.
    pub fn stop(&self) {
        // Phase 1: Stop acquiring new work.
        self.is_acquiring.store(false, Ordering::SeqCst);

        // Phase 2: Drain - wait for in-flight work to complete.
        self.drain();

        // Phase 3: Join the poll-loop thread.
        let mut handle_opt = self.handle.lock().unwrap();
        if let Some(handle) = handle_opt.take() {
            let _ = handle.join();
        }
    }

    /// Stops acquiring new work but does NOT wait for in-flight work or
    /// join the poll thread. Use `drain()` afterwards to wait.
    pub fn stop_acquiring(&self) {
        self.is_acquiring.store(false, Ordering::SeqCst);
    }

    /// Blocks until all in-flight work has completed.
    /// This is typically called after `stop_acquiring()`.
    pub fn drain(&self) {
        let (lock, cvar) = &*self.drain_pair;
        let guard = lock.lock().unwrap();
        // Wait until in-flight count drops to zero.
        let _guard = cvar
            .wait_while(guard, |_| self.in_flight_count.load(Ordering::SeqCst) > 0)
            .unwrap();
    }
}
