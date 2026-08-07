// C8: parentCompletionRule / completionNeutralRule parity tests.
//
// Java references (PlanItemInstanceContainerUtil.java, tag flowable 6.x):
// - :86       ParentCompletionRule IGNORE always skips the plan item.
// - :122-125  IGNORE_IF_AVAILABLE_OR_ENABLED skips AVAILABLE or ENABLED plan items.
// - :128-131  IGNORE_IF_AVAILABLE (and completionNeutral) skips AVAILABLE plan items.
// - :91-97    ACTIVE plan items keep blocking unless shouldIgnorePlanItemForCompletion applies
//             (which the plain ignoreIf* rules never do for a non-repeatable ACTIVE item).
//
// These exercise the reload-based, bucket-aware subtraction wired into
// maybe_complete_case: a rule-bearing plan item that Java would skip must no longer
// block completion, while a state the rule does not cover must still block.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnMilestone, CmmnModel, CmmnPlanItem, CmmnPlanItemOnPart, CmmnSentry,
};
use serde_json::json;

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

/// Case plan model with an immediately active task and a manual-activation (ENABLED)
/// human task whose plan item carries `rule`/`completion_neutral`.
fn model_with_available_rule_task(
    case_key: &str,
    parent_completion_rule: Option<&str>,
    completion_neutral: Option<&str>,
) -> CmmnModel {
    let mut optional_plan_item = CmmnPlanItem::new("plan-item-optional", "task-optional")
        .with_manual_activation_rule("manualActivation == true");
    if let Some(rule) = parent_completion_rule {
        optional_plan_item = optional_plan_item.with_parent_completion_rule(rule);
    }
    if let Some(expression) = completion_neutral {
        optional_plan_item = optional_plan_item.with_completion_neutral_rule(expression);
    }

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-active", "Active task"))
        .with_human_task(CmmnHumanTask::new("task-optional", "Optional task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-active", "task-active"))
        .with_plan_item(optional_plan_item);

    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "Parent completion rule case",
        plan_model,
    )])
}

/// Case plan model with an active task and a manual-activation milestone (ENABLED marker)
/// whose plan item carries `parent_completion_rule`.
fn model_with_enabled_milestone(case_key: &str, parent_completion_rule: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-active", "Active task"))
        .with_milestone(CmmnMilestone::new(
            "milestone-optional",
            "Optional milestone",
        ))
        .with_plan_item(CmmnPlanItem::new("plan-item-active", "task-active"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-milestone", "milestone-optional")
                .with_manual_activation_rule("manualActivation == true")
                .with_parent_completion_rule(parent_completion_rule),
        );

    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "Parent completion rule milestone case",
        plan_model,
    )])
}

fn model_with_available_milestone(case_key: &str, required: bool) -> CmmnModel {
    let mut milestone_plan_item =
        CmmnPlanItem::new("plan-item-milestone", "milestone-waiting")
            .with_entry_criterion("sentry-never");
    if required {
        milestone_plan_item = milestone_plan_item.with_required_rule("required == true");
    }
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-active", "Active task"))
        .with_milestone(CmmnMilestone::new("milestone-waiting", "Waiting milestone"))
        .with_plan_item(CmmnPlanItem::new("plan-item-active", "task-active"))
        .with_plan_item(milestone_plan_item)
        .with_sentry(CmmnSentry::new(
            "sentry-never",
            CmmnPlanItemOnPart::new("on-active-disable", "plan-item-active", "disable"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "Available milestone completion case",
        plan_model,
    )])
}

#[test]
fn unrequired_available_milestone_does_not_block_parent_completion() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model_with_available_milestone("c8AvailableMilestone", false),
        "c8AvailableMilestone",
        json!({}),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Completed,
        "PlanItemInstanceContainerUtil.java:86-146 does not let an optional AVAILABLE milestone block"
    );
}

#[test]
fn required_available_milestone_blocks_parent_completion() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model_with_available_milestone("c8RequiredAvailableMilestone", true),
        "c8RequiredAvailableMilestone",
        json!({ "required": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Active,
        "PlanItemInstanceContainerUtil.java:102-118 keeps required AVAILABLE milestones blocking"
    );
}

// Java :86 - IGNORE always skips, so the ENABLED task never blocks completion.
#[test]
fn ignore_rule_unlocks_enabled_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model_with_available_rule_task("c8IgnoreAvailableCase", Some("ignore"), None),
        "c8IgnoreAvailableCase",
        json!({ "manualActivation": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Completed,
        "an IGNORE plan item must not block completion"
    );
}

// Java :128-131 - IGNORE_IF_AVAILABLE does not skip an ENABLED task.
#[test]
fn ignore_if_available_does_not_unlock_enabled_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model_with_available_rule_task("c8IgnoreIfAvailableCase", Some("ignoreIfAvailable"), None),
        "c8IgnoreIfAvailableCase",
        json!({ "manualActivation": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Active,
        "ignoreIfAvailable must not skip an ENABLED plan item"
    );
}

// Java :91-97 - IGNORE_IF_AVAILABLE only covers AVAILABLE; an ACTIVE task still blocks.
#[test]
fn ignore_if_available_does_not_unlock_active_task() {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-first", "First task"))
        .with_human_task(CmmnHumanTask::new("task-second", "Second task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-first", "task-first"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-second", "task-second")
                .with_parent_completion_rule("ignoreIfAvailable"),
        );
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-c8IgnoreIfAvailableActiveCase",
        "c8IgnoreIfAvailableActiveCase",
        "Ignore if available active case",
        plan_model,
    )]);

    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id =
        deploy_and_start(&engine, model, "c8IgnoreIfAvailableActiveCase", json!({}));

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
        "ignoreIfAvailable must not skip an ACTIVE plan item"
    );

    complete_single_active_task(&engine, &case_instance_id, "task-second");
    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Completed
    );
}

// Java :122-125 - IGNORE_IF_AVAILABLE_OR_ENABLED skips an ENABLED (manual-activation) plan item.
#[test]
fn ignore_if_available_or_enabled_unlocks_enabled_milestone() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model_with_enabled_milestone(
            "c8IgnoreIfAvailableOrEnabledCase",
            "ignoreIfAvailableOrEnabled",
        ),
        "c8IgnoreIfAvailableOrEnabledCase",
        json!({ "manualActivation": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Completed,
        "ignoreIfAvailableOrEnabled must skip an ENABLED plan item"
    );
}

// Java :128-131 - IGNORE_IF_AVAILABLE does not cover ENABLED, so the marker keeps blocking.
#[test]
fn ignore_if_available_does_not_unlock_enabled_milestone() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model_with_enabled_milestone("c8IgnoreIfAvailableEnabledCase", "ignoreIfAvailable"),
        "c8IgnoreIfAvailableEnabledCase",
        json!({ "manualActivation": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Active,
        "ignoreIfAvailable must not skip an ENABLED plan item"
    );
}

// Java :128-131 - completionNeutral only skips AVAILABLE, never ENABLED.
#[test]
fn completion_neutral_true_does_not_unlock_enabled_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model_with_available_rule_task(
            "c8CompletionNeutralTrueCase",
            None,
            Some("neutral == true"),
        ),
        "c8CompletionNeutralTrueCase",
        json!({ "manualActivation": true, "neutral": true }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Active,
        "a true completionNeutral condition must not skip an ENABLED plan item"
    );
}

// Java :128-131 - a false completionNeutral condition leaves the ENABLED task blocking.
#[test]
fn completion_neutral_false_keeps_enabled_task_blocking() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        model_with_available_rule_task(
            "c8CompletionNeutralFalseCase",
            None,
            Some("neutral == true"),
        ),
        "c8CompletionNeutralFalseCase",
        json!({ "manualActivation": true, "neutral": false }),
    );

    complete_single_active_task(&engine, &case_instance_id, "task-active");

    assert_eq!(
        case_state(&engine, &case_instance_id),
        CmmnCaseInstanceState::Active,
        "a false completionNeutral condition must keep the ENABLED plan item blocking"
    );
}
