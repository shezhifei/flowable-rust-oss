// C3: Stage / case plan model autoComplete parity tests.
//
// Java references:
// - Stage.java:29-30 (autoComplete flag + autoCompleteCondition expression)
// - ExpressionUtil.java:260-265 (evaluateAutoComplete: condition overrides static flag)
// - PlanItemInstanceContainerUtil.java:73-169 (shouldPlanItemContainerComplete):
//   :91-97 ACTIVE plan items always block completion
//   :102-118 required plan items always block completion
//   :143-146 AVAILABLE/ENABLED plan items only block when the container is not autocomplete

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnChangePlanItemStateRequest, CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask,
    CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnModel, CmmnPlanItem, CmmnStage,
};
use serde_json::json;

/// Stage with one immediately active task and one manual-activation (ENABLED) task.
fn stage_auto_complete_model(
    case_key: &str,
    configure_stage: impl FnOnce(CmmnStage) -> CmmnStage,
) -> CmmnModel {
    let stage = configure_stage(
        CmmnStage::new("stage-work", "Work stage")
            .with_human_task(CmmnHumanTask::new("task-active", "Active task"))
            .with_human_task(CmmnHumanTask::new("task-optional", "Optional task"))
            .with_plan_item(CmmnPlanItem::new("plan-item-active", "task-active"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-optional", "task-optional")
                    .with_manual_activation_rule("manualActivation == true"),
            ),
    );

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_stage(stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-stage", "stage-work"));

    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "Stage auto complete case",
        plan_model,
    )])
}

fn deploy_and_start(
    engine: &CmmnEngine,
    model: CmmnModel,
    case_key: &str,
    variables: serde_json::Value,
) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{case_key}-deployment"))
                .with_resource(format!("{case_key}.cmmn"), model),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(
            case_key,
            CmmnCaseInstanceStartRequest::new().with_variables(variables),
        )
        .expect("case instance")
        .id
}

fn complete_single_active_task(engine: &CmmnEngine, case_instance_id: &str, definition_id: &str) {
    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("active task query")
        .expect("active task");
    assert_eq!(task.task_definition_id, definition_id);
    engine
        .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("task completion");
}

fn case_state(engine: &CmmnEngine, case_instance_id: &str) -> CmmnCaseInstanceState {
    engine
        .runtime_service()
        .get_case_instance(case_instance_id)
        .expect("case instance")
        .state
}

fn enabled_task_definition_ids(engine: &CmmnEngine, case_instance_id: &str) -> Vec<String> {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Enabled)
        .list()
        .expect("enabled tasks")
        .into_iter()
        .map(|task| task.task_definition_id)
        .collect()
}

// Java: PlanItemInstanceContainerUtil.java:143-146 - without autocomplete an ENABLED
// plan item blocks stage (and thus case) completion.
#[test]
fn stage_without_auto_complete_stays_open_with_enabled_manual_activation_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        stage_auto_complete_model("nonAutoCompleteStageCase", |stage| stage),
        "nonAutoCompleteStageCase",
        json!({ "manualActivation": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Active,
        "ENABLED manual-activation task must block a non-autocomplete stage"
    );
    assert_eq!(
        enabled_task_definition_ids(&engine, &case_instance_id),
        vec!["task-optional".to_string()]
    );
}

// Java: PlanItemInstanceContainerUtil.java:143-146 - with autocomplete the ENABLED
// plan item no longer blocks; the container completes and exits the skipped child.
#[test]
fn auto_complete_stage_completes_despite_enabled_task_and_terminates_it() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        stage_auto_complete_model("autoCompleteStageCase", |stage| {
            stage.with_auto_complete(true)
        }),
        "autoCompleteStageCase",
        json!({ "manualActivation": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Completed,
        "autocomplete stage must complete even with an ENABLED manual-activation task"
    );
    assert!(
        enabled_task_definition_ids(&engine, &case_instance_id).is_empty(),
        "residual ENABLED task must be exited when the autocomplete stage completes"
    );
    let case_instance = engine
        .runtime_service()
        .get_case_instance(&case_instance_id)
        .expect("case instance");
    assert!(case_instance.ended_at.is_some());
}

// Java: ExpressionUtil.java:260-265 - a non-empty autoCompleteCondition overrides the
// static autoComplete flag (here: flag false, condition true).
#[test]
fn auto_complete_condition_true_overrides_static_flag_false() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        stage_auto_complete_model("autoCompleteConditionTrueCase", |stage| {
            stage.with_auto_complete_condition("autoComplete == true")
        }),
        "autoCompleteConditionTrueCase",
        json!({ "manualActivation": true, "autoComplete": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Completed,
        "true autoCompleteCondition must enable autocomplete even when the flag is false"
    );
    assert!(enabled_task_definition_ids(&engine, &case_instance_id).is_empty());
}

// Java: ExpressionUtil.java:260-265 - the condition also overrides in the other
// direction (flag true, condition false -> no autocomplete).
#[test]
fn auto_complete_condition_false_overrides_static_flag_true() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        stage_auto_complete_model("autoCompleteConditionFalseCase", |stage| {
            stage
                .with_auto_complete(true)
                .with_auto_complete_condition("autoComplete == true")
        }),
        "autoCompleteConditionFalseCase",
        json!({ "manualActivation": true, "autoComplete": false }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Active,
        "false autoCompleteCondition must disable autocomplete even when the flag is true"
    );
    assert_eq!(
        enabled_task_definition_ids(&engine, &case_instance_id),
        vec!["task-optional".to_string()]
    );
}

// Java: PlanItemInstanceContainerUtil.java:91-97 - ACTIVE plan items always block,
// autocomplete or not.
#[test]
fn auto_complete_stage_still_blocked_by_active_task() {
    let stage = CmmnStage::new("stage-work", "Work stage")
        .with_auto_complete(true)
        .with_human_task(CmmnHumanTask::new("task-first", "First task"))
        .with_human_task(CmmnHumanTask::new("task-second", "Second task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-first", "task-first"))
        .with_plan_item(CmmnPlanItem::new("plan-item-second", "task-second"));
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_stage(stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-stage", "stage-work"));
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-autoCompleteActiveBlocksCase",
        "autoCompleteActiveBlocksCase",
        "Auto complete active blocks case",
        plan_model,
    )]);

    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id =
        deploy_and_start(&engine, model, "autoCompleteActiveBlocksCase", json!({}));

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 2);
    let first_task = active_tasks
        .iter()
        .find(|task| task.task_definition_id == "task-first")
        .expect("first task");
    engine
        .complete_human_task(&first_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("first completion");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Active,
        "an ACTIVE task must block completion even in an autocomplete stage"
    );

    complete_single_active_task(&engine, &case_instance_id, "task-second");
    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Completed
    );
}

// Java: PlanItemInstanceContainerUtil.java:102-118 - required plan items always block,
// autocomplete or not.
#[test]
fn auto_complete_stage_still_blocked_by_incomplete_required_task() {
    let stage = CmmnStage::new("stage-work", "Work stage")
        .with_auto_complete(true)
        .with_human_task(CmmnHumanTask::new("task-active", "Active task"))
        .with_human_task(CmmnHumanTask::new("task-required", "Required task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-active", "task-active"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-required", "task-required")
                .with_manual_activation_rule("manualActivation == true")
                .with_required_rule("required == true"),
        );
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_stage(stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-stage", "stage-work"));
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-autoCompleteRequiredBlocksCase",
        "autoCompleteRequiredBlocksCase",
        "Auto complete required blocks case",
        plan_model,
    )]);

    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model,
        "autoCompleteRequiredBlocksCase",
        json!({ "manualActivation": true, "required": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Active,
        "an incomplete required task must block completion even in an autocomplete stage"
    );

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance_id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["task-required".to_string()],
                ..CmmnChangePlanItemStateRequest::default()
            },
        )
        .expect("activate required task");
    complete_single_active_task(&engine, &case_instance_id, "task-required");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Completed,
        "case must complete once the required task is done"
    );
}

// Java: PlanItemInstanceContainerUtil.java:143-146 applied to the case plan model itself
// (the case plan model is a Stage in Java, Stage.java:29-30).
#[test]
fn case_plan_model_auto_complete_ignores_available_manual_activation_task() {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_auto_complete(true)
        .with_human_task(CmmnHumanTask::new("task-main", "Main task"))
        .with_human_task(CmmnHumanTask::new("task-optional", "Optional task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-main", "task-main"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-optional", "task-optional")
                .with_manual_activation_rule("manualActivation == true"),
        );
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-autoCompletePlanModelCase",
        "autoCompletePlanModelCase",
        "Auto complete case plan model case",
        plan_model,
    )]);

    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model,
        "autoCompletePlanModelCase",
        json!({ "manualActivation": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-main");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Completed,
        "autocomplete case plan model must complete despite the AVAILABLE task"
    );
    assert!(
        enabled_task_definition_ids(&engine, &case_instance_id).is_empty(),
        "residual AVAILABLE task must be exited when the autocomplete case completes"
    );
}
