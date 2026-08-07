use crate::engine::runtime_service::RuntimeService;
use crate::engine::timer_worker::TimerWork;
use std::sync::Arc;

pub struct JobRunnable {
    runtime_service: Arc<RuntimeService>,
    work: TimerWork,
    fencing_token: i64,
    /// When set, the job was pre-locked by the active async executor and handed
    /// over by a post-commit hint; it is executed through the direct-hint path
    /// (coordinator lease skipped, executor row lock re-verified) rather than
    /// the coordinator-lease-gated path.
    direct_hint: bool,
}

impl JobRunnable {
    pub fn new(runtime_service: Arc<RuntimeService>, work: TimerWork, fencing_token: i64) -> Self {
        Self {
            runtime_service,
            work,
            fencing_token,
            direct_hint: false,
        }
    }

    pub fn new_direct_hint(runtime_service: Arc<RuntimeService>, work: TimerWork) -> Self {
        Self {
            runtime_service,
            work,
            fencing_token: 0,
            direct_hint: true,
        }
    }

    pub fn run(self) {
        if self.direct_hint {
            let _ = self
                .runtime_service
                .execute_timer_work_direct_hint(&self.work);
        } else {
            let _ = self
                .runtime_service
                .execute_timer_work(&self.work, self.fencing_token);
        }
    }
}

pub fn spawn_timer_work(
    runtime_service: Arc<RuntimeService>,
    work: TimerWork,
    fencing_token: i64,
) -> Box<dyn FnOnce() + Send> {
    Box::new(move || {
        JobRunnable::new(runtime_service, work, fencing_token).run();
    })
}

/// Spawn a pre-locked job delivered via a post-commit hint. Executed through the
/// direct-hint path (see [`JobRunnable::new_direct_hint`]).
pub fn spawn_direct_hint_work(
    runtime_service: Arc<RuntimeService>,
    work: TimerWork,
) -> Box<dyn FnOnce() + Send> {
    Box::new(move || {
        JobRunnable::new_direct_hint(runtime_service, work).run();
    })
}
