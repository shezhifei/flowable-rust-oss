//! P65-job-query storage contract: every queryable job dimension must round-trip
//! through JSON serialization and physical columns, and survive family moves.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::{
    RuntimeJobType, RuntimeTimerJobState, copy_job_query_metadata, stamp_new_job_metadata,
};
use chrono::{TimeZone, Utc};
use serde_json::json;
use std::sync::Arc;

fn engine(name: &str) -> ProcessEngine {
    let now = Utc.with_ymd_and_hms(2026, 7, 30, 12, 0, 0).unwrap();
    ProcessEngine::build(
        name.to_string(),
        Arc::new(TestTimeSource::new(now)),
        Arc::new(DbStore::new_in_memory().unwrap()),
    )
}

fn sample_job(id: &str) -> RuntimeTimerJobState {
    RuntimeTimerJobState {
        timer_job_id: id.to_string(),
        process_instance_id: "pi-1".to_string(),
        execution_id: "exec-1".to_string(),
        activity_id: "activity-1".to_string(),
        job_state: Some("timer".to_string()),
        category: Some("orders".to_string()),
        correlation_id: Some("corr-1".to_string()),
        scope_type: Some("cmmn".to_string()),
        scope_id: Some("case-1".to_string()),
        sub_scope_id: Some("plan-item-1".to_string()),
        scope_definition_id: Some("case-def-1".to_string()),
        handler_type: Some("external-worker-complete".to_string()),
        create_time: Some(1_722_000_000_000),
        tenant_id: Some("tenant-a".to_string()),
        process_definition_id: Some("pd-1".to_string()),
        element_name: Some("Wait for worker".to_string()),
        due_time: Some(1_722_000_100_000),
        retries: Some(3),
        ..Default::default()
    }
}

#[test]
fn queryable_job_dimensions_round_trip_through_serde() {
    let job = sample_job("job-serde");
    let encoded = serde_json::to_value(&job).expect("serialize");
    assert_eq!(encoded["category"], json!("orders"));
    assert_eq!(encoded["correlation_id"], json!("corr-1"));
    assert_eq!(encoded["scope_type"], json!("cmmn"));
    assert_eq!(encoded["scope_id"], json!("case-1"));
    assert_eq!(encoded["sub_scope_id"], json!("plan-item-1"));
    assert_eq!(encoded["scope_definition_id"], json!("case-def-1"));
    assert_eq!(encoded["handler_type"], json!("external-worker-complete"));

    let decoded: RuntimeTimerJobState =
        serde_json::from_value(encoded).expect("deserialize full payload");
    assert_eq!(decoded.category.as_deref(), Some("orders"));
    assert_eq!(decoded.correlation_id.as_deref(), Some("corr-1"));
    assert_eq!(decoded.scope_type.as_deref(), Some("cmmn"));
    assert_eq!(decoded.scope_id.as_deref(), Some("case-1"));
    assert_eq!(decoded.sub_scope_id.as_deref(), Some("plan-item-1"));
    assert_eq!(decoded.scope_definition_id.as_deref(), Some("case-def-1"));
    assert_eq!(
        decoded.handler_type.as_deref(),
        Some("external-worker-complete")
    );
}

#[test]
fn old_rows_without_new_scope_fields_deserialize_with_defaults() {
    let legacy = json!({
        "timer_job_id": "legacy-job",
        "process_instance_id": "pi-legacy",
        "execution_id": "exec-legacy",
        "activity_id": "act",
        "is_boundary": false,
        "cancel_activity": false,
        "category": "legacy-cat",
        "correlation_id": "legacy-corr",
        "scope_type": "bpmn"
    });
    let job: RuntimeTimerJobState =
        serde_json::from_value(legacy).expect("legacy job without new fields");
    assert_eq!(job.category.as_deref(), Some("legacy-cat"));
    assert_eq!(job.correlation_id.as_deref(), Some("legacy-corr"));
    assert_eq!(job.scope_type.as_deref(), Some("bpmn"));
    assert_eq!(job.scope_id, None);
    assert_eq!(job.sub_scope_id, None);
    assert_eq!(job.scope_definition_id, None);
}

#[test]
fn physical_columns_persist_query_metadata_and_external_worker_type() {
    let engine = engine("p65-storage-columns");
    let store = engine.get_runtime_store();
    let job = sample_job("job-cols");

    {
        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state_with_type(
            &job,
            Some(&RuntimeJobType::ExternalWorker),
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let mut session = store.create_session().unwrap();
    let raw = session
        .find_raw("timer_job_states", "job-cols")
        .unwrap()
        .expect("row must exist");
    assert_eq!(
        raw.extras.get("category").and_then(|v| v.as_ref()).map(String::as_str),
        Some("orders")
    );
    assert_eq!(
        raw.extras
            .get("correlation_id")
            .and_then(|v| v.as_ref())
            .map(String::as_str),
        Some("corr-1")
    );
    assert_eq!(
        raw.extras
            .get("scope_type")
            .and_then(|v| v.as_ref())
            .map(String::as_str),
        Some("cmmn")
    );
    assert_eq!(
        raw.extras
            .get("scope_id")
            .and_then(|v| v.as_ref())
            .map(String::as_str),
        Some("case-1")
    );
    assert_eq!(
        raw.extras
            .get("sub_scope_id")
            .and_then(|v| v.as_ref())
            .map(String::as_str),
        Some("plan-item-1")
    );
    assert_eq!(
        raw.extras
            .get("scope_definition_id")
            .and_then(|v| v.as_ref())
            .map(String::as_str),
        Some("case-def-1")
    );
    assert_eq!(
        raw.extras
            .get("handler_type")
            .and_then(|v| v.as_ref())
            .map(String::as_str),
        Some("external-worker-complete")
    );
    assert_eq!(
        raw.extras
            .get("job_type")
            .and_then(|v| v.as_ref())
            .map(String::as_str),
        Some("externalWorker")
    );

    let loaded = store
        .find_timer_job_state("job-cols", &mut session)
        .expect("json payload must load");
    assert_eq!(loaded.scope_id.as_deref(), Some("case-1"));
    assert_eq!(loaded.sub_scope_id.as_deref(), Some("plan-item-1"));
    assert_eq!(loaded.scope_definition_id.as_deref(), Some("case-def-1"));
    assert_eq!(loaded.scope_type.as_deref(), Some("cmmn"));
    assert_eq!(loaded.category.as_deref(), Some("orders"));
    assert_eq!(
        store.find_timer_job_type("job-cols", &mut session),
        Some(RuntimeJobType::ExternalWorker)
    );
    let _ = session.rollback();
}

#[test]
fn family_move_preserves_all_query_metadata() {
    let engine = engine("p65-storage-family-move");
    let store = engine.get_runtime_store();
    let job = sample_job("job-move");

    {
        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state_with_type(
            &job,
            Some(&RuntimeJobType::ExternalWorker),
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    // Simulate deadletter then suspended moves the way management commands do:
    // load, flip job_state, re-insert with the same typed job_type.
    for next_state in ["deadletter", "suspended", "executable"] {
        let mut session = store.create_session().unwrap();
        let mut current = store
            .find_timer_job_state("job-move", &mut session)
            .expect("job");
        let job_type = store.find_timer_job_type("job-move", &mut session);
        current.job_state = Some(next_state.to_string());
        store.insert_timer_job_state_with_type(&current, job_type.as_ref(), &mut session);
        session.flush_and_commit().unwrap();

        let mut session = store.create_session().unwrap();
        let reloaded = store
            .find_timer_job_state("job-move", &mut session)
            .expect("reloaded");
        assert_eq!(reloaded.job_state.as_deref(), Some(next_state));
        assert_eq!(reloaded.category.as_deref(), Some("orders"));
        assert_eq!(reloaded.correlation_id.as_deref(), Some("corr-1"));
        assert_eq!(reloaded.scope_type.as_deref(), Some("cmmn"));
        assert_eq!(reloaded.scope_id.as_deref(), Some("case-1"));
        assert_eq!(reloaded.sub_scope_id.as_deref(), Some("plan-item-1"));
        assert_eq!(reloaded.scope_definition_id.as_deref(), Some("case-def-1"));
        assert_eq!(
            reloaded.handler_type.as_deref(),
            Some("external-worker-complete")
        );
        assert_eq!(
            store.find_timer_job_type("job-move", &mut session),
            Some(RuntimeJobType::ExternalWorker)
        );
        let _ = session.rollback();
    }
}

#[test]
fn stamp_and_copy_helpers_propagate_query_metadata() {
    let mut source = RuntimeTimerJobState {
        timer_job_id: "src".to_string(),
        process_instance_id: "pi".to_string(),
        execution_id: "ex".to_string(),
        activity_id: "act".to_string(),
        category: Some("billing".to_string()),
        correlation_id: Some("keep-me".to_string()),
        handler_type: Some("trigger-timer".to_string()),
        tenant_id: Some("t1".to_string()),
        process_definition_id: Some("pd".to_string()),
        element_name: Some("Timer A".to_string()),
        scope_type: Some("cmmn".to_string()),
        scope_id: Some("case-9".to_string()),
        sub_scope_id: Some("pi-9".to_string()),
        scope_definition_id: Some("cd-9".to_string()),
        ..Default::default()
    };

    stamp_new_job_metadata(
        &mut source,
        99,
        "ignored-when-set",
        Some("other-tenant".to_string()),
        Some("other-pd".to_string()),
        Some("other-name".to_string()),
    );
    assert_eq!(source.create_time, Some(99));
    assert_eq!(source.correlation_id.as_deref(), Some("keep-me"));
    assert_eq!(source.handler_type.as_deref(), Some("trigger-timer"));
    assert_eq!(source.tenant_id.as_deref(), Some("t1"));
    assert_eq!(source.category.as_deref(), Some("billing"));
    assert_eq!(source.scope_id.as_deref(), Some("case-9"));

    let mut dest = RuntimeTimerJobState {
        timer_job_id: "dest".to_string(),
        process_instance_id: "pi2".to_string(),
        execution_id: "ex2".to_string(),
        activity_id: "act2".to_string(),
        job_state: Some("deadletter".to_string()),
        ..Default::default()
    };
    copy_job_query_metadata(&source, &mut dest);
    assert_eq!(dest.category.as_deref(), Some("billing"));
    assert_eq!(dest.correlation_id.as_deref(), Some("keep-me"));
    assert_eq!(dest.handler_type.as_deref(), Some("trigger-timer"));
    assert_eq!(dest.tenant_id.as_deref(), Some("t1"));
    assert_eq!(dest.process_definition_id.as_deref(), Some("pd"));
    assert_eq!(dest.element_name.as_deref(), Some("Timer A"));
    assert_eq!(dest.scope_type.as_deref(), Some("cmmn"));
    assert_eq!(dest.scope_id.as_deref(), Some("case-9"));
    assert_eq!(dest.sub_scope_id.as_deref(), Some("pi-9"));
    assert_eq!(dest.scope_definition_id.as_deref(), Some("cd-9"));
}

#[test]
fn schema_migration_adds_query_dimension_columns() {
    let store = DbStore::new_in_memory().unwrap();
    let mut session = store.create_session().unwrap();
    session
        .execute_raw_sql("DROP TABLE timer_job_states")
        .unwrap();
    session
        .execute_raw_sql(
            "CREATE TABLE timer_job_states (
                id TEXT PRIMARY KEY,
                process_instance_id TEXT,
                execution_id TEXT,
                lock_owner TEXT,
                lock_time INTEGER,
                lock_expiration_time INTEGER,
                retries INTEGER,
                error_message TEXT,
                error_details TEXT,
                due_time INTEGER,
                job_state TEXT,
                data TEXT NOT NULL
            )",
        )
        .unwrap();
    session.flush_and_commit().unwrap();

    // Re-run the same legacy migration path used at DbStore bootstrap.
    // `ensure_legacy_tables` commits its own session work.
    let mut session = store.create_session().unwrap();
    flowable_engine::persistence::db_store::ensure_legacy_tables_for_test(&mut session).unwrap();

    let store = Arc::new(store);
    let engine = ProcessEngine::build(
        "p65-schema-migration".to_string(),
        Arc::new(TestTimeSource::new(
            Utc.with_ymd_and_hms(2026, 7, 30, 12, 0, 0).unwrap(),
        )),
        Arc::clone(&store),
    );
    let runtime = engine.get_runtime_store();
    let mut session = runtime.create_session().unwrap();
    let mut job = sample_job("migrated-job");
    job.job_state = Some("deadletter".to_string());
    runtime.insert_timer_job_state(&job, &mut session);
    session.flush_and_commit().unwrap();

    let mut session = runtime.create_session().unwrap();
    let raw = session
        .find_raw("timer_job_states", "migrated-job")
        .unwrap()
        .expect("migrated insert must succeed");
    for column in [
        "category",
        "scope_id",
        "sub_scope_id",
        "scope_type",
        "scope_definition_id",
        "correlation_id",
        "handler_type",
    ] {
        assert!(
            raw.extras.contains_key(column),
            "expected physical column {column} after migration"
        );
    }
    let _ = session.rollback();
}
