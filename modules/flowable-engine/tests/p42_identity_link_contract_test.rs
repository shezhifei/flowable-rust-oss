// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::task::Task;

fn standalone_task(engine: &ProcessEngine, task_id: &str) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let task = Task::new(
        task_id.to_string(),
        String::new(),
        String::new(),
        "standalone".to_string(),
        "Standalone".to_string(),
    );
    store.insert_task(&task, &mut session);
    session.flush_and_commit().unwrap();
}

#[test]
fn assignee_and_owner_identity_links_update_task_without_creating_link_rows() {
    let engine = ProcessEngine::new("p42-identity-link-direct".to_string()).unwrap();
    standalone_task(&engine, "task-1");
    let service = engine.get_task_service();

    service
        .add_identity_link(
            "task-1".to_string(),
            Some("kermit".to_string()),
            None,
            "assignee".to_string(),
        )
        .unwrap();
    service
        .add_identity_link(
            "task-1".to_string(),
            Some("fozzie".to_string()),
            None,
            "owner".to_string(),
        )
        .unwrap();

    let task = service
        .create_task_query()
        .list()
        .unwrap()
        .into_iter()
        .find(|task| task.id == "task-1")
        .unwrap();
    assert_eq!(task.assignee.as_deref(), Some("kermit"));
    assert_eq!(task.owner.as_deref(), Some("fozzie"));
    assert!(
        service
            .get_identity_links_for_task("task-1".to_string())
            .unwrap()
            .is_empty()
    );

    service
        .delete_identity_link(
            "task-1".to_string(),
            Some("kermit".to_string()),
            None,
            "assignee".to_string(),
        )
        .unwrap();
    service
        .delete_identity_link(
            "task-1".to_string(),
            Some("fozzie".to_string()),
            None,
            "owner".to_string(),
        )
        .unwrap();

    let task = service
        .create_task_query()
        .list()
        .unwrap()
        .into_iter()
        .find(|task| task.id == "task-1")
        .unwrap();
    assert!(task.assignee.is_none());
    assert!(task.owner.is_none());
}

#[test]
fn candidate_identity_links_still_use_identity_link_rows() {
    let engine = ProcessEngine::new("p42-identity-link-candidate".to_string()).unwrap();
    standalone_task(&engine, "task-1");

    engine
        .get_task_service()
        .add_identity_link(
            "task-1".to_string(),
            Some("kermit".to_string()),
            None,
            "candidate".to_string(),
        )
        .unwrap();

    let links = engine
        .get_task_service()
        .get_identity_links_for_task("task-1".to_string())
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].link_type, "candidate");
}
