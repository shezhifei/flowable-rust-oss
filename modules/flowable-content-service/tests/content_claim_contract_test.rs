//! Contract tests for `claim_content_item_for_field_in_session` (P1 tenant fix).
//!
//! Rules under test:
//! - symmetric tenant ownership (tenantless ↔ tenantless, tenant ↔ same tenant);
//! - no implicit tenant adoption of tenantless content by a tenant context;
//! - no reassignment of already-associated content (idempotent for same target);
//! - guarded UPDATE detects concurrent claims (stale read vs physical columns).

use flowable_content_service::ContentItem;
use flowable_content_service::repository::{
    self, ContentClaimError, claim_content_item_for_field_in_session,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::DbParams;
use std::sync::Arc;

fn engine(name: &str) -> Arc<ProcessEngine> {
    Arc::new(ProcessEngine::new(name.to_string()))
}

fn unowned_item(id: &str, tenant: Option<&str>) -> ContentItem {
    ContentItem {
        id: id.to_string(),
        name: format!("{id}.txt"),
        mime_type: Some("text/plain".to_string()),
        description: None,
        attachment_type: None,
        external_url: None,
        content: None,
        content_size: 0,
        task_id: None,
        process_instance_id: None,
        scope_type: None,
        scope_id: None,
        field: None,
        tenant_id: tenant.map(str::to_string),
        created_by: Some("tester".to_string()),
        created_at: 0,
        updated_at: 0,
        storage_id: None,
        storage_backend: None,
        version: None,
        expires_at: None,
    }
}

fn seed_item(engine: &Arc<ProcessEngine>, item: &ContentItem) {
    let store = engine.get_runtime_store();
    let mut session = store.db_store().create_session().unwrap();
    repository::insert_content_item_in_session(&mut session, item, None).unwrap();
    session.flush_and_commit().unwrap();
}

fn reload_item(engine: &Arc<ProcessEngine>, id: &str) -> ContentItem {
    let store = engine.get_runtime_store();
    let mut session = store.db_store().create_session().unwrap();
    let item = repository::find_content_item_in_session(&mut session, id)
        .unwrap()
        .expect("item must exist");
    session.rollback().ok();
    item
}

#[test]
fn claim_writes_owner_fields_and_is_idempotent_for_identical_target() {
    let engine = engine("content-claim-happy");
    seed_item(&engine, &unowned_item("item-1", None));

    let store = engine.get_runtime_store();
    let mut session = store.db_store().create_session().unwrap();
    let claimed = claim_content_item_for_field_in_session(
        &mut session,
        "item-1",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("files"),
        None,
    )
    .unwrap();
    session.flush_and_commit().unwrap();

    assert_eq!(claimed.process_instance_id.as_deref(), Some("proc-1"));
    assert_eq!(claimed.field.as_deref(), Some("files"));
    assert_eq!(claimed.tenant_id, None);

    let stored = reload_item(&engine, "item-1");
    assert_eq!(stored.process_instance_id.as_deref(), Some("proc-1"));
    assert_eq!(stored.scope_type.as_deref(), Some("start"));
    assert_eq!(stored.field.as_deref(), Some("files"));

    // Identical target again → idempotent Ok, no error.
    let mut session = store.db_store().create_session().unwrap();
    claim_content_item_for_field_in_session(
        &mut session,
        "item-1",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("files"),
        None,
    )
    .unwrap();
    session.rollback().ok();
}

#[test]
fn claim_missing_item_returns_not_found() {
    let engine = engine("content-claim-missing");
    let store = engine.get_runtime_store();
    let mut session = store.db_store().create_session().unwrap();
    let err = claim_content_item_for_field_in_session(
        &mut session,
        "no-such-item",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("files"),
        None,
    )
    .unwrap_err();
    session.rollback().ok();
    assert!(matches!(err, ContentClaimError::NotFound));
}

#[test]
fn claim_rejects_tenant_mismatch_symmetrically() {
    let engine = engine("content-claim-tenant");
    seed_item(&engine, &unowned_item("item-tenant-b", Some("tenant-b")));
    seed_item(&engine, &unowned_item("item-tenantless", None));

    let store = engine.get_runtime_store();

    // Tenantless context must not touch tenant-b content (previous bug: the
    // old association path silently cleared tenant_id to None here).
    let mut session = store.db_store().create_session().unwrap();
    let err = claim_content_item_for_field_in_session(
        &mut session,
        "item-tenant-b",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("files"),
        None,
    )
    .unwrap_err();
    session.rollback().ok();
    match err {
        ContentClaimError::TenantMismatch { item_tenant } => {
            assert_eq!(item_tenant.as_deref(), Some("tenant-b"));
        }
        other => panic!("expected TenantMismatch, got {other:?}"),
    }

    // Tenant context must not implicitly adopt tenantless content.
    let mut session = store.db_store().create_session().unwrap();
    let err = claim_content_item_for_field_in_session(
        &mut session,
        "item-tenantless",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("files"),
        Some("tenant-a"),
    )
    .unwrap_err();
    session.rollback().ok();
    assert!(matches!(err, ContentClaimError::TenantMismatch { .. }));

    // Neither item may have been modified by the rejected claims.
    let untouched_b = reload_item(&engine, "item-tenant-b");
    assert_eq!(untouched_b.tenant_id.as_deref(), Some("tenant-b"));
    assert_eq!(untouched_b.process_instance_id, None);
    assert_eq!(untouched_b.field, None);
    let untouched_none = reload_item(&engine, "item-tenantless");
    assert_eq!(untouched_none.tenant_id, None);
    assert_eq!(untouched_none.process_instance_id, None);
}

#[test]
fn claim_rejects_cross_tenant_context() {
    let engine = engine("content-claim-cross-tenant");
    seed_item(&engine, &unowned_item("item-tenant-a", Some("tenant-a")));

    let store = engine.get_runtime_store();

    // A tenant-b context must never claim tenant-a content, even though both
    // sides are tenant-scoped (the earlier tests only cover tenantless mixes).
    let mut session = store.db_store().create_session().unwrap();
    let err = claim_content_item_for_field_in_session(
        &mut session,
        "item-tenant-a",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("files"),
        Some("tenant-b"),
    )
    .unwrap_err();
    session.rollback().ok();
    match err {
        ContentClaimError::TenantMismatch { item_tenant } => {
            assert_eq!(item_tenant.as_deref(), Some("tenant-a"));
        }
        other => panic!("expected TenantMismatch, got {other:?}"),
    }

    let untouched = reload_item(&engine, "item-tenant-a");
    assert_eq!(untouched.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(untouched.process_instance_id, None);
    assert_eq!(untouched.field, None);
}

#[test]
fn claim_accepts_matching_tenant_context() {
    let engine = engine("content-claim-same-tenant");
    seed_item(&engine, &unowned_item("item-tenant-a", Some("tenant-a")));

    let store = engine.get_runtime_store();
    let mut session = store.db_store().create_session().unwrap();
    let claimed = claim_content_item_for_field_in_session(
        &mut session,
        "item-tenant-a",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("files"),
        Some("tenant-a"),
    )
    .unwrap();
    session.flush_and_commit().unwrap();

    assert_eq!(claimed.process_instance_id.as_deref(), Some("proc-1"));
    assert_eq!(claimed.field.as_deref(), Some("files"));
    // The claim must keep — not rewrite — the item's tenant.
    assert_eq!(claimed.tenant_id.as_deref(), Some("tenant-a"));

    let stored = reload_item(&engine, "item-tenant-a");
    assert_eq!(stored.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(stored.process_instance_id.as_deref(), Some("proc-1"));
}

#[test]
fn claim_rejects_reassignment_to_a_different_target() {
    let engine = engine("content-claim-reassign");
    seed_item(&engine, &unowned_item("item-owned", None));

    let store = engine.get_runtime_store();
    let mut session = store.db_store().create_session().unwrap();
    claim_content_item_for_field_in_session(
        &mut session,
        "item-owned",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("files"),
        None,
    )
    .unwrap();
    session.flush_and_commit().unwrap();

    // Same tenant but different process → must not be re-assigned.
    let mut session = store.db_store().create_session().unwrap();
    let err = claim_content_item_for_field_in_session(
        &mut session,
        "item-owned",
        None,
        Some("proc-2"),
        Some("proc-2"),
        Some("start"),
        Some("files"),
        None,
    )
    .unwrap_err();
    session.rollback().ok();
    assert!(matches!(err, ContentClaimError::AlreadyAssociated));

    // Different field on the same process → also a reassignment.
    let mut session = store.db_store().create_session().unwrap();
    let err = claim_content_item_for_field_in_session(
        &mut session,
        "item-owned",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("attachments"),
        None,
    )
    .unwrap_err();
    session.rollback().ok();
    assert!(matches!(err, ContentClaimError::AlreadyAssociated));

    let stored = reload_item(&engine, "item-owned");
    assert_eq!(stored.process_instance_id.as_deref(), Some("proc-1"));
    assert_eq!(stored.field.as_deref(), Some("files"));
}

#[test]
fn claim_detects_concurrent_claim_via_guarded_update() {
    let engine = engine("content-claim-concurrent");
    seed_item(&engine, &unowned_item("item-race", None));

    // Simulate a concurrent transaction that already claimed the item at the
    // physical-column level while our JSON snapshot still looks unowned.
    let store = engine.get_runtime_store();
    let mut session = store.db_store().create_session().unwrap();
    let mut params = DbParams::new();
    params.push("item-race");
    session
        .execute_raw(
            "UPDATE m14_content_items SET task_id = 'stolen-by-other-tx' WHERE id = ?",
            params,
        )
        .unwrap();

    // Same session still reads the stale JSON data (unowned) but the guarded
    // UPDATE must see the occupied task_id column and refuse the claim.
    let err = claim_content_item_for_field_in_session(
        &mut session,
        "item-race",
        None,
        Some("proc-1"),
        Some("proc-1"),
        Some("start"),
        Some("files"),
        None,
    )
    .unwrap_err();
    session.rollback().ok();
    assert!(matches!(err, ContentClaimError::ConcurrentClaim));
}
