// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! Identity-store read sessions must not disguise a storage failure as a
//! genuinely-missing row.
//!
//! `IdentityService`'s read-only helpers open a session, call a `RuntimeStore`
//! read, then roll the session back. The store's read helpers record a failed
//! read in the session's sticky error slot and return `None` / `Vec::new()`
//! (e.g. `RuntimeStore::find_token`, `find_user`), and `DbSession::rollback`
//! clears that slot (`flowable-engine/src/persistence/db_session.rs:1138`). A
//! helper that rolled back first and returned `Ok(None)` therefore reported
//! "no such row" for an unreachable store.
//!
//! Java parity: reads go straight to MyBatis and the `PersistenceException`
//! escapes — `DbSqlSession.selectOne`/`selectList`
//! (flowable-engine-common/src/main/java/org/flowable/common/engine/impl/db/DbSqlSession.java:282-299)
//! has no error-swallowing catch — so `null`/empty is reachable only from a
//! successful zero-row query. A store failure surfaces as HTTP 500, never as
//! "unknown user"/"no such token".

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::error::FlowableError;
use flowable_engine::identity::entities::{Token, User};
use flowable_engine::persistence::db_store::DbStore;
use std::sync::Arc;

fn engine_with_store(name: &str, store: Arc<DbStore>) -> ProcessEngine {
    let now = Utc.with_ymd_and_hms(2026, 7, 30, 12, 0, 0).unwrap();
    ProcessEngine::build(name.to_string(), Arc::new(TestTimeSource::new(now)), store)
}

fn in_memory_engine(name: &str) -> ProcessEngine {
    engine_with_store(name, Arc::new(DbStore::new_in_memory().unwrap()))
}

/// Makes every later read of `table` fail at the driver level, the way an
/// unreachable database or a dropped/misnamed table fails in production.
fn drop_table(engine: &ProcessEngine, table: &str) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("session");
    session
        .execute_raw_sql(&format!("DROP TABLE {table}"))
        .expect("drop table");
    session.flush_and_commit().expect("commit drop");
}

#[test]
fn missing_token_row_is_ok_none() {
    let engine = in_memory_engine("identity-read-contract-missing-token");
    let identity = engine.get_identity_service();

    let found = identity
        .find_token_by_id("no-such-series")
        .expect("a successful no-row query is not an error");
    assert!(found.is_none());
}

#[test]
fn token_lookup_storage_failure_is_not_disguised_as_missing_row() {
    let engine = in_memory_engine("identity-read-contract-token-storage-failure");
    let identity = engine.get_identity_service();

    identity
        .save_token(Token {
            id: "series-1".to_string(),
            token_value: "value-1".to_string(),
            user_id: Some("user-1".to_string()),
            token_date: Some(1_722_000_000_000),
            ip_address: None,
            user_agent: None,
        })
        .expect("save token");

    drop_table(&engine, "tokens");

    let result = identity.find_token_by_id("series-1");
    match result {
        // The disguise: a broken store reported as "this session does not
        // exist", which the UI auth layer turns into a 401 instead of a 500.
        Ok(None) => panic!(
            "storage failure was disguised as a missing token row: \
             find_token_by_id returned Ok(None) after the tokens table was dropped"
        ),
        Ok(Some(token)) => panic!(
            "impossible: the tokens table was dropped but the row {token:?} was still returned"
        ),
        Err(FlowableError::Internal(message)) => {
            assert!(
                !message.is_empty(),
                "the storage failure must carry its cause"
            );
        }
        Err(other) => panic!("storage failures must map to Internal (HTTP 500), got {other:?}"),
    }
}

#[test]
fn missing_user_row_is_ok_none() {
    let engine = in_memory_engine("identity-read-contract-missing-user");
    let identity = engine.get_identity_service();

    let found = identity
        .find_user_by_id("no-such-user")
        .expect("a successful no-row query is not an error");
    assert!(found.is_none());
}

#[test]
fn user_lookup_storage_failure_is_not_disguised_as_missing_row() {
    let engine = in_memory_engine("identity-read-contract-user-storage-failure");
    let identity = engine.get_identity_service();

    identity
        .save_user(User {
            id: "user-1".to_string(),
            first_name: Some("Ada".to_string()),
            last_name: Some("Lovelace".to_string()),
            email: Some("ada@example.com".to_string()),
            password: None,
            tenant_id: None,
        })
        .expect("save user");

    drop_table(&engine, "users");

    match identity.find_user_by_id("user-1") {
        Ok(None) => panic!(
            "storage failure was disguised as a missing user row: \
             find_user_by_id returned Ok(None) after the users table was dropped"
        ),
        Ok(Some(user)) => {
            panic!("impossible: the users table was dropped but {user:?} was still returned")
        }
        Err(FlowableError::Internal(_)) => {}
        Err(other) => panic!("storage failures must map to Internal (HTTP 500), got {other:?}"),
    }
}
