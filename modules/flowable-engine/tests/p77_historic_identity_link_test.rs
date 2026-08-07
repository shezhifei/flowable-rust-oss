//! P77 — Historic identity link independent storage (`ACT_HI_IDENTITYLINK` /
//! `historic_identity_links`).
//!
//! Java parity notes (re-confirmed 2026-08-01):
//! - Create: `DefaultHistoryManager.recordIdentityLinkCreated:396-410` inserts
//!   a historic row with the **same id** when history ≥ AUDIT and the link has
//!   a taskId or processInstanceId.
//! - Delete: `recordIdentityLinkDeleted:414-417` **deletes** the historic row
//!   by id (not an append-only audit trail for participant/candidate links).
//! - Cascade: delete historic PI/task removes historic ILs by procInst/task.
//! - `involvedUser` on historic PI query reads ACT_HI_IDENTITYLINK.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::IdentityLink;
use flowable_engine::service::config::{HistoryLevel, ProcessEngineConfiguration};

fn simple_user_task_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
  <process id="{process_id}" isExecutable="true">
    <startEvent id="start"/>
    <sequenceFlow id="f1" sourceRef="start" targetRef="task1"/>
    <userTask id="task1" name="Task"/>
    <sequenceFlow id="f2" sourceRef="task1" targetRef="end"/>
    <endEvent id="end"/>
  </process>
</definitions>"#
    )
}

fn deploy_and_start(engine: &ProcessEngine, key: &str) -> (String, String) {
    let xml = simple_user_task_xml(key);
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

#[test]
fn participant_create_writes_historic_snapshot_delete_removes_historic_row() {
    // Java: create inserts historic with same id; delete removes historic row
    // (DefaultHistoryManager:396-417) — not an append-only snapshot.
    let engine = ProcessEngine::new("p77-participant-mirror".to_string());
    let (pi_id, _task_id) = deploy_and_start(&engine, "p77Participant");

    let link = IdentityLink {
        id: "hil-participant-1".to_string(),
        link_type: "participant".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some(pi_id.clone()),
        process_definition_id: None,
    };
    engine.get_identity_link_service().add_identity_link(link);

    let historic = engine
        .get_history_service()
        .get_historic_identity_links_for_process_instance(&pi_id)
        .unwrap();
    assert!(
        historic
            .iter()
            .any(|l| l.user_id.as_deref() == Some("kermit") && l.link_type == "participant"),
        "historic should contain participant after add: {historic:?}"
    );
    assert!(
        historic
            .iter()
            .any(|l| l.id == "hil-participant-1" && l.create_time.is_some()),
        "historic row should reuse runtime id and set create_time"
    );

    engine
        .get_identity_link_service()
        .remove_identity_link("hil-participant-1");

    let historic_after = engine
        .get_history_service()
        .get_historic_identity_links_for_process_instance(&pi_id)
        .unwrap();
    assert!(
        !historic_after
            .iter()
            .any(|l| l.id == "hil-participant-1"),
        "Java deletes historic IL on runtime delete; got {historic_after:?}"
    );
}

#[test]
fn task_candidate_create_and_delete_mirror_historic_table() {
    let engine = ProcessEngine::new("p77-task-candidate".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "p77Candidate");

    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("fozzie".to_string()),
            None,
            "candidate".to_string(),
        )
        .unwrap();

    let historic = engine
        .get_history_service()
        .get_historic_identity_links_for_task(&task_id)
        .unwrap();
    assert_eq!(historic.len(), 1);
    assert_eq!(historic[0].user_id.as_deref(), Some("fozzie"));
    assert_eq!(historic[0].link_type, "candidate");

    engine
        .get_task_service()
        .delete_identity_link(
            task_id.clone(),
            Some("fozzie".to_string()),
            None,
            "candidate".to_string(),
        )
        .unwrap();

    let historic_after = engine
        .get_history_service()
        .get_historic_identity_links_for_task(&task_id)
        .unwrap();
    assert!(historic_after.is_empty());
}

#[test]
fn audit_gate_skips_historic_identity_link_when_history_none() {
    // Java isHistoryEnabledForIdentityLink requires AUDIT+
    // (DefaultHistoryConfigurationSettings:291-294).
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = HistoryLevel::None;
    let engine = ProcessEngine::new_with_config("p77-history-none".to_string(), config);

    // With history None, process start may not create historic PI; still test
    // identity-link service path with an explicit PI id.
    let link = IdentityLink {
        id: "hil-none-1".to_string(),
        link_type: "participant".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some("pi-none".to_string()),
        process_definition_id: None,
    };
    engine.get_identity_link_service().add_identity_link(link);

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let historic = engine
        .get_runtime_store()
        .find_historic_identity_link("hil-none-1", &mut session);
    assert!(
        historic.is_none(),
        "history=None must not write historic identity links"
    );
}

#[test]
fn process_definition_only_links_are_not_historicized() {
    // Java DefaultHistoryManager:397-400 skips links without task/processInstance.
    let engine = ProcessEngine::new("p77-procdef-only".to_string());
    let link = IdentityLink {
        id: "hil-pd-1".to_string(),
        link_type: "candidate".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: None,
        process_definition_id: Some("pd-1".to_string()),
    };
    engine.get_identity_link_service().add_identity_link(link);

    let mut session = engine.get_runtime_store().create_session().unwrap();
    assert!(
        engine
            .get_runtime_store()
            .find_historic_identity_link("hil-pd-1", &mut session)
            .is_none()
    );
    // Runtime row still exists.
    assert!(
        engine
            .get_runtime_store()
            .find_identity_link("hil-pd-1", &mut session)
            .is_some()
    );
}

#[test]
fn cascade_delete_historic_process_instance_removes_historic_identity_links() {
    let engine = ProcessEngine::new("p77-cascade-pi".to_string());
    let (pi_id, _task_id) = deploy_and_start(&engine, "p77CascadePi");

    engine.get_identity_link_service().add_identity_link(IdentityLink {
        id: "hil-cascade-1".to_string(),
        link_type: "participant".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some(pi_id.clone()),
        process_definition_id: None,
    });

    assert_eq!(
        engine
            .get_history_service()
            .get_historic_identity_links_for_process_instance(&pi_id)
            .unwrap()
            .iter()
            .filter(|l| l.id == "hil-cascade-1")
            .count(),
        1
    );

    engine
        .get_history_service()
        .delete_historic_process_instance(pi_id.clone())
        .unwrap();

    let remaining = engine
        .get_history_service()
        .get_historic_identity_links_for_process_instance(&pi_id)
        .unwrap();
    assert!(
        remaining.is_empty(),
        "cascade should remove historic ILs: {remaining:?}"
    );
}

#[test]
fn cascade_delete_historic_task_removes_task_historic_identity_links() {
    let engine = ProcessEngine::new("p77-cascade-task".to_string());
    let (_pi_id, task_id) = deploy_and_start(&engine, "p77CascadeTask");

    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("kermit".to_string()),
            None,
            "candidate".to_string(),
        )
        .unwrap();
    assert_eq!(
        engine
            .get_history_service()
            .get_historic_identity_links_for_task(&task_id)
            .unwrap()
            .len(),
        1
    );

    engine
        .get_history_service()
        .delete_historic_task_instance(task_id.clone())
        .unwrap();

    assert!(
        engine
            .get_history_service()
            .get_historic_identity_links_for_task(&task_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn involved_user_historic_query_uses_historic_identity_links() {
    let engine = ProcessEngine::new("p77-involved-user".to_string());
    let (pi_id, _task_id) = deploy_and_start(&engine, "p77Involved");

    engine.get_identity_link_service().add_identity_link(IdentityLink {
        id: "hil-involved-1".to_string(),
        link_type: "participant".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some(pi_id.clone()),
        process_definition_id: None,
    });

    let found = engine
        .get_history_service()
        .create_historic_process_instance_query()
        .involved_user("kermit".to_string())
        .list()
        .unwrap();
    assert!(
        found.iter().any(|pi| pi.id == pi_id),
        "involvedUser should match via historic_identity_links"
    );

    // After deleting the link, historic row is gone → no match (Java parity).
    engine
        .get_identity_link_service()
        .remove_identity_link("hil-involved-1");
    let after = engine
        .get_history_service()
        .create_historic_process_instance_query()
        .involved_user("kermit".to_string())
        .list()
        .unwrap();
    assert!(!after.iter().any(|pi| pi.id == pi_id));
}

#[test]
fn historic_identity_link_query_filters_by_task_and_process() {
    let engine = ProcessEngine::new("p77-query-dims".to_string());
    let (pi_id, task_id) = deploy_and_start(&engine, "p77QueryDims");

    engine
        .get_task_service()
        .add_identity_link(
            task_id.clone(),
            Some("kermit".to_string()),
            None,
            "candidate".to_string(),
        )
        .unwrap();
    engine.get_identity_link_service().add_identity_link(IdentityLink {
        id: "hil-pi-only".to_string(),
        link_type: "participant".to_string(),
        user_id: Some("gonzo".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some(pi_id.clone()),
        process_definition_id: None,
    });

    let by_task = engine
        .get_history_service()
        .create_historic_identity_link_query()
        .task_id(task_id)
        .list()
        .unwrap();
    assert_eq!(by_task.len(), 1);
    assert_eq!(by_task[0].user_id.as_deref(), Some("kermit"));

    let by_pi = engine
        .get_history_service()
        .create_historic_identity_link_query()
        .process_instance_id(pi_id)
        .list()
        .unwrap();
    assert!(by_pi.iter().any(|l| l.user_id.as_deref() == Some("gonzo")));
    assert!(by_pi.iter().any(|l| l.user_id.as_deref() == Some("kermit")));
}
