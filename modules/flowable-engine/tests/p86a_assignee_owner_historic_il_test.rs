//! P86a — assignee/owner historic identity-link accumulation.
//!
//! Java parity (`HistoricTaskServiceImpl.recordTaskInfoChange:142-152` +
//! `createHistoricIdentityLink:265-273`):
//! - Every assignee/owner *change* inserts a **new** historic row (new id);
//!   previous rows are never deleted — accumulate, not mirror.
//! - Rows carry only taskId / type / userId / createTime (no processInstanceId).
//! - Initial BPMN assignee/owner also produce a row (Java reaches this via
//!   post-insert `changeTaskAssignee`/`changeTaskOwner`; Rust emits from
//!   `record_task_created` when the runtime task already carries the value).
//! - Runtime `identity_links` is **not** written for assignee/owner (P42).
//! - Cascade: historic PI delete removes per-task assignee/owner rows; historic
//!   task delete does the same via task_id.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::task_service::TaskUpdate;
use flowable_engine::history::historic_entities::HistoricIdentityLink;

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

fn historic_for_task(engine: &ProcessEngine, task_id: &str) -> Vec<HistoricIdentityLink> {
    engine
        .get_history_service()
        .get_historic_identity_links_for_task(task_id)
        .unwrap()
}

fn assignee_rows(links: &[HistoricIdentityLink]) -> Vec<&HistoricIdentityLink> {
    links.iter().filter(|l| l.link_type == "assignee").collect()
}

fn owner_rows(links: &[HistoricIdentityLink]) -> Vec<&HistoricIdentityLink> {
    links.iter().filter(|l| l.link_type == "owner").collect()
}

#[test]
fn initial_assignee_writes_one_historic_identity_link_no_runtime_row() {
    // BPMN flowable:assignee on create → one accumulating historic assignee row.
    // Runtime identity_links stays empty (P42 assignee path).
    let engine = ProcessEngine::new("p86a-initial-assignee".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "p86aInitAssignee", Some("kermit"), None);

    let historic = historic_for_task(&engine, &task_id);
    let assignees = assignee_rows(&historic);
    assert_eq!(
        assignees.len(),
        1,
        "initial assignee must produce exactly one historic IL: {historic:?}"
    );
    assert_eq!(assignees[0].user_id.as_deref(), Some("kermit"));
    assert!(assignees[0].process_instance_id.is_none());
    assert!(assignees[0].group_id.is_none());
    assert!(assignees[0].create_time.is_some());
    assert_eq!(assignees[0].task_id.as_deref(), Some(task_id.as_str()));

    let runtime = engine
        .get_task_service()
        .get_identity_links_for_task(task_id)
        .unwrap();
    assert!(
        runtime.is_empty(),
        "assignee must not write runtime identity_links: {runtime:?}"
    );
}

#[test]
fn initial_owner_writes_one_historic_identity_link() {
    let engine = ProcessEngine::new("p86a-initial-owner".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "p86aInitOwner", None, Some("fozzie"));

    let historic = historic_for_task(&engine, &task_id);
    let owners = owner_rows(&historic);
    assert_eq!(
        owners.len(),
        1,
        "initial owner must produce exactly one historic IL: {historic:?}"
    );
    assert_eq!(owners[0].user_id.as_deref(), Some("fozzie"));
    assert!(owners[0].process_instance_id.is_none());
}

#[test]
fn set_assignee_twice_accumulates_two_historic_rows() {
    // Start unassigned, then two successive assignee changes → two rows.
    let engine = ProcessEngine::new("p86a-assignee-accumulate".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "p86aAssigneeAcc", None, None);

    assert!(assignee_rows(&historic_for_task(&engine, &task_id)).is_empty());

    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("alice".to_string()),
            None,
            "assignee".to_string(),
        )
        .unwrap();
    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("bob".to_string()),
            None,
            "assignee".to_string(),
        )
        .unwrap();

    let historic = historic_for_task(&engine, &task_id);
    let assignees = assignee_rows(&historic);
    assert_eq!(
        assignees.len(),
        2,
        "two setAssignee calls must accumulate two rows: {assignees:?}"
    );
    let users: Vec<_> = assignees
        .iter()
        .map(|l| l.user_id.as_deref())
        .collect();
    assert!(users.contains(&Some("alice")));
    assert!(users.contains(&Some("bob")));
    // Distinct ids (not mirrored / not overwritten).
    assert_ne!(assignees[0].id, assignees[1].id);
}

#[test]
fn set_owner_twice_accumulates_two_historic_rows() {
    let engine = ProcessEngine::new("p86a-owner-accumulate".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "p86aOwnerAcc", None, None);

    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("owner1".to_string()),
            None,
            "owner".to_string(),
        )
        .unwrap();
    engine
        .get_task_service()
        .update_task_by_id(
            task_id.clone(),
            TaskUpdate {
                owner: Some(Some("owner2".to_string())),
                ..TaskUpdate::default()
            },
        )
        .unwrap();

    let historic = historic_for_task(&engine, &task_id);
    let owners = owner_rows(&historic);
    assert_eq!(
        owners.len(),
        2,
        "two owner changes must accumulate two rows: {owners:?}"
    );
    let users: Vec<_> = owners.iter().map(|l| l.user_id.as_deref()).collect();
    assert!(users.contains(&Some("owner1")));
    assert!(users.contains(&Some("owner2")));
}

#[test]
fn claim_appends_assignee_historic_identity_link() {
    let engine = ProcessEngine::new("p86a-claim".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "p86aClaim", None, None);

    engine
        .get_task_service()
        .claim_task_by_id(task_id.clone(), "claimer".to_string())
        .unwrap();

    let historic = historic_for_task(&engine, &task_id);
    let assignees = assignee_rows(&historic);
    assert_eq!(assignees.len(), 1);
    assert_eq!(assignees[0].user_id.as_deref(), Some("claimer"));
}

#[test]
fn historic_query_and_cascade_delete_cover_assignee_owner_rows() {
    // Initial assignee + owner, then reassign; PI cascade must wipe task-scoped rows
    // even though they carry no process_instance_id (P86a cascade fix).
    let engine = ProcessEngine::new("p86a-cascade".to_string());
    let (pi_id, task_id) =
        deploy_and_start(&engine, "p86aCascade", Some("kermit"), Some("fozzie"));

    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("gonzo".to_string()),
            None,
            "assignee".to_string(),
        )
        .unwrap();

    let before = historic_for_task(&engine, &task_id);
    assert!(
        assignee_rows(&before).len() >= 2,
        "expected accumulated assignee rows: {before:?}"
    );
    assert_eq!(owner_rows(&before).len(), 1);

    // Query path used by REST `/history/historic-task-instances/{id}/identitylinks`.
    let by_query = engine
        .get_history_service()
        .create_historic_identity_link_query()
        .task_id(task_id.clone())
        .list()
        .unwrap();
    assert_eq!(by_query.len(), before.len());

    // Complete + delete runtime, then cascade historic PI.
    engine
        .get_task_service()
        .complete_task_by_id(task_id.clone())
        .unwrap();
    engine
        .get_history_service()
        .delete_historic_process_instance(pi_id)
        .unwrap();

    let after = historic_for_task(&engine, &task_id);
    assert!(
        after.is_empty(),
        "PI cascade must remove task-scoped assignee/owner historic ILs: {after:?}"
    );
}

#[test]
fn cascade_delete_historic_task_removes_assignee_owner_rows() {
    let engine = ProcessEngine::new("p86a-cascade-task".to_string());
    let (_pi_id, task_id) =
        deploy_and_start(&engine, "p86aCascadeTask", Some("kermit"), Some("fozzie"));

    assert!(!historic_for_task(&engine, &task_id).is_empty());

    engine
        .get_history_service()
        .delete_historic_task_instance(task_id.clone())
        .unwrap();

    assert!(historic_for_task(&engine, &task_id).is_empty());
}

/// P90b pin — standalone (non-process) task with initial assignee does **not**
/// write a historic identity link.
///
/// Java parity (`TaskEntityManagerImpl.createTask(TaskBuilder):59-99`):
/// - assignee/owner are set on the entity at `:66-67`
/// - insert at `:77`
/// - only `recordTaskCreated` at `:96` — **no** `recordTaskInfoChange`
///
/// Same for `SaveTaskCmd.java:67-68` → `TaskHelper.insertTask:317-355`.
/// Standalone initial assignee is therefore intentionally without historic IL;
/// this pins that conclusion (do not "fix" by adding IL here).
#[test]
fn standalone_initial_assignee_does_not_write_historic_identity_link() {
    use flowable_engine::task::Task;

    let engine = ProcessEngine::new("p90b-standalone-initial-assignee".to_string());
    let mut task = Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "standalone".to_string(),
    );
    task.assignee = Some("kermit".to_string());

    let created = engine
        .get_task_service()
        .create_task(task)
        .expect("create standalone task");
    assert_eq!(created.assignee.as_deref(), Some("kermit"));

    let historic_ils = historic_for_task(&engine, &created.id);
    assert!(
        historic_ils.is_empty(),
        "standalone initial assignee must not write historic IL \
         (Java TaskEntityManagerImpl.createTask:66-67,77,96): {historic_ils:?}"
    );

    // Historic task row still carries the assignee copy from CreateTaskCmd.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let historic_task = store
        .get_historic_task_instance(&created.id, &mut session)
        .expect("standalone create must insert historic task");
    session.rollback().unwrap();
    assert_eq!(
        historic_task.assignee.as_deref(),
        Some("kermit"),
        "historic task assignee must be copied even without IL"
    );
}
