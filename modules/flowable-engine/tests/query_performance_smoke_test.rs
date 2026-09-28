// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_query_performance_smoke() {
    let engine = ProcessEngine::new("test".to_string()).unwrap();
    let store = engine.get_runtime_store();

    let mut session = store.create_session().unwrap();

    let idx_count: i64 = session
        .raw_query_one(
            "SELECT COUNT(*) AS RES_ FROM sqlite_master WHERE type='index' AND name='idx_proc_inst_def'",
            flowable_engine::persistence::DbParams::new(),
        )
        .unwrap()
        .and_then(|r| r.get_integer("RES_"))
        .unwrap_or(0);

    assert!(idx_count > 0, "Missing required index idx_proc_inst_def");

    let idx_count2: i64 = session
        .raw_query_one(
            "SELECT COUNT(*) AS RES_ FROM sqlite_master WHERE type='index' AND name='idx_hist_proc_inst_id'",
            flowable_engine::persistence::DbParams::new(),
        )
        .unwrap()
        .and_then(|r| r.get_integer("RES_"))
        .unwrap_or(0);

    assert!(
        idx_count2 > 0,
        "Missing required index idx_hist_proc_inst_id"
    );

    let rows = session.raw_query(
        "EXPLAIN QUERY PLAN SELECT * FROM process_instances WHERE process_definition_id = 'test'",
        flowable_engine::persistence::DbParams::new(),
    ).unwrap();
    let mut uses_index = false;
    for row in rows {
        if let Some(detail) = row.get_text("detail")
            && detail.contains("USING INDEX idx_proc_inst_def")
        {
            uses_index = true;
            break;
        }
    }
    assert!(
        uses_index,
        "Query plan should use idx_proc_inst_def index for process definition queries"
    );

    let _ = session.rollback();
}
