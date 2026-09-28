// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! P73a contract: dual-write failures hard-fail instead of being swallowed.
//!
//! Transition state (ADR-0001 Phase 5): JSON tables remain the read path while
//! ACT_* normalized tables are dual-written. Swallowing DataManager errors with
//! `let _ =` is unsafe:
//! - On PostgreSQL a failed statement aborts the whole transaction, so later
//!   work on the same session fails with "current transaction is aborted".
//! - Worse, when the backend tolerates the failure, the primary JSON write can
//!   succeed while ACT_* silently diverges.
//!
//! Decision: sticky-record the dual-write error on the session so the commit
//! fails with a typed error (Java command/transaction failure), never panic and
//! never silently commit divergent state.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::execution::Execution;

#[test]
fn dual_write_execution_insert_failure_fails_commit() {
    let engine = ProcessEngine::new_with_memory_backend("dual-write-fail-mem".to_string()).unwrap();
    let store = engine.get_runtime_store();

    // Drop normalized table so dual-write insert cannot succeed.
    {
        let mut session = store.create_session().expect("session");
        session
            .execute_raw_sql("DROP TABLE IF EXISTS ACT_RU_EXECUTION")
            .expect("drop ACT_RU_EXECUTION");
        session.flush_and_commit().expect("commit drop");
    }

    let mut session = store.create_session().expect("session for insert");
    let execution = Execution {
        id: "exec-dual-write-fail".to_string(),
        process_instance_id: Some("pi-dual-write-fail".to_string()),
        activity_id: Some("userTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };

    store.insert_execution(&execution, &mut session);
    let err = session
        .flush_and_commit()
        .expect_err("dual-write insert must fail the transaction, not succeed");
    let msg = err.to_string();
    assert!(
        msg.contains("dual-write") && msg.contains("ACT_RU_EXECUTION"),
        "error must identify dual-write failure (queue or flush), got: {msg:?}"
    );
}

#[test]
fn dual_write_execution_succeeds_when_act_table_present() {
    let engine = ProcessEngine::new_with_memory_backend("dual-write-ok-mem".to_string()).unwrap();
    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("session");

    let execution = Execution {
        id: "exec-dual-write-ok".to_string(),
        process_instance_id: Some("pi-dual-write-ok".to_string()),
        activity_id: Some("userTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    store.insert_execution(&execution, &mut session);
    session.flush_and_commit().expect("commit");

    let mut session = store.create_session().expect("session re-open");
    let found = store
        .find_execution("exec-dual-write-ok", &mut session)
        .expect("JSON execution row");
    assert_eq!(found.activity_id.as_deref(), Some("userTask1"));

    let act = flowable_persistence::ExecutionDataManager::new()
        .find_by_id(session.inner_mut(), "exec-dual-write-ok")
        .expect("query ACT_RU_EXECUTION")
        .expect("ACT_RU_EXECUTION dual-write row");
    assert_eq!(act.id, "exec-dual-write-ok");
    assert_eq!(act.activity_id.as_deref(), Some("userTask1"));
}
