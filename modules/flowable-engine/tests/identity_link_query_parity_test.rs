use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::IdentityLink;
use std::sync::Arc;

fn setup() -> Arc<ProcessEngine> {
    Arc::new(ProcessEngine::new("identity-link-query-parity".to_string()))
}

#[test]
fn identity_link_query_filters_by_task_id() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link(IdentityLink {
        id: "link-1".to_string(),
        link_type: "candidate".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: Some("task-1".to_string()),
        process_instance_id: None,
        process_definition_id: None,
    });
    svc.add_identity_link(IdentityLink {
        id: "link-2".to_string(),
        link_type: "assignee".to_string(),
        user_id: Some("fozzie".to_string()),
        group_id: None,
        task_id: Some("task-2".to_string()),
        process_instance_id: None,
        process_definition_id: None,
    });

    let links = svc
        .create_identity_link_query()
        .task_id("task-1".to_string())
        .list()
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].id, "link-1");
}

#[test]
fn identity_link_query_filters_by_process_instance_id() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link(IdentityLink {
        id: "link-1".to_string(),
        link_type: "candidate".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some("proc-1".to_string()),
        process_definition_id: None,
    });
    svc.add_identity_link(IdentityLink {
        id: "link-2".to_string(),
        link_type: "candidate".to_string(),
        user_id: None,
        group_id: Some("admins".to_string()),
        task_id: None,
        process_instance_id: Some("proc-2".to_string()),
        process_definition_id: None,
    });

    let links = svc
        .create_identity_link_query()
        .process_instance_id("proc-1".to_string())
        .list()
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].user_id.as_deref(), Some("kermit"));
}

#[test]
fn identity_link_query_filters_by_process_definition_id() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link(IdentityLink {
        id: "link-1".to_string(),
        link_type: "candidate".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: None,
        process_definition_id: Some("procdef-1".to_string()),
    });

    let links = svc
        .create_identity_link_query()
        .process_definition_id("procdef-1".to_string())
        .list()
        .unwrap();
    assert_eq!(links.len(), 1);
}

#[test]
fn identity_link_query_filters_by_user_id_and_group_id() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link(IdentityLink {
        id: "link-user".to_string(),
        link_type: "candidate".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: None,
        process_definition_id: None,
    });
    svc.add_identity_link(IdentityLink {
        id: "link-group".to_string(),
        link_type: "candidate".to_string(),
        user_id: None,
        group_id: Some("admins".to_string()),
        task_id: None,
        process_instance_id: None,
        process_definition_id: None,
    });

    let user_links = svc
        .create_identity_link_query()
        .user_id("kermit".to_string())
        .list()
        .unwrap();
    assert_eq!(user_links.len(), 1);
    assert_eq!(user_links[0].id, "link-user");

    let group_links = svc
        .create_identity_link_query()
        .group_id("admins".to_string())
        .list()
        .unwrap();
    assert_eq!(group_links.len(), 1);
    assert_eq!(group_links[0].id, "link-group");
}

#[test]
fn identity_link_query_filters_by_link_type() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link(IdentityLink {
        id: "link-1".to_string(),
        link_type: "candidate".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: None,
        process_definition_id: None,
    });
    svc.add_identity_link(IdentityLink {
        id: "link-2".to_string(),
        link_type: "assignee".to_string(),
        user_id: Some("fozzie".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: None,
        process_definition_id: None,
    });

    let candidates = svc
        .create_identity_link_query()
        .link_type("candidate".to_string())
        .list()
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, "link-1");
}

#[test]
fn identity_link_query_returns_all_when_no_filters() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    for i in 0..3 {
        svc.add_identity_link(IdentityLink {
            id: format!("link-{}", i),
            link_type: "candidate".to_string(),
            user_id: Some(format!("user-{}", i)),
            group_id: None,
            task_id: None,
            process_instance_id: None,
            process_definition_id: None,
        });
    }

    let all = svc.create_identity_link_query().list().unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn identity_link_remove_deletes_link() {
    let engine = setup();
    let svc = engine.get_identity_link_service();

    svc.add_identity_link(IdentityLink {
        id: "link-to-delete".to_string(),
        link_type: "candidate".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: None,
        process_definition_id: None,
    });

    assert_eq!(svc.create_identity_link_query().list().unwrap().len(), 1);

    svc.remove_identity_link("link-to-delete");

    assert_eq!(svc.create_identity_link_query().list().unwrap().len(), 0);
}
