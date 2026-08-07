//! PostgreSQL integration tests for ProcessEngine multi-backend assembly.
//!
//! Requires a reachable PostgreSQL instance. Defaults to:
//! `postgres://postgres:postgres@localhost:5432/flowable_test`
//! Override with `FLOWABLE_TEST_POSTGRES_URL`.
//!
//! Tests **skip gracefully** when the database is unreachable so default CI
//! without Postgres does not fail.
//!
//! ```powershell
//! $env:FLOWABLE_TEST_POSTGRES_URL = "postgres://user:pass@localhost:5432/flowable_test"
//! cargo test -p flowable-engine --features postgres --test postgres_engine_integration_test
//! ```

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::service::config::{
    DatabaseConfiguration, EngineDatabaseKind, ProcessEngineConfiguration,
};
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

static PG_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Cached availability probe so we only attempt a connection once per process
/// when the DB is down (keeps skip logs quiet and suite fast).
static PG_AVAILABLE: OnceLock<bool> = OnceLock::new();

fn postgres_url() -> String {
    std::env::var("FLOWABLE_TEST_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/flowable_test".to_string())
}

fn lock_pg() -> std::sync::MutexGuard<'static, ()> {
    PG_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn postgres_available() -> bool {
    *PG_AVAILABLE.get_or_init(|| {
        let config = ProcessEngineConfiguration {
            database: DatabaseConfiguration {
                kind: EngineDatabaseKind::Postgres,
                url: postgres_url(),
                pool_size: 1,
                busy_timeout_ms: 2000,
                journal_mode: Default::default(),
            },
            ..Default::default()
        };
        match ProcessEngine::build_with_config(
            "pg-availability-probe".to_string(),
            Arc::new(SystemTimeSource),
            config,
        ) {
            Ok(_) => true,
            Err(err) => {
                eprintln!(
                    "Skipping PostgreSQL engine integration tests: database unreachable ({err}). \
                     Set FLOWABLE_TEST_POSTGRES_URL to a live instance to run them."
                );
                false
            }
        }
    })
}

/// Build a Postgres-backed engine, or return `None` (after logging) when DB is down.
fn try_build_postgres_engine(name: &str) -> Option<ProcessEngine> {
    if !postgres_available() {
        return None;
    }
    let config = ProcessEngineConfiguration {
        database: DatabaseConfiguration {
            kind: EngineDatabaseKind::Postgres,
            url: postgres_url(),
            pool_size: 4,
            busy_timeout_ms: 5000,
            journal_mode: Default::default(),
        },
        ..Default::default()
    };
    match ProcessEngine::build_with_config(name.to_string(), Arc::new(SystemTimeSource), config) {
        Ok(engine) => Some(engine),
        Err(err) => {
            eprintln!("Skipping PostgreSQL test '{name}': failed to build engine ({err})");
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
fn postgres_deploy_and_query_resources() {
    let _guard = lock_pg();
    let Some(engine) = try_build_postgres_engine("pg-repo-resources") else {
        return;
    };
    let repository_service = engine.get_repository_service();

    let process_key = format!("pgResource_{}", Uuid::new_v4().simple());
    let deployment = repository_service
        .create_deployment()
        .name("PG Resource Deployment".to_string())
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
fn postgres_delete_deployment_removes_process_definitions() {
    let _guard = lock_pg();
    let Some(engine) = try_build_postgres_engine("pg-repo-delete") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let process_key = format!("pgDeletable_{}", Uuid::new_v4().simple());

    let deployment = repository_service
        .create_deployment()
        .name("PG Deletable Deployment".to_string())
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
fn postgres_dual_write_populates_normalized_act_tables() {
    let _guard = lock_pg();
    let Some(engine) = try_build_postgres_engine("pg-dual-write") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let process_key = format!("pgDualWrite_{}", Uuid::new_v4().simple());

    let builder = repository_service
        .create_deployment()
        .name("PG Dual Write Deployment".to_string())
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
        .name("PG Dual Write Instance".to_string());
    let process_instance = runtime_service
        .start_process_instance(builder)
        .expect("start");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("session");

    // ACT_RE_DEPLOYMENT dual-write
    let dep = flowable_persistence::DeploymentDataManager::new()
        .find_by_id(session.inner_mut(), &deployment.id)
        .expect("query deployment")
        .expect("ACT_RE_DEPLOYMENT row");
    assert_eq!(dep.id, deployment.id);

    // ACT_RE_PROCDEF dual-write
    let pd = flowable_persistence::ProcessDefinitionDataManager::new()
        .find_by_id(session.inner_mut(), &process_definition_id)
        .expect("query procdef")
        .expect("ACT_RE_PROCDEF row");
    assert_eq!(pd.key, process_key);
    assert_eq!(pd.deployment_id.as_deref(), Some(deployment.id.as_str()));

    // ACT_RU_EXECUTION dual-write
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
fn postgres_runtime_state_persists_after_start() {
    let _guard = lock_pg();
    let Some(engine) = try_build_postgres_engine("pg-runtime-state") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let process_key = format!("pgRuntime_{}", Uuid::new_v4().simple());

    let builder = repository_service
        .create_deployment()
        .name("PG Runtime State Deployment".to_string())
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
        .name("PG Runtime State Instance".to_string());
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
fn postgres_repeated_deployment_increments_version() {
    let _guard = lock_pg();
    let Some(engine) = try_build_postgres_engine("pg-versioning") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let process_key = format!("pgVersioned_{}", Uuid::new_v4().simple());
    let xml = simple_process_xml(&process_key);

    for i in 1..=3 {
        let builder = repository_service
            .create_deployment()
            .name(format!("PG Version Deployment {i}"))
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
fn postgres_deploy_start_complete_user_task() {
    let _guard = lock_pg();
    let Some(engine) = try_build_postgres_engine("pg-complete-user-task") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let process_key = format!("pgCompleteUt_{}", Uuid::new_v4().simple());

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("PG Complete User Task Deployment".to_string())
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
                .name("PG Complete UT Instance".to_string()),
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
fn postgres_history_present_after_complete() {
    let _guard = lock_pg();
    let Some(engine) = try_build_postgres_engine("pg-history-presence") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history_service = engine.get_history_service();
    let process_key = format!("pgHistory_{}", Uuid::new_v4().simple());

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("PG History Presence Deployment".to_string())
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
                .name("PG History Instance".to_string()),
        )
        .expect("start");

    // Historic process instance should exist immediately after start.
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

    // Historic process instance still present (and ideally ended).
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

    // Historic task instance for the completed user task.
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

    // Historic activities should include start and user task (and ideally end).
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
fn postgres_timer_intermediate_catch_creates_timer_job() {
    let _guard = lock_pg();
    let Some(engine) = try_build_postgres_engine("pg-timer-job") else {
        return;
    };
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let process_key = format!("pgTimer_{}", Uuid::new_v4().simple());

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("PG Timer Job Deployment".to_string())
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
                .name("PG Timer Instance".to_string()),
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

/// P73a: dual-write insert failure hard-fails with an explicit dual-write panic
/// (does not swallow via `let _ =`). Also verifies a subsequent session on the
/// same pool is healthy after cleanup (no silent pool pollution).
#[test]
fn postgres_dual_write_failure_hard_fails_without_silent_divergence() {
    let _guard = lock_pg();
    let Some(engine) = try_build_postgres_engine("pg-dual-write-fail") else {
        return;
    };
    let store = engine.get_runtime_store();

    // Ensure clean slate for the poison column even if a prior run aborted.
    {
        let mut session = store.create_session().expect("session cleanup");
        let _ = session.execute_raw_sql(
            "ALTER TABLE ACT_RU_EXECUTION DROP COLUMN IF EXISTS p73_poison",
        );
        let _ = session.flush_and_commit();
    }

    // Poison ACT_RU_EXECUTION so dual-write INSERT fails (column required, no default).
    {
        let mut session = store.create_session().expect("session poison");
        session
            .execute_raw_sql(
                "ALTER TABLE ACT_RU_EXECUTION ADD COLUMN IF NOT EXISTS p73_poison INTEGER NOT NULL DEFAULT 0",
            )
            .expect("add poison column");
        session
            .execute_raw_sql(
                "ALTER TABLE ACT_RU_EXECUTION ALTER COLUMN p73_poison DROP DEFAULT",
            )
            .expect("drop poison default");
        session.flush_and_commit().expect("commit poison");
    }

    struct PoisonGuard<'a> {
        engine: &'a ProcessEngine,
    }
    impl Drop for PoisonGuard<'_> {
        fn drop(&mut self) {
            let store = self.engine.get_runtime_store();
            if let Ok(mut session) = store.create_session() {
                let _ = session.execute_raw_sql(
                    "ALTER TABLE ACT_RU_EXECUTION DROP COLUMN IF EXISTS p73_poison",
                );
                let _ = session.flush_and_commit();
            }
        }
    }
    let _poison_guard = PoisonGuard { engine: &engine };

    let execution = flowable_engine::runtime::execution::Execution {
        id: format!("exec-p73-fail-{}", Uuid::new_v4().simple()),
        process_instance_id: Some(format!("pi-p73-fail-{}", Uuid::new_v4().simple())),
        activity_id: Some("userTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };

    let insert_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut session = store.create_session().expect("session for insert");
        store.insert_execution(&execution, &mut session);
    }));

    let payload = insert_result.expect_err("dual-write insert must hard-fail under poison");
    let msg = payload
        .downcast_ref::<String>()
        .map(|s| s.as_str())
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic");
    assert!(
        msg.contains("dual-write") && msg.contains("ACT_RU_EXECUTION"),
        "panic should identify dual-write failure (queue or flush), got: {msg}"
    );
    // PoisonGuard Drop removes the column. Do not re-use this engine for further
    // ACT_RU_EXECUTION dual-writes: PG prepared-statement caches may still target
    // the old row type after ADD/DROP COLUMN on a shared pool.
}
