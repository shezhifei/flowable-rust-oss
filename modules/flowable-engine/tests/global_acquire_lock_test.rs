//! Tests for the cluster global job acquire lock (M79).

use chrono::{TimeZone, Utc};
use flowable_engine::engine::lock_manager::{
    ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK, GlobalLockReleaseOutcome, LockManager, LockManagerConfig,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::RuntimeStore;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

fn unique_sqlite_path(prefix: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!(
        "{prefix}-{}-{}.sqlite",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let path = path.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{path}-wal"));
    let _ = std::fs::remove_file(format!("{path}-shm"));
    path
}

#[test]
fn lock_value_encode_parse_roundtrip() {
    let v = RuntimeStore::encode_property_lock_value("owner-1", 1_000, 2_000);
    let (owner, acquired, expiry) = RuntimeStore::parse_property_lock_value(&v).unwrap();
    assert_eq!(owner, "owner-1");
    assert_eq!(acquired, 1_000);
    assert_eq!(expiry, 2_000);
    assert!(RuntimeStore::parse_property_lock_value("").is_none());
}

#[test]
fn acquire_release_and_contention_on_memory_db() {
    let time = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "global-lock-unit".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::new(DbStore::new_in_memory().unwrap()),
    );
    let rs = engine.get_runtime_service();
    let lock_name = ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK;
    let lease_ms = 60_000i64;

    // First owner acquires.
    assert!(
        rs.try_acquire_global_lock(lock_name, "node-a", lease_ms)
            .unwrap()
    );
    // Second owner is blocked.
    assert!(
        !rs.try_acquire_global_lock(lock_name, "node-b", lease_ms)
            .unwrap()
    );
    // Same owner can renew.
    assert!(
        rs.try_acquire_global_lock(lock_name, "node-a", lease_ms)
            .unwrap()
    );
    // Release by wrong owner fails.
    assert!(!rs.release_global_lock(lock_name, "node-b").unwrap());
    // Release by owner succeeds.
    assert!(rs.release_global_lock(lock_name, "node-a").unwrap());
    // Now node-b can take it.
    assert!(
        rs.try_acquire_global_lock(lock_name, "node-b", lease_ms)
            .unwrap()
    );
    assert!(rs.release_global_lock(lock_name, "node-b").unwrap());
}

#[test]
fn force_reclaim_expired_lock() {
    let time = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "global-lock-expiry".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::new(DbStore::new_in_memory().unwrap()),
    );
    let rs = engine.get_runtime_service();
    let lock_name = "testExpiryLock";
    let lease_ms = 5_000i64;

    assert!(
        rs.try_acquire_global_lock(lock_name, "holder", lease_ms)
            .unwrap()
    );
    // Still within lease — cannot reclaim.
    assert!(
        !rs.try_acquire_global_lock(lock_name, "reclaimer", lease_ms)
            .unwrap()
    );

    // Advance past expiry.
    time.advance_time(lease_ms + 1);
    assert!(
        rs.try_acquire_global_lock(lock_name, "reclaimer", lease_ms)
            .unwrap(),
        "expired lock should be force-reclaimed"
    );
    // Original holder can no longer release.
    assert!(!rs.release_global_lock(lock_name, "holder").unwrap());
    assert!(rs.release_global_lock(lock_name, "reclaimer").unwrap());
}

#[test]
fn lock_manager_wait_run_and_release() {
    let time = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "global-lock-manager".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::new(DbStore::new_in_memory().unwrap()),
    );
    let rs = engine.get_runtime_service();

    let lm = LockManager::new(
        Arc::clone(&rs),
        LockManagerConfig {
            lock_name: "managerLock".to_string(),
            owner: "mgr-1".to_string(),
            wait_ms: 2_000,
            poll_rate_ms: 50,
            lease_ms: 30_000,
        },
    );

    let result = lm.wait_for_lock_run_and_release(|| 42);
    assert_eq!(result, Some(42));
    assert!(!lm.has_acquired());

    // Lock is free again after run-and-release.
    assert!(
        rs.try_acquire_global_lock("managerLock", "other", 30_000)
            .unwrap()
    );
    let _ = rs.release_global_lock("managerLock", "other");
}

#[test]
fn lock_managers_with_the_same_job_owner_still_exclude_each_other() {
    let time = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "global-lock-same-owner".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::new(DbStore::new_in_memory().unwrap()),
    );
    let runtime_service = engine.get_runtime_service();
    let config = LockManagerConfig {
        lock_name: "sameOwnerLock".to_string(),
        owner: "shared-job-owner".to_string(),
        wait_ms: 10,
        poll_rate_ms: 1,
        lease_ms: 30_000,
    };
    let first = LockManager::new(Arc::clone(&runtime_service), config.clone());
    let second = LockManager::new(runtime_service, config);

    assert_ne!(first.owner(), second.owner());
    assert!(first.acquire());
    assert!(!second.acquire());
    assert_eq!(
        first.try_release().unwrap(),
        GlobalLockReleaseOutcome::Released
    );
    assert!(second.acquire());
    assert_eq!(
        second.try_release().unwrap(),
        GlobalLockReleaseOutcome::Released
    );
}

#[test]
fn stale_token_release_clears_local_state_without_releasing_new_owner() {
    let time = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "global-lock-stale-token".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::new(DbStore::new_in_memory().unwrap()),
    );
    let runtime_service = engine.get_runtime_service();
    let config = LockManagerConfig {
        lock_name: "staleTokenLock".to_string(),
        owner: "shared-job-owner".to_string(),
        wait_ms: 10,
        poll_rate_ms: 1,
        lease_ms: 20,
    };
    let first = LockManager::new(Arc::clone(&runtime_service), config.clone());
    let second = LockManager::new(Arc::clone(&runtime_service), config.clone());
    let third = LockManager::new(runtime_service, config);

    assert!(first.acquire());
    thread::sleep(Duration::from_millis(40));
    assert!(second.acquire());

    assert_eq!(
        first.try_release().unwrap(),
        GlobalLockReleaseOutcome::OwnershipLost
    );
    assert!(!first.has_acquired());
    assert!(
        !third.acquire(),
        "stale release must not clear the new owner"
    );

    assert_eq!(
        second.try_release().unwrap(),
        GlobalLockReleaseOutcome::Released
    );
    assert!(third.acquire());
    third.release();
}

#[test]
fn release_error_preserves_local_acquired_state_until_retry_succeeds() {
    let time = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap(),
    ));
    let db_path = unique_sqlite_path("flowable-global-release-fault");
    let engine = ProcessEngine::build(
        "global-lock-release-fault".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::new(DbStore::new_file(&db_path).unwrap()),
    );
    let runtime_service = engine.get_runtime_service();
    let manager = LockManager::new(
        Arc::clone(&runtime_service),
        LockManagerConfig {
            lock_name: "releaseFaultLock".to_string(),
            owner: "release-fault-owner".to_string(),
            wait_ms: 10,
            poll_rate_ms: 1,
            lease_ms: 30_000,
        },
    );
    assert!(manager.acquire());

    {
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection
            .execute_batch("ALTER TABLE ACT_GE_PROPERTY RENAME TO ACT_GE_PROPERTY_RELEASE_FAULT")
            .unwrap();
    }
    assert!(manager.try_release().is_err());
    assert!(
        manager.has_acquired(),
        "storage failure must preserve local acquired state"
    );

    {
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection
            .execute_batch("ALTER TABLE ACT_GE_PROPERTY_RELEASE_FAULT RENAME TO ACT_GE_PROPERTY")
            .unwrap();
    }
    assert_eq!(
        manager.try_release().unwrap(),
        GlobalLockReleaseOutcome::Released
    );
    assert!(!manager.has_acquired());

    let contender = LockManager::new(
        runtime_service,
        LockManagerConfig {
            lock_name: "releaseFaultLock".to_string(),
            owner: "contender".to_string(),
            wait_ms: 10,
            poll_rate_ms: 1,
            lease_ms: 30_000,
        },
    );
    assert!(contender.acquire());
    contender.release();

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{db_path}-wal"));
    let _ = std::fs::remove_file(format!("{db_path}-shm"));
}

#[test]
fn malformed_global_lock_value_is_reported_instead_of_treated_as_free() {
    let time = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap(),
    ));
    let db_path = unique_sqlite_path("flowable-global-corrupt-value");
    let engine = ProcessEngine::build(
        "global-lock-corrupt-value".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::new(DbStore::new_file(&db_path).unwrap()),
    );
    let runtime_service = engine.get_runtime_service();
    let lock_name = "corruptValueLock";

    assert!(
        runtime_service
            .try_acquire_global_lock(lock_name, "seed-owner", 30_000)
            .unwrap()
    );
    assert!(
        runtime_service
            .release_global_lock(lock_name, "seed-owner")
            .unwrap()
    );

    {
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection
            .execute(
                "UPDATE ACT_GE_PROPERTY SET VALUE_ = ?1 WHERE NAME_ = ?2",
                ("only-an-owner", lock_name),
            )
            .unwrap();
    }

    let error = runtime_service
        .try_acquire_global_lock(lock_name, "next-owner", 30_000)
        .expect_err("malformed property lock must fail closed");
    assert!(error.to_string().contains("corrupt global lock value"));

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{db_path}-wal"));
    let _ = std::fs::remove_file(format!("{db_path}-shm"));
}

#[test]
fn legacy_lock_manager_api_signatures_remain_compatible() {
    let _: fn(&LockManager) -> bool = LockManager::acquire;
    let _: fn(&LockManager) -> bool = LockManager::wait_for_lock;
    let _: fn(&LockManager) = LockManager::release;
}

#[test]
fn executor_global_lock_expiry_uses_wall_clock_when_engine_clock_is_frozen() {
    let time = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::build(
        "global-lock-wall-clock".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::new(DbStore::new_in_memory().unwrap()),
    );
    let runtime_service = engine.get_runtime_service();
    let config = LockManagerConfig {
        lock_name: "wallClockLock".to_string(),
        owner: "shared-job-owner".to_string(),
        wait_ms: 10,
        poll_rate_ms: 1,
        lease_ms: 20,
    };
    let first = LockManager::new(Arc::clone(&runtime_service), config.clone());
    let second = LockManager::new(runtime_service, config);

    assert!(first.acquire());
    std::thread::sleep(std::time::Duration::from_millis(40));
    assert!(second.acquire());
    second.release();
}

#[test]
fn two_nodes_contend_for_lock_on_shared_sqlite() {
    let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    let time = Arc::new(TestTimeSource::new(now));
    let db_path = unique_sqlite_path("flowable-global-lock");
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine_a = Arc::new(ProcessEngine::build(
        "node-a".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::clone(&db_store),
    ));
    let engine_b = Arc::new(ProcessEngine::build(
        "node-b".to_string(),
        Arc::clone(&time) as Arc<dyn TimeSource>,
        Arc::clone(&db_store),
    ));

    let lock_name = "sharedAcquireLock";
    let lease_ms = 60_000i64;

    // Node A holds the executor lock using the same wall-clock/token semantics
    // as the contending LockManager on node B.
    let manager_a = LockManager::new(
        engine_a.get_runtime_service(),
        LockManagerConfig {
            lock_name: lock_name.to_string(),
            owner: "node-a".to_string(),
            wait_ms: 5_000,
            poll_rate_ms: 50,
            lease_ms: lease_ms as u64,
        },
    );
    assert!(manager_a.acquire());

    let acquired_b = Arc::new(AtomicUsize::new(0));
    let acquired_b_clone = Arc::clone(&acquired_b);
    let engine_b_clone = Arc::clone(&engine_b);

    // Node B spins until A releases (or times out).
    let handle = thread::spawn(move || {
        let rs = engine_b_clone.get_runtime_service();
        let lm = LockManager::new(
            Arc::clone(&rs),
            LockManagerConfig {
                lock_name: lock_name.to_string(),
                owner: "node-b".to_string(),
                wait_ms: 5_000,
                poll_rate_ms: 50,
                lease_ms: lease_ms as u64,
            },
        );
        if lm.wait_for_lock() {
            acquired_b_clone.fetch_add(1, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(50));
            lm.release();
        }
    });

    // Hold briefly then release so B can win.
    thread::sleep(Duration::from_millis(200));
    manager_a.release();

    handle.join().unwrap();
    assert_eq!(
        acquired_b.load(Ordering::SeqCst),
        1,
        "node-b should acquire the lock after node-a releases"
    );

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{db_path}-wal"));
    let _ = std::fs::remove_file(format!("{db_path}-shm"));
}
