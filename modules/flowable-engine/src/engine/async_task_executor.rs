use crossbeam_channel::{Receiver, Sender, bounded};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedExecutionError;

impl std::fmt::Display for RejectedExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "async task queue is full or executor is shut down")
    }
}

impl std::error::Error for RejectedExecutionError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskPoolShutdownOutcome {
    Terminated,
    TimedOut { remaining_workers: usize },
}

#[derive(Clone, Debug)]
pub struct AsyncTaskExecutorConfig {
    pub pool_size: usize,
    pub queue_size: usize,
    pub keep_alive_ms: u64,
    pub thread_name_prefix: String,
    pub await_termination_period_ms: u64,
}

impl Default for AsyncTaskExecutorConfig {
    fn default() -> Self {
        Self {
            pool_size: 8,
            queue_size: 2048,
            keep_alive_ms: 5000,
            thread_name_prefix: "flowable-async".to_string(),
            await_termination_period_ms: 60_000,
        }
    }
}

pub type AsyncTask = Box<dyn FnOnce() + Send>;
pub type AsyncTaskSender = Sender<AsyncTask>;

pub struct AsyncTaskExecutor {
    sender: Arc<Mutex<Option<AsyncTaskSender>>>,
    queue_size: usize,
    workers: Vec<JoinHandle<()>>,
    completion_rx: Option<Receiver<()>>,
    worker_count: usize,
    await_termination_period: Duration,
    active_count: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
}

impl std::fmt::Debug for AsyncTaskExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncTaskExecutor")
            .field("queue_size", &self.queue_size)
            .field("active_count", &self.active_count.load(Ordering::SeqCst))
            .field("shutdown", &self.shutdown.load(Ordering::SeqCst))
            .field("workers", &self.workers.len())
            .finish()
    }
}

impl AsyncTaskExecutor {
    pub fn new(config: AsyncTaskExecutorConfig) -> Self {
        let pool_size = config.pool_size.max(1);
        let queue_size = config.queue_size.max(1);
        let (sender, receiver) = bounded::<AsyncTask>(queue_size);
        let (completion_tx, completion_rx) = bounded::<()>(pool_size);
        let shutdown = Arc::new(AtomicBool::new(false));
        let active_count = Arc::new(AtomicUsize::new(0));

        let mut workers = Vec::with_capacity(pool_size);
        for index in 0..pool_size {
            let receiver = receiver.clone();
            let shutdown = Arc::clone(&shutdown);
            let active_count = Arc::clone(&active_count);
            let prefix = config.thread_name_prefix.clone();
            let keep_alive = Duration::from_millis(config.keep_alive_ms);
            let completion_tx = completion_tx.clone();
            workers.push(thread::spawn(move || {
                worker_loop(
                    receiver,
                    shutdown,
                    active_count,
                    prefix,
                    index,
                    keep_alive,
                    completion_tx,
                );
            }));
        }
        drop(completion_tx);

        Self {
            sender: Arc::new(Mutex::new(Some(sender))),
            queue_size,
            workers,
            completion_rx: Some(completion_rx),
            worker_count: pool_size,
            await_termination_period: Duration::from_millis(config.await_termination_period_ms),
            active_count,
            shutdown,
        }
    }

    pub fn execute(&self, task: AsyncTask) -> Result<(), RejectedExecutionError> {
        let sender = self.try_clone_sender().ok_or(RejectedExecutionError)?;
        sender.try_send(task).map_err(|_| RejectedExecutionError)
    }

    pub fn try_clone_sender(&self) -> Option<AsyncTaskSender> {
        if self.shutdown.load(Ordering::SeqCst) {
            return None;
        }
        self.sender.lock().unwrap().clone()
    }

    pub fn remaining_capacity(&self) -> usize {
        if self.shutdown.load(Ordering::SeqCst) {
            return 0;
        }
        let guard = self.sender.lock().unwrap();
        let Some(sender) = guard.as_ref() else {
            return 0;
        };
        self.queue_size.saturating_sub(sender.len())
    }

    pub fn active_count(&self) -> usize {
        self.active_count.load(Ordering::SeqCst)
    }

    pub fn shutdown(self) {
        let timeout = self.await_termination_period;
        match self.try_shutdown(timeout) {
            TaskPoolShutdownOutcome::Terminated => {}
            TaskPoolShutdownOutcome::TimedOut { remaining_workers } => {
                tracing::warn!(
                    remaining_workers,
                    timeout_secs = timeout.as_secs(),
                    "Timeout during shutdown of async task pool. The current running jobs could not end within {} seconds after shutdown operation.",
                    timeout.as_secs()
                );
            }
        }
    }

    pub fn try_shutdown(mut self, timeout: Duration) -> TaskPoolShutdownOutcome {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(sender) = self.sender.lock().unwrap().take() {
            drop(sender);
        }
        let completion_rx = self
            .completion_rx
            .take()
            .expect("completion_rx is present until shutdown");

        let mut completed = 0usize;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match completion_rx.recv_timeout(remaining) {
                Ok(()) => {
                    completed += 1;
                    if completed == self.worker_count {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }

        let remaining_workers = self.worker_count.saturating_sub(completed);
        if remaining_workers == 0 {
            for handle in self.workers.drain(..) {
                let _ = handle.join();
            }
            TaskPoolShutdownOutcome::Terminated
        } else {
            TaskPoolShutdownOutcome::TimedOut { remaining_workers }
        }
    }
}

fn worker_loop(
    receiver: Receiver<AsyncTask>,
    shutdown: Arc<AtomicBool>,
    active_count: Arc<AtomicUsize>,
    prefix: String,
    index: usize,
    keep_alive: Duration,
    completion_tx: Sender<()>,
) {
    let _thread_name = format!("{}-{}", prefix, index);
    loop {
        if shutdown.load(Ordering::SeqCst) {
            while let Ok(task) = receiver.try_recv() {
                active_count.fetch_add(1, Ordering::SeqCst);
                task();
                active_count.fetch_sub(1, Ordering::SeqCst);
            }
            break;
        }
        match receiver.recv_timeout(keep_alive) {
            Ok(task) => {
                active_count.fetch_add(1, Ordering::SeqCst);
                task();
                active_count.fetch_sub(1, Ordering::SeqCst);
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = completion_tx.send(());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn submits_and_executes_tasks() {
        let executor = AsyncTaskExecutor::new(AsyncTaskExecutorConfig::default());
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        executor
            .execute(Box::new(move || {
                flag_clone.store(true, Ordering::SeqCst);
            }))
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(flag.load(Ordering::SeqCst));
        executor.shutdown();
    }

    #[test]
    fn rejects_when_queue_is_full() {
        let config = AsyncTaskExecutorConfig {
            pool_size: 1,
            queue_size: 1,
            keep_alive_ms: 60_000,
            ..Default::default()
        };
        let executor = AsyncTaskExecutor::new(config);
        let (release_tx, release_rx) = crossbeam_channel::bounded::<()>(0);
        let (started_tx, started_rx) = crossbeam_channel::bounded::<()>(0);
        executor
            .execute(Box::new(move || {
                started_tx.send(()).unwrap();
                let _ = release_rx.recv();
            }))
            .unwrap();
        started_rx.recv().unwrap();
        executor
            .execute(Box::new(|| thread::sleep(Duration::from_millis(50))))
            .unwrap();
        assert!(executor.execute(Box::new(|| {})).is_err());
        let _ = release_tx.send(());
        thread::sleep(Duration::from_millis(100));
        executor.shutdown();
    }

    #[test]
    fn shutdown_drains_tasks_that_were_already_accepted() {
        let executor = AsyncTaskExecutor::new(AsyncTaskExecutorConfig {
            pool_size: 1,
            queue_size: 1,
            keep_alive_ms: 60_000,
            ..AsyncTaskExecutorConfig::default()
        });
        let (running_tx, running_rx) = crossbeam_channel::bounded::<()>(1);
        let (release_tx, release_rx) = crossbeam_channel::bounded::<()>(1);
        executor
            .execute(Box::new(move || {
                running_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }))
            .unwrap();
        running_rx.recv().unwrap();

        let queued_task_ran = Arc::new(AtomicBool::new(false));
        let queued_task_ran_for_task = Arc::clone(&queued_task_ran);
        executor
            .execute(Box::new(move || {
                queued_task_ran_for_task.store(true, Ordering::SeqCst);
            }))
            .unwrap();

        let release_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            release_tx.send(()).unwrap();
        });
        executor.shutdown();
        release_thread.join().unwrap();

        assert!(
            queued_task_ran.load(Ordering::SeqCst),
            "shutdown must finish work accepted before shutdown, matching Java executor shutdown"
        );
    }

    #[test]
    fn try_shutdown_returns_terminated_when_workers_finish_within_timeout() {
        let executor = AsyncTaskExecutor::new(AsyncTaskExecutorConfig {
            pool_size: 2,
            queue_size: 4,
            keep_alive_ms: 60_000,
            ..AsyncTaskExecutorConfig::default()
        });
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = Arc::clone(&done);
        executor
            .execute(Box::new(move || {
                thread::sleep(Duration::from_millis(50));
                done_clone.store(true, Ordering::SeqCst);
            }))
            .unwrap();
        let outcome = executor.try_shutdown(Duration::from_secs(5));
        assert_eq!(outcome, TaskPoolShutdownOutcome::Terminated);
        assert!(done.load(Ordering::SeqCst));
    }

    #[test]
    fn try_shutdown_returns_timed_out_when_workers_exceed_timeout() {
        let executor = AsyncTaskExecutor::new(AsyncTaskExecutorConfig {
            pool_size: 1,
            queue_size: 1,
            keep_alive_ms: 60_000,
            ..AsyncTaskExecutorConfig::default()
        });
        let (started_tx, started_rx) = crossbeam_channel::bounded::<()>(0);
        let (release_tx, release_rx) = crossbeam_channel::bounded::<()>(0);
        executor
            .execute(Box::new(move || {
                started_tx.send(()).unwrap();
                let _ = release_rx.recv();
            }))
            .unwrap();
        started_rx.recv().unwrap();

        let outcome = executor.try_shutdown(Duration::from_millis(50));
        match outcome {
            TaskPoolShutdownOutcome::TimedOut { remaining_workers } => {
                assert_eq!(remaining_workers, 1);
            }
            other => panic!("expected TimedOut, got {other:?}"),
        }

        let _ = release_tx.send(());
    }

    #[test]
    fn default_config_uses_java_compatible_sixty_second_await_period() {
        let config = AsyncTaskExecutorConfig::default();
        assert_eq!(config.await_termination_period_ms, 60_000);
    }
}
