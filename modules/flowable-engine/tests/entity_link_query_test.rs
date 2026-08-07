use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::EntityLink;
use std::sync::Arc;

fn setup() -> Arc<ProcessEngine> {
    Arc::new(ProcessEngine::new("entity-link-query".to_string()))
}

#[test]
fn entity_link_query_filters_by_scope_id_and_scope_type() {
    let engine = setup();
    let svc = engine.get_entity_link_service();

    svc.add_entity_link(EntityLink {
        id: "el-1".to_string(),
        link_type: "reference".to_string(),
        scope_id: Some("scope-1".to_string()),
        scope_type: Some("processInstance".to_string()),
        reference_scope_id: Some("ref-1".to_string()),
        reference_scope_type: Some("task".to_string()),
        hierarchy_type: Some("child".to_string()),
    });
    svc.add_entity_link(EntityLink {
        id: "el-2".to_string(),
        link_type: "reference".to_string(),
        scope_id: Some("scope-2".to_string()),
        scope_type: Some("caseInstance".to_string()),
        reference_scope_id: Some("ref-2".to_string()),
        reference_scope_type: Some("task".to_string()),
        hierarchy_type: Some("child".to_string()),
    });

    let by_scope = svc
        .create_entity_link_query()
        .scope_id("scope-1".to_string())
        .list()
        .unwrap();
    assert_eq!(by_scope.len(), 1);
    assert_eq!(by_scope[0].id, "el-1");

    let by_scope_type = svc
        .create_entity_link_query()
        .scope_type("caseInstance".to_string())
        .list()
        .unwrap();
    assert_eq!(by_scope_type.len(), 1);
    assert_eq!(by_scope_type[0].scope_id.as_deref(), Some("scope-2"));
}

#[test]
fn entity_link_query_filters_by_reference_scope() {
    let engine = setup();
    let svc = engine.get_entity_link_service();

    svc.add_entity_link(EntityLink {
        id: "el-1".to_string(),
        link_type: "reference".to_string(),
        scope_id: Some("scope-1".to_string()),
        scope_type: None,
        reference_scope_id: Some("ref-a".to_string()),
        reference_scope_type: Some("task".to_string()),
        hierarchy_type: None,
    });
    svc.add_entity_link(EntityLink {
        id: "el-2".to_string(),
        link_type: "reference".to_string(),
        scope_id: Some("scope-1".to_string()),
        scope_type: None,
        reference_scope_id: Some("ref-b".to_string()),
        reference_scope_type: Some("subProcess".to_string()),
        hierarchy_type: None,
    });

    let by_ref_id = svc
        .create_entity_link_query()
        .reference_scope_id("ref-a".to_string())
        .list()
        .unwrap();
    assert_eq!(by_ref_id.len(), 1);
    assert_eq!(by_ref_id[0].id, "el-1");

    let by_ref_type = svc
        .create_entity_link_query()
        .reference_scope_type("subProcess".to_string())
        .list()
        .unwrap();
    assert_eq!(by_ref_type.len(), 1);
    assert_eq!(by_ref_type[0].reference_scope_id.as_deref(), Some("ref-b"));
}

#[test]
fn entity_link_query_filters_by_link_type() {
    let engine = setup();
    let svc = engine.get_entity_link_service();

    svc.add_entity_link(EntityLink {
        id: "el-1".to_string(),
        link_type: "reference".to_string(),
        scope_id: Some("s1".to_string()),
        scope_type: None,
        reference_scope_id: Some("r1".to_string()),
        reference_scope_type: None,
        hierarchy_type: None,
    });
    svc.add_entity_link(EntityLink {
        id: "el-2".to_string(),
        link_type: "dependency".to_string(),
        scope_id: Some("s1".to_string()),
        scope_type: None,
        reference_scope_id: Some("r2".to_string()),
        reference_scope_type: None,
        hierarchy_type: None,
    });

    let refs = svc
        .create_entity_link_query()
        .link_type("reference".to_string())
        .list()
        .unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].id, "el-1");
}

#[test]
fn entity_link_query_returns_all_when_no_filter() {
    let engine = setup();
    let svc = engine.get_entity_link_service();

    for i in 0..3 {
        svc.add_entity_link(EntityLink {
            id: format!("el-{}", i),
            link_type: "reference".to_string(),
            scope_id: Some(format!("scope-{}", i)),
            scope_type: None,
            reference_scope_id: Some(format!("ref-{}", i)),
            reference_scope_type: None,
            hierarchy_type: None,
        });
    }

    let all = svc.create_entity_link_query().list().unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn entity_link_remove_deletes_link() {
    let engine = setup();
    let svc = engine.get_entity_link_service();

    svc.add_entity_link(EntityLink {
        id: "el-to-delete".to_string(),
        link_type: "reference".to_string(),
        scope_id: Some("s1".to_string()),
        scope_type: None,
        reference_scope_id: Some("r1".to_string()),
        reference_scope_type: None,
        hierarchy_type: None,
    });

    assert_eq!(svc.create_entity_link_query().list().unwrap().len(), 1);

    svc.remove_entity_link("el-to-delete");

    assert_eq!(svc.create_entity_link_query().list().unwrap().len(), 0);
}

#[test]
fn entity_link_query_combined_filters() {
    let engine = setup();
    let svc = engine.get_entity_link_service();

    svc.add_entity_link(EntityLink {
        id: "el-1".to_string(),
        link_type: "reference".to_string(),
        scope_id: Some("scope-1".to_string()),
        scope_type: Some("processInstance".to_string()),
        reference_scope_id: Some("ref-1".to_string()),
        reference_scope_type: Some("task".to_string()),
        hierarchy_type: Some("child".to_string()),
    });
    svc.add_entity_link(EntityLink {
        id: "el-2".to_string(),
        link_type: "dependency".to_string(),
        scope_id: Some("scope-1".to_string()),
        scope_type: Some("processInstance".to_string()),
        reference_scope_id: Some("ref-2".to_string()),
        reference_scope_type: Some("task".to_string()),
        hierarchy_type: None,
    });

    let combined = svc
        .create_entity_link_query()
        .scope_id("scope-1".to_string())
        .scope_type("processInstance".to_string())
        .link_type("reference".to_string())
        .list()
        .unwrap();
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].id, "el-1");
}
