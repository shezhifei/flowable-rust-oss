use crate::engine::async_task_executor::AsyncTaskExecutor;
use crate::engine::job_runnable::spawn_timer_work;
use crate::engine::runtime_service::RuntimeService;
use crate::engine::timer_worker::TimerWork;
use crate::persistence::runtime_store::RuntimeStore;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DISPATCHER_RECV_TIMEOUT_MS: u64 = 5_000;

/// A thread that receives history job IDs from a post-commit channel and
/// submits them to the `AsyncTaskExecutor` pool for immediate execution,
/// bypassing the normal polling acquisition cycle.
///
/// Java equivalent: `TriggerAsyncHistoryExecutorTransactionListener`
/// which triggers history job acquisition immediately after commit.
pub struct HistoryJobDispatcher {
    is_active: Arc<AtomicBool>,
    handle: std::sync::Mutex<Option<JoinHandle<()>>>,
    rx: Option<crossbeam_channel::Receiver<Vec<String>>>,
}

impl HistoryJobDispatcher {
    pub fn new(rx: crossbeam_channel::Receiver<Vec<String>>) -> Self {
        Self {
            is_active: Arc::new(AtomicBool::new(false)),
            handle: std::sync::Mutex::new(None),
            rx: Some(rx),
        }
    }

    pub fn start(
        &mut self,
        runtime_service: Arc<RuntimeService>,
        task_executor: Arc<Mutex<Option<AsyncTaskExecutor>>>,
        runtime_store: RuntimeStore,
    ) {
        let Some(rx) = self.rx.take() else {
            tracing::warn!("HistoryJobDispatcher rx already taken; start ignored");
            return;
        };
        self.is_active.store(true, Ordering::SeqCst);
        let is_active = Arc::clone(&self.is_active);
        // Clone the sender once so the per-job submissions do not contend on
        // the executor-wide mutex.
        let task_sender: Option<crate::engine::async_task_executor::AsyncTaskSender> = {
            let guard = task_executor.lock().unwrap_or_else(|e| e.into_inner());
            guard.as_ref().and_then(|e| e.try_clone_sender())
        };
        let handle = thread::spawn(move || {
            dispatcher_loop(rx, runtime_service, task_sender, runtime_store, is_active);
        });
        *self.handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }

    pub fn stop(&self) {
        self.is_active.store(false, Ordering::SeqCst);
        let mut guard = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = guard.take() {
            let _ = handle.join();
        }
    }
}

fn dispatcher_loop(
    rx: crossbeam_channel::Receiver<Vec<String>>,
    runtime_service: Arc<RuntimeService>,
    task_sender: Option<crate::engine::async_task_executor::AsyncTaskSender>,
    runtime_store: RuntimeStore,
    is_active: Arc<AtomicBool>,
) {
    while is_active.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(DISPATCHER_RECV_TIMEOUT_MS)) {
            Ok(job_ids) => {
                for job_id in job_ids {
                    let mut session = runtime_store.create_session().unwrap();
                    if let Some(job) = runtime_store.find_timer_job_state(&job_id, &mut session) {
                        let work = TimerWork::RuntimeJob(job);
                        let task = spawn_timer_work(Arc::clone(&runtime_service), work, 0);
                        let res = match task_sender.as_ref() {
                            Some(s) => s.try_send(task),
                            None => Err(crossbeam_channel::TrySendError::Full(task)),
                        };
                        if res.is_err() {
                            break;
                        }
                    }
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
}
