//! P28: delegate/resolve semantics + engine API stubs
//! (setDueDate/setPriority NeedsActiveTask, claim idempotency,
//! resolveTask(taskId, variables)).
//!
//! Java evidence:
//! - DelegateTaskCmd.java:37-40: PENDING + owner=assignee only when owner is
//!   unset; no fallback to the delegate target.
//! - ResolveTaskCmd.java:45-56: no delegation-state precondition; variables are
//!   applied (:46-48), then unconditional RESOLVED + assignee=owner (:53-54).
//! - ClaimTaskCmd.java:50-62: claim state set unconditionally; re-claim by the
//!   same user is idempotent (:54 comment, :62), another user conflicts.
//! - SetTaskDueDateCmd / SetTaskPriorityCmd extend NeedsActiveTaskCmd:
//!   suspended tasks are rejected.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::query::Query;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use serde_json::json;
use std::collections::HashMap;

fn deploy_and_start(engine: &ProcessEngine, id_suffix: &str) -> String {
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="process{id_suffix}">
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
            .add_string(format!("process{id_suffix}.bpmn20.xml"), xml),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap()
        .id
}

fn single_task_id(engine: &ProcessEngine, pi_id: &str) -> String {
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id.to_string())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    tasks[0].id.clone()
}

fn find_task(engine: &ProcessEngine, pi_id: &str, task_id: &str) -> flowable_engine::task::Task {
    engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id.to_string())
        .unwrap()
        .into_iter()
        .find(|t| t.id == task_id)
        .expect("task should still exist")
}

#[test]
fn delegate_without_owner_keeps_owner_unset() {
    // Java DelegateTaskCmd.java:37-40: owner=assignee — a never-assigned task
    // keeps a null owner (no fallback to the delegate target).
    let engine = ProcessEngine::new("p28-delegate-no-owner".to_string());
    let pi_id = deploy_and_start(&engine, "1");
    let task_id = single_task_id(&engine, &pi_id);

    engine
        .get_task_service()
        .delegate_task_by_id(task_id.clone(), "gonzo".to_string())
        .unwrap();

    let task = find_task(&engine, &pi_id, &task_id);
    assert_eq!(task.owner, None, "owner must stay unset (Java parity)");
    assert_eq!(task.assignee.as_deref(), Some("gonzo"));
    assert_eq!(task.delegation_state.as_deref(), Some("pending"));
}

#[test]
fn delegate_sets_owner_to_previous_assignee() {
    // Java DelegateTaskCmd.java:37-40 + TaskHelper.changeTaskAssignee.
    let engine = ProcessEngine::new("p28-delegate-owner-assignee".to_string());
    let pi_id = deploy_and_start(&engine, "1");
    let task_id = single_task_id(&engine, &pi_id);
    let task_service = engine.get_task_service();

    task_service
        .claim_task_by_id(task_id.clone(), "kermit".to_string())
        .unwrap();
    task_service
        .delegate_task_by_id(task_id.clone(), "gonzo".to_string())
        .unwrap();

    let task = find_task(&engine, &pi_id, &task_id);
    assert_eq!(task.owner.as_deref(), Some("kermit"));
    assert_eq!(task.assignee.as_deref(), Some("gonzo"));
    assert_eq!(task.delegation_state.as_deref(), Some("pending"));
}

#[test]
fn resolve_non_delegated_task_succeeds() {
    // Java ResolveTaskCmd.java:53-54: no precondition — resolving a task that
    // was never delegated silently marks it RESOLVED and sets assignee=owner.
    let engine = ProcessEngine::new("p28-resolve-non-delegated".to_string());
    let pi_id = deploy_and_start(&engine, "1");
    let task_id = single_task_id(&engine, &pi_id);
    let task_service = engine.get_task_service();

    task_service
        .claim_task_by_id(task_id.clone(), "kermit".to_string())
        .unwrap();
    task_service
        .resolve_task_by_id(task_id.clone())
        .expect("resolve on a non-delegated task must succeed (Java parity)");

    let task = find_task(&engine, &pi_id, &task_id);
    assert_eq!(task.delegation_state.as_deref(), Some("resolved"));
    // owner was never set, so assignee is reset to null (assignee=owner).
    assert_eq!(task.owner, None);
    assert_eq!(task.assignee, None);
}

#[test]
fn resolve_with_variables_applies_variables_and_returns_to_owner() {
    // Java ResolveTaskCmd.java:46-48 (variables applied) + :53-54.
    let engine = ProcessEngine::new("p28-resolve-with-vars".to_string());
    let pi_id = deploy_and_start(&engine, "1");
    let task_id = single_task_id(&engine, &pi_id);
    let task_service = engine.get_task_service();

    task_service
        .claim_task_by_id(task_id.clone(), "kermit".to_string())
        .unwrap();
    task_service
        .delegate_task_by_id(task_id.clone(), "gonzo".to_string())
        .unwrap();

    let mut variables = HashMap::new();
    variables.insert("reviewOutcome".to_string(), json!("approved"));
    task_service
        .resolve_task_by_id_with_variables(task_id.clone(), variables)
        .unwrap();

    let task = find_task(&engine, &pi_id, &task_id);
    assert_eq!(task.delegation_state.as_deref(), Some("resolved"));
    assert_eq!(task.assignee.as_deref(), Some("kermit"));
    assert_eq!(task.owner.as_deref(), Some("kermit"));
    assert_eq!(
        task_service
            .get_task_variable(task_id.clone(), "reviewOutcome".to_string())
            .unwrap(),
        Some(json!("approved")),
        "resolve variables must be applied to the execution scope"
    );
}

#[test]
fn claim_already_claimed_by_same_user_is_idempotent() {
    // Java ClaimTaskCmd.java:54,62: re-claim by the same user does not throw;
    // claim state (claimTime/state) is refreshed.
    let engine = ProcessEngine::new("p28-claim-idempotent".to_string());
    let pi_id = deploy_and_start(&engine, "1");
    let task_id = single_task_id(&engine, &pi_id);
    let task_service = engine.get_task_service();

    task_service
        .claim_task_by_id(task_id.clone(), "kermit".to_string())
        .unwrap();
    task_service
        .claim_task_by_id(task_id.clone(), "kermit".to_string())
        .expect("re-claim by the same user must be idempotent (Java parity)");

    let task = find_task(&engine, &pi_id, &task_id);
    assert_eq!(task.assignee.as_deref(), Some("kermit"));
    assert_eq!(task.state, "claimed");
    assert!(task.claim_time.is_some());
}

#[test]
fn claim_already_claimed_by_other_user_conflicts() {
    // Java ClaimTaskCmd.java:56-58: FlowableTaskAlreadyClaimedException.
    let engine = ProcessEngine::new("p28-claim-conflict".to_string());
    let pi_id = deploy_and_start(&engine, "1");
    let task_id = single_task_id(&engine, &pi_id);
    let task_service = engine.get_task_service();

    task_service
        .claim_task_by_id(task_id.clone(), "kermit".to_string())
        .unwrap();
    let err = task_service
        .claim_task_by_id(task_id.clone(), "gonzo".to_string())
        .expect_err("claim by another user must conflict");
    assert!(err.to_string().contains("already claimed"));
}

#[test]
fn set_task_due_date_and_priority_update_task_and_history() {
    // Java SetTaskDueDateCmd / SetTaskPriorityCmd (task + recordTaskInfoChange).
    let engine = ProcessEngine::new("p28-set-due-priority".to_string());
    let pi_id = deploy_and_start(&engine, "1");
    let task_id = single_task_id(&engine, &pi_id);
    let task_service = engine.get_task_service();

    let due = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
    task_service
        .set_task_due_date(task_id.clone(), Some(due))
        .unwrap();
    task_service.set_task_priority(task_id.clone(), 77).unwrap();

    let task = find_task(&engine, &pi_id, &task_id);
    assert_eq!(task.due_date, Some(due));
    assert_eq!(task.priority, Some(77));

    let historic = engine
        .get_history_service()
        .create_historic_task_instance_query()
        .process_instance_id(pi_id.clone())
        .list()
        .unwrap()
        .into_iter()
        .find(|h| h.id == task_id)
        .expect("historic task should exist");
    assert_eq!(historic.due_date, Some(due));
    assert_eq!(historic.priority, Some(77));
}

#[test]
fn set_due_date_and_priority_rejected_on_suspended_task() {
    // Java SetTaskDueDateCmd / SetTaskPriorityCmd extend NeedsActiveTaskCmd.
    let engine = ProcessEngine::new("p28-set-suspended".to_string());
    let pi_id = deploy_and_start(&engine, "1");
    let task_id = single_task_id(&engine, &pi_id);
    let task_service = engine.get_task_service();

    engine
        .get_runtime_service()
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let err = task_service
        .set_task_due_date(task_id.clone(), Some(Utc::now()))
        .expect_err("setDueDate on a suspended task must fail");
    assert!(err.to_string().contains("suspended task"));

    let err = task_service
        .set_task_priority(task_id.clone(), 10)
        .expect_err("setPriority on a suspended task must fail");
    assert!(err.to_string().contains("suspended task"));
}
