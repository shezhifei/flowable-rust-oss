//! P2-11: runtime job query pushdown contract.
//!
//! Verifies that every predicate, sort, and page slice of `RuntimeJobQuery`
//! is evaluated in SQL (`RuntimeStore::query_runtime_jobs`) with the same
//! semantics the old in-memory implementation had, that legacy rows get the
//! `activity_id` column backfilled from the JSON payload, and that CMMN case
//! definition lookup failures propagate as errors instead of silently
//! returning an empty job list.

use chrono::{DateTime, TimeZone, Utc};
use flowable_engine::engine::management_service::{RuntimeJobFamily, RuntimeJobQuery};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::{RuntimeJobType, RuntimeTimerJobState};
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 30, 12, 0, 0).unwrap()
}

fn engine(name: &str) -> ProcessEngine {
    ProcessEngine::build(
        name.to_string(),
        Arc::new(TestTimeSource::new(now())),
        Arc::new(DbStore::new_in_memory().unwrap()),
    )
}

fn base_job(id: &str) -> RuntimeTimerJobState {
    RuntimeTimerJobState {
        timer_job_id: id.to_string(),
        process_instance_id: "pi-1".to_string(),
        execution_id: "exec-1".to_string(),
        activity_id: "act-1".to_string(),
        job_state: Some("timer".to_string()),
        due_time: Some(now().timestamp_millis()),
        retries: Some(1),
        create_time: Some(now().timestamp_millis()),
        ..Default::default()
    }
}

fn insert(engine: &ProcessEngine, job: RuntimeTimerJobState, job_type: Option<RuntimeJobType>) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state_with_type(&job, job_type.as_ref(), &mut session);
    session.flush_and_commit().unwrap();
}

fn query(engine: &ProcessEngine) -> RuntimeJobQuery {
    engine.get_management_service().create_runtime_job_query()
}

fn ids(jobs: &[RuntimeTimerJobState]) -> Vec<&str> {
    jobs.iter().map(|j| j.timer_job_id.as_str()).collect()
}

#[test]
fn due_date_and_executable_gate_pushdown() {
    let engine = engine("p2_11-due");
    let now_ms = now().timestamp_millis();

    let mut past = base_job("due-past");
    past.due_time = Some(now_ms - 60_000);
    insert(&engine, past, Some(RuntimeJobType::Timer));

    let mut future = base_job("due-future");
    future.due_time = Some(now_ms + 60_000);
    insert(&engine, future, Some(RuntimeJobType::Timer));

    let mut none = base_job("due-none");
    none.due_time = None;
    insert(&engine, none, Some(RuntimeJobType::Timer));

    let executable = query(&engine)
        .family(RuntimeJobFamily::Timer)
        .executable()
        .list()
        .unwrap();
    assert_eq!(ids(&executable), vec!["due-past"], "executable = due in the past only");

    let before = query(&engine)
        .family(RuntimeJobFamily::Timer)
        .due_before(now_ms)
        .list()
        .unwrap();
    assert_eq!(ids(&before), vec!["due-past"]);

    let after = query(&engine)
        .family(RuntimeJobFamily::Timer)
        .due_after(now_ms)
        .list()
        .unwrap();
    assert_eq!(ids(&after), vec!["due-future"]);

    // NULL due dates never match a due-date range predicate.
    let count = query(&engine)
        .family(RuntimeJobFamily::Timer)
        .due_before(now_ms + 120_000)
        .count()
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn retries_exception_and_lock_predicates_pushdown() {
    let engine = engine("p2_11-retries");

    let mut a = base_job("job-a");
    a.retries = None;
    insert(&engine, a, Some(RuntimeJobType::Timer));

    let mut b = base_job("job-b");
    b.retries = Some(0);
    b.error_message = Some("boom".to_string());
    b.lock_owner = Some("node-1".to_string());
    insert(&engine, b, Some(RuntimeJobType::Timer));

    let mut c = base_job("job-c");
    c.retries = Some(2);
    c.error_message = Some(String::new());
    insert(&engine, c, Some(RuntimeJobType::Timer));

    let with_retries = query(&engine).with_retries_left().list().unwrap();
    assert_eq!(ids(&with_retries), vec!["job-a", "job-c"], "NULL retries counts as retries left");

    let no_retries = query(&engine).no_retries_left().list().unwrap();
    assert_eq!(ids(&no_retries), vec!["job-b"]);

    let with_exception = query(&engine).with_exception().list().unwrap();
    assert_eq!(ids(&with_exception), vec!["job-b"], "empty error message is not an exception");

    let without_exception = query(&engine).without_exception().list().unwrap();
    assert_eq!(ids(&without_exception), vec!["job-a", "job-c"]);

    let by_message = query(&engine).exception_message("boom").list().unwrap();
    assert_eq!(ids(&by_message), vec!["job-b"]);

    let locked = query(&engine).locked().list().unwrap();
    assert_eq!(ids(&locked), vec!["job-b"]);

    let unlocked = query(&engine).unlocked().list().unwrap();
    assert_eq!(ids(&unlocked), vec!["job-a", "job-c"]);
}

#[test]
fn tenant_element_and_handler_predicates_pushdown() {
    let engine = engine("p2_11-tenant");

    let mut t1 = base_job("job-t1");
    t1.tenant_id = Some("acme".to_string());
    t1.activity_id = "reviewTask".to_string();
    t1.element_name = Some("Review".to_string());
    t1.handler_type = Some("trigger-timer".to_string());
    insert(&engine, t1, Some(RuntimeJobType::Timer));

    let mut t2 = base_job("job-t2");
    t2.tenant_id = Some("acme-eu".to_string());
    t2.activity_id = "shipTask".to_string();
    t2.handler_type = Some("async-continuation".to_string());
    t2.process_instance_id = String::new();
    insert(&engine, t2, Some(RuntimeJobType::Timer));

    let mut t3 = base_job("job-t3");
    t3.tenant_id = None;
    t3.activity_id = "reviewTask".to_string();
    t3.handler_type = Some("trigger-timer".to_string());
    insert(&engine, t3, Some(RuntimeJobType::Timer));

    let by_tenant = query(&engine).tenant_id("acme").list().unwrap();
    assert_eq!(ids(&by_tenant), vec!["job-t1"]);

    let tenant_like = query(&engine).tenant_id_like("acme%").list().unwrap();
    assert_eq!(ids(&tenant_like), vec!["job-t1", "job-t2"]);

    let without_tenant = query(&engine).without_tenant_id().list().unwrap();
    assert_eq!(ids(&without_tenant), vec!["job-t3"]);

    // elementId is served by the projected activity_id column.
    let by_element = query(&engine).element_id("reviewTask").list().unwrap();
    assert_eq!(ids(&by_element), vec!["job-t1", "job-t3"]);

    let by_element_name = query(&engine).element_name("Review").list().unwrap();
    assert_eq!(ids(&by_element_name), vec!["job-t1"]);

    let by_handlers = query(&engine)
        .handler_types(["trigger-timer", "async-continuation"])
        .list()
        .unwrap();
    assert_eq!(by_handlers.len(), 3);

    let without_pi = query(&engine).without_process_instance_id().list().unwrap();
    assert_eq!(ids(&without_pi), vec!["job-t2"]);
}

#[test]
fn type_flags_use_job_type_column_with_legacy_fallback() {
    let engine = engine("p2_11-flags");

    let mut ja = base_job("flag-a");
    ja.job_state = None;
    insert(&engine, ja, None);

    let mut jb = base_job("flag-b");
    jb.job_state = Some("executable".to_string());
    insert(&engine, jb, None);

    let mut jc = base_job("flag-c");
    jc.job_state = Some("async".to_string());
    jc.handler_type = Some("async-continuation".to_string());
    insert(&engine, jc, None);

    let mut jd = base_job("flag-d");
    jd.job_state = Some("deadletter".to_string());
    jd.handler_type = Some("external-worker-complete".to_string());
    insert(&engine, jd, None);

    let mut je = base_job("flag-e");
    je.job_state = Some("timer".to_string());
    insert(&engine, je, Some(RuntimeJobType::Other("message".to_string())));

    let mut jf = base_job("flag-f");
    jf.job_state = Some("deadletter".to_string());
    insert(&engine, jf, Some(RuntimeJobType::Timer));

    // Persisted job_type wins (flag-e, flag-f); NULL job_type falls back to
    // job_state / handler_type inference (flag-a..d).
    let timers = query(&engine).timers_only().list().unwrap();
    assert_eq!(ids(&timers), vec!["flag-a", "flag-b", "flag-f"]);

    let messages = query(&engine).messages_only().list().unwrap();
    assert_eq!(ids(&messages), vec!["flag-c", "flag-e"]);

    let workers = query(&engine).external_workers().list().unwrap();
    assert_eq!(ids(&workers), vec!["flag-d"]);
}

#[test]
fn nullable_sort_ordering_and_paging_totals() {
    let engine = engine("p2_11-sort");

    let mut s1 = base_job("sort-1");
    s1.due_time = None;
    s1.retries = Some(0);
    insert(&engine, s1, Some(RuntimeJobType::Timer));

    let mut s2 = base_job("sort-2");
    s2.due_time = Some(1_000);
    s2.retries = None;
    insert(&engine, s2, Some(RuntimeJobType::Timer));

    let mut s3 = base_job("sort-3");
    s3.due_time = Some(2_000);
    s3.retries = Some(5);
    insert(&engine, s3, Some(RuntimeJobType::Timer));

    // Option ordering parity: None first ascending, None last descending.
    let asc = query(&engine).order_by("dueDate").asc().list().unwrap();
    assert_eq!(ids(&asc), vec!["sort-1", "sort-2", "sort-3"]);

    let desc = query(&engine).order_by("dueDate").desc().list().unwrap();
    assert_eq!(ids(&desc), vec!["sort-3", "sort-2", "sort-1"]);

    // NULL retries sorts as the default retry count of 1.
    let by_retries = query(&engine).order_by("retries").asc().list().unwrap();
    assert_eq!(ids(&by_retries), vec!["sort-1", "sort-2", "sort-3"]);

    let page = query(&engine)
        .order_by("dueDate")
        .asc()
        .page(1, 1)
        .list_page()
        .unwrap();
    assert_eq!(page.total, 3, "total counts all filtered rows");
    assert_eq!(page.start, 1);
    assert_eq!(ids(&page.data), vec!["sort-2"]);
}

#[test]
fn legacy_rows_get_activity_id_backfilled_from_json() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("legacy-jobs.db");

    // Simulate a pre-P2-11 database: the full legacy shape minus the
    // activity_id column, whose value lives only inside the JSON payload.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE timer_job_states (id TEXT PRIMARY KEY, process_instance_id TEXT, \
             execution_id TEXT, lock_owner TEXT, lock_time BIGINT, lock_expiration_time BIGINT, \
             retries INTEGER, error_message TEXT, error_details TEXT, due_time BIGINT, \
             job_state TEXT, job_type TEXT, create_time BIGINT, correlation_id TEXT, \
             handler_type TEXT, tenant_id TEXT, process_definition_id TEXT, element_name TEXT, \
             category TEXT, scope_id TEXT, sub_scope_id TEXT, scope_type TEXT, \
             scope_definition_id TEXT, data TEXT NOT NULL)",
        )
        .unwrap();
        let mut legacy = base_job("legacy-1");
        legacy.activity_id = "legacy-act".to_string();
        conn.execute(
            "INSERT INTO timer_job_states (id, data) VALUES (?1, ?2)",
            rusqlite::params![
                legacy.timer_job_id,
                serde_json::to_string(&legacy).unwrap()
            ],
        )
        .unwrap();
    }

    let store = DbStore::from_config(flowable_persistence::DatabaseConfig {
        url: db_path.to_string_lossy().to_string(),
        ..Default::default()
    })
    .unwrap();
    let engine = ProcessEngine::build(
        "p2_11-backfill".to_string(),
        Arc::new(TestTimeSource::new(now())),
        Arc::new(store),
    );

    let found = query(&engine).element_id("legacy-act").list().unwrap();
    assert_eq!(ids(&found), vec!["legacy-1"], "schema upgrade must backfill activity_id");
}

#[test]
fn legacy_rows_get_category_and_scope_type_backfilled_from_json() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("legacy-scope-jobs.db");

    // Simulate a pre-projection database missing every column this bootstrap
    // adds: the category / scope_type values live only in the JSON payload.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE timer_job_states (id TEXT PRIMARY KEY, process_instance_id TEXT, \
             execution_id TEXT, lock_owner TEXT, lock_time BIGINT, lock_expiration_time BIGINT, \
             retries INTEGER, error_message TEXT, error_details TEXT, due_time BIGINT, \
             job_state TEXT, job_type TEXT, data TEXT NOT NULL)",
        )
        .unwrap();

        let mut billing = base_job("legacy-billing");
        billing.category = Some("billing".to_string());
        billing.scope_type = Some("cmmn".to_string());
        billing.scope_id = Some("case-1".to_string());

        let mut shipping = base_job("legacy-shipping");
        shipping.category = Some("ship-west".to_string());
        shipping.scope_type = None;

        for legacy in [&billing, &shipping] {
            conn.execute(
                "INSERT INTO timer_job_states (id, data) VALUES (?1, ?2)",
                rusqlite::params![
                    legacy.timer_job_id,
                    serde_json::to_string(legacy).unwrap()
                ],
            )
            .unwrap();
        }
    }

    let store = DbStore::from_config(flowable_persistence::DatabaseConfig {
        url: db_path.to_string_lossy().to_string(),
        ..Default::default()
    })
    .unwrap();
    let engine = ProcessEngine::build(
        "p2_11-scope-backfill".to_string(),
        Arc::new(TestTimeSource::new(now())),
        Arc::new(store),
    );

    let exact = query(&engine).category("billing").list().unwrap();
    assert_eq!(ids(&exact), vec!["legacy-billing"], "category must be backfilled from JSON");

    let like = query(&engine).category_like("ship%").list().unwrap();
    assert_eq!(ids(&like), vec!["legacy-shipping"], "category LIKE must see backfilled values");

    let cmmn = query(&engine).scope_type("cmmn").list().unwrap();
    assert_eq!(ids(&cmmn), vec!["legacy-billing"], "scope_type must be backfilled from JSON");

    let scopeless = query(&engine).without_scope_type().list().unwrap();
    assert_eq!(
        ids(&scopeless),
        vec!["legacy-shipping"],
        "jobs without a JSON scope_type must stay NULL after backfill"
    );
}

#[test]
fn legacy_backfill_preserves_existing_physical_column_values() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("legacy-physical-jobs.db");

    // The category column already exists with a physical value that diverges
    // from the JSON copy; only scope_type (and friends) are missing here.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE timer_job_states (id TEXT PRIMARY KEY, process_instance_id TEXT, \
             execution_id TEXT, lock_owner TEXT, lock_time BIGINT, lock_expiration_time BIGINT, \
             retries INTEGER, error_message TEXT, error_details TEXT, due_time BIGINT, \
             job_state TEXT, job_type TEXT, category TEXT, data TEXT NOT NULL)",
        )
        .unwrap();

        let mut legacy = base_job("legacy-physical");
        legacy.category = Some("json-cat".to_string());
        legacy.scope_type = Some("bpmn".to_string());
        conn.execute(
            "INSERT INTO timer_job_states (id, category, data) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                legacy.timer_job_id,
                "physical-cat",
                serde_json::to_string(&legacy).unwrap()
            ],
        )
        .unwrap();
    }

    let store = DbStore::from_config(flowable_persistence::DatabaseConfig {
        url: db_path.to_string_lossy().to_string(),
        ..Default::default()
    })
    .unwrap();
    let engine = ProcessEngine::build(
        "p2_11-physical-wins".to_string(),
        Arc::new(TestTimeSource::new(now())),
        Arc::new(store),
    );

    // Pre-existing physical value wins over the JSON copy…
    let physical = query(&engine).category("physical-cat").list().unwrap();
    assert_eq!(ids(&physical), vec!["legacy-physical"]);
    let json_copy = query(&engine).category("json-cat").count().unwrap();
    assert_eq!(json_copy, 0, "backfill must not overwrite an existing physical value");

    // …while genuinely new columns are still hydrated from JSON.
    let bpmn = query(&engine).scope_type("bpmn").list().unwrap();
    assert_eq!(ids(&bpmn), vec!["legacy-physical"]);
}

#[test]
fn cmmn_lookup_failure_propagates_instead_of_empty_result() {
    let dir = tempfile::tempdir().unwrap();
    let cmmn_path = dir.path().join("cmmn-jobs.db");
    let cmmn = flowable_cmmn_engine::CmmnEngine::new_sqlite(&cmmn_path).unwrap();

    let mut config = ProcessEngineConfiguration::default();
    config.cmmn_engine = Some(Arc::new(cmmn));
    let engine = ProcessEngine::build_with_db_store_and_config(
        "p2_11-cmmn-err".to_string(),
        Arc::new(TestTimeSource::new(now())),
        Arc::new(DbStore::new_in_memory().unwrap()),
        config,
    )
    .unwrap();
    insert(&engine, base_job("cmmn-job"), Some(RuntimeJobType::Timer));

    // Break the CMMN repository out-of-band: the lookup must surface the
    // failure, never read it as "no case definitions → empty job list".
    rusqlite::Connection::open(&cmmn_path)
        .unwrap()
        .execute_batch("DROP TABLE ACT_CMMN_CASE_DEFINITION")
        .unwrap();

    let err = query(&engine)
        .case_definition_key("orders")
        .list()
        .expect_err("broken CMMN repository must fail the job query");
    assert!(
        err.to_string().contains("Case definition lookup for job query failed"),
        "unexpected error: {err}"
    );
}

#[test]
fn case_definition_key_without_cmmn_engine_matches_nothing() {
    let mut config = ProcessEngineConfiguration::default();
    config.cmmn_engine = None;
    let engine = ProcessEngine::build_with_db_store_and_config(
        "p2_11-cmmn-none".to_string(),
        Arc::new(TestTimeSource::new(now())),
        Arc::new(DbStore::new_in_memory().unwrap()),
        config,
    )
    .unwrap();
    insert(&engine, base_job("job-1"), Some(RuntimeJobType::Timer));

    let page = query(&engine)
        .case_definition_key("orders")
        .list_page()
        .unwrap();
    assert_eq!(page.total, 0, "unresolvable case key must never match all jobs");
    assert!(page.data.is_empty());
}
