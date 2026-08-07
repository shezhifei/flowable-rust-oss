//! Engine contract tests for the atomic task-variable mutation command
//! (P2-TVAR): scope (local/global), modes (create-only/update-only/upsert),
//! validation-before-write atomicity, suspension guards, read resolution and
//! history lifecycle. Java parity: SetTaskVariablesCmd / RemoveTaskVariablesCmd
//! plus the REST task-variable resource semantics.

use flowable_engine::cmd::task_variable_cmd::{
    MutateTaskVariablesCmd, TaskVariableMutation, TaskVariableScope, VariableMutationMode,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::error::FlowableError;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use flowable_engine::task::Task;
use serde_json::json;
use std::collections::HashMap;

fn deploy_and_start(engine: &ProcessEngine, process_key: &str) -> (String, String) {
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="{process_key}">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Task 1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );

    repo.deploy(
        repo.create_deployment()
            .add_string(format!("{process_key}.bpmn20.xml"), xml),
    )
    .unwrap();

    let pi = runtime.start_process_instance_by_key(process_key).unwrap();
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    (pi.id, tasks[0].id.clone())
}

fn insert_standalone_task(engine: &ProcessEngine, task_id: &str) {
    let task = Task::new(
        task_id.to_string(),
        String::new(),
        String::new(),
        "standaloneTask".to_string(),
        "Standalone task".to_string(),
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_task(&task, &mut session);
    session.flush_and_commit().unwrap();
}

fn variables(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect()
}

#[test]
fn missing_task_rejected_and_nothing_written() {
    let engine = ProcessEngine::new("task-var-missing-task".to_string());
    let task_service = engine.get_task_service();

    let err = task_service
        .set_task_variable("missing-task".to_string(), "a".to_string(), json!(1))
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::NotFound(message) if message == "Cannot find task with id missing-task"),
        "unexpected error: {err}"
    );

    let err = task_service
        .set_task_variables_local("missing-task".to_string(), variables(&[("a", json!(1))]))
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::NotFound(message) if message == "Cannot find task with id missing-task"),
        "unexpected error: {err}"
    );

    let err = task_service
        .remove_task_variables_local("missing-task".to_string(), vec!["a".to_string()])
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::NotFound(message) if message == "Cannot find task with id missing-task"),
        "unexpected error: {err}"
    );
}

#[test]
fn suspended_task_rejects_local_write_without_side_effects() {
    let engine = ProcessEngine::new("task-var-suspended-local".to_string());
    let (pi_id, task_id) = deploy_and_start(&engine, "suspendedLocalProcess");

    engine
        .get_runtime_service()
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let err = engine
        .get_task_service()
        .set_task_variables_local(task_id.clone(), variables(&[("a", json!(1))]))
        .unwrap_err();
    // Java `SetTaskVariablesCmd#getSuspendedTaskExceptionPrefix`:
    // "Cannot add variables to" (NeedsActiveTaskCmd checks the TASK for both
    // local and global writes).
    assert!(
        matches!(&err, FlowableError::ExecutionError(message) if message == &format!("Cannot add variables to a suspended task '{task_id}'")),
        "unexpected error: {err}"
    );

    // Reads carry no suspension guard and prove nothing was written.
    let locals = engine
        .get_task_service()
        .get_task_local_variables(task_id.clone())
        .unwrap();
    assert!(
        locals.is_empty(),
        "no local variable may be written: {locals:?}"
    );

    let historic = engine
        .get_history_service()
        .create_historic_variable_instance_query()
        .task_id(task_id.clone())
        .list()
        .unwrap();
    assert!(historic.is_empty(), "no history row may be written");
}

#[test]
fn suspended_process_rejects_global_write_without_side_effects() {
    let engine = ProcessEngine::new("task-var-suspended-global".to_string());
    let (pi_id, task_id) = deploy_and_start(&engine, "suspendedGlobalProcess");

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap()
        .pop()
        .unwrap();

    engine
        .get_runtime_service()
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let err = engine
        .get_task_service()
        .set_task_variable(task_id.clone(), "a".to_string(), json!(1))
        .unwrap_err();
    // Java `NeedsActiveTaskCmd` checks the TASK before the write reaches the
    // execution, and suspending a process instance suspends its tasks — so the
    // observable message is the task-side one
    // (`SetTaskVariablesCmd#getSuspendedTaskExceptionPrefix`).
    assert!(
        matches!(&err, FlowableError::ExecutionError(message) if message == &format!("Cannot add variables to a suspended task '{task_id}'")),
        "unexpected error: {err}"
    );

    let value = engine
        .get_variable_service()
        .get_variable(task.execution_id.clone(), "a".to_string())
        .unwrap();
    assert_eq!(value, None, "no execution variable may be written");

    let merged = engine
        .get_task_service()
        .get_task_variables(task_id)
        .unwrap();
    assert!(!merged.contains_key("a"));
}

#[test]
fn suspended_task_rejects_remove_without_side_effects() {
    let engine = ProcessEngine::new("task-var-suspended-remove".to_string());
    let (pi_id, task_id) = deploy_and_start(&engine, "suspendedRemoveProcess");

    let task_service = engine.get_task_service();
    task_service
        .set_task_variables_local(task_id.clone(), variables(&[("local", json!(1))]))
        .unwrap();
    task_service
        .set_task_variable(task_id.clone(), "global".to_string(), json!(2))
        .unwrap();

    engine
        .get_runtime_service()
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    // Java `RemoveTaskVariablesCmd#getSuspendedTaskExceptionPrefix`:
    // "Cannot remove variables from" — the task check fires for both scopes.
    let err = task_service
        .remove_task_variables_local(task_id.clone(), vec!["local".to_string()])
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::ExecutionError(message) if message == &format!("Cannot remove variables from a suspended task '{task_id}'")),
        "unexpected error: {err}"
    );

    let err = task_service
        .remove_task_variable(task_id.clone(), "global".to_string())
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::ExecutionError(message) if message == &format!("Cannot remove variables from a suspended task '{task_id}'")),
        "unexpected error: {err}"
    );

    // Nothing was removed.
    let locals = task_service
        .get_task_local_variables(task_id.clone())
        .unwrap();
    assert_eq!(locals.get("local"), Some(&json!(1)));
    let merged = task_service.get_task_variables(task_id).unwrap();
    assert_eq!(merged.get("global"), Some(&json!(2)));
}

#[test]
fn standalone_task_rejects_global_scope() {
    let engine = ProcessEngine::new("task-var-standalone-global".to_string());
    insert_standalone_task(&engine, "standalone-task-1");
    let task_service = engine.get_task_service();

    let err = task_service
        .set_task_variable("standalone-task-1".to_string(), "a".to_string(), json!(1))
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::BadRequest(message) if message == "Cannot set global variables on task 'standalone-task-1', task is not part of process."),
        "unexpected error: {err}"
    );

    let err = task_service
        .remove_task_variable("standalone-task-1".to_string(), "a".to_string())
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::BadRequest(message) if message == "Cannot remove global variables on task 'standalone-task-1', task is not part of process."),
        "unexpected error: {err}"
    );

    // Local scope and resolved reads still work on a standalone task.
    task_service
        .set_task_variables_local(
            "standalone-task-1".to_string(),
            variables(&[("l", json!(1))]),
        )
        .unwrap();
    assert_eq!(
        task_service
            .get_task_variable("standalone-task-1".to_string(), "l".to_string())
            .unwrap(),
        Some(json!(1))
    );
    let merged = task_service
        .get_task_variables("standalone-task-1".to_string())
        .unwrap();
    assert_eq!(merged, variables(&[("l", json!(1))]));
}

#[test]
fn create_only_conflict_aborts_whole_batch() {
    let engine = ProcessEngine::new("task-var-create-conflict".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "createConflictProcess");
    let task_service = engine.get_task_service();

    task_service
        .set_task_variables_local(task_id.clone(), variables(&[("existing", json!("old"))]))
        .unwrap();

    let err = task_service
        .create_task_variables(
            task_id.clone(),
            TaskVariableScope::Local,
            variables(&[("new1", json!(1)), ("existing", json!("new"))]),
        )
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::Conflict(message) if message == &format!("Variable 'existing' is already present on task '{}'.", task_id)),
        "unexpected error: {err}"
    );

    // Re-read in a fresh transaction: none of the batch may be persisted.
    let locals = task_service.get_task_local_variables(task_id).unwrap();
    assert_eq!(locals.get("existing"), Some(&json!("old")));
    assert!(!locals.contains_key("new1"), "new1 must not be written");
}

#[test]
fn update_only_missing_variable_aborts_whole_batch() {
    let engine = ProcessEngine::new("task-var-update-missing".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "updateMissingProcess");
    let task_service = engine.get_task_service();

    task_service
        .set_task_variables_local(task_id.clone(), variables(&[("present", json!("old"))]))
        .unwrap();

    // Batch update-only: one variable is missing on the scope, so the whole
    // batch must fail with NotFound before anything is written.
    let cmd = MutateTaskVariablesCmd::new(
        task_id.clone(),
        TaskVariableScope::Local,
        VariableMutationMode::UpdateOnly,
        vec![
            TaskVariableMutation {
                name: "present".to_string(),
                value: json!("updated"),
            },
            TaskVariableMutation {
                name: "missing".to_string(),
                value: json!("nope"),
            },
        ],
    );
    let err = engine.get_command_executor().execute(&cmd).unwrap_err();
    assert!(
        matches!(&err, FlowableError::NotFound(message) if message.contains("'missing'")),
        "unexpected error: {err}"
    );

    let locals = task_service.get_task_local_variables(task_id).unwrap();
    assert_eq!(
        locals.get("present"),
        Some(&json!("old")),
        "present must stay untouched"
    );
    assert!(!locals.contains_key("missing"));
}

#[test]
fn invalid_name_aborts_whole_batch() {
    let engine = ProcessEngine::new("task-var-invalid-name".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "invalidNameProcess");
    let task_service = engine.get_task_service();

    let err = task_service
        .create_task_variables(
            task_id.clone(),
            TaskVariableScope::Local,
            variables(&[("good", json!(1)), ("", json!(2))]),
        )
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::BadRequest(message) if message == "Variable name is required"),
        "unexpected error: {err}"
    );

    // Rollback proof: re-read after the error shows the valid mutation was
    // not persisted either.
    let locals = task_service.get_task_local_variables(task_id).unwrap();
    assert!(locals.is_empty(), "nothing may be persisted: {locals:?}");
}

#[test]
fn local_variable_shadows_global_on_read() {
    let engine = ProcessEngine::new("task-var-shadow".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "shadowProcess");
    let task_service = engine.get_task_service();

    task_service
        .set_task_variable(task_id.clone(), "shared".to_string(), json!("global"))
        .unwrap();
    task_service
        .set_task_variable(task_id.clone(), "globalOnly".to_string(), json!("g"))
        .unwrap();
    task_service
        .set_task_variables_local(
            task_id.clone(),
            variables(&[("shared", json!("local")), ("localOnly", json!("l"))]),
        )
        .unwrap();

    assert_eq!(
        task_service
            .get_task_variable(task_id.clone(), "shared".to_string())
            .unwrap(),
        Some(json!("local")),
        "local value shadows the global one"
    );
    assert_eq!(
        task_service
            .get_task_variable(task_id.clone(), "globalOnly".to_string())
            .unwrap(),
        Some(json!("g")),
        "read falls back to the execution scope"
    );

    let merged = task_service.get_task_variables(task_id).unwrap();
    assert_eq!(merged.get("shared"), Some(&json!("local")));
    assert_eq!(merged.get("globalOnly"), Some(&json!("g")));
    assert_eq!(merged.get("localOnly"), Some(&json!("l")));
}

#[test]
fn read_falls_back_to_global_after_local_removal() {
    let engine = ProcessEngine::new("task-var-fallback".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "fallbackProcess");
    let task_service = engine.get_task_service();

    task_service
        .set_task_variable(task_id.clone(), "shared".to_string(), json!("global"))
        .unwrap();
    task_service
        .set_task_variables_local(task_id.clone(), variables(&[("shared", json!("local"))]))
        .unwrap();

    task_service
        .remove_task_variable_on_scope(
            task_id.clone(),
            TaskVariableScope::Local,
            "shared".to_string(),
        )
        .unwrap();

    assert_eq!(
        task_service
            .get_task_variable(task_id.clone(), "shared".to_string())
            .unwrap(),
        Some(json!("global")),
        "after removing the local value the global one is visible again"
    );
}

#[test]
fn remove_all_local_leaves_global_variables_untouched() {
    let engine = ProcessEngine::new("task-var-remove-all-local".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "removeAllLocalProcess");
    let task_service = engine.get_task_service();

    task_service
        .set_task_variables(
            task_id.clone(),
            variables(&[("g1", json!("g1")), ("g2", json!("g2"))]),
        )
        .unwrap();
    task_service
        .set_task_variables_local(task_id.clone(), variables(&[("l1", json!("l1"))]))
        .unwrap();

    task_service
        .remove_all_task_local_variables(task_id.clone())
        .unwrap();

    assert!(
        task_service
            .get_task_local_variables(task_id.clone())
            .unwrap()
            .is_empty(),
        "all local variables are removed"
    );
    let merged = task_service.get_task_variables(task_id).unwrap();
    assert_eq!(merged.get("g1"), Some(&json!("g1")));
    assert_eq!(merged.get("g2"), Some(&json!("g2")));
}

#[test]
fn history_create_update_delete_lifecycle() {
    let engine = ProcessEngine::new("task-var-history".to_string());
    let (pi_id, task_id) = deploy_and_start(&engine, "historyProcess");
    let task_service = engine.get_task_service();
    let history_service = engine.get_history_service();

    let task = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap()
        .pop()
        .unwrap();

    // Global create: historic row on the OWNING execution (root process
    // instance), without a task id, mirroring SetVariableCmd.
    task_service
        .set_task_variable(task_id.clone(), "hvar".to_string(), json!(1))
        .unwrap();
    let historic = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(pi_id.clone())
        .variable_name("hvar".to_string())
        .list()
        .unwrap();
    assert_eq!(historic.len(), 1);
    assert_eq!(historic[0].value(), &json!(1));
    assert_eq!(historic[0].process_instance_id, pi_id);
    assert_eq!(
        historic[0].execution_id.as_deref(),
        Some(pi_id.as_str()),
        "new global variables land on the root process-instance execution"
    );
    assert_eq!(historic[0].task_id, None);

    // Global update: same historic row, refreshed value.
    task_service
        .set_task_variable(task_id.clone(), "hvar".to_string(), json!(2))
        .unwrap();
    let historic = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(pi_id.clone())
        .variable_name("hvar".to_string())
        .list()
        .unwrap();
    assert_eq!(historic.len(), 1);
    assert_eq!(historic[0].value(), &json!(2));

    // Local create: historic row carries the task id and the task's execution
    // id, mirroring record_task_local_variable.
    task_service
        .set_task_variables_local(task_id.clone(), variables(&[("lvar", json!("x"))]))
        .unwrap();
    let historic = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(pi_id.clone())
        .variable_name("lvar".to_string())
        .list()
        .unwrap();
    assert_eq!(historic.len(), 1);
    assert_eq!(historic[0].task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(
        historic[0].execution_id.as_deref(),
        Some(task.execution_id.as_str())
    );
    assert_eq!(historic[0].process_instance_id, pi_id);

    // Deletes remove the historic rows.
    task_service
        .remove_task_variable_on_scope(
            task_id.clone(),
            TaskVariableScope::Global,
            "hvar".to_string(),
        )
        .unwrap();
    let historic = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(pi_id.clone())
        .variable_name("hvar".to_string())
        .list()
        .unwrap();
    assert!(
        historic.is_empty(),
        "deleted global variable leaves no historic row"
    );

    task_service
        .delete_task_local_variable(task_id.clone(), "lvar".to_string())
        .unwrap();
    let historic = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(pi_id.clone())
        .variable_name("lvar".to_string())
        .list()
        .unwrap();
    assert!(
        historic.is_empty(),
        "deleted local variable leaves no historic row"
    );
}

#[test]
fn duplicate_create_only_second_writer_conflicts() {
    let engine = ProcessEngine::new("task-var-duplicate-create".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "duplicateCreateProcess");
    let task_service = engine.get_task_service();

    task_service
        .create_task_variables(
            task_id.clone(),
            TaskVariableScope::Local,
            variables(&[("dup", json!("first"))]),
        )
        .unwrap();

    let err = task_service
        .create_task_variables(
            task_id.clone(),
            TaskVariableScope::Local,
            variables(&[("dup", json!("second"))]),
        )
        .unwrap_err();
    assert!(
        matches!(&err, FlowableError::Conflict(message) if message == &format!("Variable 'dup' is already present on task '{}'.", task_id)),
        "unexpected error: {err}"
    );

    assert_eq!(
        task_service
            .get_task_local_variable(task_id, "dup".to_string())
            .unwrap(),
        Some(json!("first")),
        "the first writer's value wins"
    );
}
