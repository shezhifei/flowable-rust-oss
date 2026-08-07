//! P2-9 contract tests: event registry change revisions come from a single-row
//! allocator (monotonic + unique, enforced by a unique index) and change-log
//! polling is pushed down to SQL with a bounded, resumable cursor.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_session::DbParams;
use flowable_engine::persistence::runtime_store::EventRegistryChangeRecord;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

fn shared_engines(label: &str) -> (Arc<ProcessEngine>, Arc<ProcessEngine>, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "event-registry-revision-{}-{}.sqlite",
        label,
        Uuid::new_v4()
    ));
    let engine_a = Arc::new(ProcessEngine::new_with_db_path(
        format!("{label}-a"),
        path.to_str().unwrap(),
    ));
    let engine_b = Arc::new(ProcessEngine::new_with_db_path(
        format!("{label}-b"),
        path.to_str().unwrap(),
    ));
    (engine_a, engine_b, path)
}

fn change_record(revision: u64, id: &str) -> EventRegistryChangeRecord {
    EventRegistryChangeRecord {
        id: id.to_string(),
        revision,
        change_type: "deploy".to_string(),
        entity_type: "channel".to_string(),
        entity_id: format!("channel:{id}"),
        entity_key: "orders".to_string(),
        tenant_id: None,
        version: Some(1),
        deployment_id: None,
        created_at: 0,
    }
}

/// Allocates one revision and appends the matching change record in its own
/// committed transaction, mirroring what deployment/update code paths do.
fn allocate_and_insert(engine: &ProcessEngine, id: &str) -> u64 {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let revision = store.next_event_registry_change_revision(&mut session).unwrap();
    store
        .insert_event_registry_change_record(change_record(revision, id), &mut session)
        .unwrap();
    session.flush_and_commit().unwrap();
    revision
}

#[test]
fn revisions_are_monotonic_and_unique_across_instances() {
    let (engine_a, engine_b, path) = shared_engines("monotonic");

    let mut revisions = Vec::new();
    for index in 0..10 {
        let engine = if index % 2 == 0 { &engine_a } else { &engine_b };
        revisions.push(allocate_and_insert(engine, &format!("rec-{index}")));
    }

    // Strictly increasing regardless of which instance allocated.
    for pair in revisions.windows(2) {
        assert!(
            pair[0] < pair[1],
            "expected strictly increasing revisions, got {revisions:?}"
        );
    }

    let store = engine_a.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let records = store.list_event_registry_change_records(&mut session);
    assert_eq!(records.len(), 10);
    let listed: Vec<u64> = records.iter().map(|record| record.revision).collect();
    assert_eq!(listed, revisions, "listing must be ordered by revision");

    let _ = std::fs::remove_file(path);
}

#[test]
fn concurrent_allocation_yields_unique_revisions() {
    let (engine_a, engine_b, path) = shared_engines("concurrent");

    let mut handles = Vec::new();
    for worker in 0..4u32 {
        let engine = if worker % 2 == 0 {
            Arc::clone(&engine_a)
        } else {
            Arc::clone(&engine_b)
        };
        handles.push(std::thread::spawn(move || {
            let mut allocated = Vec::new();
            for round in 0..5u32 {
                allocated.push(allocate_and_insert(
                    &engine,
                    &format!("worker-{worker}-round-{round}"),
                ));
            }
            allocated
        }));
    }

    let mut revisions: Vec<u64> = handles
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect();
    revisions.sort_unstable();
    let before_dedup = revisions.len();
    revisions.dedup();
    assert_eq!(
        before_dedup,
        revisions.len(),
        "concurrent allocation produced duplicate revisions: {revisions:?}"
    );
    assert_eq!(revisions.len(), 20);

    let store = engine_a.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert_eq!(
        store.list_event_registry_change_records(&mut session).len(),
        20
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn poll_after_revision_is_pushed_down_bounded_and_resumable() {
    let (engine_a, _engine_b, path) = shared_engines("poll");

    let store = engine_a.get_runtime_store();
    let mut session = store.create_session().unwrap();
    for index in 0..25 {
        let revision = store.next_event_registry_change_revision(&mut session).unwrap();
        store
            .insert_event_registry_change_record(
                change_record(revision, &format!("poll-{index:02}")),
                &mut session,
            )
            .unwrap();
    }
    session.flush_and_commit().unwrap();

    // Page through the change log with a single-revision cursor; the pushed
    // down query must not skip or duplicate records at page boundaries.
    let mut session = store.create_session().unwrap();
    let mut cursor = 0u64;
    let mut seen = Vec::new();
    loop {
        let page = store.list_event_registry_change_records_after(cursor, 10, &mut session);
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 10, "page exceeded requested limit");
        for record in &page {
            assert!(
                record.revision > cursor,
                "poll returned revision {} not after cursor {cursor}",
                record.revision
            );
        }
        cursor = page.last().unwrap().revision;
        seen.extend(page.into_iter().map(|record| record.revision));
    }
    assert_eq!(seen.len(), 25);
    let mut deduped = seen.clone();
    deduped.sort_unstable();
    deduped.dedup();
    assert_eq!(deduped.len(), 25, "pagination duplicated revisions");
    assert!(
        seen.windows(2).all(|pair| pair[0] < pair[1]),
        "pages must be ordered by revision"
    );

    // Cursor in the middle only returns strictly greater revisions.
    let middle = seen[12];
    let tail = store.list_event_registry_change_records_after(middle, 100, &mut session);
    assert_eq!(tail.len(), 12);
    assert!(tail.iter().all(|record| record.revision > middle));

    let _ = std::fs::remove_file(path);
}

#[test]
fn allocator_reseeds_from_high_water_mark_when_seq_row_missing() {
    let (engine_a, _engine_b, path) = shared_engines("reseed");

    let last = (0..3)
        .map(|index| allocate_and_insert(&engine_a, &format!("seed-{index}")))
        .last()
        .unwrap();

    // Simulate a database that predates the allocator table seed.
    let store = engine_a.get_runtime_store();
    let mut session = store.create_session().unwrap();
    session
        .execute_raw(
            "DELETE FROM event_registry_change_revision_seq",
            DbParams::new(),
        )
        .unwrap();
    session.flush_and_commit().unwrap();

    let next = allocate_and_insert(&engine_a, "after-reseed");
    assert_eq!(
        next,
        last + 1,
        "allocator must reseed from MAX(revision), not restart at 1"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn duplicate_revision_insert_is_rejected_by_unique_index() {
    let (engine_a, _engine_b, path) = shared_engines("unique");

    let revision = allocate_and_insert(&engine_a, "original");

    let store = engine_a.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let clash = change_record(revision, "clashing");
    let result = session.insert_exclusive_with_extra(
        "event_registry_change_records",
        &clash.id,
        &clash,
        &[
            ("revision".into(), Some(clash.revision.to_string())),
            ("change_type".into(), Some(clash.change_type.clone())),
            ("entity_type".into(), Some(clash.entity_type.clone())),
            ("entity_key".into(), Some(clash.entity_key.clone())),
        ],
    );
    assert!(
        result.is_err(),
        "inserting a second record with revision {revision} must fail"
    );
    drop(session);

    // The original record must still be present (no silent REPLACE).
    let mut session = store.create_session().unwrap();
    let records = store.list_event_registry_change_records(&mut session);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "original");

    let _ = std::fs::remove_file(path);
}
