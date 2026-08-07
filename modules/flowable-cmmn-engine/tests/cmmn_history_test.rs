use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
};
use serde_json::json;

fn history_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));

    CmmnModel::new(vec![CmmnCase::new(
        "case-history",
        "historyCase",
        "History case",
        plan_model,
    )])
}

#[test]
fn records_case_and_human_task_history_and_filters_by_business_key_and_completion_actor() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("history")
                .with_resource("history-case.cmmn", history_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "historyCase",
            CmmnCaseInstanceStartRequest::new()
                .with_business_key("BK-42")
                .with_started_by("starter")
                .with_variables(json!({ "customer": "acme" })),
        )
        .expect("case instance");

    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task query")
        .expect("task");

    engine
        .complete_human_task(
            &task.id,
            CmmnHumanTaskCompletionRequest::new().with_completed_by("reviewer"),
        )
        .expect("task completion");

    let historic_case = engine
        .history_service()
        .create_historic_case_instance_query()
        .business_key("BK-42")
        .single_result()
        .expect("historic case query")
        .expect("historic case");

    assert_eq!(historic_case.case_instance_id, case_instance.id);
    assert_eq!(historic_case.state, CmmnCaseInstanceState::Completed);
    assert_eq!(historic_case.started_by.as_deref(), Some("starter"));
    assert_eq!(historic_case.variables["customer"], json!("acme"));

    let historic_task = engine
        .history_service()
        .create_historic_human_task_query()
        .case_instance_id(&case_instance.id)
        .completed_by("reviewer")
        .single_result()
        .expect("historic task query")
        .expect("historic task");

    assert_eq!(historic_task.task_id, task.id);
    assert_eq!(historic_task.state, CmmnHumanTaskState::Completed);
    assert_eq!(historic_task.completed_by.as_deref(), Some("reviewer"));
    assert!(historic_task.completed_at.is_some());
}

#[test]
fn returns_historic_records_by_id() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("history")
                .with_resource("history-case.cmmn", history_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("historyCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");
    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task query")
        .expect("task");

    let historic_case = engine
        .history_service()
        .get_historic_case_instance(&case_instance.id)
        .expect("historic case by id");
    let historic_task = engine
        .history_service()
        .get_historic_human_task(&task.id)
        .expect("historic task by id");

    assert_eq!(historic_case.case_instance_id, case_instance.id);
    assert_eq!(historic_task.task_id, task.id);
}
