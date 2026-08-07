use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::IdentityLink;
use std::sync::Arc;

fn setup() -> Arc<ProcessEngine> {
    Arc::new(ProcessEngine::new(
        "process-instance-identity-link-parity".to_string(),
    ))
}

#[test]
fn process_instance_identity_links_can_be_added_and_queried() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link(IdentityLink {
        id: "il-1".to_string(),
        link_type: "participant".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some("proc-inst-1".to_string()),
        process_definition_id: None,
    });
    svc.add_identity_link(IdentityLink {
        id: "il-2".to_string(),
        link_type: "candidate".to_string(),
        user_id: None,
        group_id: Some("admins".to_string()),
        task_id: None,
        process_instance_id: Some("proc-inst-1".to_string()),
        process_definition_id: None,
    });
    svc.add_identity_link(IdentityLink {
        id: "il-3".to_string(),
        link_type: "participant".to_string(),
        user_id: Some("fozzie".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some("proc-inst-2".to_string()),
        process_definition_id: None,
    });

    let proc1_links = svc
        .create_identity_link_query()
        .process_instance_id("proc-inst-1".to_string())
        .list()
        .unwrap();
    assert_eq!(proc1_links.len(), 2);

    let proc2_links = svc
        .create_identity_link_query()
        .process_instance_id("proc-inst-2".to_string())
        .list()
        .unwrap();
    assert_eq!(proc2_links.len(), 1);
    assert_eq!(proc2_links[0].user_id.as_deref(), Some("fozzie"));
}

#[test]
fn process_instance_identity_links_support_user_and_group_filtering() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link(IdentityLink {
        id: "il-1".to_string(),
        link_type: "participant".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some("proc-1".to_string()),
        process_definition_id: None,
    });
    svc.add_identity_link(IdentityLink {
        id: "il-2".to_string(),
        link_type: "candidate".to_string(),
        user_id: None,
        group_id: Some("devs".to_string()),
        task_id: None,
        process_instance_id: Some("proc-1".to_string()),
        process_definition_id: None,
    });

    let user_links = svc
        .create_identity_link_query()
        .user_id("kermit".to_string())
        .list()
        .unwrap();
    assert_eq!(user_links.len(), 1);
    assert_eq!(user_links[0].link_type, "participant");
    assert_eq!(user_links[0].process_instance_id.as_deref(), Some("proc-1"));

    let group_links = svc
        .create_identity_link_query()
        .group_id("devs".to_string())
        .list()
        .unwrap();
    assert_eq!(group_links.len(), 1);
    assert_eq!(group_links[0].link_type, "candidate");
}

#[test]
fn process_instance_identity_link_delete_removes_specific_link() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link(IdentityLink {
        id: "il-keep".to_string(),
        link_type: "participant".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some("proc-1".to_string()),
        process_definition_id: None,
    });
    svc.add_identity_link(IdentityLink {
        id: "il-delete".to_string(),
        link_type: "candidate".to_string(),
        user_id: Some("fozzie".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some("proc-1".to_string()),
        process_definition_id: None,
    });

    assert_eq!(
        svc.create_identity_link_query()
            .process_instance_id("proc-1".to_string())
            .list()
            .unwrap()
            .len(),
        2
    );

    svc.remove_identity_link("il-delete");

    let remaining = svc
        .create_identity_link_query()
        .process_instance_id("proc-1".to_string())
        .list()
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "il-keep");
}

/// Java records a TYPE_EVENT comment (`AddUserLink`/`DeleteUserLink`) on the
/// process instance for identity-link changes, authored by the authenticated
/// user (`AbstractHistoryManager.createProcessInstanceIdentityLinkComment`).
#[test]
fn process_instance_identity_link_add_and_remove_write_comment_events() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link_with_author(
        IdentityLink {
            id: "il-event-1".to_string(),
            link_type: "participant".to_string(),
            user_id: Some("kermit".to_string()),
            group_id: None,
            task_id: None,
            process_instance_id: Some("proc-event-1".to_string()),
            process_definition_id: None,
        },
        Some("admin".to_string()),
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let comments =
        store.find_historic_comments_by_process_instance_id("proc-event-1", &mut session);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].action.as_deref(), Some("AddUserLink"));
    assert_eq!(comments[0].author.as_deref(), Some("admin"));
    assert!(comments[0].message.contains("kermit"));
    assert!(comments[0].message.contains("participant"));
    let _ = session.rollback();

    svc.remove_identity_link_with_author("il-event-1", Some("admin".to_string()));

    let mut session = store.create_session().unwrap();
    let comments =
        store.find_historic_comments_by_process_instance_id("proc-event-1", &mut session);
    assert_eq!(comments.len(), 2);
    assert!(
        comments
            .iter()
            .any(|comment| comment.action.as_deref() == Some("AddUserLink"))
    );
    assert!(
        comments
            .iter()
            .any(|comment| comment.action.as_deref() == Some("DeleteUserLink"))
    );
    let _ = session.rollback();
}
