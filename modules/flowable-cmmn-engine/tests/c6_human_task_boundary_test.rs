// C6 parity tests: human task boundary behavior.
// Java references:
// - Task.java:20 — blocking defaults to true.
// - HumanTaskActivityBehavior.java:83,173-177 — a non-blocking human task creates no
//   task entry and completes its plan item immediately (manual task semantics).
// - HumanTaskActivityBehavior.java:148,456-464 — a declared taskIdVariableName stores
//   the created task id in a variable (literal in Rust: no expression engine).
// - HumanTaskActivityBehavior.java:498-507 — on the complete transition a declared
//   taskCompleterVariableName stores the completing user.
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnModel, CmmnPlanItem, CmmnPlanItemOnPart, CmmnSentry,
};
use serde_json::json;

fn deploy_case(engine: &CmmnEngine, deployment_key: &str, case_key: &str, model: CmmnModel) {
    engine
        .deploy(CmmnDeploymentRequest::new(deployment_key).with_resource("case.cmmn", model))
        .expect("deployment");
    let _ = case_key;
}

fn start_case(engine: &CmmnEngine, case_key: &str) -> String {
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn single_case_model(case_key: &str, plan_model: CmmnCasePlanModel) -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        "case-human-task-boundary",
        case_key,
        "Human task boundary case",
        plan_model,
    )])
}

#[test]
fn non_blocking_human_task_creates_no_task_entry() {
    // HumanTaskActivityBehavior.java:173-177 — "if not blocking, treat as a manual
    // task. No need to create a task entry."
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("human-task-non-blocking", "Non blocking").with_blocking(false),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-non-blocking",
            "human-task-non-blocking",
        ))
        .with_human_task(CmmnHumanTask::new("human-task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-keepalive",
            "human-task-keepalive",
        ));
    deploy_case(
        &engine,
        "non-blocking-entry",
        "nonBlockingEntryCase",
        single_case_model("nonBlockingEntryCase", plan_model),
    );

    let case_id = start_case(&engine, "nonBlockingEntryCase");

    // Only the blocking keepalive task exists; no entry was inserted for the
    // non-blocking plan item.
    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .list()
        .expect("task list");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Keep alive");
    assert_eq!(tasks[0].state, CmmnHumanTaskState::Active);
}

#[test]
fn case_with_only_non_blocking_human_task_completes_immediately() {
    // The immediate plan-item completion leaves nothing active, so the case
    // completion evaluation at the end of the start closes the case.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("human-task-non-blocking", "Non blocking").with_blocking(false),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-non-blocking",
            "human-task-non-blocking",
        ));
    deploy_case(
        &engine,
        "non-blocking-complete",
        "nonBlockingCompleteCase",
        single_case_model("nonBlockingCompleteCase", plan_model),
    );

    let case_id = start_case(&engine, "nonBlockingCompleteCase");

    let case_instance = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert_eq!(case_instance.state, CmmnCaseInstanceState::Completed);
}

#[test]
fn non_blocking_completion_fires_downstream_sentry() {
    // The immediate completion emits the complete standard event, so a dependent
    // sentry onPart activates its plan item during the same start.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("human-task-non-blocking", "Non blocking").with_blocking(false),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-non-blocking",
            "human-task-non-blocking",
        ))
        .with_human_task(CmmnHumanTask::new("human-task-follow-up", "Follow up"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-follow-up", "human-task-follow-up")
                .with_entry_criterion("sentry-after-non-blocking"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-non-blocking",
            CmmnPlanItemOnPart::new(
                "on-non-blocking-complete",
                "plan-item-non-blocking",
                "complete",
            ),
        ));
    deploy_case(
        &engine,
        "non-blocking-sentry",
        "nonBlockingSentryCase",
        single_case_model("nonBlockingSentryCase", plan_model),
    );

    let case_id = start_case(&engine, "nonBlockingSentryCase");

    let follow_up = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("follow-up query")
        .expect("follow-up task");
    assert_eq!(follow_up.name, "Follow up");
}

#[test]
fn task_id_variable_name_stores_task_id_on_case() {
    // HumanTaskActivityBehavior.java:456-464 — the created task id is stored under
    // the declared variable name right after the task insert.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("human-task-review", "Review")
                .with_task_id_variable_name("reviewTaskId"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));
    deploy_case(
        &engine,
        "task-id-variable",
        "taskIdVariableCase",
        single_case_model("taskIdVariableCase", plan_model),
    );

    let case_id = start_case(&engine, "taskIdVariableCase");

    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task query")
        .expect("review task");
    let case_instance = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert_eq!(case_instance.variables["reviewTaskId"], json!(task.id));
}

#[test]
fn task_completer_variable_name_stores_completer_on_complete() {
    // HumanTaskActivityBehavior.java:498-507 — the completing user lands in the
    // declared variable on the complete transition.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("human-task-review", "Review")
                .with_task_completer_variable_name("reviewCompletedBy"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
        .with_human_task(CmmnHumanTask::new("human-task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-keepalive",
            "human-task-keepalive",
        ));
    deploy_case(
        &engine,
        "task-completer-variable",
        "taskCompleterVariableCase",
        single_case_model("taskCompleterVariableCase", plan_model),
    );

    let case_id = start_case(&engine, "taskCompleterVariableCase");

    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("task list")
        .into_iter()
        .find(|task| task.name == "Review")
        .expect("review task");
    engine
        .complete_human_task(
            &task.id,
            CmmnHumanTaskCompletionRequest::new().with_completed_by("kermit"),
        )
        .expect("completion");

    let case_instance = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert_eq!(case_instance.variables["reviewCompletedBy"], json!("kermit"));
}

#[test]
fn blocking_human_task_without_declarations_keeps_default_behavior() {
    // Regression guard: the default blocking task keeps the pre-C6 behavior — a
    // task entry is created and no boundary variables are written.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));
    deploy_case(
        &engine,
        "blocking-default",
        "blockingDefaultCase",
        single_case_model("blockingDefaultCase", plan_model),
    );

    let case_id = start_case(&engine, "blockingDefaultCase");

    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task query")
        .expect("review task");
    engine
        .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("completion");

    let case_instance = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert!(case_instance.variables.is_empty());
    assert_eq!(case_instance.state, CmmnCaseInstanceState::Completed);
}
