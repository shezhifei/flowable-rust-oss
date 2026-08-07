//! MySQL integration tests for ProcessEngine multi-backend assembly.
//!
//! Requires a reachable MySQL instance. Defaults to:
//! `mysql://flowable:flowable@localhost:3306/flowable_test`
//! Override with `FLOWABLE_TEST_MYSQL_URL`.
//!
//! Tests **skip gracefully** when the database is unreachable so default CI
//! without MySQL does not fail.
//!
//! ```powershell
//! $env:FLOWABLE_TEST_MYSQL_URL = "mysql://user:pass@localhost:3306/flowable_test"
//! cargo test -p flowable-engine --features mysql --test mysql_engine_integration_test
//! ```

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::persistence::db_session::DbParams;
use flowable_engine::persistence::runtime_store::EventRegistryChangeRecord;
use flowable_engine::service::config::{
    DatabaseConfiguration, EngineDatabaseKind, ProcessEngineConfiguration,
};
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

static MYSQL_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Cached availability probe so we only attempt a connection once per process
/// when the DB is down (keeps skip logs quiet and suite fast).
static MYSQL_AVAILABLE: OnceLock<bool> = OnceLock::new();

fn mysql_url() -> String {
    std::env::var("FLOWABLE_TEST_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://flowable:flowable@localhost:3306/flowable_test".to_string())
}

fn lock_mysql() -> std::sync::MutexGuard<'static, ()> {
    MYSQL_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn mysql_available() -> bool {
    *MYSQL_AVAILABLE.get_or_init(|| {
        let config = ProcessEngineConfiguration {
            database: DatabaseConfiguration {
                kind: EngineDatabaseKind::Mysql,
                url: mysql_url(),
                pool_size: 1,
                busy_timeout_ms: 2000,
                journal_mode: Default::default(),
            },
            ..Default::default()
        };
        match ProcessEngine::build_with_config(
            "mysql-availability-probe".to_string(),
            Arc::new(SystemTimeSource),
            config,
        ) {
            Ok(_) => true,
            Err(err) => {
                eprintln!(
                    "Skipping MySQL engine integration tests: database unreachable ({err}). \
                     Set FLOWABLE_TEST_MYSQL_URL to a live instance to run them."
                );
                false
            }
        }
    })
}

/// Build a MySQL-backed engine, or return `None` (after logging) when DB is down.
fn try_build_mysql_engine(name: &str) -> Option<ProcessEngine> {
    if !mysql_available() {
        return None;
    }
    let config = ProcessEngineConfiguration {
        database: DatabaseConfiguration {
            kind: EngineDatabaseKind::Mysql,
            url: mysql_url(),
            pool_size: 2,
            busy_timeout_ms: 5000,
            journal_mode: Default::default(),
        },
        ..Default::default()
    };
    match ProcessEngine::build_with_config(name.to_string(), Arc::new(SystemTimeSource), config) {
        Ok(engine) => Some(engine),
        Err(err) => {
            eprintln!("Skipping MySQL test '{name}': failed to build engine ({err})");
            None
        }
    }
}

fn simple_process_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="{process_id}" name="{process_id}">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="First Task" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="end1" />
            <endEvent id="end1" />
        </process>
    </definitions>"#
    )
}

fn timer_process_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="Examples">
        <process id="{process_id}" name="{process_id}" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="timerCatch" />
            <intermediateCatchEvent id="timerCatch">
                <timerEventDefinition>
                    <timeDuration>PT5M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="timerCatch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

fn extract_process_definition_version(process_definition_id: &str) -> i32 {
    process_definition_id
        .split(':')
        .nth(1)
        .expect("process definition id should contain a version segment")
        .parse()
        .expect("version segment should be an integer")
}

// ---------------------------------------------------------------------------
// Existing contract cases
// ---------------------------------------------------------------------------

#[test]
fn mysql_deploy_and_query_resources() {
    let _guard = lock_mysql();
    let Some(engine) = try_build_mysql_engine("mysql-repo-resources") else {
        return;
    };
    let repository_service = engine.get_repository_service();

    let process_key = format!("mysqlResource_{}", Uuid::new_v4().simple());
    let deployment = repository_service
        .create_deployment()
        .name("MySQL Resource Deployment".to_string())
        .add_string(
            "resource-process.bpmn20.xml".to_string(),
            simple_process_xml(&process_key),
        );
    let deployment = repository_service.deploy(deployment).expect("deploy");
    let resource_names = repository_service
        .get_deployment_resource_names(&deployment.id)
        .expect("resource names");

    assert_eq!(
        resource_names,
        vec!["resource-process.bpmn20.xml".to_string()]
    );
}

#[test]
fn mysql_delete_deployment_removes_process_definitions() {
    let _guard = lock_mysql();
    let Some(engine) = try_build_mysql_engine("mysql-repo-delete") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let process_key = format!("mysqlDeletable_{}", Uuid::new_v4().simple());

    let deployment = repository_service
        .create_deployment()
        .name("MySQL Deletable Deployment".to_string())
        .add_string(
            "deletable-process.bpmn20.xml".to_string(),
            simple_process_xml(&process_key),
        );
    let deployment = repository_service.deploy(deployment).expect("deploy");

    let defs_for_key = repository_service
        .get_process_definitions()
        .expect("defs")
        .into_iter()
        .filter(|d| d.key == process_key)
        .count();
    assert_eq!(defs_for_key, 1);

    repository_service
        .delete_deployment(&deployment.id)
        .expect("delete deployment");

    let remaining = repository_service
        .get_process_definitions()
        .expect("defs after delete")
        .into_iter()
        .filter(|d| d.key == process_key)
        .count();
    assert_eq!(remaining, 0);
    assert!(
        repository_service
            .get_deployment_resource_names(&deployment.id)
            .expect("resources after delete")
            .is_empty()
    );
}

#[test]
fn mysql_dual_write_populates_normalized_act_tables() {
    let _guard = lock_mysql();
    let Some(engine) = try_build_mysql_engine("mysql-dual-write") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let process_key = format!("mysqlDualWrite_{}", Uuid::new_v4().simple());

    let builder = repository_service
        .create_deployment()
        .name("MySQL Dual Write Deployment".to_string())
        .add_string(
            "dual.bpmn20.xml".to_string(),
            simple_process_xml(&process_key),
        );
    let deployment = repository_service.deploy(builder).expect("deploy");

    let process_definition_id = repository_service
        .get_process_definitions()
        .expect("defs")
        .into_iter()
        .find(|d| d.key == process_key)
        .expect("definition")
        .id;

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("MySQL Dual Write Instance".to_string());
    let process_instance = runtime_service
        .start_process_instance(builder)
        .expect("start");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("session");

    let dep = flowable_persistence::DeploymentDataManager::new()
        .find_by_id(session.inner_mut(), &deployment.id)
        .expect("query deployment")
        .expect("ACT_RE_DEPLOYMENT row");
    assert_eq!(dep.id, deployment.id);

    let pd = flowable_persistence::ProcessDefinitionDataManager::new()
        .find_by_id(session.inner_mut(), &process_definition_id)
        .expect("query procdef")
        .expect("ACT_RE_PROCDEF row");
    assert_eq!(pd.key, process_key);
    assert_eq!(pd.deployment_id.as_deref(), Some(deployment.id.as_str()));

    let ex = flowable_persistence::ExecutionDataManager::new()
        .find_by_id(session.inner_mut(), &process_instance.id)
        .expect("query execution")
        .expect("ACT_RU_EXECUTION row");
    assert_eq!(ex.id, process_instance.id);
    assert_eq!(ex.activity_id.as_deref(), Some("userTask1"));
    assert_eq!(
        ex.process_definition_id.as_deref(),
        Some(process_definition_id.as_str())
    );
}

#[test]
fn mysql_runtime_state_persists_after_start() {
    let _guard = lock_mysql();
    let Some(engine) = try_build_mysql_engine("mysql-runtime-state") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let process_key = format!("mysqlRuntime_{}", Uuid::new_v4().simple());

    let builder = repository_service
        .create_deployment()
        .name("MySQL Runtime State Deployment".to_string())
        .add_string(
            "myProcess.bpmn20.xml".to_string(),
            simple_process_xml(&process_key),
        );
    repository_service.deploy(builder).expect("deploy");

    let process_definition_id = repository_service
        .get_process_definitions()
        .expect("defs")
        .into_iter()
        .find(|d| d.key == process_key)
        .expect("deployed definition")
        .id;

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("MySQL Runtime State Instance".to_string());
    let process_instance = runtime_service
        .start_process_instance(builder)
        .expect("start");

    let shared_runtime_store = engine.get_runtime_store();
    let mut session = shared_runtime_store.create_session().expect("session");
    let execution = shared_runtime_store
        .find_execution(&process_instance.id, &mut session)
        .expect("execution should remain after command returns");

    assert_eq!(execution.id, process_instance.id);
    assert_eq!(execution.process_definition_id, Some(process_definition_id));
    assert_eq!(execution.activity_id.as_deref(), Some("userTask1"));
    assert!(!execution.is_active);
}

#[test]
fn mysql_repeated_deployment_increments_version() {
    let _guard = lock_mysql();
    let Some(engine) = try_build_mysql_engine("mysql-versioning") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let process_key = format!("mysqlVersioned_{}", Uuid::new_v4().simple());
    let xml = simple_process_xml(&process_key);

    for i in 1..=3 {
        let builder = repository_service
            .create_deployment()
            .name(format!("MySQL Version Deployment {i}"))
            .add_string("versioned.bpmn20.xml".to_string(), xml.clone());
        repository_service.deploy(builder).expect("deploy");
    }

    let mut versions: Vec<i32> = repository_service
        .get_process_definitions()
        .expect("defs")
        .into_iter()
        .filter(|d| d.key == process_key)
        .map(|d| extract_process_definition_version(&d.id))
        .collect();
    versions.sort_unstable();
    assert_eq!(versions, vec![1, 2, 3]);
}

// ---------------------------------------------------------------------------
// Expanded multi-backend contract: complete user task, history, timer
// ---------------------------------------------------------------------------

/// Deploy → start → complete a user task; process ends with no remaining tasks.
#[test]
fn mysql_deploy_start_complete_user_task() {
    let _guard = lock_mysql();
    let Some(engine) = try_build_mysql_engine("mysql-complete-user-task") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let process_key = format!("mysqlCompleteUt_{}", Uuid::new_v4().simple());

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("MySQL Complete User Task Deployment".to_string())
                .add_string(
                    "complete-ut.bpmn20.xml".to_string(),
                    simple_process_xml(&process_key),
                ),
        )
        .expect("deploy");

    let process_definition_id = repository_service
        .get_process_definitions()
        .expect("defs")
        .into_iter()
        .find(|d| d.key == process_key)
        .expect("definition")
        .id;

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("MySQL Complete UT Instance".to_string()),
        )
        .expect("start");

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .expect("list tasks");
    assert_eq!(tasks.len(), 1, "exactly one user task should be active");
    assert_eq!(tasks[0].task_definition_key, "userTask1");
    assert_eq!(tasks[0].name, "First Task");
    assert!(!tasks[0].is_completed);

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .expect("complete task");

    let remaining = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .expect("list tasks after complete");
    assert!(
        remaining.is_empty(),
        "no active tasks should remain after complete"
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("session");
    let pi = store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance after complete");
    assert!(
        pi.is_ended,
        "process instance should be ended after completing the only user task"
    );
}

/// After deploy+start+complete, historic process/task/activity rows are present.
#[test]
fn mysql_history_present_after_complete() {
    let _guard = lock_mysql();
    let Some(engine) = try_build_mysql_engine("mysql-history-presence") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history_service = engine.get_history_service();
    let process_key = format!("mysqlHistory_{}", Uuid::new_v4().simple());

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("MySQL History Presence Deployment".to_string())
                .add_string(
                    "history.bpmn20.xml".to_string(),
                    simple_process_xml(&process_key),
                ),
        )
        .expect("deploy");

    let process_definition_id = repository_service
        .get_process_definitions()
        .expect("defs")
        .into_iter()
        .find(|d| d.key == process_key)
        .expect("definition")
        .id;

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("MySQL History Instance".to_string()),
        )
        .expect("start");

    let hist_pis_after_start = history_service
        .create_historic_process_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .expect("historic PI query after start");
    assert_eq!(
        hist_pis_after_start.len(),
        1,
        "historic process instance should be recorded on start"
    );
    assert_eq!(hist_pis_after_start[0].id(), &process_instance.id);

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .expect("list tasks");
    assert_eq!(tasks.len(), 1);
    let task_id = tasks[0].id.clone();

    task_service
        .complete_task_by_id(task_id.clone())
        .expect("complete");

    let hist_pis = history_service
        .create_historic_process_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .expect("historic PI query after complete");
    assert_eq!(hist_pis.len(), 1);
    assert!(
        hist_pis[0].end_time.is_some(),
        "historic process instance should have end_time after complete"
    );

    let hist_tasks = history_service
        .create_historic_task_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .expect("historic task query");
    assert!(
        !hist_tasks.is_empty(),
        "at least one historic task should be present"
    );
    assert!(
        hist_tasks.iter().any(|t| t.id == task_id),
        "historic task should include completed task id {task_id}"
    );
    let completed_hist_task = hist_tasks
        .iter()
        .find(|t| t.id == task_id)
        .expect("completed historic task");
    assert!(
        completed_hist_task.end_time.is_some(),
        "historic task should have end_time after complete"
    );

    let hist_acts = history_service
        .create_historic_activity_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .expect("historic activity query");
    assert!(
        !hist_acts.is_empty(),
        "historic activity instances should be present"
    );
    assert!(
        hist_acts
            .iter()
            .any(|a| a.activity_id() == "startEvent1" || a.activity_id() == "userTask1"),
        "expected startEvent1 or userTask1 in historic activities, got: {:?}",
        hist_acts
            .iter()
            .map(|a| a.activity_id().clone())
            .collect::<Vec<_>>()
    );
}

/// Intermediate timer catch creates a timer job on start (presence only; no fire).
#[test]
fn mysql_timer_intermediate_catch_creates_timer_job() {
    let _guard = lock_mysql();
    let Some(engine) = try_build_mysql_engine("mysql-timer-job") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let process_key = format!("mysqlTimer_{}", Uuid::new_v4().simple());

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("MySQL Timer Job Deployment".to_string())
                .add_string(
                    "timer.bpmn20.xml".to_string(),
                    timer_process_xml(&process_key),
                ),
        )
        .expect("deploy");

    let process_definition_id = repository_service
        .get_process_definitions()
        .expect("defs")
        .into_iter()
        .find(|d| d.key == process_key)
        .expect("definition")
        .id;

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("MySQL Timer Instance".to_string()),
        )
        .expect("start");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("session");
    let timer_jobs = store.snapshot_timer_job_states(&mut session);

    let for_pi: Vec<_> = timer_jobs
        .into_values()
        .filter(|j| j.process_instance_id == process_instance.id)
        .collect();
    assert_eq!(
        for_pi.len(),
        1,
        "exactly one timer job should be created for intermediate catch"
    );
    assert_eq!(
        for_pi[0].activity_id, "timerCatch",
        "timer job activity should be timerCatch"
    );

    let execution = store
        .find_execution(&process_instance.id, &mut session)
        .expect("execution after timer wait");
    assert_eq!(execution.activity_id.as_deref(), Some("timerCatch"));
}

// ---------------------------------------------------------------------------
// Event registry change revision schema: correctness-critical on MySQL even
// though secondary indexes are skipped during bootstrap.
// ---------------------------------------------------------------------------

fn event_change_record(revision: u64, id: &str) -> EventRegistryChangeRecord {
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
/// committed transaction, mirroring the deployment/update code paths.
fn allocate_event_revision(engine: &ProcessEngine, id: &str) -> u64 {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("session");
    let revision = store
        .next_event_registry_change_revision(&mut session)
        .expect("allocate revision");
    store
        .insert_event_registry_change_record(event_change_record(revision, id), &mut session)
        .expect("insert change record");
    session.flush_and_commit().expect("commit");
    revision
}

/// Two independent MySQL-backed instances: bootstrap must seed the revision
/// allocator (so first allocations never race a missing-row reseed), the
/// unique revision index must exist, and concurrent allocation across both
/// instances must yield unique revisions without errors or panics.
#[test]
fn mysql_event_registry_revision_allocator_seeded_and_unique_across_instances() {
    let _guard = lock_mysql();
    let Some(engine_a) = try_build_mysql_engine("mysql-evt-revision-a") else {
        return;
    };
    let Some(engine_b) = try_build_mysql_engine("mysql-evt-revision-b") else {
        return;
    };
    let engine_a = Arc::new(engine_a);
    let engine_b = Arc::new(engine_b);

    // Bootstrap must have seeded the allocator row on MySQL, not deferred it
    // with the skipped secondary indexes.
    let store = engine_a.get_runtime_store();
    let mut session = store.create_session().expect("session");
    let seed_rows = session
        .raw_query(
            "SELECT id FROM event_registry_change_revision_seq WHERE id = 'event-registry'",
            DbParams::new(),
        )
        .expect("query allocator seed");
    assert_eq!(
        seed_rows.len(),
        1,
        "MySQL bootstrap must seed the event registry revision allocator row"
    );
    drop(session);

    // First concurrent allocations from two independent instances: the seeded
    // single-row allocator serializes them via the UPDATE row lock.
    let run = Uuid::new_v4().simple().to_string();
    let mut handles = Vec::new();
    for worker in 0..4u32 {
        let engine = if worker % 2 == 0 {
            Arc::clone(&engine_a)
        } else {
            Arc::clone(&engine_b)
        };
        let run = run.clone();
        handles.push(std::thread::spawn(move || {
            (0..5u32)
                .map(|round| {
                    allocate_event_revision(&engine, &format!("evt-rev-{run}-{worker}-{round}"))
                })
                .collect::<Vec<u64>>()
        }));
    }
    let mut revisions: Vec<u64> = handles
        .into_iter()
        .flat_map(|handle| handle.join().expect("allocation worker must not panic"))
        .collect();
    revisions.sort_unstable();
    let total = revisions.len();
    revisions.dedup();
    assert_eq!(
        total,
        revisions.len(),
        "concurrent allocation across instances produced duplicate revisions: {revisions:?}"
    );
    assert_eq!(total, 20);

    // The unique revision index must exist on MySQL: a duplicate insert fails
    // instead of silently corrupting the single-revision poll cursor.
    let clash_revision = *revisions.last().unwrap();
    let mut session = store.create_session().expect("session");
    let clash = event_change_record(clash_revision, &format!("evt-rev-{run}-clash"));
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
        "duplicate revision {clash_revision} must violate the unique index on MySQL"
    );
}
