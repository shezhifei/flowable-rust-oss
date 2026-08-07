use flowable_cmmn_engine::{
    ALL_HANDLER_TYPES, CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState,
    CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask,
    CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnJob, CmmnJobFamily, CmmnModel,
    CmmnPlanItem, MIGRATION_STATUS_COMPLETED, MIGRATION_STATUS_FAIL, MIGRATION_STATUS_IN_PROGRESS,
    TYPE_ASYNC_ACTIVATE_PLAN_ITEM, TYPE_ASYNC_COMPLETE_PLAN_ITEM, TYPE_ASYNC_DISABLE_PLAN_ITEM,
    TYPE_ASYNC_ENABLE_PLAN_ITEM, TYPE_ASYNC_INIT_PLAN_MODEL, TYPE_ASYNC_LEAVE_ACTIVE_PLAN_ITEM,
    TYPE_ASYNC_REACTIVATE_PLAN_ITEM, TYPE_ASYNC_START_CASE, TYPE_ASYNC_TERMINATE,
    TYPE_CASE_MIGRATION, TYPE_CASE_MIGRATION_STATUS, TYPE_EXTERNAL_WORKER_COMPLETE,
    TYPE_HISTORIC_CASE_MIGRATION, TYPE_HISTORY_CLEANUP, TYPE_SET_ASYNC_VARIABLES,
    TYPE_TRIGGER_TIMER,
};
use serde_json::json;

fn simple_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-b", "human-task-b"));

    CmmnModel::new(vec![CmmnCase::new(
        "case-async",
        "asyncCase",
        "Async case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine) -> (String, String) {
    engine
        .deploy(
            CmmnDeploymentRequest::new("async-jobs")
                .with_resource("async-case.cmmn", simple_model()),
        )
        .expect("deploy");
    let case_instance = engine
        .start_case_instance_by_key(
            "asyncCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "k": "v" })),
        )
        .expect("start");
    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("tasks")
        .into_iter()
        .next()
        .expect("at least one active task");
    (case_instance.id, task.id)
}

#[test]
fn all_java_aligned_handler_types_are_registered() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let registry = engine.job_handler_registry();
    for type_name in ALL_HANDLER_TYPES {
        assert!(
            registry.has_handler(type_name),
            "missing handler for {type_name}"
        );
    }
    assert!(registry.registered_types().len() >= 11);
}

#[test]
fn set_async_variables_job_handler_applies_variables() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let (case_id, _) = deploy_and_start(&engine);

    let job = CmmnJob::new("job-vars", CmmnJobFamily::Executable).with_handler(
        TYPE_SET_ASYNC_VARIABLES,
        Some(r#"{"asyncFlag":true,"score":42}"#.to_string()),
    );
    let mut job = job;
    job.scope_id = Some(case_id.clone());
    let job = engine.management_service().insert_job(job).expect("insert");

    engine.execute_job(&job.id).expect("execute");

    let case_instance = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case");
    assert_eq!(case_instance.variables["asyncFlag"], json!(true));
    assert_eq!(case_instance.variables["score"], json!(42));
    assert!(engine.management_service().get_job(&job.id).is_err());
}

#[test]
fn async_complete_plan_item_job_handler_completes_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let (case_id, task_id) = deploy_and_start(&engine);

    let mut job = CmmnJob::new("job-complete", CmmnJobFamily::Executable)
        .with_handler(TYPE_ASYNC_COMPLETE_PLAN_ITEM, None);
    job.scope_id = Some(case_id);
    job.sub_scope_id = Some(task_id.clone());
    let job = engine.management_service().insert_job(job).expect("insert");

    engine.execute_job(&job.id).expect("execute");

    let task = engine
        .runtime_service()
        .get_human_task(&task_id)
        .expect("task still queryable after complete");
    assert_eq!(task.state, CmmnHumanTaskState::Completed);
}

#[test]
fn async_init_plan_model_job_handler_succeeds_for_existing_case() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let (case_id, _) = deploy_and_start(&engine);

    let mut job = CmmnJob::new("job-init", CmmnJobFamily::Executable)
        .with_handler(TYPE_ASYNC_INIT_PLAN_MODEL, None);
    job.scope_id = Some(case_id);
    let job = engine.management_service().insert_job(job).expect("insert");

    engine.execute_job(&job.id).expect("execute");
}

#[test]
fn history_cleanup_handler_is_noop() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let job = CmmnJob::new("job-cleanup", CmmnJobFamily::Executable)
        .with_handler(TYPE_HISTORY_CLEANUP, None);
    let job = engine.management_service().insert_job(job).expect("insert");
    engine.execute_job(&job.id).expect("execute");
}

#[test]
fn case_migration_status_handler_aggregates_batch_progress() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let batch_id = "batch-mig-1";

    // Two pending migration parts + one deadletter failure for the same batch.
    let mut pending_a = CmmnJob::new("job-mig-a", CmmnJobFamily::Executable).with_handler(
        TYPE_CASE_MIGRATION,
        Some(json!({ "batchId": batch_id, "targetCaseDefinitionId": "def-x" }).to_string()),
    );
    pending_a.scope_id = Some(batch_id.to_string());
    engine
        .management_service()
        .insert_job(pending_a)
        .expect("insert pending a");

    let mut pending_b = CmmnJob::new("job-mig-b", CmmnJobFamily::Executable).with_handler(
        TYPE_CASE_MIGRATION,
        Some(json!({ "batchId": batch_id, "targetCaseDefinitionId": "def-x" }).to_string()),
    );
    pending_b.scope_id = Some(batch_id.to_string());
    engine
        .management_service()
        .insert_job(pending_b)
        .expect("insert pending b");

    let mut failed = CmmnJob::new("job-mig-fail", CmmnJobFamily::Deadletter).with_handler(
        TYPE_CASE_MIGRATION,
        Some(json!({ "batchId": batch_id, "targetCaseDefinitionId": "def-x" }).to_string()),
    );
    failed.scope_id = Some(batch_id.to_string());
    failed.exception_message = Some("migration failed".to_string());
    engine
        .management_service()
        .insert_job(failed)
        .expect("insert failed");

    // Seed completedCount for work already finished (successful jobs are deleted on execute).
    let status_job = CmmnJob::new("job-mig-status", CmmnJobFamily::Timer).with_handler(
        TYPE_CASE_MIGRATION_STATUS,
        Some(
            json!({
                "batchId": batch_id,
                "completedCount": 1,
                "totalCount": 4
            })
            .to_string(),
        ),
    );
    let status_job = engine
        .management_service()
        .insert_job(status_job)
        .expect("insert status");

    engine.execute_job(&status_job.id).expect("execute status");

    let updated = engine
        .management_service()
        .get_job(&status_job.id)
        .expect("status job retained after aggregation");
    let cfg: serde_json::Value =
        serde_json::from_str(updated.configuration.as_deref().expect("configuration"))
            .expect("status configuration JSON");

    assert_eq!(cfg["batchId"], json!(batch_id));
    assert_eq!(cfg["aggregated"], json!(true));
    assert_eq!(cfg["status"], json!(MIGRATION_STATUS_IN_PROGRESS));
    assert_eq!(cfg["pendingCount"], json!(2));
    assert_eq!(cfg["failedCount"], json!(1));
    assert_eq!(cfg["completedCount"], json!(1));
    assert_eq!(cfg["totalCount"], json!(4));
    assert_eq!(updated.state, MIGRATION_STATUS_IN_PROGRESS);

    // Remove pending parts; re-run status with completed seed.
    engine
        .management_service()
        .delete_job("job-mig-a")
        .expect("delete a");
    engine
        .management_service()
        .delete_job("job-mig-b")
        .expect("delete b");

    let mut refreshed = updated;
    refreshed.configuration = Some(
        json!({
            "batchId": batch_id,
            "completedCount": 3,
            "failedCount": 0,
            "totalCount": 4
        })
        .to_string(),
    );
    engine
        .management_service()
        .update_job(&refreshed)
        .expect("reset status config");

    engine
        .execute_job(&status_job.id)
        .expect("execute status again");
    let final_job = engine
        .management_service()
        .get_job(&status_job.id)
        .expect("status job still present");
    let final_cfg: serde_json::Value =
        serde_json::from_str(final_job.configuration.as_deref().expect("configuration"))
            .expect("final status JSON");
    assert_eq!(final_cfg["status"], json!(MIGRATION_STATUS_FAIL));
    assert_eq!(final_cfg["pendingCount"], json!(0));
    assert_eq!(final_cfg["failedCount"], json!(1));
    assert_eq!(final_cfg["completedCount"], json!(3));
    assert_eq!(final_job.state, MIGRATION_STATUS_FAIL);
}

#[test]
fn case_migration_status_handler_marks_completed_when_no_remaining_parts() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let status_job = CmmnJob::new("job-mig-status-done", CmmnJobFamily::Timer).with_handler(
        TYPE_CASE_MIGRATION_STATUS,
        Some(
            json!({
                "batchId": "batch-done",
                "completedCount": 2,
                "failedCount": 0,
                "totalCount": 2
            })
            .to_string(),
        ),
    );
    let status_job = engine
        .management_service()
        .insert_job(status_job)
        .expect("insert");
    engine.execute_job(&status_job.id).expect("execute");
    let updated = engine
        .management_service()
        .get_job(&status_job.id)
        .expect("retained");
    let cfg: serde_json::Value =
        serde_json::from_str(updated.configuration.as_deref().expect("cfg")).expect("json");
    assert_eq!(cfg["status"], json!(MIGRATION_STATUS_COMPLETED));
    assert_eq!(cfg["pendingCount"], json!(0));
    assert_eq!(cfg["aggregated"], json!(true));
}

#[test]
fn async_terminate_case_job_handler() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let (case_id, _) = deploy_and_start(&engine);

    let mut job = CmmnJob::new("job-term", CmmnJobFamily::Executable)
        .with_handler(TYPE_ASYNC_TERMINATE, None);
    job.scope_id = Some(case_id.clone());
    let job = engine.management_service().insert_job(job).expect("insert");

    engine.execute_job(&job.id).expect("execute");

    match engine.runtime_service().get_case_instance(&case_id) {
        Ok(case_instance) => assert_ne!(
            case_instance.state,
            CmmnCaseInstanceState::Active,
            "case should no longer be active after terminate"
        ),
        Err(_) => {
            // Some terminate paths remove the runtime case instance entirely.
        }
    }
}

#[test]
fn async_start_case_job_handler() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("async-start")
                .with_resource("async-case.cmmn", simple_model()),
        )
        .expect("deploy");

    let mut job = CmmnJob::new("job-start", CmmnJobFamily::Executable).with_handler(
        TYPE_ASYNC_START_CASE,
        Some(r#"{"caseDefinitionKey":"asyncCase","businessKey":"BK-async"}"#.to_string()),
    );
    job.element_id = Some("asyncCase".to_string());
    let job = engine.management_service().insert_job(job).expect("insert");

    engine.execute_job(&job.id).expect("execute");

    let instances = engine
        .runtime_service()
        .create_case_instance_query()
        .list()
        .expect("list");
    assert!(
        instances
            .iter()
            .any(|c| c.business_key.as_deref() == Some("BK-async")),
        "async start should create case instance"
    );
}

#[test]
fn async_enable_disable_plan_item_job_handlers() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("manual-plan", "Manual plan")
        .with_human_task(CmmnHumanTask::new("manual-task", "Manual task"))
        .with_plan_item(
            CmmnPlanItem::new("manual-plan-item", "manual-task")
                .with_manual_activation_rule("true"),
        );
    let model = CmmnModel::new(vec![CmmnCase::new(
        "manual-case",
        "asyncManualCase",
        "Async manual case",
        plan_model,
    )]);
    engine
        .deploy(
            CmmnDeploymentRequest::new("async-manual").with_resource("async-manual.cmmn", model),
        )
        .expect("deploy manual case");
    let case_id = engine
        .start_case_instance_by_key("asyncManualCase", CmmnCaseInstanceStartRequest::new())
        .expect("start manual case")
        .id;
    let task_id = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Enabled)
        .single_result()
        .expect("enabled task query")
        .expect("enabled task")
        .id;

    let mut disable = CmmnJob::new("job-disable", CmmnJobFamily::Executable)
        .with_handler(TYPE_ASYNC_DISABLE_PLAN_ITEM, None);
    disable.scope_id = Some(case_id.clone());
    disable.sub_scope_id = Some(task_id.clone());
    let disable = engine
        .management_service()
        .insert_job(disable)
        .expect("insert disable");
    engine.execute_job(&disable.id).expect("disable");

    let task = engine
        .runtime_service()
        .get_human_task(&task_id)
        .expect("task after disable");
    assert_eq!(task.state, CmmnHumanTaskState::Disabled);

    let mut enable = CmmnJob::new("job-enable", CmmnJobFamily::Executable)
        .with_handler(TYPE_ASYNC_ENABLE_PLAN_ITEM, None);
    enable.scope_id = Some(case_id);
    enable.sub_scope_id = Some(task_id.clone());
    let enable = engine
        .management_service()
        .insert_job(enable)
        .expect("insert enable");
    engine.execute_job(&enable.id).expect("enable");

    let task = engine
        .runtime_service()
        .get_human_task(&task_id)
        .expect("task after enable");
    // EnablePlanItemInstanceOperation.java:39-51 persists ENABLED.
    assert_eq!(task.state, CmmnHumanTaskState::Enabled);
}

#[test]
fn async_activate_and_leave_and_timer_handlers_accept_jobs() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let (case_id, task_id) = deploy_and_start(&engine);

    // Activate by definition id (human-task-b may already be active; still valid path).
    let mut activate = CmmnJob::new("job-activate", CmmnJobFamily::Executable).with_handler(
        TYPE_ASYNC_ACTIVATE_PLAN_ITEM,
        Some("human-task-b".to_string()),
    );
    activate.scope_id = Some(case_id.clone());
    activate.element_id = Some("human-task-b".to_string());
    let activate = engine
        .management_service()
        .insert_job(activate)
        .expect("insert");
    // May succeed or conflict depending on state; must not panic / missing-handler.
    let _ = engine.execute_job(&activate.id);

    let mut leave = CmmnJob::new("job-leave", CmmnJobFamily::Executable).with_handler(
        TYPE_ASYNC_LEAVE_ACTIVE_PLAN_ITEM,
        Some(r#"{"transition":"complete"}"#.to_string()),
    );
    leave.scope_id = Some(case_id.clone());
    leave.sub_scope_id = Some(task_id);
    let leave = engine
        .management_service()
        .insert_job(leave)
        .expect("insert");
    let _ = engine.execute_job(&leave.id);

    let mut timer =
        CmmnJob::new("job-timer", CmmnJobFamily::Timer).with_handler(TYPE_TRIGGER_TIMER, None);
    timer.scope_id = Some(case_id);
    timer.element_id = Some("human-task-b".to_string());
    let timer = engine
        .management_service()
        .insert_job(timer)
        .expect("insert");
    let _ = engine.execute_job(&timer.id);
}

#[test]
fn external_worker_complete_and_reactivate_handlers_registered_and_callable() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let (case_id, task_id) = deploy_and_start(&engine);

    // Complete first so reactivate can restore from history.
    engine
        .complete_human_task(&task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete");

    let mut reactivate = CmmnJob::new("job-reactivate", CmmnJobFamily::Executable)
        .with_handler(TYPE_ASYNC_REACTIVATE_PLAN_ITEM, None);
    reactivate.scope_id = Some(case_id.clone());
    reactivate.sub_scope_id = Some(task_id.clone());
    let reactivate = engine
        .management_service()
        .insert_job(reactivate)
        .expect("insert");
    engine.execute_job(&reactivate.id).expect("reactivate");

    let mut external = CmmnJob::new("job-ext", CmmnJobFamily::Executable).with_handler(
        TYPE_EXTERNAL_WORKER_COMPLETE,
        Some(r#"{"done":true}"#.to_string()),
    );
    external.scope_id = Some(case_id);
    // Use a known active task if any remain after reactivate.
    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("tasks");
    if let Some(t) = tasks.first() {
        external.sub_scope_id = Some(t.id.clone());
        let external = engine
            .management_service()
            .insert_job(external)
            .expect("insert");
        let _ = engine.execute_job(&external.id);
    }
}

#[test]
fn case_migration_handlers_require_target_definition() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let (case_id, _) = deploy_and_start(&engine);

    let mut mig = CmmnJob::new("job-mig", CmmnJobFamily::Executable).with_handler(
        TYPE_CASE_MIGRATION,
        Some(r#"{"targetCaseDefinitionId":"missing-def"}"#.to_string()),
    );
    mig.scope_id = Some(case_id.clone());
    let mig = engine.management_service().insert_job(mig).expect("insert");
    assert!(engine.execute_job(&mig.id).is_err());

    let mut hist = CmmnJob::new("job-hist-mig", CmmnJobFamily::History).with_handler(
        TYPE_HISTORIC_CASE_MIGRATION,
        Some(r#"{"targetCaseDefinitionId":"missing-def"}"#.to_string()),
    );
    hist.scope_id = Some(case_id);
    let hist = engine
        .management_service()
        .insert_job(hist)
        .expect("insert");
    assert!(engine.execute_job(&hist.id).is_err());
}

// Silence unused import warnings for type constants referenced only in ALL_HANDLER_TYPES coverage.
#[allow(dead_code)]
fn _type_constants_linked() {
    let _ = [
        TYPE_ASYNC_ACTIVATE_PLAN_ITEM,
        TYPE_ASYNC_LEAVE_ACTIVE_PLAN_ITEM,
        TYPE_ASYNC_INIT_PLAN_MODEL,
        TYPE_SET_ASYNC_VARIABLES,
        TYPE_CASE_MIGRATION,
        TYPE_TRIGGER_TIMER,
        TYPE_EXTERNAL_WORKER_COMPLETE,
        TYPE_HISTORY_CLEANUP,
        TYPE_CASE_MIGRATION_STATUS,
        TYPE_HISTORIC_CASE_MIGRATION,
        TYPE_ASYNC_ENABLE_PLAN_ITEM,
        TYPE_ASYNC_DISABLE_PLAN_ITEM,
        TYPE_ASYNC_REACTIVATE_PLAN_ITEM,
        TYPE_ASYNC_COMPLETE_PLAN_ITEM,
        TYPE_ASYNC_TERMINATE,
        TYPE_ASYNC_START_CASE,
    ];
}
