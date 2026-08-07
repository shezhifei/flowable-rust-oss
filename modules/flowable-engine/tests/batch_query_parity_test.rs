use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::{BatchEntity, BatchPartEntity};
use std::sync::Arc;

fn setup() -> Arc<ProcessEngine> {
    Arc::new(ProcessEngine::new("batch-query-parity".to_string()))
}

fn make_batch(id: &str, batch_type: &str, status: &str, tenant: Option<&str>) -> BatchEntity {
    BatchEntity {
        id: id.to_string(),
        batch_type: batch_type.to_string(),
        search_key: None,
        search_key2: None,
        status: status.to_string(),
        total_items: 10,
        items_processed: 0,
        create_time: 0,
        end_time: None,
        tenant_id: tenant.map(|s| s.to_string()),
        batch_document_json: None,
    }
}

#[test]
fn batch_query_filters_by_batch_type() {
    let engine = setup();
    let svc = engine.get_batch_service();

    svc.create_batch(make_batch("b1", "processMigration", "completed", None));
    svc.create_batch(make_batch("b2", "decisionTable", "running", None));

    let migration = svc
        .create_batch_query()
        .batch_type("processMigration".to_string())
        .list()
        .unwrap();
    assert_eq!(migration.len(), 1);
    assert_eq!(migration[0].id, "b1");
}

#[test]
fn batch_query_filters_by_status() {
    let engine = setup();
    let svc = engine.get_batch_service();

    svc.create_batch(make_batch("b1", "processMigration", "completed", None));
    svc.create_batch(make_batch("b2", "processMigration", "running", None));
    svc.create_batch(make_batch("b3", "processMigration", "completed", None));

    let completed = svc
        .create_batch_query()
        .status("completed".to_string())
        .list()
        .unwrap();
    assert_eq!(completed.len(), 2);

    let running = svc
        .create_batch_query()
        .status("running".to_string())
        .list()
        .unwrap();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, "b2");
}

#[test]
fn batch_query_filters_by_tenant_id() {
    let engine = setup();
    let svc = engine.get_batch_service();

    svc.create_batch(make_batch(
        "b1",
        "processMigration",
        "completed",
        Some("tenant-a"),
    ));
    svc.create_batch(make_batch(
        "b2",
        "processMigration",
        "completed",
        Some("tenant-b"),
    ));
    svc.create_batch(make_batch("b3", "processMigration", "completed", None));

    let tenant_a = svc
        .create_batch_query()
        .tenant_id("tenant-a".to_string())
        .list()
        .unwrap();
    assert_eq!(tenant_a.len(), 1);
    assert_eq!(tenant_a[0].id, "b1");
}

#[test]
fn batch_query_filters_by_without_tenant_id() {
    let engine = setup();
    let svc = engine.get_batch_service();

    svc.create_batch(make_batch(
        "b1",
        "processMigration",
        "completed",
        Some("tenant-a"),
    ));
    svc.create_batch(make_batch("b2", "processMigration", "completed", None));

    let no_tenant = svc.create_batch_query().without_tenant_id().list().unwrap();
    assert_eq!(no_tenant.len(), 1);
    assert_eq!(no_tenant[0].id, "b2");
}

#[test]
fn batch_query_filters_by_tenant_id_like() {
    let engine = setup();
    let svc = engine.get_batch_service();

    svc.create_batch(make_batch(
        "b1",
        "processMigration",
        "completed",
        Some("tenant-alpha"),
    ));
    svc.create_batch(make_batch(
        "b2",
        "processMigration",
        "completed",
        Some("tenant-beta"),
    ));
    svc.create_batch(make_batch(
        "b3",
        "processMigration",
        "completed",
        Some("other"),
    ));

    let like = svc
        .create_batch_query()
        .tenant_id_like("tenant".to_string())
        .list()
        .unwrap();
    assert_eq!(like.len(), 2);
}

#[test]
fn batch_query_returns_all_when_no_filter() {
    let engine = setup();
    let svc = engine.get_batch_service();

    for i in 0..4 {
        svc.create_batch(make_batch(
            &format!("b{}", i),
            "processMigration",
            "completed",
            None,
        ));
    }

    let all = svc.create_batch_query().list().unwrap();
    assert_eq!(all.len(), 4);
}

#[test]
fn batch_query_count_returns_correct_number() {
    let engine = setup();
    let svc = engine.get_batch_service();

    for i in 0..3 {
        svc.create_batch(make_batch(
            &format!("b{}", i),
            "processMigration",
            "completed",
            None,
        ));
    }

    let count = svc.create_batch_query().count().unwrap();
    assert_eq!(count, 3);
}

#[test]
fn batch_part_create_and_query_by_batch_id() {
    let engine = setup();
    let svc = engine.get_batch_service();

    svc.create_batch(make_batch("batch-1", "processMigration", "running", None));

    svc.create_batch_part(BatchPartEntity {
        id: "part-1".to_string(),
        batch_id: "batch-1".to_string(),
        batch_type: "processMigration".to_string(),
        search_key: None,
        search_key2: None,
        scope_id: Some("scope-1".to_string()),
        sub_scope_id: None,
        scope_type: None,
        create_time: 0,
        complete_time: None,
        status: "completed".to_string(),
        tenant_id: None,
        batch_part_document_json: None,
    });
    svc.create_batch_part(BatchPartEntity {
        id: "part-2".to_string(),
        batch_id: "batch-1".to_string(),
        batch_type: "processMigration".to_string(),
        search_key: None,
        search_key2: None,
        scope_id: Some("scope-2".to_string()),
        sub_scope_id: None,
        scope_type: None,
        create_time: 0,
        complete_time: None,
        status: "pending".to_string(),
        tenant_id: None,
        batch_part_document_json: None,
    });

    let parts = svc.find_batch_parts_by_batch_id("batch-1");
    assert_eq!(parts.len(), 2);

    let completed = svc.find_batch_parts_by_batch_id_and_status("batch-1", "completed");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].id, "part-1");

    let pending = svc.find_batch_parts_by_batch_id_and_status("batch-1", "pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "part-2");
}

#[test]
fn batch_delete_removes_batch() {
    let engine = setup();
    let svc = engine.get_batch_service();

    svc.create_batch(make_batch("b1", "processMigration", "completed", None));
    assert_eq!(svc.create_batch_query().list().unwrap().len(), 1);

    svc.delete_batch("b1");
    assert_eq!(svc.create_batch_query().list().unwrap().len(), 0);
    assert!(svc.find_batch_by_id("b1").is_none());
}
