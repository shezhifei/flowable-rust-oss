//! Flowable 8 AddCommentCmd distinguishes a failed query from a missing row.
//! Java DbSqlSession.selectOne propagates the database failure before the
//! command can construct FlowableObjectNotFoundException.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::task::Task;

fn engine_with_task(name: &str) -> ProcessEngine {
    let engine = ProcessEngine::new(name.to_string()).expect("create engine");
    let task = Task::new(
        "task-1".into(),
        String::new(),
        String::new(),
        "standalone".into(),
        "Standalone task".into(),
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("create session");
    store.insert_task(&task, &mut session);
    session.flush_and_commit().expect("persist task");
    engine
}

fn execute_sql(engine: &ProcessEngine, sql: &str) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("create session");
    session.execute_raw_sql(sql).expect("inject storage fault");
    session.flush_and_commit().expect("commit storage fault");
}

#[test]
fn failed_task_lookup_keeps_storage_error_instead_of_not_found() {
    let engine = engine_with_task("java8-task-read-failure");
    execute_sql(&engine, "DROP TABLE tasks");

    let result = engine.get_history_service().create_task_comment(
        "task-1", None, "comment", None,
    );
    assert!(matches!(result, Err(FlowableError::Internal(_))), "{result:?}");
}

#[test]
fn failed_process_lookup_keeps_storage_error_instead_of_not_found() {
    let engine = engine_with_task("java8-process-read-failure");
    execute_sql(&engine, "DROP TABLE process_instances");

    let result = engine.get_history_service().create_task_comment(
        "task-1", Some("process-1"), "comment", None,
    );
    assert!(matches!(result, Err(FlowableError::Internal(_))), "{result:?}");
}

#[test]
fn successful_missing_task_lookup_remains_not_found() {
    let engine = engine_with_task("java8-task-missing");
    let result = engine.get_history_service().create_task_comment(
        "missing", None, "comment", None,
    );
    assert!(matches!(result, Err(FlowableError::NotFound(_))), "{result:?}");
}

#[test]
fn corrupt_task_payload_keeps_deserialization_failure() {
    let engine = engine_with_task("java8-task-corrupt");
    execute_sql(&engine, "UPDATE tasks SET data = '{' WHERE id = 'task-1'");
    let result = engine.get_history_service().create_task_comment(
        "task-1", None, "comment", None,
    );
    assert!(matches!(result, Err(FlowableError::Internal(_))), "{result:?}");
}
