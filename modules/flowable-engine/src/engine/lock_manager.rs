//! Cluster-safe global acquire lock for multi-node async job acquisition.

use crate::engine::runtime_service::RuntimeService;
use crate::error::FlowableError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK: &str = "acquireAsyncJobsLock";
pub const ACQUIRE_TIMER_JOBS_GLOBAL_LOCK: &str = "acquireTimerJobsLock";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalLockReleaseOutcome {
    NotHeldLocally,
    Released,
    OwnershipLost,
}

#[derive(Clone, Debug)]
pub struct LockManagerConfig {
    pub lock_name: String,
    pub owner: String,
    pub wait_ms: u64,
    pub poll_rate_ms: u64,
    pub lease_ms: u64,
}

impl Default for LockManagerConfig {
    fn default() -> Self {
        Self {
            lock_name: ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK.to_string(),
            owner: "default".to_string(),
            wait_ms: 60_000,
            poll_rate_ms: 500,
            lease_ms: 600_000,
        }
    }
}

pub struct LockManager {
    runtime_service: Arc<RuntimeService>,
    config: LockManagerConfig,
    acquisition_owner: String,
    has_acquired: AtomicBool,
}

#[must_use = "dropping the permit releases the global acquisition lock"]
pub(crate) struct GlobalAcquirePermit<'manager> {
    manager: &'manager LockManager,
    release_on_drop: bool,
}

impl GlobalAcquirePermit<'_> {
    pub(crate) fn ensure_lock(&self, expected_lock_name: &str) -> Result<(), FlowableError> {
        if self.manager.lock_name().ends_with(expected_lock_name) {
            return Ok(());
        }
        Err(FlowableError::Internal(format!(
            "global acquire permit for '{}' cannot authorize '{}'",
            self.manager.lock_name(),
            expected_lock_name
        )))
    }

    pub(crate) fn finish(mut self) -> Result<GlobalLockReleaseOutcome, FlowableError> {
        self.release_on_drop = false;
        self.manager.try_release()
    }
}

impl Drop for GlobalAcquirePermit<'_> {
    fn drop(&mut self) {
        if !self.release_on_drop {
            return;
        }
        if let Err(error) = self.manager.try_release() {
            tracing::error!(
                lock_name = self.manager.lock_name(),
                owner = self.manager.owner(),
                "failed to release global acquire lock while dropping permit: {error}"
            );
        }
    }
}

impl LockManager {
    pub fn new(runtime_service: Arc<RuntimeService>, config: LockManagerConfig) -> Self {
        let acquisition_owner = format!(
            "{}:global-acquisition:{}",
            config.owner,
            uuid::Uuid::new_v4()
        );
        Self {
            runtime_service,
            config,
            acquisition_owner,
            has_acquired: AtomicBool::new(false),
        }
    }

    pub fn lock_name(&self) -> &str {
        &self.config.lock_name
    }
    pub fn owner(&self) -> &str {
        &self.acquisition_owner
    }
    pub fn has_acquired(&self) -> bool {
        self.has_acquired.load(Ordering::SeqCst)
    }

    fn try_acquire(&self) -> Result<bool, FlowableError> {
        if self.has_acquired.load(Ordering::SeqCst) {
            return Ok(true);
        }
        let acquired = self.runtime_service.try_acquire_executor_global_lock(
            &self.config.lock_name,
            &self.acquisition_owner,
            self.config.lease_ms as i64,
        )?;
        if acquired {
            self.has_acquired.store(true, Ordering::SeqCst);
        }
        Ok(acquired)
    }

    pub fn acquire(&self) -> bool {
        match self.try_acquire() {
            Ok(acquired) => acquired,
            Err(error) => {
                tracing::warn!(
                    lock_name = self.lock_name(),
                    owner = self.owner(),
                    "failed to acquire global acquire lock: {error}"
                );
                false
            }
        }
    }

    pub(crate) fn try_acquire_permit(
        &self,
    ) -> Result<Option<GlobalAcquirePermit<'_>>, FlowableError> {
        if !self.try_acquire()? {
            return Ok(None);
        }
        Ok(Some(GlobalAcquirePermit {
            manager: self,
            release_on_drop: true,
        }))
    }

    pub fn wait_for_lock(&self) -> bool {
        self.wait_for_lock_with_timeout(self.config.wait_ms)
    }

    pub fn wait_for_lock_with_timeout(&self, wait_ms: u64) -> bool {
        if self.acquire() {
            return true;
        }
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(self.config.poll_rate_ms.max(1)));
            if self.acquire() {
                return true;
            }
        }
        false
    }

    pub(crate) fn try_wait_for_permit(
        &self,
    ) -> Result<Option<GlobalAcquirePermit<'_>>, FlowableError> {
        self.try_wait_for_permit_with_timeout(self.config.wait_ms)
    }

    pub(crate) fn try_wait_for_permit_with_timeout(
        &self,
        wait_ms: u64,
    ) -> Result<Option<GlobalAcquirePermit<'_>>, FlowableError> {
        if let Some(permit) = self.try_acquire_permit()? {
            return Ok(Some(permit));
        }
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(self.config.poll_rate_ms.max(1)));
            if let Some(permit) = self.try_acquire_permit()? {
                return Ok(Some(permit));
            }
        }
        Ok(None)
    }

    pub fn try_release(&self) -> Result<GlobalLockReleaseOutcome, FlowableError> {
        if !self.has_acquired.load(Ordering::SeqCst) {
            return Ok(GlobalLockReleaseOutcome::NotHeldLocally);
        }

        let released = self
            .runtime_service
            .release_global_lock(&self.config.lock_name, &self.acquisition_owner)?;
        self.has_acquired.store(false, Ordering::SeqCst);
        Ok(if released {
            GlobalLockReleaseOutcome::Released
        } else {
            GlobalLockReleaseOutcome::OwnershipLost
        })
    }

    pub fn release(&self) {
        if let Err(error) = self.try_release() {
            tracing::warn!(
                lock_name = self.lock_name(),
                owner = self.owner(),
                "failed to release global acquire lock: {error}"
            );
        }
    }

    pub fn wait_for_lock_run_and_release<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce() -> R,
    {
        if !self.wait_for_lock() {
            return None;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        self.release();
        match result {
            Ok(value) => Some(value),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

impl Drop for LockManager {
    fn drop(&mut self) {
        if let Err(error) = self.try_release() {
            tracing::error!(
                lock_name = self.lock_name(),
                owner = self.owner(),
                "failed to release global acquire lock while dropping manager: {error}"
            );
        }
    }
}

#[cfg(test)]
mod value_format_tests {
    use super::*;
    use crate::engine::process_engine::ProcessEngine;
    use crate::engine::time_source::{SystemTimeSource, TimeSource};
    use crate::persistence::db_store::DbStore;
    use crate::persistence::runtime_store::PersistedPropertyLockState;
    use crate::persistence::runtime_store::RuntimeStore;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn lock_manager(
        runtime_service: Arc<RuntimeService>,
        lock_name: &str,
        owner: &str,
    ) -> LockManager {
        LockManager::new(
            runtime_service,
            LockManagerConfig {
                lock_name: lock_name.to_string(),
                owner: owner.to_string(),
                wait_ms: 10,
                poll_rate_ms: 1,
                lease_ms: 30_000,
            },
        )
    }

    fn runtime_service() -> Arc<RuntimeService> {
        let engine = ProcessEngine::build(
            "global-lock-permit-unit".to_string(),
            Arc::new(SystemTimeSource) as Arc<dyn TimeSource>,
            Arc::new(DbStore::new_in_memory().unwrap()),
        );
        engine.get_runtime_service()
    }

    #[test]
    fn encode_parse_roundtrip() {
        let encoded = RuntimeStore::encode_property_lock_value("node-a", 1000, 5000);
        assert_eq!(encoded, "node-a|1000|5000");
        let parsed = RuntimeStore::parse_property_lock_value(&encoded).unwrap();
        assert_eq!(parsed.0, "node-a");
        assert_eq!(parsed.1, 1000);
        assert_eq!(parsed.2, 5000);
    }

    #[test]
    fn empty_value_is_free() {
        assert!(RuntimeStore::parse_property_lock_value("").is_none());
        assert!(RuntimeStore::parse_property_lock_value("   ").is_none());
    }

    #[test]
    fn legacy_parser_remains_compatible_while_typed_parser_marks_corruption() {
        assert!(RuntimeStore::parse_property_lock_value("only-owner").is_none());
        assert!(RuntimeStore::parse_property_lock_value("a|notanumber|2").is_none());
        assert_eq!(
            RuntimeStore::parse_property_lock_state("only-owner"),
            PersistedPropertyLockState::Corrupt
        );
        assert_eq!(
            RuntimeStore::parse_property_lock_state("a|notanumber|2"),
            PersistedPropertyLockState::Corrupt
        );
    }

    #[test]
    fn permit_drop_releases_lock_on_early_return() {
        fn fail_after_acquire(manager: &LockManager) -> Result<(), FlowableError> {
            let _permit = manager
                .try_acquire_permit()?
                .expect("first manager should acquire permit");
            Err(FlowableError::Generic("operation failed".to_string()))
        }

        let runtime_service = runtime_service();
        let first = lock_manager(Arc::clone(&runtime_service), "earlyReturnLock", "first");
        let second = lock_manager(runtime_service, "earlyReturnLock", "second");

        assert!(fail_after_acquire(&first).is_err());
        assert!(!first.has_acquired());
        assert!(second.acquire());
        second.release();
    }

    #[test]
    fn permit_drop_releases_lock_during_unwind() {
        let runtime_service = runtime_service();
        let first = lock_manager(Arc::clone(&runtime_service), "panicLock", "first");
        let second = lock_manager(runtime_service, "panicLock", "second");

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _permit = first
                .try_acquire_permit()
                .unwrap()
                .expect("first manager should acquire permit");
            panic!("boom");
        }));

        assert!(panic.is_err());
        assert!(!first.has_acquired());
        assert!(second.acquire());
        second.release();
    }

    #[test]
    fn permit_cannot_authorize_a_different_global_lock() {
        let runtime_service = runtime_service();
        let manager = lock_manager(
            runtime_service,
            ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK,
            "async-owner",
        );
        let permit = manager
            .try_acquire_permit()
            .unwrap()
            .expect("manager should acquire permit");

        let error = permit
            .ensure_lock(ACQUIRE_TIMER_JOBS_GLOBAL_LOCK)
            .expect_err("async permit must not authorize timer bulk acquisition");
        assert!(error.to_string().contains("cannot authorize"));
        assert_eq!(permit.finish().unwrap(), GlobalLockReleaseOutcome::Released);
    }
}
