use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::{RuntimeStore, RuntimeTokenRevocation};
use std::sync::Arc;

#[test]
fn test_revocation_store_cleanup() {
    let db_path = "test_revocation_store_cleanup.db";
    let _ = std::fs::remove_file(db_path);

    let db_store = Arc::new(DbStore::new_file(db_path).unwrap());
    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);

    let now = runtime_store.time_source().now().timestamp_millis();

    let mut session = runtime_store.create_session().unwrap();
    runtime_store.insert_token_revocation(
        RuntimeTokenRevocation {
            jti: "expired-jti".to_string(),
            issuer: "issuer-1".to_string(),
            reason: "expired reason".to_string(),
            expires_at: now - 1000,
            created_at: now - 2000,
        },
        &mut session,
    );

    runtime_store.insert_token_revocation(
        RuntimeTokenRevocation {
            jti: "active-jti".to_string(),
            issuer: "issuer-1".to_string(),
            reason: "active reason".to_string(),
            expires_at: now + 3_600_000,
            created_at: now,
        },
        &mut session,
    );

    assert_eq!(
        runtime_store.count_active_token_revocations(&mut session),
        1
    );

    let deleted = runtime_store.cleanup_expired_token_revocations(&mut session);
    assert_eq!(deleted, 1);

    assert!(
        runtime_store
            .find_token_revocation("expired-jti", &mut session)
            .is_none()
    );
    assert!(
        runtime_store
            .find_token_revocation("active-jti", &mut session)
            .is_some()
    );

    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_revocation_store_lists_active_entries_with_created_at() {
    let db_path = "test_revocation_store_list_active.db";
    let _ = std::fs::remove_file(db_path);

    let db_store = Arc::new(DbStore::new_file(db_path).unwrap());
    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let now = runtime_store.time_source().now().timestamp_millis();

    let mut session = runtime_store.create_session().unwrap();
    runtime_store.insert_token_revocation(
        RuntimeTokenRevocation {
            jti: "expired-jti".to_string(),
            issuer: "issuer-1".to_string(),
            reason: "expired reason".to_string(),
            expires_at: now - 1000,
            created_at: now - 5000,
        },
        &mut session,
    );

    runtime_store.insert_token_revocation(
        RuntimeTokenRevocation {
            jti: "active-jti".to_string(),
            issuer: "issuer-2".to_string(),
            reason: "active reason".to_string(),
            expires_at: now + 3_600_000,
            created_at: now - 1000,
        },
        &mut session,
    );

    let active = runtime_store.list_active_token_revocations(&mut session);
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].jti, "active-jti");
    assert_eq!(active[0].issuer, "issuer-2");
    assert_eq!(active[0].reason, "active reason");
    assert_eq!(active[0].created_at, now - 1000);

    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_token_revocations_schema_has_expiry_index() {
    let db_path = "test_revocation_store_schema_index.db";
    let _ = std::fs::remove_file(db_path);

    let db_store = Arc::new(DbStore::new_file(db_path).unwrap());
    let mut session = db_store.create_session().unwrap();
    let rows = session
        .raw_query(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'token_revocations'",
            flowable_engine::persistence::DbParams::new(),
        )
        .unwrap();
    let names: Vec<String> = rows
        .into_iter()
        .filter_map(|r| r.get_text("name"))
        .collect();

    assert!(
        names
            .iter()
            .any(|name| name == "idx_token_revocations_expires_at"),
        "token_revocations cleanup should be backed by an expires_at index"
    );

    session.flush_and_commit().unwrap();
    let _ = std::fs::remove_file(db_path);
}
