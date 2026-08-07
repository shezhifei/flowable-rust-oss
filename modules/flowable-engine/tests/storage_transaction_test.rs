use flowable_engine::persistence::db_store::DbStore;
use std::sync::Arc;

fn create_store() -> Arc<DbStore> {
    Arc::new(DbStore::new_in_memory().unwrap())
}

#[test]
fn command_rolls_back_after_write_then_read_then_error() {
    let store = create_store();
    let mut s = store.create_session().unwrap();

    s.insert("executions", "e1", &serde_json::json!({"v": 1}))
        .unwrap();

    let seen: Option<serde_json::Value> = s.find("executions", "e1").unwrap();
    assert!(
        seen.is_some(),
        "read-your-writes should see the pending insert"
    );

    s.rollback().unwrap();

    let mut s2 = store.create_session().unwrap();
    let after: Option<serde_json::Value> = s2.find("executions", "e1").unwrap();
    assert!(after.is_none(), "rolled-back write must not be visible");
    s2.rollback().unwrap();
}

#[test]
fn command_commit_is_visible_to_next_command() {
    let store = create_store();
    let mut s1 = store.create_session().unwrap();
    s1.insert("executions", "e1", &serde_json::json!({"v": 42}))
        .unwrap();
    s1.flush_and_commit().unwrap();

    let mut s2 = store.create_session().unwrap();
    let seen: Option<serde_json::Value> = s2.find("executions", "e1").unwrap();
    assert!(
        seen.is_some(),
        "committed write must be visible to next session"
    );
    assert_eq!(seen.unwrap()["v"], 42);
    s2.rollback().unwrap();
}

#[test]
fn sqlite_backend_transaction_rollback() {
    let store = create_store();
    let mut s = store.create_session().unwrap();
    s.insert("executions", "e1", &serde_json::json!({"v": 1}))
        .unwrap();
    assert!(
        s.find::<serde_json::Value>("executions", "e1")
            .unwrap()
            .is_some()
    );
    s.rollback().unwrap();

    let mut s2 = store.create_session().unwrap();
    assert!(
        s2.find::<serde_json::Value>("executions", "e1")
            .unwrap()
            .is_none()
    );
    s2.rollback().unwrap();
}
