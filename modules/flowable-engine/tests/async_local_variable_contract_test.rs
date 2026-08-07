//! Contract tests for the asynchronous execution variable APIs, mirroring Java
//! `RuntimeService#setVariableLocalAsync` / `#setVariablesLocalAsync` and the
//! GLOBAL variants `#setVariableAsync` / `#setVariablesAsync`
//! (`SetAsyncExecutionVariablesCmd` with `isLocal` true/false):
//!   - the call does not write the variable; it stores it as a pending payload and
//!     schedules a `set-async-variables` job on the execution
//!     (`SetAsyncExecutionVariablesCmd`);
//!   - the variable becomes visible only after that job runs
//!     (`SetAsyncVariablesJobHandler`, `metaInfo == "true"` → `setVariableLocal`,
//!     otherwise `setVariable` with owning-scope resolution);
//!   - the pending write keeps local scope semantics: it lands on the target
//!     execution's own scope, shadowing same-named ancestor variables;
//!   - unknown executions are rejected (Java `NeedsActiveExecutionCmd`), as are
//!     suspended ones, and no job is created in either case;
//!   - an empty variable map is a no-op that schedules no job.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::runtime_store::{RuntimeTimerJobState, job_handler_types};
use flowable_engine::runtime::process_instance::{ProcessInstance, ProcessInstanceUpdate};
use serde_json::json;
use std::collections::HashMap;

/// Same embedded-subprocess topology as `execution_local_variable_scope_contract_test`:
/// a genuine ancestor scope execution (id == process instance id) plus two sibling
/// child executions, which the scope-semantics assertions need.
fn subprocess_fork_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="asyncLocalScopeProcess" name="Async Local Scope Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="outerSub" />
            <subProcess id="outerSub" name="Outer Sub">
                <startEvent id="subStart" />
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="fork" />
                <parallelGateway id="fork" />
                <sequenceFlow id="sf2" sourceRef="fork" targetRef="taskA" />
                <userTask id="taskA" name="Task A" />
                <sequenceFlow id="sf3" sourceRef="fork" targetRef="taskB" />
                <userTask id="taskB" name="Task B" />
                <sequenceFlow id="sf4" sourceRef="taskA" targetRef="join" />
                <sequenceFlow id="sf5" sourceRef="taskB" targetRef="join" />
                <parallelGateway id="join" />
                <sequenceFlow id="sf6" sourceRef="join" targetRef="subEnd" />
                <endEvent id="subEnd" />
            </subProcess>
            <sequenceFlow id="f2" sourceRef="outerSub" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
        .to_string()
}

fn deploy_and_start(engine: &ProcessEngine) -> ProcessInstance {
    let repo = engine.get_repository_service();
    repo.deploy(repo.create_deployment().add_string(
        "asyncLocalScope.bpmn20.xml".to_string(),
        subprocess_fork_xml(),
    ))
    .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap()
}

/// Id of the child execution sitting at `activity_id` under the subprocess scope execution.
fn child_execution_id(engine: &ProcessEngine, activity_id: &str) -> String {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| {
            execution.activity_id.as_deref() == Some(activity_id)
                && execution.parent_id.is_some()
                && !execution.is_ended
        })
        .expect("a child execution should exist at the requested activity")
        .id
}

/// The subprocess scope execution (id == process instance id in this topology).
fn scope_execution_id(engine: &ProcessEngine) -> String {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| execution.parent_id.is_none() && !execution.is_ended)
        .expect("the subprocess scope execution should exist")
        .id
}

/// Pending `set-async-variables` jobs, optionally restricted to one execution.
fn pending_async_variable_jobs(
    engine: &ProcessEngine,
    execution_id: Option<&str>,
) -> Vec<RuntimeTimerJobState> {
    engine
        .get_management_service()
        .list_executable_jobs()
        .into_iter()
        .filter(|job| job.handler_type.as_deref() == Some(job_handler_types::SET_ASYNC_VARIABLES))
        .filter(|job| execution_id.is_none_or(|id| job.execution_id == id))
        .collect()
}

#[test]
fn async_local_variable_is_not_visible_until_the_job_runs() {
    let engine = ProcessEngine::new("async-local-deferred".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .set_variable_local_async(task_a.clone(), "branch".to_string(), json!("a"))
        .expect("scheduling an async local variable should succeed");

    // Java: the value lives in a pending async-variables entry, not on the execution.
    assert_eq!(
        runtime
            .get_variable_local(task_a.clone(), "branch".to_string())
            .unwrap(),
        None,
        "the variable must not be visible before the job runs"
    );
    assert_eq!(
        runtime
            .get_variable(task_a.clone(), "branch".to_string())
            .unwrap(),
        None
    );

    let jobs = pending_async_variable_jobs(&engine, Some(&task_a));
    assert_eq!(
        jobs.len(),
        1,
        "exactly one set-async-variables job should be scheduled for the execution"
    );

    engine
        .get_management_service()
        .execute_job(&jobs[0].timer_job_id)
        .expect("the set-async-variables job should execute successfully");

    assert_eq!(
        runtime
            .get_variable_local(task_a.clone(), "branch".to_string())
            .unwrap(),
        Some(json!("a"))
    );
    assert!(
        pending_async_variable_jobs(&engine, Some(&task_a)).is_empty(),
        "the job should be consumed after applying the variables"
    );
}

#[test]
fn async_local_variables_apply_to_the_owning_executions_scope() {
    let engine = ProcessEngine::new("async-local-scope".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let scope = scope_execution_id(&engine);
    let task_a = child_execution_id(&engine, "taskA");
    let task_b = child_execution_id(&engine, "taskB");

    let mut variables = HashMap::new();
    variables.insert("one".to_string(), json!(1));
    variables.insert("two".to_string(), json!(2));
    runtime
        .set_variables_local_async(task_a.clone(), variables)
        .unwrap();

    // Nothing visible anywhere before the job runs.
    assert!(
        runtime
            .get_variables_local(task_a.clone())
            .unwrap()
            .get("one")
            .is_none()
    );
    assert_eq!(
        runtime
            .get_variable(scope.clone(), "one".to_string())
            .unwrap(),
        None
    );

    let jobs = pending_async_variable_jobs(&engine, Some(&task_a));
    assert_eq!(jobs.len(), 1);
    engine
        .get_management_service()
        .execute_job(&jobs[0].timer_job_id)
        .unwrap();

    // After the job: local to the owning execution only.
    let locals = runtime.get_variables_local(task_a.clone()).unwrap();
    assert_eq!(locals.get("one"), Some(&json!(1)));
    assert_eq!(locals.get("two"), Some(&json!(2)));
    assert_eq!(
        runtime
            .get_variables_local(scope.clone())
            .unwrap()
            .get("one"),
        None,
        "the ancestor scope must not receive a copy"
    );
    assert_eq!(
        runtime.get_variable(scope, "one".to_string()).unwrap(),
        None
    );
    assert_eq!(
        runtime.get_variable(task_b, "one".to_string()).unwrap(),
        None,
        "a sibling branch must not see the child's local variable"
    );
}

#[test]
fn async_local_variable_shadows_ancestor_value_after_the_job_runs() {
    let engine = ProcessEngine::new("async-local-shadowing".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let scope = scope_execution_id(&engine);
    let task_a = child_execution_id(&engine, "taskA");
    let task_b = child_execution_id(&engine, "taskB");

    runtime
        .set_variable(scope.clone(), "reviewer".to_string(), json!("global"))
        .unwrap();
    runtime
        .set_variable_local_async(task_a.clone(), "reviewer".to_string(), json!("local-a"))
        .unwrap();

    // Before the job runs the child still resolves the ancestor value.
    assert_eq!(
        runtime
            .get_variable(task_a.clone(), "reviewer".to_string())
            .unwrap(),
        Some(json!("global"))
    );

    let jobs = pending_async_variable_jobs(&engine, Some(&task_a));
    assert_eq!(jobs.len(), 1);
    engine
        .get_management_service()
        .execute_job(&jobs[0].timer_job_id)
        .unwrap();

    assert_eq!(
        runtime
            .get_variable(task_a.clone(), "reviewer".to_string())
            .unwrap(),
        Some(json!("local-a")),
        "the local value shadows the ancestor's once the job has run"
    );
    assert_eq!(
        runtime.get_variable(scope, "reviewer".to_string()).unwrap(),
        Some(json!("global")),
        "the ancestor value is untouched"
    );
    assert_eq!(
        runtime
            .get_variable(task_b, "reviewer".to_string())
            .unwrap(),
        Some(json!("global"))
    );
}

#[test]
fn async_local_set_rejects_unknown_execution() {
    let engine = ProcessEngine::new("async-local-unknown".to_string());
    let runtime = engine.get_runtime_service();

    let error = runtime
        .set_variable_local_async("missing-execution".to_string(), "x".to_string(), json!(1))
        .expect_err("an unknown execution must not accept async variables");
    assert!(matches!(error, FlowableError::NotFound(_)));
    assert!(error.to_string().contains("missing-execution"));

    let error = runtime
        .set_variables_local_async(
            "missing-execution".to_string(),
            HashMap::from([("x".to_string(), json!(1))]),
        )
        .expect_err("an unknown execution must not accept async variables");
    assert!(matches!(error, FlowableError::NotFound(_)));

    assert!(
        pending_async_variable_jobs(&engine, None).is_empty(),
        "no job may be scheduled for a rejected call"
    );
}

#[test]
fn async_local_set_rejects_suspended_execution() {
    let engine = ProcessEngine::new("async-local-suspended".to_string());
    let process_instance = deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .suspend_process_instance(
            process_instance.id.clone(),
            ProcessInstanceUpdate::default(),
        )
        .unwrap();

    let error = runtime
        .set_variable_local_async(task_a.clone(), "x".to_string(), json!(1))
        .expect_err("Java NeedsActiveExecutionCmd rejects a suspended execution");
    assert!(matches!(error, FlowableError::ExecutionError(_)));
    assert!(
        pending_async_variable_jobs(&engine, None).is_empty(),
        "no job may be scheduled for a suspended execution"
    );

    runtime
        .activate_process_instance(
            process_instance.id.clone(),
            ProcessInstanceUpdate::default(),
        )
        .unwrap();
}

#[test]
fn empty_variables_map_creates_no_job() {
    let engine = ProcessEngine::new("async-local-empty".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .set_variables_local_async(task_a.clone(), HashMap::new())
        .expect("Java treats an empty map as a no-op");

    assert!(
        pending_async_variable_jobs(&engine, None).is_empty(),
        "an empty map must not schedule a job"
    );
}

/// Regression guard: the synchronous local write stays immediate and schedules no job.
/// (Green before this package's implementation lands.)
#[test]
fn sync_local_set_remains_immediate_and_schedules_no_job() {
    let engine = ProcessEngine::new("async-local-sync-guard".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .set_variable_local(task_a.clone(), "branch".to_string(), json!("a"))
        .unwrap();

    assert_eq!(
        runtime
            .get_variable_local(task_a, "branch".to_string())
            .unwrap(),
        Some(json!("a"))
    );
    assert!(
        pending_async_variable_jobs(&engine, None).is_empty(),
        "the synchronous path must not schedule async jobs"
    );
}

#[test]
fn async_global_variable_is_not_visible_until_the_job_runs() {
    let engine = ProcessEngine::new("async-global-deferred".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .set_variable_async(task_a.clone(), "branch".to_string(), json!("a"))
        .expect("scheduling an async variable should succeed");

    // Java: the value lives in a pending async-variables entry, not on any execution.
    assert_eq!(
        runtime
            .get_variable(task_a.clone(), "branch".to_string())
            .unwrap(),
        None,
        "the variable must not be visible before the job runs"
    );

    let jobs = pending_async_variable_jobs(&engine, Some(&task_a));
    assert_eq!(
        jobs.len(),
        1,
        "exactly one set-async-variables job should be scheduled for the execution"
    );

    engine
        .get_management_service()
        .execute_job(&jobs[0].timer_job_id)
        .expect("the set-async-variables job should execute successfully");

    assert_eq!(
        runtime
            .get_variable(task_a.clone(), "branch".to_string())
            .unwrap(),
        Some(json!("a"))
    );
    assert!(
        pending_async_variable_jobs(&engine, Some(&task_a)).is_empty(),
        "the job should be consumed after applying the variables"
    );
}

/// Java `SetAsyncVariablesJobHandler` runs `executionEntity.setVariable` for a
/// non-local payload, so a name an ancestor already owns is updated there in
/// place — never copied onto the job's execution.
#[test]
fn async_global_variable_updates_the_existing_ancestor_value() {
    let engine = ProcessEngine::new("async-global-owning-scope".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let scope = scope_execution_id(&engine);
    let task_a = child_execution_id(&engine, "taskA");
    let task_b = child_execution_id(&engine, "taskB");

    runtime
        .set_variable(scope.clone(), "reviewer".to_string(), json!("global"))
        .unwrap();
    runtime
        .set_variables_async(
            task_a.clone(),
            HashMap::from([("reviewer".to_string(), json!("updated"))]),
        )
        .unwrap();

    // Before the job runs every branch still resolves the ancestor value.
    assert_eq!(
        runtime
            .get_variable(task_a.clone(), "reviewer".to_string())
            .unwrap(),
        Some(json!("global"))
    );

    let jobs = pending_async_variable_jobs(&engine, Some(&task_a));
    assert_eq!(jobs.len(), 1);
    engine
        .get_management_service()
        .execute_job(&jobs[0].timer_job_id)
        .unwrap();

    assert_eq!(
        runtime
            .get_variable(scope.clone(), "reviewer".to_string())
            .unwrap(),
        Some(json!("updated")),
        "the ancestor's own variable is updated in place"
    );
    assert_eq!(
        runtime
            .get_variable(task_a.clone(), "reviewer".to_string())
            .unwrap(),
        Some(json!("updated"))
    );
    assert_eq!(
        runtime
            .get_variable(task_b.clone(), "reviewer".to_string())
            .unwrap(),
        Some(json!("updated")),
        "a sibling sees the ancestor update, proving nothing was written to the child"
    );
    assert_eq!(
        runtime
            .get_variable_local(task_a.clone(), "reviewer".to_string())
            .unwrap(),
        None,
        "the job's execution must not receive a local copy"
    );
}

#[test]
fn async_global_set_rejects_unknown_execution() {
    let engine = ProcessEngine::new("async-global-unknown".to_string());
    let runtime = engine.get_runtime_service();

    let error = runtime
        .set_variable_async("missing-execution".to_string(), "x".to_string(), json!(1))
        .expect_err("an unknown execution must not accept async variables");
    assert!(matches!(error, FlowableError::NotFound(_)));
    assert!(error.to_string().contains("missing-execution"));

    let error = runtime
        .set_variables_async(
            "missing-execution".to_string(),
            HashMap::from([("x".to_string(), json!(1))]),
        )
        .expect_err("an unknown execution must not accept async variables");
    assert!(matches!(error, FlowableError::NotFound(_)));

    assert!(
        pending_async_variable_jobs(&engine, None).is_empty(),
        "no job may be scheduled for a rejected call"
    );
}

#[test]
fn async_global_set_rejects_suspended_execution() {
    let engine = ProcessEngine::new("async-global-suspended".to_string());
    let process_instance = deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .suspend_process_instance(
            process_instance.id.clone(),
            ProcessInstanceUpdate::default(),
        )
        .unwrap();

    let error = runtime
        .set_variable_async(task_a.clone(), "x".to_string(), json!(1))
        .expect_err("Java NeedsActiveExecutionCmd rejects a suspended execution");
    assert!(matches!(error, FlowableError::ExecutionError(_)));
    assert!(
        pending_async_variable_jobs(&engine, None).is_empty(),
        "no job may be scheduled for a suspended execution"
    );

    runtime
        .activate_process_instance(
            process_instance.id.clone(),
            ProcessInstanceUpdate::default(),
        )
        .unwrap();
}

/// Regression guard: the synchronous global write stays immediate and schedules no job.
/// (Green before this package's implementation lands.)
#[test]
fn sync_global_set_remains_immediate_and_schedules_no_job() {
    let engine = ProcessEngine::new("async-global-sync-guard".to_string());
    deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();
    let task_a = child_execution_id(&engine, "taskA");

    runtime
        .set_variable(task_a.clone(), "branch".to_string(), json!("a"))
        .unwrap();

    assert_eq!(
        runtime.get_variable(task_a, "branch".to_string()).unwrap(),
        Some(json!("a"))
    );
    assert!(
        pending_async_variable_jobs(&engine, None).is_empty(),
        "the synchronous path must not schedule async jobs"
    );
}
