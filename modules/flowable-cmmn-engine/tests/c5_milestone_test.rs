// C5 parity tests: milestone reach edge behavior.
// Java references:
// - MilestoneActivityBehavior.java:47-53 — a declared milestoneVariable is set to true on the
//   case instance when the milestone is reached.
// - MilestoneActivityBehavior.java:55-61 — a declared businessStatus updates the case instance
//   business status via CmmnRuntimeService#updateBusinessStatus.
// - MilestoneActivityBehavior.java:64 — the occur operation is planned after both updates, so
//   downstream sentry ifParts observe the new values.
// - Milestone.java:22-23 — milestoneVariable/businessStatus model fields (literals in Rust:
//   no expression engine).
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnHumanTask, CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnMilestone, CmmnModel,
    CmmnPlanItem, CmmnPlanItemOnPart, CmmnSentry,
};
use serde_json::json;

// Trigger task -> milestone (entry criterion on trigger completion); keepalive task keeps the
// case open after the milestone occurs.
fn milestone_model(case_key: &str, milestone: CmmnMilestone) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-trigger", "Trigger"))
        .with_plan_item(CmmnPlanItem::new("plan-item-trigger", "human-task-trigger"))
        .with_milestone(milestone)
        .with_plan_item(
            CmmnPlanItem::new("plan-item-milestone", "milestone-shipped")
                .with_entry_criterion("sentry-after-trigger"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-keepalive",
            "human-task-keepalive",
        ))
        .with_sentry(CmmnSentry::new(
            "sentry-after-trigger",
            CmmnPlanItemOnPart::new("on-trigger-complete", "plan-item-trigger", "complete"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-milestone",
        case_key,
        "Milestone case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine, case_key: &str, milestone: CmmnMilestone) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(case_key)
                .with_resource("case.cmmn", milestone_model(case_key, milestone)),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn complete_trigger_task(engine: &CmmnEngine, case_instance_id: &str) {
    let trigger_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .find(|task| task.name == "Trigger")
        .expect("trigger task");
    engine
        .complete_human_task(&trigger_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("trigger completion");
}

#[test]
fn milestone_variable_is_set_true_on_case_when_reached() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(
        &engine,
        "milestoneVariableCase",
        CmmnMilestone::new("milestone-shipped", "Shipped")
            .with_milestone_variable("shippedMilestoneReached"),
    );

    complete_trigger_task(&engine, &case_id);

    // MilestoneActivityBehavior.java:51 — planItemInstanceEntity.setVariable(name, true).
    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert_eq!(refreshed.variables["shippedMilestoneReached"], json!(true));
}

#[test]
fn milestone_business_status_updates_case_instance() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(
        &engine,
        "milestoneBusinessStatusCase",
        CmmnMilestone::new("milestone-shipped", "Shipped").with_business_status("shipped"),
    );

    let before = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert_eq!(before.business_status, None);

    complete_trigger_task(&engine, &case_id);

    // MilestoneActivityBehavior.java:59 — updateBusinessStatus(caseInstanceId, status).
    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert_eq!(refreshed.business_status.as_deref(), Some("shipped"));
}

#[test]
fn milestone_without_declarations_leaves_case_untouched() {
    // Regression guard: a plain milestone keeps the pre-C5 behavior (no variable writes, no
    // business status change).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(
        &engine,
        "milestonePlainCase",
        CmmnMilestone::new("milestone-shipped", "Shipped"),
    );

    complete_trigger_task(&engine, &case_id);

    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert!(refreshed.variables.is_empty());
    assert_eq!(refreshed.business_status, None);
}

#[test]
fn milestone_variable_feeds_downstream_sentry_if_part() {
    // MilestoneActivityBehavior.java:47-64 ordering — the milestone variable is set before the
    // occur operation, so a downstream sentry (onPart occur + ifPart on the variable) fires.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-trigger", "Trigger"))
        .with_plan_item(CmmnPlanItem::new("plan-item-trigger", "human-task-trigger"))
        .with_milestone(
            CmmnMilestone::new("milestone-shipped", "Shipped")
                .with_milestone_variable("shippedMilestoneReached"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-milestone", "milestone-shipped")
                .with_entry_criterion("sentry-after-trigger"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-follow-up", "Follow up"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-follow-up", "human-task-follow-up")
                .with_entry_criterion("sentry-after-milestone"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-trigger",
            CmmnPlanItemOnPart::new("on-trigger-complete", "plan-item-trigger", "complete"),
        ))
        .with_sentry(
            CmmnSentry::new(
                "sentry-after-milestone",
                CmmnPlanItemOnPart::new("on-milestone-occur", "plan-item-milestone", "occur"),
            )
            .with_if_part("shippedMilestoneReached == true"),
        );
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-milestone-sentry",
        "milestoneSentryCase",
        "Milestone sentry case",
        plan_model,
    )]);
    engine
        .deploy(CmmnDeploymentRequest::new("milestone-sentry").with_resource("case.cmmn", model))
        .expect("deployment");
    let case_id = engine
        .start_case_instance_by_key("milestoneSentryCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id;

    complete_trigger_task(&engine, &case_id);

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
fn update_business_status_api_sets_status_on_case() {
    // Java parity: CmmnRuntimeService#updateBusinessStatus as a standalone runtime API.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(
        &engine,
        "businessStatusApiCase",
        CmmnMilestone::new("milestone-shipped", "Shipped"),
    );

    engine
        .runtime_service()
        .update_business_status(&case_id, "in-review")
        .expect("business status update");

    let refreshed = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert_eq!(refreshed.business_status.as_deref(), Some("in-review"));
}
