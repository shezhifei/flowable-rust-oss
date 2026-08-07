//! Multi-tenant AsyncExecutor job acquisition filter.
//!
//! Java-style tenant-scoped job acquisition: empty `tenant_ids` = all tenants
//! (shared default); non-empty restricts acquire to matching process instances.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::async_executor::AsyncExecutor;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::service::config::{
    AsyncExecutorConfiguration, AsyncExecutorTenantScope, AsyncExecutorTopology,
    ProcessEngineConfiguration,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn now_fixed() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap()
}

fn fast_async_executor_config() -> AsyncExecutorConfiguration {
    AsyncExecutorConfiguration {
        enabled: true,
        // These tests validate tenant scoping, not parallel worker throughput.
        // Shared-cache in-memory SQLite is a single-writer backend, so one
        // worker keeps the fixture deterministic while still exercising the
        // real acquisition and executor lifecycle end to end.
        pool_size: 1,
        queue_size: 64,
        async_job_acquire_wait_ms: 50,
        timer_job_acquire_wait_ms: 50,
        queue_full_wait_ms: 50,
        max_jobs_per_acquisition: 16,
        async_job_lock_time_ms: 5_000,
        timer_lock_time_ms: 5_000,
        reset_expired_jobs_interval_ms: 50,
        reset_expired_jobs_page_size: 50,
        async_job_acquisition_enabled: true,
        timer_job_acquisition_enabled: true,
        reset_expired_job_enabled: true,
        tenant_ids: Vec::new(),
        ..AsyncExecutorConfiguration::default()
    }
}

fn async_continuation_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="multiTenantAsyncContinuation" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="asyncTask" />
        <serviceTask id="asyncTask" flowable:async="true" />
        <sequenceFlow id="flow2" sourceRef="asyncTask" targetRef="afterAsyncTask" />
        <userTask id="afterAsyncTask" name="After Async" />
        <sequenceFlow id="flow3" sourceRef="afterAsyncTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#
}

fn deploy(engine: &ProcessEngine, xml: &str, resource_name: &str) {
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(resource_name.to_string())
                .add_string(resource_name.to_string(), xml.to_string()),
        )
        .unwrap();
}

fn process_definition_id(engine: &ProcessEngine) -> String {
    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone()
}

fn start_with_tenant(
    engine: &ProcessEngine,
    process_definition_id: String,
    tenant_id: &str,
) -> String {
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .tenant_id(tenant_id.to_string()),
        )
        .unwrap()
        .id
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    predicate()
}

fn job_count_for_pi(engine: &ProcessEngine, pi_id: &str) -> usize {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let jobs = store.find_timer_job_states_by_process_instance_id(pi_id, &mut session);
    session.rollback().unwrap();
    jobs.len()
}

fn only_job_for_pi(
    engine: &ProcessEngine,
    pi_id: &str,
) -> flowable_engine::persistence::runtime_store::RuntimeTimerJobState {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut jobs = store.find_timer_job_states_by_process_instance_id(pi_id, &mut session);
    session.rollback().unwrap();
    assert_eq!(jobs.len(), 1, "expected exactly one async job for {pi_id}");
    jobs.remove(0)
}

fn has_user_task(engine: &ProcessEngine, pi_id: &str) -> bool {
    !engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id.to_string())
        .unwrap()
        .is_empty()
}

#[test]
fn default_tenant_ids_is_empty_shared_mode() {
    let config = AsyncExecutorConfiguration::default();
    assert!(
        config.tenant_ids.is_empty(),
        "default must remain shared (all tenants)"
    );

    let executor = AsyncExecutor::new(config);
    assert!(executor.tenant_ids().is_empty());
}

#[test]
fn for_tenant_sets_single_tenant_filter() {
    let config = AsyncExecutorConfiguration {
        enabled: true,
        ..AsyncExecutorConfiguration::default()
    };
    let executor = AsyncExecutor::for_tenant(config, "tenant-a".to_string());
    assert_eq!(executor.tenant_ids(), &["tenant-a".to_string()]);
    assert_eq!(
        executor.configuration().tenant_ids,
        vec!["tenant-a".to_string()]
    );
}

/// Direct acquisition API: filtered acquire only returns jobs for matching tenants.
#[test]
fn acquire_async_jobs_respects_tenant_filter() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let engine = ProcessEngine::build_with_config(
        "mt-acquire-filter".to_string(),
        time_source,
        ProcessEngineConfiguration::default(),
    )
    .unwrap();

    deploy(
        &engine,
        async_continuation_xml(),
        "mt-async-acquire.bpmn20.xml",
    );
    let def_id = process_definition_id(&engine);

    let pi_a = start_with_tenant(&engine, def_id.clone(), "tenant-a");
    let pi_b = start_with_tenant(&engine, def_id, "tenant-b");

    assert_eq!(job_count_for_pi(&engine, &pi_a), 1);
    assert_eq!(job_count_for_pi(&engine, &pi_b), 1);

    let runtime = engine.get_runtime_service();

    // Tenant A filter acquires only A.
    let acquired_a =
        runtime.acquire_async_jobs_for_tenants(5_000, 10, &["tenant-a".to_string()], &[]);
    assert_eq!(
        acquired_a.len(),
        1,
        "tenant-a filter should acquire exactly one job"
    );
    assert_eq!(acquired_a[0].process_instance_id, pi_a);

    // Release so B can still be acquired (and re-check empty filter).
    assert!(runtime.release_timer_job_lock(&acquired_a[0].timer_job_id));

    let acquired_b =
        runtime.acquire_async_jobs_for_tenants(5_000, 10, &["tenant-b".to_string()], &[]);
    assert_eq!(acquired_b.len(), 1);
    assert_eq!(acquired_b[0].process_instance_id, pi_b);
    assert!(runtime.release_timer_job_lock(&acquired_b[0].timer_job_id));

    // Empty tenant_ids (shared) acquires both.
    let acquired_all = runtime.acquire_async_jobs_for_tenants(5_000, 10, &[], &[]);
    assert_eq!(
        acquired_all.len(),
        2,
        "empty tenant filter must acquire jobs for all tenants"
    );
    let mut pi_ids: Vec<_> = acquired_all
        .iter()
        .map(|j| j.process_instance_id.clone())
        .collect();
    pi_ids.sort();
    let mut expected = vec![pi_a, pi_b];
    expected.sort();
    assert_eq!(pi_ids, expected);
}

/// RuntimeStore filtered path used by timer acquisition also scopes by tenant.
#[test]
fn acquire_due_timer_jobs_filtered_respects_tenant() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let engine = ProcessEngine::build_with_config(
        "mt-store-filter".to_string(),
        time_source.clone(),
        ProcessEngineConfiguration::default(),
    )
    .unwrap();

    deploy(
        &engine,
        async_continuation_xml(),
        "mt-store-filter.bpmn20.xml",
    );
    let def_id = process_definition_id(&engine);
    let pi_a = start_with_tenant(&engine, def_id.clone(), "tenant-a");
    let pi_b = start_with_tenant(&engine, def_id, "tenant-b");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let now = time_source.now().timestamp_millis();

    let (acquired, _, _) = store
        .acquire_due_async_timer_jobs_filtered(
            "owner-a",
            now,
            5_000,
            10,
            Some(&["tenant-a".to_string()]),
            None,
            &mut session,
        )
        .unwrap();
    session.flush_and_commit().unwrap();

    assert_eq!(acquired.len(), 1);
    assert_eq!(acquired[0].process_instance_id, pi_a);
    assert_ne!(acquired[0].process_instance_id, pi_b);

    // B job must still be unlocked / available.
    assert_eq!(job_count_for_pi(&engine, &pi_b), 1);
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let jobs_b = store.find_timer_job_states_by_process_instance_id(&pi_b, &mut session);
    session.rollback().unwrap();
    assert!(
        jobs_b[0].lock_owner.is_none(),
        "tenant-b job must not be locked by tenant-a acquisition"
    );
}

/// Executor configured for tenant A executes only A; B remains waiting.
#[test]
fn async_executor_tenant_filter_executes_only_matching_tenant() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor = fast_async_executor_config();
    config.async_executor.tenant_ids = vec!["tenant-a".to_string()];

    let engine =
        ProcessEngine::build_with_config("mt-executor-filter".to_string(), time_source, config)
            .unwrap();

    let async_exec = engine.get_async_executor().expect("async executor enabled");
    assert_eq!(async_exec.tenant_ids(), &["tenant-a".to_string()]);

    deploy(
        &engine,
        async_continuation_xml(),
        "mt-executor-filter.bpmn20.xml",
    );
    let def_id = process_definition_id(&engine);
    let pi_a = start_with_tenant(&engine, def_id.clone(), "tenant-a");
    let pi_b = start_with_tenant(&engine, def_id, "tenant-b");

    assert_eq!(job_count_for_pi(&engine, &pi_a), 1);
    assert_eq!(job_count_for_pi(&engine, &pi_b), 1);

    engine.start_timer_executor();

    let a_advanced = wait_until(Duration::from_secs(5), || has_user_task(&engine, &pi_a));
    // Give the executor a moment; B must not advance under tenant-a filter.
    std::thread::sleep(Duration::from_millis(300));
    let b_advanced = has_user_task(&engine, &pi_b);
    engine.stop_timer_executor();

    assert!(
        a_advanced,
        "tenant-a async job should be executed by tenant-scoped AsyncExecutor"
    );
    assert!(
        !b_advanced,
        "tenant-b async job must not be executed by tenant-a AsyncExecutor"
    );
    assert_eq!(
        job_count_for_pi(&engine, &pi_b),
        1,
        "tenant-b job should still be pending"
    );
}

/// Shared executor (empty tenant_ids) executes jobs for every tenant.
#[test]
fn async_executor_empty_tenant_ids_executes_all_tenants() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor = fast_async_executor_config();
    assert!(
        config.async_executor.tenant_ids.is_empty(),
        "shared mode uses empty tenant_ids"
    );

    let engine =
        ProcessEngine::build_with_config("mt-executor-shared".to_string(), time_source, config)
            .unwrap();

    deploy(
        &engine,
        async_continuation_xml(),
        "mt-executor-shared.bpmn20.xml",
    );
    let def_id = process_definition_id(&engine);
    let pi_a = start_with_tenant(&engine, def_id.clone(), "tenant-a");
    let pi_b = start_with_tenant(&engine, def_id, "tenant-b");

    engine.start_timer_executor();

    let mut a_done = false;
    let mut b_done = false;
    // Under the full engine suite, other executor-heavy tests can delay one
    // tenant until after the 5-second test lock expires and is reset. Keep the
    // assertion bounded, but allow the next acquisition cycle to complete.
    let both_advanced = wait_until(Duration::from_secs(15), || {
        a_done = has_user_task(&engine, &pi_a);
        b_done = has_user_task(&engine, &pi_b);
        a_done && b_done
    });
    engine.stop_timer_executor();

    assert!(
        both_advanced,
        "shared AsyncExecutor (empty tenant_ids) should execute jobs for all tenants (a_done={a_done}, b_done={b_done})"
    );
}

#[test]
fn unlock_owned_jobs_respects_lifecycle_owner_and_tenant_scope() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor = AsyncExecutorConfiguration {
        enabled: true,
        auto_activate: false,
        lock_owner: Some("tenant-a-executor".to_string()),
        unlock_owned_jobs: true,
        async_job_acquisition_enabled: false,
        timer_job_acquisition_enabled: false,
        reset_expired_job_enabled: false,
        tenant_ids: vec!["tenant-a".to_string()],
        ..AsyncExecutorConfiguration::default()
    };
    let engine = ProcessEngine::build_with_config(
        "mt-unlock-owned-jobs".to_string(),
        time_source.clone(),
        config,
    )
    .unwrap();

    deploy(
        &engine,
        async_continuation_xml(),
        "mt-unlock-owned-jobs.bpmn20.xml",
    );
    let def_id = process_definition_id(&engine);
    let pi_a = start_with_tenant(&engine, def_id.clone(), "tenant-a");
    let pi_b = start_with_tenant(&engine, def_id, "tenant-b");
    let runtime = engine.get_runtime_service();

    let acquired = runtime.acquire_async_jobs_for_tenants(5_000, 10, &[], &[]);
    assert_eq!(acquired.len(), 2);
    assert!(only_job_for_pi(&engine, &pi_a).lock_time.is_some());
    assert!(only_job_for_pi(&engine, &pi_b).lock_time.is_some());

    engine.start_timer_executor();
    let startup_a = only_job_for_pi(&engine, &pi_a);
    let startup_b = only_job_for_pi(&engine, &pi_b);
    assert!(startup_a.lock_owner.is_none());
    assert!(startup_a.lock_time.is_none());
    assert!(startup_a.lock_expiration_time.is_none());
    assert_eq!(startup_b.lock_owner.as_deref(), Some("tenant-a-executor"));

    let reacquired_a =
        runtime.acquire_async_jobs_for_tenants(5_000, 10, &["tenant-a".to_string()], &[]);
    assert_eq!(reacquired_a.len(), 1);
    engine.start_timer_executor();
    assert_eq!(
        only_job_for_pi(&engine, &pi_a).lock_owner.as_deref(),
        Some("tenant-a-executor"),
        "repeated start while active must be a no-op"
    );

    engine.stop_timer_executor();
    let shutdown_a = only_job_for_pi(&engine, &pi_a);
    let shutdown_b = only_job_for_pi(&engine, &pi_b);
    assert!(shutdown_a.lock_owner.is_none());
    assert!(shutdown_a.lock_time.is_none());
    assert!(shutdown_a.lock_expiration_time.is_none());
    assert_eq!(shutdown_b.lock_owner.as_deref(), Some("tenant-a-executor"));

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let now = time_source.now().timestamp_millis();
    let (other_owner_jobs, _, _) = store
        .acquire_due_async_timer_jobs_filtered(
            "other-owner",
            now,
            5_000,
            10,
            Some(&["tenant-a".to_string()]),
            None,
            &mut session,
        )
        .unwrap();
    session.flush_and_commit().unwrap();
    assert_eq!(other_owner_jobs.len(), 1);

    engine.start_timer_executor();
    assert_eq!(
        only_job_for_pi(&engine, &pi_a).lock_owner.as_deref(),
        Some("other-owner"),
        "startup must not release a different executor owner's lock"
    );
    engine.stop_timer_executor();
    assert_eq!(
        only_job_for_pi(&engine, &pi_a).lock_owner.as_deref(),
        Some("other-owner"),
        "shutdown must not release a different executor owner's lock"
    );
}

#[test]
fn shared_multi_tenant_explicit_unlock_is_shutdown_only_for_registered_tenants() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut async_executor = AsyncExecutorConfiguration::default()
        .shared_multi_tenant()
        .with_tenant_scope(AsyncExecutorTenantScope::Tenants(vec![
            "tenant-a".to_string(),
            "tenant-b".to_string(),
        ]));
    assert!(!async_executor.unlock_owned_jobs);
    async_executor.enabled = true;
    async_executor.lock_owner = Some("shared-multi-tenant-owner".to_string());
    async_executor.unlock_owned_jobs = true;
    async_executor.async_job_acquisition_enabled = false;
    async_executor.timer_job_acquisition_enabled = false;
    async_executor.reset_expired_job_enabled = false;
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor = async_executor;
    let engine = ProcessEngine::build_with_config(
        "shared-multi-tenant-unlock".to_string(),
        time_source,
        config,
    )
    .unwrap();

    let executor = engine.get_async_executor().unwrap();
    assert_eq!(
        executor.topology(),
        AsyncExecutorTopology::SharedMultiTenant
    );
    assert!(!executor.unlocks_owned_jobs_on_start());
    assert!(executor.unlocks_owned_jobs_on_shutdown());

    deploy(
        &engine,
        async_continuation_xml(),
        "shared-multi-tenant-unlock.bpmn20.xml",
    );
    let def_id = process_definition_id(&engine);
    let pi_a = start_with_tenant(&engine, def_id.clone(), "tenant-a");
    let pi_b = start_with_tenant(&engine, def_id.clone(), "tenant-b");
    let pi_c = start_with_tenant(&engine, def_id, "tenant-c");
    let acquired = engine
        .get_runtime_service()
        .acquire_async_jobs_for_tenants(5_000, 10, &[], &[]);
    assert_eq!(acquired.len(), 3);

    engine.start_timer_executor();
    for process_instance_id in [&pi_a, &pi_b, &pi_c] {
        assert_eq!(
            only_job_for_pi(&engine, process_instance_id)
                .lock_owner
                .as_deref(),
            Some("shared-multi-tenant-owner"),
            "shared topology must never unlock during startup"
        );
    }

    engine.stop_timer_executor();
    assert!(only_job_for_pi(&engine, &pi_a).lock_owner.is_none());
    assert!(only_job_for_pi(&engine, &pi_b).lock_owner.is_none());
    assert_eq!(
        only_job_for_pi(&engine, &pi_c).lock_owner.as_deref(),
        Some("shared-multi-tenant-owner"),
        "unregistered tenant must remain locked during shared shutdown"
    );
}

#[test]
fn reset_expired_jobs_respects_tenant_scope() {
    use flowable_engine::persistence::runtime_store::{ExpiredJobClass, RuntimeTimerJobState};
    use flowable_engine::runtime::process_instance::ProcessInstance;

    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor.enabled = false;
    let engine = ProcessEngine::build_with_config(
        "reset-expired-tenant-scope".to_string(),
        time_source.clone(),
        config,
    )
    .unwrap();
    let store = engine.get_runtime_store();
    let now_ms = time_source.now().timestamp_millis();

    let mut session = store.create_session().unwrap();
    for (pi_id, tenant) in [("pi-a", "tenant-a"), ("pi-b", "tenant-b")] {
        store.insert_process_instance(
            &ProcessInstance {
                id: pi_id.to_string(),
                name: None,
                process_definition_id: "pd".to_string(),
                process_definition_key: "pd".to_string(),
                process_definition_name: None,
                process_definition_version: 1,
                business_key: None,
                business_status: None,
                is_suspended: false,
                tenant_id: Some(tenant.to_string()),
                start_time: None,
                start_user_id: None,
                callback_id: None,
                callback_type: None,
                reference_id: None,
                reference_type: None,
                is_ended: false,
                super_execution_id: None,
                root_process_instance_id: None,
            },
            &mut session,
        );
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: format!("job-{tenant}"),
                process_instance_id: pi_id.to_string(),
                execution_id: format!("ex-{tenant}"),
                activity_id: "asyncTask".to_string(),
                job_state: Some("async".to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                due_time: Some(now_ms - 2_000),
                lock_owner: Some("dead-owner".to_string()),
                lock_time: Some(now_ms - 2_000),
                lock_expiration_time: Some(now_ms - 1_000),
                retries: Some(3),
                error_message: None,
                error_details: None,
                category: None,
                ..Default::default()
            },
            &mut session,
        );
    }
    session.flush_and_commit().unwrap();

    let outcome = engine
        .get_runtime_service()
        .reset_expired_jobs_batch_scoped(ExpiredJobClass::Async, 10, &["tenant-a".to_string()], &[])
        .unwrap();
    assert_eq!(outcome.reset, 1);
    assert_eq!(outcome.scanned, 1);

    let mut session = store.create_session().unwrap();
    let job_a = store
        .find_timer_job_state("job-tenant-a", &mut session)
        .unwrap();
    let job_b = store
        .find_timer_job_state("job-tenant-b", &mut session)
        .unwrap();
    session.rollback().unwrap();
    assert!(job_a.lock_owner.is_none(), "tenant-a job must be reset");
    assert_eq!(
        job_b.lock_owner.as_deref(),
        Some("dead-owner"),
        "tenant-b job must not be reset by tenant-a scope"
    );
}
