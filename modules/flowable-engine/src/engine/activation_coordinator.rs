//! Shared activation coordinator (Java parity for active-async-executor job hints).
//!
//! Flowable Java's `DefaultJobManager` reacts to a *live* async executor when a
//! job becomes executable: it pre-locks the row (owner + expiration) inside the
//! same transaction and, on commit, hands the job to the executor via a
//! `COMMITTED` transaction listener (`JobAddedTransactionListener`). Whether the
//! executor is hinted also depends on the configured *enabled job categories*.
//!
//! In the Rust engine the `AsyncExecutor` and the `DefaultCommandExecutor` are
//! constructed at different points and neither can observe the other's live
//! state through the frozen configuration snapshot. This coordinator is the
//! single shared, `Clone`-able handle that both sides hold:
//!
//! * a live `AtomicBool` active flag (the *same* flag the executor lifecycle
//!   flips on start/shutdown), so a command can see the real runtime state;
//! * the executor lock owner and async-job lock duration used when pre-locking;
//! * the enabled-category predicate that decides whether to *hint* (never
//!   whether to *pre-lock* — Java pre-locks regardless of category);
//! * the tenant predicate;
//! * a post-commit submit handle that actually offers the job to the executor.
//!
//! Commands never submit to the executor directly. They register pending hints
//! on the [`CommandContext`](crate::interceptor::command_context::CommandContext)
//! and the command executor drains them *after* the database transaction
//! commits, mirroring the Java `COMMITTED` listener ordering.

use crate::error::FlowableError;
use crate::persistence::runtime_store::RuntimeTimerJobState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Outcome of offering a committed job to the live async executor.
#[derive(Debug, Clone)]
pub enum HintSubmitOutcome {
    /// The job was accepted by the executor task pool.
    Submitted,
    /// The executor task pool rejected the job (queue full / shut down). The
    /// caller must dispatch `JOB_REJECTED` and CAS-release the pre-lock so a
    /// later acquisition can pick the job up again.
    Rejected,
    /// No submit handle is wired (executor never built). Treated like a
    /// rejection for release purposes.
    NoExecutor,
    /// A committed listener failed after the database transaction had already
    /// committed. The caller must propagate this typed error without rolling
    /// back the persisted row.
    Fatal(FlowableError),
}

/// Submit closure installed once the async executor and runtime service exist.
type SubmitHandle = Arc<dyn Fn(RuntimeTimerJobState) -> HintSubmitOutcome + Send + Sync>;

struct Inner {
    /// Shared with `AsyncExecutor`; the executor lifecycle owns writes.
    active: Arc<AtomicBool>,
    lock_owner: Mutex<String>,
    async_job_lock_ms: Mutex<i64>,
    /// Empty means "all categories enabled".
    enabled_categories: Mutex<Vec<String>>,
    /// Empty means "all tenants".
    tenant_ids: Mutex<Vec<String>>,
    submit: Mutex<Option<SubmitHandle>>,
}

/// Clone-able handle to the shared activation state. Cloning shares the same
/// underlying atomics/handle (it is `Arc`-backed).
#[derive(Clone)]
pub struct ActivationCoordinator {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ActivationCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivationCoordinator")
            .field("active", &self.is_active())
            .field("lock_owner", &self.lock_owner())
            .field("async_job_lock_ms", &self.async_job_lock_ms())
            .field(
                "has_submit_handle",
                &self.inner.submit.lock().unwrap().is_some(),
            )
            .finish()
    }
}

impl Default for ActivationCoordinator {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                active: Arc::new(AtomicBool::new(false)),
                lock_owner: Mutex::new(String::new()),
                async_job_lock_ms: Mutex::new(300_000),
                enabled_categories: Mutex::new(Vec::new()),
                tenant_ids: Mutex::new(Vec::new()),
                submit: Mutex::new(None),
            }),
        }
    }
}

impl ActivationCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// The live active flag. `AsyncExecutor` is built to share this exact
    /// `Arc<AtomicBool>` so the coordinator observes true runtime state.
    pub fn active_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.inner.active)
    }

    pub fn is_active(&self) -> bool {
        self.inner.active.load(Ordering::SeqCst)
    }

    /// Configure the executor identity used when pre-locking. Called during
    /// engine construction once the resolved lock owner and lock duration are
    /// known.
    pub fn configure(
        &self,
        lock_owner: impl Into<String>,
        async_job_lock_ms: i64,
        enabled_categories: Vec<String>,
        tenant_ids: Vec<String>,
    ) {
        *self.inner.lock_owner.lock().unwrap() = lock_owner.into();
        *self.inner.async_job_lock_ms.lock().unwrap() = async_job_lock_ms;
        *self.inner.enabled_categories.lock().unwrap() = enabled_categories;
        *self.inner.tenant_ids.lock().unwrap() = tenant_ids;
    }

    pub fn lock_owner(&self) -> String {
        self.inner.lock_owner.lock().unwrap().clone()
    }

    pub fn async_job_lock_ms(&self) -> i64 {
        *self.inner.async_job_lock_ms.lock().unwrap()
    }

    /// Install the post-commit submit handle. Idempotent; the last handle wins.
    pub fn set_submit_handle(&self, handle: SubmitHandle) {
        *self.inner.submit.lock().unwrap() = Some(handle);
    }

    /// Category predicate mirroring Java `isJobApplicableForExecutorExecution`:
    /// no enabled categories => hint everything; an empty job category with a
    /// non-empty enabled list => do not hint; otherwise hint iff the enabled
    /// list contains the job category. This decides *hinting only*, never
    /// whether the row is pre-locked.
    pub fn category_enabled_for_hint(&self, job_category: Option<&str>) -> bool {
        let enabled = self.inner.enabled_categories.lock().unwrap();
        if enabled.is_empty() {
            return true;
        }
        match job_category {
            Some(category) if !category.is_empty() => enabled.iter().any(|c| c == category),
            _ => false,
        }
    }

    /// Tenant predicate. Empty tenant filter means all tenants are eligible.
    pub fn tenant_enabled(&self, _tenant_id: Option<&str>) -> bool {
        // Runtime jobs in this engine are not tenant-partitioned at the
        // activation layer; tenant scoping is enforced during acquisition.
        // Kept as an explicit hook so the predicate has a single home.
        true
    }

    /// Offer a committed job to the executor. Returns [`HintSubmitOutcome`].
    pub fn submit(&self, job: RuntimeTimerJobState) -> HintSubmitOutcome {
        let handle = self.inner.submit.lock().unwrap().clone();
        match handle {
            Some(handle) => handle(job),
            None => HintSubmitOutcome::NoExecutor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_predicate_matches_java_semantics() {
        let coordinator = ActivationCoordinator::new();
        // No enabled categories => hint everything.
        coordinator.configure("owner", 1000, Vec::new(), Vec::new());
        assert!(coordinator.category_enabled_for_hint(None));
        assert!(coordinator.category_enabled_for_hint(Some("anything")));

        // Enabled list set => only matching, and empty/absent category never hints.
        coordinator.configure("owner", 1000, vec!["urgent".to_string()], Vec::new());
        assert!(coordinator.category_enabled_for_hint(Some("urgent")));
        assert!(!coordinator.category_enabled_for_hint(Some("bulk")));
        assert!(!coordinator.category_enabled_for_hint(Some("")));
        assert!(!coordinator.category_enabled_for_hint(None));
    }

    #[test]
    fn active_flag_is_shared() {
        let coordinator = ActivationCoordinator::new();
        let flag = coordinator.active_flag();
        assert!(!coordinator.is_active());
        flag.store(true, Ordering::SeqCst);
        assert!(coordinator.is_active());
        // A clone observes the same flag.
        let clone = coordinator.clone();
        flag.store(false, Ordering::SeqCst);
        assert!(!clone.is_active());
    }

    #[test]
    fn submit_without_handle_reports_no_executor() {
        let coordinator = ActivationCoordinator::new();
        let job = sample_job();
        assert!(matches!(
            coordinator.submit(job),
            HintSubmitOutcome::NoExecutor
        ));
    }

    #[test]
    fn submit_dispatches_to_installed_handle() {
        let coordinator = ActivationCoordinator::new();
        coordinator.set_submit_handle(Arc::new(|_job| HintSubmitOutcome::Submitted));
        assert!(matches!(
            coordinator.submit(sample_job()),
            HintSubmitOutcome::Submitted
        ));
    }

    fn sample_job() -> RuntimeTimerJobState {
        RuntimeTimerJobState {
            timer_job_id: "job-1".to_string(),
            process_instance_id: "pi-1".to_string(),
            execution_id: "ex-1".to_string(),
            activity_id: "task".to_string(),
            job_state: Some("executable".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(1),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(3),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        }
    }
}
