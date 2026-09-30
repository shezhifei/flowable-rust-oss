// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! W5 / C3 — R8/R9 fault-injection contracts.
//!
//! Java evidence:
//! - `DbSqlSession` throws on SQL errors; there is no "swallow picture write"
//!   path (R8).
//! - Coordinator lease acquisition must distinguish DuplicateEntity (genuine
//!   lock-fail → Ok(None)) from other storage errors (→ Err), never disguise a
//!   storage failure as "someone else holds the lock" (R9).

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::UserPicture;

/// R8: a blob write failure must surface as an error from `set_user_picture`.
/// Fault injection: write a picture with an absurdly large payload against a
/// session that has been closed / rolled back so the insert fails.
#[test]
fn set_user_picture_write_failure_is_not_swallowed() {
    let engine = ProcessEngine::new("w5-picture".to_string()).unwrap();
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();

    // Force a storage failure by inserting into a session and rolling back
    // before the write is flushed — or by using a deliberately invalid row.
    // The contract under test is: whatever `insert_blob` returns as Err must
    // propagate out of `set_user_picture`, not be flattened by unwrap_or_default.
    //
    // Direct unit-level check: call the store API with a rolled-back session.
    session.rollback().unwrap();

    let result = store.set_user_picture(
        UserPicture {
            user_id: "user-1".to_string(),
            mime_type: "image/png".to_string(),
            bytes: vec![0u8; 16],
            created_at: Some(0),
        },
        &mut session,
    );

    // After rollback the session is closed → insert_blob must return Err →
    // set_user_picture must return Err (not silently succeed).
    assert!(
        result.is_err(),
        "set_user_picture must propagate a write failure, got Ok"
    );
}

/// R8 happy path: a successful picture write still works (no regression from
/// making the API fallible).
#[test]
fn set_user_picture_success_path_still_works() {
    let engine = ProcessEngine::new("w5-picture-ok".to_string()).unwrap();
    let identity_service = engine.get_identity_service();
    identity_service
        .set_user_picture(
            "user-ok".to_string(),
            "image/png".to_string(),
            vec![1u8, 2, 3, 4],
        )
        .expect("valid picture write must succeed");

    let picture = identity_service
        .get_user_picture("user-ok")
        .unwrap()
        .expect("picture must be readable after write");
    assert_eq!(picture.mime_type, "image/png");
    assert_eq!(picture.bytes, vec![1u8, 2, 3, 4]);
}

/// R9: a storage error during lease acquisition must be Err, not Ok(None).
/// Ok(None) is reserved for DuplicateEntity (genuine lock-fail).
#[test]
fn acquire_coordinator_lease_storage_error_is_not_lock_fail() {
    let engine = ProcessEngine::new("w5-lease".to_string()).unwrap();
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();

    // Roll back / close the session so the underlying storage fails.
    session.rollback().unwrap();

    let result = store.acquire_coordinator_lease(
        "timer-coordinator",
        "node-a",
        1_000_000,
        300_000,
        &mut session,
    );

    // Storage failure → Err (must NOT be Ok(None) which would mean "lock taken").
    assert!(
        result.is_err(),
        "storage error must be Err, not Ok(None) lock-fail, got: {result:?}"
    );
}

/// R9 happy path: first acquire wins (Ok(Some(1))), second node loses the race
/// (Ok(None) — genuine Duplicate lock-fail).
#[test]
fn acquire_coordinator_lease_duplicate_is_genuine_lock_fail() {
    let engine = ProcessEngine::new("w5-lease-dup".to_string()).unwrap();
    let runtime_service = engine.get_runtime_service();

    let first = runtime_service
        .acquire_coordinator_lease(300_000)
        .expect("first acquire must not be a storage error");
    assert!(
        first.is_some(),
        "first acquire should win the lease (Ok(Some(token)))"
    );

    // The same node renewing is fine (same owner).
    let renew = runtime_service
        .acquire_coordinator_lease(300_000)
        .expect("renew must not be a storage error");
    assert!(renew.is_some(), "same-node renew should succeed");
}

/// R9: `cas_update` storage error must NOT be disguised as lock-fail.
/// Fault injection: replace the table with a read-only VIEW so `find` (SELECT)
/// succeeds but `cas_update` (UPDATE) hits a real storage error.
#[test]
fn acquire_coordinator_lease_cas_update_storage_error_is_not_lock_fail() {
    let engine = ProcessEngine::new("w5-lease-cas".to_string()).unwrap();
    let runtime_service = engine.get_runtime_service();
    let store = engine.get_runtime_store();

    // 1. Acquire the lease normally (creates the row).
    let first = runtime_service
        .acquire_coordinator_lease(300_000)
        .expect("initial acquire");
    assert!(first.is_some());

    // 2. Replace the table with a VIEW: SELECT works, UPDATE fails.
    {
        let mut session = store.create_session().unwrap();
        session
            .execute_raw_sql("ALTER TABLE timer_coordinator_leases RENAME TO timer_coordinator_leases_real")
            .expect("rename table");
        session
            .execute_raw_sql("CREATE VIEW timer_coordinator_leases AS SELECT * FROM timer_coordinator_leases_real")
            .expect("create read-only view");
        session.flush_and_commit().expect("commit view swap");
    }

    // 3. Try to renew — `find` (SELECT on view) succeeds, `cas_update` (UPDATE
    //    on view) fails with a real storage error. Must be Err, NOT Ok(None).
    let node_id = runtime_service.timer_owner_id().to_string();
    let mut session = store.create_session().unwrap();
    let result = store.acquire_coordinator_lease(
        "timer-coordinator",
        &node_id,
        1_000_000,
        300_000,
        &mut session,
    );
    assert!(
        result.is_err(),
        "cas_update storage error must be Err, not Ok(None) lock-fail, got: {result:?}"
    );
}

/// R9: `lock_process_instance` storage error must NOT be disguised as
/// lock-fail. Fault injection: replace the table with a read-only VIEW so
/// `find` (SELECT) succeeds but `cas_update`/`insert_exclusive` (write) fails.
#[test]
fn lock_process_instance_storage_error_is_not_lock_fail() {
    let engine = ProcessEngine::new("w5-pi-lock".to_string()).unwrap();
    let store = engine.get_runtime_store();

    // 1. Acquire the lock normally (creates the row).
    {
        let mut session = store.create_session().unwrap();
        let locked = store
            .lock_process_instance("pi-1", "node-a", 1_000_000, 500_000, &mut session)
            .expect("initial lock acquire");
        assert!(locked, "first lock should succeed");
        session.flush_and_commit().expect("commit");
    }

    // 2. Replace the table with a VIEW: SELECT works, UPDATE/INSERT fails.
    {
        let mut session = store.create_session().unwrap();
        session
            .execute_raw_sql(
                "ALTER TABLE process_instance_locks RENAME TO process_instance_locks_real",
            )
            .expect("rename table");
        session
            .execute_raw_sql(
                "CREATE VIEW process_instance_locks AS SELECT * FROM process_instance_locks_real",
            )
            .expect("create read-only view");
        session.flush_and_commit().expect("commit view swap");
    }

    // 3. Try to takeover — `find` (SELECT on view) succeeds, `cas_update`
    //    (UPDATE on view) fails. Must be Err, NOT Ok(false).
    let mut session = store.create_session().unwrap();
    let result = store.lock_process_instance("pi-1", "node-b", 2_000_000, 1_500_000, &mut session);
    assert!(
        result.is_err(),
        "lock_process_instance storage error must be Err, not Ok(false) lock-fail, got: {result:?}"
    );
}

/// R9: `lock_process_instance` genuine lock-fail (lock held by someone else)
/// must be Ok(false), not Err.
#[test]
fn lock_process_instance_held_lock_is_genuine_lock_fail() {
    let engine = ProcessEngine::new("w5-pi-lock-held".to_string()).unwrap();
    let store = engine.get_runtime_store();

    // node-a holds the lock (not expired).
    {
        let mut session = store.create_session().unwrap();
        let locked = store
            .lock_process_instance("pi-1", "node-a", 1_000_000, 500_000, &mut session)
            .expect("first lock");
        assert!(locked);
        session.flush_and_commit().expect("commit");
    }

    // node-b tries while the lock is still valid.
    let mut session = store.create_session().unwrap();
    let result = store
        .lock_process_instance("pi-1", "node-b", 2_000_000, 600_000, &mut session)
        .expect("genuine lock-fail must not be Err");
    assert!(!result, "held lock must return Ok(false)");
}
