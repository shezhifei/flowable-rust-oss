//! P65-job-query engine contract: category/scope/correlation/external-worker
//! and withoutScope predicates across deadletter + suspended families.

use flowable_engine::engine::management_service::{RuntimeJobFamily, RuntimeJobQuery};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::{RuntimeJobType, RuntimeTimerJobState};
use chrono::{TimeZone, Utc};
use std::sync::Arc;

fn engine(name: &str) -> ProcessEngine {
    let now = Utc.with_ymd_and_hms(2026, 7, 30, 12, 0, 0).unwrap();
    ProcessEngine::build(
        name.to_string(),
        Arc::new(TestTimeSource::new(now)),
        Arc::new(DbStore::new_in_memory().unwrap()),
    )
}

fn base_job(id: &str, state: &str) -> RuntimeTimerJobState {
    RuntimeTimerJobState {
        timer_job_id: id.to_string(),
        process_instance_id: "pi-1".to_string(),
        execution_id: "exec-1".to_string(),
        activity_id: "act-1".to_string(),
        job_state: Some(state.to_string()),
        due_time: Some(1_722_000_000_000),
        retries: Some(1),
        create_time: Some(1_722_000_000_000),
        correlation_id: Some(format!("corr-{id}")),
        ..Default::default()
    }
}

fn insert(engine: &ProcessEngine, job: RuntimeTimerJobState, job_type: Option<RuntimeJobType>) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state_with_type(&job, job_type.as_ref(), &mut session);
    session.flush_and_commit().unwrap();
}

fn seed_dimension_jobs(engine: &ProcessEngine, family_state: &str) {
    let mut scoped = base_job(&format!("{family_state}-scoped"), family_state);
    scoped.category = Some("orders".to_string());
    scoped.scope_type = Some("cmmn".to_string());
    scoped.scope_id = Some("case-1".to_string());
    scoped.sub_scope_id = Some("plan-1".to_string());
    scoped.scope_definition_id = Some("case-def-orders".to_string());
    scoped.correlation_id = Some("corr-scoped".to_string());
    scoped.handler_type = Some("external-worker-complete".to_string());
    insert(
        engine,
        scoped,
        Some(RuntimeJobType::ExternalWorker),
    );

    let mut billing = base_job(&format!("{family_state}-billing"), family_state);
    billing.category = Some("billing".to_string());
    billing.scope_type = Some("cmmn".to_string());
    billing.scope_id = Some("case-2".to_string());
    billing.sub_scope_id = Some("plan-2".to_string());
    billing.scope_definition_id = Some("case-def-billing".to_string());
    billing.correlation_id = Some("corr-billing".to_string());
    billing.handler_type = Some("async-continuation".to_string());
    insert(engine, billing, Some(RuntimeJobType::Other("message".into())));

    let mut plain = base_job(&format!("{family_state}-plain"), family_state);
    plain.category = Some("orders-extra".to_string());
    plain.correlation_id = Some("corr-plain".to_string());
    plain.handler_type = Some("trigger-timer".to_string());
    // no scope_id / scope_type
    insert(engine, plain, Some(RuntimeJobType::Timer));
}

fn query_family(engine: &ProcessEngine, family: RuntimeJobFamily) -> RuntimeJobQuery {
    engine
        .get_management_service()
        .create_runtime_job_query()
        .family(family)
}

#[test]
fn category_exact_and_like_filter_deadletter() {
    let engine = engine("p65-category-dl");
    seed_dimension_jobs(&engine, "deadletter");

    let exact = query_family(&engine, RuntimeJobFamily::Deadletter)
        .category("orders")
        .list()
        .unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].timer_job_id, "deadletter-scoped");

    let like = query_family(&engine, RuntimeJobFamily::Deadletter)
        .category_like("order%")
        .list()
        .unwrap();
    assert_eq!(like.len(), 2);
    let ids: Vec<_> = like.iter().map(|j| j.timer_job_id.as_str()).collect();
    assert!(ids.contains(&"deadletter-scoped"));
    assert!(ids.contains(&"deadletter-plain"));
}

#[test]
fn scope_dimensions_filter_suspended() {
    let engine = engine("p65-scope-susp");
    seed_dimension_jobs(&engine, "suspended");

    let by_scope = query_family(&engine, RuntimeJobFamily::Suspended)
        .scope_id("case-1")
        .list()
        .unwrap();
    assert_eq!(by_scope.len(), 1);
    assert_eq!(by_scope[0].sub_scope_id.as_deref(), Some("plan-1"));

    let by_sub = query_family(&engine, RuntimeJobFamily::Suspended)
        .sub_scope_id("plan-2")
        .list()
        .unwrap();
    assert_eq!(by_sub.len(), 1);
    assert_eq!(by_sub[0].timer_job_id, "suspended-billing");

    let by_type = query_family(&engine, RuntimeJobFamily::Suspended)
        .scope_type("cmmn")
        .list()
        .unwrap();
    assert_eq!(by_type.len(), 2);

    let by_def = query_family(&engine, RuntimeJobFamily::Suspended)
        .scope_definition_id("case-def-orders")
        .list()
        .unwrap();
    assert_eq!(by_def.len(), 1);
    assert_eq!(by_def[0].timer_job_id, "suspended-scoped");
}

#[test]
fn correlation_and_external_workers_filter() {
    let engine = engine("p65-corr-ew");
    seed_dimension_jobs(&engine, "deadletter");

    let by_corr = query_family(&engine, RuntimeJobFamily::Deadletter)
        .correlation_id("corr-billing")
        .list()
        .unwrap();
    assert_eq!(by_corr.len(), 1);
    assert_eq!(by_corr[0].timer_job_id, "deadletter-billing");

    let ew = query_family(&engine, RuntimeJobFamily::Deadletter)
        .external_workers()
        .list()
        .unwrap();
    assert_eq!(ew.len(), 1);
    assert_eq!(ew[0].timer_job_id, "deadletter-scoped");
    assert_eq!(
        ew[0].handler_type.as_deref(),
        Some("external-worker-complete")
    );
}

#[test]
fn case_definition_key_resolves_ids_before_filter() {
    let engine = engine("p65-case-key");
    seed_dimension_jobs(&engine, "deadletter");

    // Inject resolved IDs (simulates CMMN lookup of key "orders" → case-def-orders).
    let matched = query_family(&engine, RuntimeJobFamily::Deadletter)
        .case_definition_key("orders")
        .case_definition_ids(["case-def-orders"])
        .list()
        .unwrap();
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].timer_job_id, "deadletter-scoped");

    // Key with no resolved IDs yields empty (never silently matches all).
    let empty = query_family(&engine, RuntimeJobFamily::Deadletter)
        .case_definition_key("missing")
        .list()
        .unwrap();
    assert!(empty.is_empty());
}

#[test]
fn without_scope_id_and_type_are_real_predicates() {
    let engine = engine("p65-without-scope");
    seed_dimension_jobs(&engine, "deadletter");
    seed_dimension_jobs(&engine, "suspended");

    for family in [RuntimeJobFamily::Deadletter, RuntimeJobFamily::Suspended] {
        let without_id = query_family(&engine, family)
            .without_scope_id()
            .list()
            .unwrap();
        assert_eq!(
            without_id.len(),
            1,
            "only the plain job has no scope_id for {family:?}"
        );
        assert!(
            without_id[0].timer_job_id.ends_with("-plain"),
            "unexpected job {:?}",
            without_id[0].timer_job_id
        );

        let without_type = query_family(&engine, family)
            .without_scope_type()
            .list()
            .unwrap();
        assert_eq!(without_type.len(), 1);
        assert!(without_type[0].timer_job_id.ends_with("-plain"));
    }
}

#[test]
fn paging_totals_apply_after_filters_with_id_tiebreak() {
    let engine = engine("p65-paging");
    seed_dimension_jobs(&engine, "deadletter");

    // Two jobs match order%
    let page = query_family(&engine, RuntimeJobFamily::Deadletter)
        .category_like("order%")
        .order_by("id")
        .asc()
        .page(0, 1)
        .list_page()
        .unwrap();
    assert_eq!(page.total, 2, "total must count filtered rows, not page size");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.start, 0);
    assert_eq!(page.size, 1);

    let page2 = query_family(&engine, RuntimeJobFamily::Deadletter)
        .category_like("order%")
        .order_by("id")
        .asc()
        .page(1, 1)
        .list_page()
        .unwrap();
    assert_eq!(page2.total, 2);
    assert_eq!(page2.data.len(), 1);
    assert_ne!(page.data[0].timer_job_id, page2.data[0].timer_job_id);

    // Same due/create values → id tie-break keeps order stable.
    let ordered = query_family(&engine, RuntimeJobFamily::Deadletter)
        .order_by("dueDate")
        .asc()
        .list()
        .unwrap();
    let ids: Vec<_> = ordered.iter().map(|j| j.timer_job_id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

#[test]
fn incompatible_type_flags_are_rejected() {
    let engine = engine("p65-flags");
    let err = query_family(&engine, RuntimeJobFamily::Deadletter)
        .timers_only()
        .external_workers()
        .list()
        .expect_err("timers + externalWorkers must be rejected");
    assert!(err.to_string().contains("externalWorkers"));
}
