//! P90a — async history assignee/owner historic identity-link replay.
//!
//! Sync path (P86a) already accumulates assignee/owner historic ILs in
//! `history_manager.record_task_created` / `record_task_updated`. Async mode
//! buffers TaskCreated/TaskUpdated and returns early, so those rows were
//! missing until replay (`async_history_job_handler`) emits them.
//!
//! Java OSS 8.0 has no AsyncHistoryManager — parity target is end-state after
//! replay matching the sync path (`HistoricTaskServiceImpl.recordTaskInfoChange:142-152`
//! → `createHistoricIdentityLink:265-273`).
//!
//! Test pattern mirrors `async_history_test.rs` (collect history jobs in command
//! order, then `execute_history_job`). Mutations are issued **before** their
//! create/update jobs are replayed so `insert_task`'s silent historic projection
//! sync cannot pre-apply assignee and collapse TaskUpdated diffs (see
//! `runtime_store.rs` historic update inside `insert_task`).

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::history::historic_entities::HistoricIdentityLink;
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::collections::HashSet;
use std::sync::Arc;

fn user_task_xml(process_id: &str, assignee: Option<&str>, owner: Option<&str>) -> String {
    let assignee_attr = assignee
        .map(|a| format!(r#" flowable:assignee="{a}""#))
        .unwrap_or_default();
    let owner_attr = owner
        .map(|o| format!(r#" flowable:owner="{o}""#))
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="{process_id}" isExecutable="true">
    <startEvent id="start"/>
    <sequenceFlow id="f1" sourceRef="start" targetRef="task1"/>
    <userTask id="task1" name="Task"{assignee_attr}{owner_attr}/>
    <sequenceFlow id="f2" sourceRef="task1" targetRef="end"/>
    <endEvent id="end"/>
  </process>
</definitions>"#
    )
}

fn async_engine(name: &str) -> ProcessEngine {
    let now = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let mut config = ProcessEngineConfiguration::default();
    config.async_history.enabled = true;
    ProcessEngine::build_with_config(name.to_string(), time_source, config).unwrap()
}

fn deploy_and_start(
    engine: &ProcessEngine,
    key: &str,
    assignee: Option<&str>,
    owner: Option<&str>,
) -> (String, String) {
    let xml = user_task_xml(key, assignee, owner);
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(format!("{key}-dep"))
                .add_string(format!("{key}.bpmn20.xml"), xml),
        )
        .expect("deploy");
    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    (pi.id, tasks[0].id.clone())
}

fn history_job_ids(engine: &ProcessEngine) -> HashSet<String> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let ids = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|j| j.job_state.as_deref() == Some("history"))
        .map(|j| j.timer_job_id)
        .collect();
    session.rollback().unwrap();
    ids
}

fn take_new_history_job(
    engine: &ProcessEngine,
    before: &HashSet<String>,
) -> (HashSet<String>, String) {
    let after = history_job_ids(engine);
    let new: Vec<_> = after.difference(before).cloned().collect();
    assert_eq!(
        new.len(),
        1,
        "expected exactly one new history job, got {new:?} (before={before:?}, after={after:?})"
    );
    (after, new[0].clone())
}

fn execute_history_job(engine: &ProcessEngine, job_id: &str) {
    engine
        .get_management_service()
        .execute_history_job(job_id)
        .unwrap();
}

fn historic_for_task(engine: &ProcessEngine, task_id: &str) -> Vec<HistoricIdentityLink> {
    engine
        .get_history_service()
        .get_historic_identity_links_for_task(task_id)
        .unwrap()
}

fn assignee_rows(links: &[HistoricIdentityLink]) -> Vec<&HistoricIdentityLink> {
    links.iter().filter(|l| l.link_type == "assignee").collect()
}

/// ① async BPMN initial assignee → after replay exactly one historic IL.
#[test]
fn async_initial_assignee_replay_writes_one_historic_il() {
    let engine = async_engine("p90a-async-initial-assignee");
    let (_pi_id, task_id) = deploy_and_start(&engine, "p90aInitAsync", Some("kermit"), None);

    // Pre-replay: no historic IL (async buffers only).
    assert!(
        historic_for_task(&engine, &task_id).is_empty(),
        "async mode must not write historic IL before replay"
    );

    let start_jobs = history_job_ids(&engine);
    assert_eq!(start_jobs.len(), 1);
    execute_history_job(&engine, start_jobs.iter().next().unwrap());

    let historic = historic_for_task(&engine, &task_id);
    let assignees = assignee_rows(&historic);
    assert_eq!(
        assignees.len(),
        1,
        "initial assignee after replay must be exactly one IL: {historic:?}"
    );
    assert_eq!(assignees[0].user_id.as_deref(), Some("kermit"));
    assert!(assignees[0].process_instance_id.is_none());
    assert!(assignees[0].group_id.is_none());
    assert_eq!(assignees[0].task_id.as_deref(), Some(task_id.as_str()));
}

/// ② two setAssignee + ordered replay accumulate two distinct assignee ILs.
///
/// Spec: after each setAssignee's history job is replayed, the trail grows
/// (1 then 2). Jobs are collected in command order and replayed create→update1
/// →update2 so TaskUpdated diffs see the pre-update historic row.
#[test]
fn async_set_assignee_twice_accumulates_two_ils_after_each_replay() {
    let engine = async_engine("p90a-async-set-assignee-twice");
    let (_pi_id, task_id) = deploy_and_start(&engine, "p90aSetAsync", None, None);

    let mut pending = history_job_ids(&engine);
    assert_eq!(pending.len(), 1);
    let start_job = pending.iter().next().unwrap().clone();

    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("alice".to_string()),
            None,
            "assignee".to_string(),
        )
        .unwrap();
    let (after1, job1) = take_new_history_job(&engine, &pending);
    pending = after1;

    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("bob".to_string()),
            None,
            "assignee".to_string(),
        )
        .unwrap();
    let (_after2, job2) = take_new_history_job(&engine, &pending);

    // Replay create: unassigned → no assignee IL.
    execute_history_job(&engine, &start_job);
    assert!(
        assignee_rows(&historic_for_task(&engine, &task_id)).is_empty(),
        "create of unassigned task must not write assignee IL"
    );

    // Replay first setAssignee.
    execute_history_job(&engine, &job1);
    let after_first = historic_for_task(&engine, &task_id);
    let a1 = assignee_rows(&after_first);
    assert_eq!(a1.len(), 1, "first setAssignee replay: {after_first:?}");
    assert_eq!(a1[0].user_id.as_deref(), Some("alice"));
    let first_id = a1[0].id.clone();

    // Replay second setAssignee.
    execute_history_job(&engine, &job2);
    let after_second = historic_for_task(&engine, &task_id);
    let a2 = assignee_rows(&after_second);
    assert_eq!(
        a2.len(),
        2,
        "two setAssignee replays must accumulate two rows: {after_second:?}"
    );
    let users: Vec<_> = a2.iter().map(|l| l.user_id.as_deref()).collect();
    assert!(users.contains(&Some("alice")));
    assert!(users.contains(&Some("bob")));
    assert!(a2.iter().any(|l| l.id == first_id));
    assert_ne!(a2[0].id, a2[1].id);
}

/// ③ unclaim produces a null-userId assignee historic IL row.
#[test]
fn async_unclaim_replay_writes_null_user_id_assignee_il() {
    let engine = async_engine("p90a-async-unclaim");
    let (_pi_id, task_id) = deploy_and_start(&engine, "p90aUnclaimAsync", Some("kermit"), None);

    let pending = history_job_ids(&engine);
    assert_eq!(pending.len(), 1);
    let start_job = pending.iter().next().unwrap().clone();

    engine
        .get_task_service()
        .unclaim_task_by_id(task_id.clone())
        .unwrap();
    let (_after, unclaim_job) = take_new_history_job(&engine, &pending);

    execute_history_job(&engine, &start_job);
    let after_create = historic_for_task(&engine, &task_id);
    assert_eq!(
        assignee_rows(&after_create).len(),
        1,
        "initial assignee IL after create replay: {after_create:?}"
    );

    execute_history_job(&engine, &unclaim_job);

    let historic = historic_for_task(&engine, &task_id);
    let assignees = assignee_rows(&historic);
    // initial kermit + unclaim null
    assert_eq!(
        assignees.len(),
        2,
        "initial + unclaim must yield two assignee rows: {historic:?}"
    );
    let null_rows: Vec<_> = assignees
        .iter()
        .filter(|l| l.user_id.is_none())
        .collect();
    assert_eq!(
        null_rows.len(),
        1,
        "unclaim must produce exactly one null userId row: {assignees:?}"
    );
    assert!(assignees.iter().any(|l| l.user_id.as_deref() == Some("kermit")));
}

/// ④ TaskUpdated IL is diff-triggered: replaying the same update after the
/// historic row already matches must not insert again (job failure full-batch
/// retry safety — see handle_failure / D5).
#[test]
fn async_task_updated_retry_does_not_duplicate_il() {
    let engine = async_engine("p90a-async-retry-idempotent");
    let (_pi_id, task_id) = deploy_and_start(&engine, "p90aRetryAsync", None, None);

    let pending = history_job_ids(&engine);
    assert_eq!(pending.len(), 1);
    let start_job = pending.iter().next().unwrap().clone();

    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("alice".to_string()),
            None,
            "assignee".to_string(),
        )
        .unwrap();
    let (_after, job_id) = take_new_history_job(&engine, &pending);

    // Capture payload before first successful execute deletes the job.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let job = store
        .find_timer_job_state(&job_id, &mut session)
        .expect("history job");
    let payload = job.time_duration.clone().expect("history job payload");
    session.rollback().unwrap();

    // Ordered first pass: create then update → one alice IL.
    execute_history_job(&engine, &start_job);
    execute_history_job(&engine, &job_id);
    let after_first = historic_for_task(&engine, &task_id);
    let first_count = assignee_rows(&after_first).len();
    assert_eq!(first_count, 1, "first update replay: {after_first:?}");
    let first_ids: HashSet<_> = assignee_rows(&after_first)
        .iter()
        .map(|l| l.id.clone())
        .collect();

    // Simulate full-batch retry: re-insert an identical history job and execute.
    // Historic assignee already matches → diff=false → no second IL insert.
    let retry_job_id = uuid::Uuid::new_v4().to_string();
    let now_ms = Utc
        .with_ymd_and_hms(2026, 8, 1, 12, 0, 0)
        .unwrap()
        .timestamp_millis();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: retry_job_id.clone(),
            process_instance_id: String::new(),
            execution_id: String::new(),
            activity_id: "async-history".to_string(),
            job_state: Some("history".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: Some(payload),
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(now_ms),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(3),
            error_message: None,
            error_details: None,
            category: None,
            handler_type: Some("async-history".to_string()),
            advanced_job_handler_configuration: None,
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    execute_history_job(&engine, &retry_job_id);

    let after_retry = historic_for_task(&engine, &task_id);
    let assignees = assignee_rows(&after_retry);
    assert_eq!(
        assignees.len(),
        first_count,
        "retry of same TaskUpdated must not duplicate IL: {after_retry:?}"
    );
    let retry_ids: HashSet<_> = assignees.iter().map(|l| l.id.clone()).collect();
    assert_eq!(retry_ids, first_ids);
}
