//! P48: exclusive async jobs serialize per process instance through the
//! exclusive PI scope lock — Java `ExecuteAsyncRunnable` parity.
//!
//! Java chain under test:
//!   - `ExecuteAsyncRunnable.java:113-129`: an exclusive `JobEntity` first
//!     takes the process-instance scope lock (`LockExclusiveJobCmd.java:55-62`
//!     → `DefaultInternalJobManager.lockJobScopeInternal:184-215`) in its own
//!     transaction before execution.
//!   - `ExecuteAsyncRunnable.java:239-258` (`lockJobFailed`): when another
//!     owner holds a live scope lock, the job is *unacquired* (row lock
//!     released) and not executed.
//!   - `ExecuteAsyncRunnable.java:199-204`: on success the scope lock is
//!     cleared in the same transaction as the job execution.
//!   - `ExecuteAsyncRunnable.java:275-306`: on failure `unlockJobIfNeeded`
//!     clears the scope lock after `handleFailedJob`.
//!   - `AbstractJobEntityImpl.DEFAULT_EXCLUSIVE = true`; producers override:
//!     `ContinueProcessOperation.java:190` (`flowNode.isExclusive()`),
//!     `StartProcessInstanceAsyncCmd.java:71` (`false`).
//!   - Manual management execution (`ExecuteJobCmd`) has no exclusive
//!     handling: it neither takes nor requires the scope lock.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use std::sync::Arc;

fn create_engine(name: &str) -> (ProcessEngine, Arc<TestTimeSource>) {
    let now = Utc.with_ymd_and_hms(2026, 7, 28, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let engine = ProcessEngine::with_time_source(name.to_string(), time_source.clone());
    (engine, time_source)
}

/// `exclusive_attr` = `""` (Java default, exclusive) or
/// ` flowable:exclusive="false"` (explicit opt-out).
fn async_task_process_xml(exclusive_attr: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="p48AsyncProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="asyncTask" />
        <serviceTask id="asyncTask" flowable:async="true"{exclusive_attr} />
        <sequenceFlow id="flow2" sourceRef="asyncTask" targetRef="afterAsyncTask" />
        <userTask id="afterAsyncTask" name="After Async" />
    </process>
</definitions>"#
    )
}

fn deploy_and_start(engine: &ProcessEngine, xml: String) -> String {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .name("p48".to_string())
            .add_string("p48.bpmn20.xml".to_string(), xml),
    )
    .unwrap();
    let pd_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap()
        .id
}

fn pending_async_job(engine: &ProcessEngine, pi_id: &str) -> RuntimeTimerJobState {
    engine
        .get_management_service()
        .list_executable_jobs()
        .into_iter()
        .find(|job| job.process_instance_id == pi_id)
        .expect("an executable async continuation job must exist")
}

fn seed_pi_lock(engine: &ProcessEngine, pi_id: &str, owner: &str, expiration: i64, now: i64) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store.lock_process_instance(pi_id, owner, expiration, now, &mut session),
        "seeding the PI scope lock must succeed"
    );
    session.flush_and_commit().unwrap();
}

fn pi_lock_owner(engine: &ProcessEngine, pi_id: &str) -> Option<String> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let owner = store
        .find_process_instance_lock(pi_id, &mut session)
        .and_then(|lock| lock.lock_owner);
    session.rollback().unwrap();
    owner
}

fn now_ms(time_source: &TestTimeSource) -> i64 {
    use flowable_engine::engine::time_source::TimeSource;
    time_source.now().timestamp_millis()
}

/// Java ContinueProcessOperation.java:190 + AbstractJobEntityImpl
/// DEFAULT_EXCLUSIVE: without `flowable:exclusive="false"` the async
/// continuation job is exclusive.
#[test]
fn async_continuation_job_is_exclusive_by_default() {
    let (engine, _time) = create_engine("p48-default-exclusive");
    let pi_id = deploy_and_start(&engine, async_task_process_xml(""));
    let job = pending_async_job(&engine, &pi_id);
    assert!(
        job.exclusive,
        "Java default: createAsyncJob(job, flowNode.isExclusive()) with isExclusive() == true"
    );
}

/// Java ContinueProcessOperation.java:190: `flowable:exclusive="false"` is
/// carried onto the async continuation job.
#[test]
fn async_continuation_job_honors_exclusive_false_attribute() {
    let (engine, _time) = create_engine("p48-exclusive-false");
    let pi_id = deploy_and_start(
        &engine,
        async_task_process_xml(r#" flowable:exclusive="false""#),
    );
    let job = pending_async_job(&engine, &pi_id);
    assert!(
        !job.exclusive,
        "flowable:exclusive=\"false\" must produce a non-exclusive job"
    );
}

/// Java StartProcessInstanceAsyncCmd.java:71: `createAsyncJob(job, false)` —
/// the async-start job is NOT exclusive.
#[test]
fn start_process_instance_async_job_is_not_exclusive() {
    let (engine, _time) = create_engine("p48-start-async");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .name("p48-start".to_string())
            .add_string("p48start.bpmn20.xml".to_string(), async_task_process_xml("")),
    )
    .unwrap();
    let pd_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let pi = runtime
        .start_process_instance_async(
            runtime
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();
    let job = pending_async_job(&engine, &pi.id);
    assert!(
        !job.exclusive,
        "StartProcessInstanceAsyncCmd schedules a non-exclusive async job"
    );
}

/// Java AbstractJobEntityImpl.DEFAULT_EXCLUSIVE: timer jobs are exclusive.
#[test]
fn timer_job_defaults_to_exclusive() {
    let (engine, _time) = create_engine("p48-timer-default");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p48TimerProcess" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="catch1" />
            <intermediateCatchEvent id="catch1">
                <timerEventDefinition>
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="catch1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;
    let pi_id = deploy_and_start(&engine, xml.to_string());
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let timer_job = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .find(|job| job.process_instance_id == pi_id)
        .expect("a timer job must exist");
    session.rollback().unwrap();
    assert!(
        timer_job.exclusive,
        "timer jobs keep the Java DEFAULT_EXCLUSIVE = true"
    );
}

/// Java ExecuteAsyncRunnable.java:113-129 + :199-204: the executor takes the
/// PI scope lock, executes the exclusive job, and clears the lock in the same
/// transaction — afterwards the instance is unlocked and advanced.
#[test]
fn exclusive_job_executes_and_clears_pi_scope_lock() {
    let (engine, _time) = create_engine("p48-exec-clear");
    let pi_id = deploy_and_start(&engine, async_task_process_xml(""));
    let job = pending_async_job(&engine, &pi_id);

    let executed = engine.run_due_timers();
    assert!(
        executed.contains(&job.timer_job_id),
        "the exclusive async job must execute; executed={executed:?}"
    );
    assert_eq!(
        pi_lock_owner(&engine, &pi_id),
        None,
        "the PI scope lock must be cleared in the execution transaction \
         (ExecuteAsyncRunnable.java:199-204)"
    );
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert!(
        !tasks.is_empty(),
        "the process must have advanced to the user task"
    );
}

/// Java ExecuteAsyncRunnable.java:239-258 (`lockJobFailed`): a live foreign
/// scope lock defers the exclusive job — it is not executed and its acquired
/// row lock is released (unacquire) so it can be picked up again later.
#[test]
fn foreign_live_pi_lock_defers_exclusive_job_and_releases_row_lock() {
    let (engine, time_source) = create_engine("p48-foreign-lock");
    let pi_id = deploy_and_start(&engine, async_task_process_xml(""));
    let job = pending_async_job(&engine, &pi_id);

    let now = now_ms(&time_source);
    seed_pi_lock(&engine, &pi_id, "other-executor", now + 3_600_000, now);

    let executed = engine.run_due_timers();
    assert!(
        !executed.contains(&job.timer_job_id),
        "an exclusive job must not run while another owner holds a live scope lock"
    );
    assert_eq!(
        pi_lock_owner(&engine, &pi_id).as_deref(),
        Some("other-executor"),
        "the foreign scope lock must be left intact"
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let row = store
        .find_timer_job_state(&job.timer_job_id, &mut session)
        .expect("the deferred job row must still exist");
    session.rollback().unwrap();
    assert_eq!(
        row.lock_owner, None,
        "lockJobFailed unacquires the job row lock (ExecuteAsyncRunnable.java:249-257)"
    );
}

/// Java MybatisExecutionDataManager.updateProcessInstanceLockTime (:302-313):
/// `LOCK_TIME_ IS NULL OR LOCK_TIME_ < now` — an *expired* foreign lock is
/// taken over and the job executes.
#[test]
fn expired_foreign_pi_lock_is_taken_over() {
    let (engine, time_source) = create_engine("p48-expired-lock");
    let pi_id = deploy_and_start(&engine, async_task_process_xml(""));
    let job = pending_async_job(&engine, &pi_id);

    let now = now_ms(&time_source);
    seed_pi_lock(&engine, &pi_id, "dead-executor", now - 1, now - 3_600_000);

    let executed = engine.run_due_timers();
    assert!(
        executed.contains(&job.timer_job_id),
        "an expired foreign scope lock must not block execution; executed={executed:?}"
    );
    assert_eq!(
        pi_lock_owner(&engine, &pi_id),
        None,
        "the takeover execution clears the scope lock on success"
    );
}

/// Java ExecuteAsyncRunnable.java:113 (`job.isExclusive()` guard): a
/// non-exclusive job ignores the PI scope lock entirely.
#[test]
fn non_exclusive_job_ignores_foreign_pi_lock() {
    let (engine, time_source) = create_engine("p48-nonexclusive");
    let pi_id = deploy_and_start(
        &engine,
        async_task_process_xml(r#" flowable:exclusive="false""#),
    );
    let job = pending_async_job(&engine, &pi_id);
    assert!(!job.exclusive);

    let now = now_ms(&time_source);
    seed_pi_lock(&engine, &pi_id, "other-executor", now + 3_600_000, now);

    let executed = engine.run_due_timers();
    assert!(
        executed.contains(&job.timer_job_id),
        "a non-exclusive job must execute regardless of the scope lock"
    );
    assert_eq!(
        pi_lock_owner(&engine, &pi_id).as_deref(),
        Some("other-executor"),
        "a non-exclusive execution must not touch the foreign scope lock"
    );
}

/// Java ExecuteJobCmd (management API) has no exclusive-lock handling: manual
/// execution neither checks nor clears the PI scope lock.
#[test]
fn manual_execute_job_ignores_pi_scope_lock() {
    let (engine, time_source) = create_engine("p48-manual");
    let pi_id = deploy_and_start(&engine, async_task_process_xml(""));
    let job = pending_async_job(&engine, &pi_id);
    assert!(job.exclusive);

    let now = now_ms(&time_source);
    seed_pi_lock(&engine, &pi_id, "other-executor", now + 3_600_000, now);

    engine
        .get_management_service()
        .execute_job(&job.timer_job_id)
        .expect("manual management execution must succeed despite the scope lock");
    assert_eq!(
        pi_lock_owner(&engine, &pi_id).as_deref(),
        Some("other-executor"),
        "manual execution must leave the scope lock untouched (no ExecuteAsyncRunnable involved)"
    );
}

/// Java ExecuteAsyncRunnable.java:275-306: after a failed exclusive execution
/// the scope lock is cleared (`unlockJobIfNeeded`) so the retry cycle does not
/// keep the process instance locked.
#[test]
fn failed_exclusive_job_clears_pi_scope_lock() {
    let (engine, _time) = create_engine("p48-failure-unlock");
    // A class-delegate service task that cannot be resolved fails inside the
    // async job execution, driving the automatic-executor failure path.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="p48FailingProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="failingTask" />
        <serviceTask id="failingTask" flowable:async="true"
                     flowable:class="com.example.MissingDelegate" />
        <sequenceFlow id="flow2" sourceRef="failingTask" targetRef="theEnd" />
        <endEvent id="theEnd" />
    </process>
</definitions>"#;
    let pi_id = deploy_and_start(&engine, xml.to_string());
    let job = pending_async_job(&engine, &pi_id);
    assert!(job.exclusive);

    let executed = engine.run_due_timers();
    assert!(
        !executed.contains(&job.timer_job_id),
        "the failing job must not report success"
    );
    assert_eq!(
        pi_lock_owner(&engine, &pi_id),
        None,
        "the failure path must clear the PI scope lock (ExecuteAsyncRunnable.java:275-306)"
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let row = store
        .find_timer_job_state(&job.timer_job_id, &mut session)
        .expect("the failed job row must survive as retry/deadletter");
    session.rollback().unwrap();
    assert!(
        row.error_message.is_some(),
        "the failure must be recorded on the job row"
    );
}
