use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnChangePlanItemStateRequest,
    CmmnDeploymentRequest, CmmnEngine, CmmnEventListener, CmmnHumanTask,
    CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnMilestone, CmmnModel, CmmnPlanItem,
    CmmnPlanItemOnPart, CmmnSentry, CmmnSentryIfPartCondition, CmmnSentryIfPartExpression,
    CmmnSentryIfPartLiteral, CmmnSentryIfPartOperator, CmmnStage,
};
use serde_json::json;

fn human_task_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(CmmnPlanItem::new("plan-item-b", "human-task-b"));

    CmmnModel::new(vec![CmmnCase::new(
        "case-human-task",
        "humanTaskCase",
        "Human task case",
        plan_model,
    )])
}

fn entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b").with_entry_criterion("sentry-after-a"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-a",
            CmmnPlanItemOnPart::new("on-a-complete", "plan-item-a", "complete"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-entry-criterion",
        "entryCriterionCase",
        "Entry criterion case",
        plan_model,
    )])
}

fn enable_disable_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_human_task(CmmnHumanTask::new("human-task-c", "Task C"))
        // DisablePlanItemInstanceCmd.java:44-45 only accepts ENABLED, so the
        // source uses manual activation to exercise disable/enable sentries.
        .with_plan_item(
            CmmnPlanItem::new("plan-item-a", "human-task-a").with_manual_activation_rule("true"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b")
                .with_entry_criterion("sentry-after-a-disabled"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-c", "human-task-c")
                .with_entry_criterion("sentry-after-a-enabled"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-a-disabled",
            CmmnPlanItemOnPart::new("on-a-disable", "plan-item-a", "disable"),
        ))
        .with_sentry(CmmnSentry::new(
            "sentry-after-a-enabled",
            CmmnPlanItemOnPart::new("on-a-enable", "plan-item-a", "enable"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-enable-disable-entry-criterion",
        "enableDisableEntryCriterionCase",
        "Enable disable entry criterion case",
        plan_model,
    )])
}

fn stage_entry_criterion_model() -> CmmnModel {
    let review_stage = CmmnStage::new("stage-review", "Review stage")
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-b", "human-task-b"));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_stage(review_stage)
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review-stage", "stage-review")
                .with_entry_criterion("sentry-after-a"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-a",
            CmmnPlanItemOnPart::new("on-a-complete", "plan-item-a", "complete"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-stage-entry-criterion",
        "stageEntryCriterionCase",
        "Stage entry criterion case",
        plan_model,
    )])
}

fn terminate_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b").with_entry_criterion("sentry-after-a"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-a",
            CmmnPlanItemOnPart::new("on-a-terminate", "plan-item-a", "terminate"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-terminate-entry-criterion",
        "terminateEntryCriterionCase",
        "Terminate entry criterion case",
        plan_model,
    )])
}

fn milestone_exit_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_milestone(CmmnMilestone::new("milestone-reviewed", "Reviewed"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-reviewed", "milestone-reviewed")
                .with_exit_criterion("sentry-exit-milestone-after-a"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b")
                .with_entry_criterion("sentry-after-milestone-terminated"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-exit-milestone-after-a",
            CmmnPlanItemOnPart::new("on-a-complete-exit-milestone", "plan-item-a", "complete"),
        ))
        .with_sentry(CmmnSentry::new(
            "sentry-after-milestone-terminated",
            CmmnPlanItemOnPart::new(
                "on-milestone-terminate-start-b",
                "plan-item-reviewed",
                "terminate",
            ),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-milestone-exit-criterion",
        "milestoneExitCriterionCase",
        "Milestone exit criterion case",
        plan_model,
    )])
}

fn if_part_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b").with_entry_criterion("sentry-after-a"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-after-a",
                CmmnPlanItemOnPart::new("on-a-complete", "plan-item-a", "complete"),
            )
            .with_if_part("approved == true"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-if-part-entry-criterion",
        "ifPartEntryCriterionCase",
        "ifPart entry criterion case",
        plan_model,
    )])
}

fn manual_activation_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b")
                .with_entry_criterion("sentry-after-a")
                .with_manual_activation_rule("manualActivation == true"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-a",
            CmmnPlanItemOnPart::new("on-a-complete", "plan-item-a", "complete"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-manual-activation-entry-criterion",
        "manualActivationEntryCriterionCase",
        "Manual activation entry criterion case",
        plan_model,
    )])
}

fn repetition_rule_human_task_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-repeat", "Repeatable task"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-repeat", "human-task-repeat")
                .with_repetition_rule("repeat == true"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-repetition-rule",
        "repetitionRuleCase",
        "Repetition rule case",
        plan_model,
    )])
}

fn completion_exit_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b")
                .with_exit_criterion("sentry-exit-b-after-a"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-exit-b-after-a",
            CmmnPlanItemOnPart::new("on-a-complete-exit-b", "plan-item-a", "complete"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-completion-exit-criterion",
        "completionExitCriterionCase",
        "Completion exit criterion case",
        plan_model,
    )])
}

fn if_part_exit_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b")
                .with_exit_criterion("sentry-approved-exit-b"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-approved-exit-b",
                CmmnPlanItemOnPart::new("on-a-complete-approved-exit-b", "plan-item-a", "complete"),
            )
            .with_if_part("customer.approved == true"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-if-part-exit-criterion",
        "ifPartExitCriterionCase",
        "If part exit criterion case",
        plan_model,
    )])
}

fn if_part_only_exit_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review", "human-task-review")
                .with_exit_criterion("sentry-cancel-review"),
        )
        .with_sentry(CmmnSentry {
            id: "sentry-cancel-review".to_string(),
            plan_item_on_parts: Vec::new(),
            case_file_item_on_parts: Vec::new(),
            trigger_mode: None,
            if_part: Some(
                CmmnSentryIfPartExpression::parse("cancelReview == true")
                    .expect("ifPart expression"),
            ),
        });

    CmmnModel::new(vec![CmmnCase::new(
        "case-if-part-only-exit-criterion",
        "ifPartOnlyExitCriterionCase",
        "If part only exit criterion case",
        plan_model,
    )])
}

fn stage_exit_criterion_model() -> CmmnModel {
    let review_stage = CmmnStage::new("stage-review", "Review stage")
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_human_task(CmmnHumanTask::new("human-task-c", "Task C"))
        .with_plan_item(CmmnPlanItem::new("plan-item-b", "human-task-b"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-c", "human-task-c")
                .with_entry_criterion("sentry-after-b-terminated"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-b-terminated",
            CmmnPlanItemOnPart::new("on-b-terminate-start-c", "plan-item-b", "terminate"),
        ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_stage(review_stage)
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review-stage", "stage-review")
                .with_exit_criterion("sentry-exit-stage-after-a"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-exit-stage-after-a",
            CmmnPlanItemOnPart::new("on-a-complete-exit-stage", "plan-item-a", "complete"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-stage-exit-criterion",
        "stageExitCriterionCase",
        "Stage exit criterion case",
        plan_model,
    )])
}

fn event_listener_occur_exit_criterion_model() -> CmmnModel {
    let review_stage = CmmnStage::new("stage-review", "Review stage")
        .with_human_task(CmmnHumanTask::new("human-task-inside-stage", "Stage Task"))
        .with_event_listener(CmmnEventListener::new("stage-event-listener", "message"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-inside-stage-task",
            "human-task-inside-stage",
        ))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-stage-event-listener",
            "stage-event-listener",
        ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("cancel-event-listener", "message")
                .with_event_name("cancelRequested"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-cancel-event-listener",
            "cancel-event-listener",
        ))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review", "human-task-review")
                .with_exit_criterion("sentry-cancel-review"),
        )
        .with_stage(review_stage)
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review-stage", "stage-review")
                .with_exit_criterion("sentry-cancel-review"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-cancel-review",
            CmmnPlanItemOnPart::new(
                "on-cancel-event-occur",
                "plan-item-cancel-event-listener",
                "occur",
            ),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-event-listener-occur-exit-criterion",
        "eventListenerOccurExitCriterionCase",
        "Event listener occur exit criterion case",
        plan_model,
    )])
}

fn stage_event_subscription_exit_cleanup_model() -> CmmnModel {
    let waiting_stage = CmmnStage::new("stage-waiting", "Waiting stage")
        .with_event_listener(
            CmmnEventListener::new("stage-message-listener", "message")
                .with_event_name("stageMessage"),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-stage-message-listener",
            "stage-message-listener",
        ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("cancel-event-listener", "message")
                .with_event_name("cancelRequested"),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-cancel-event-listener",
            "cancel-event-listener",
        ))
        .with_stage(waiting_stage)
        .with_plan_item(
            CmmnPlanItem::new("plan-item-waiting-stage", "stage-waiting")
                .with_exit_criterion("sentry-cancel-waiting-stage"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-cancel-waiting-stage",
            CmmnPlanItemOnPart::new(
                "on-cancel-event-occur",
                "plan-item-cancel-event-listener",
                "occur",
            ),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-stage-event-subscription-exit-cleanup",
        "stageEventSubscriptionExitCleanupCase",
        "Stage event subscription exit cleanup case",
        plan_model,
    )])
}

fn extended_if_part_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-status", "Status Task"))
        .with_human_task(CmmnHumanTask::new("human-task-amount", "Amount Task"))
        .with_human_task(CmmnHumanTask::new("human-task-decision", "Decision Task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-status", "human-task-status")
                .with_entry_criterion("sentry-status"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-amount", "human-task-amount")
                .with_entry_criterion("sentry-amount"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-decision", "human-task-decision")
                .with_entry_criterion("sentry-decision"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-status",
                CmmnPlanItemOnPart::new("on-a-complete-status", "plan-item-a", "complete"),
            )
            .with_if_part("status == \"approved\""),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-amount",
                CmmnPlanItemOnPart::new("on-a-complete-amount", "plan-item-a", "complete"),
            )
            .with_if_part("amount == 42.5"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-decision",
                CmmnPlanItemOnPart::new("on-a-complete-decision", "plan-item-a", "complete"),
            )
            .with_if_part("decision != 'denied'"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-extended-if-part-entry-criterion",
        "extendedIfPartEntryCriterionCase",
        "Extended ifPart entry criterion case",
        plan_model,
    )])
}

fn logical_if_part_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-and", "AND Task"))
        .with_human_task(CmmnHumanTask::new("human-task-or", "OR Task"))
        .with_human_task(CmmnHumanTask::new("human-task-bare-and", "Bare AND Task"))
        .with_human_task(CmmnHumanTask::new("human-task-bare-or", "Bare OR Task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-and", "human-task-and").with_entry_criterion("sentry-and"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-or", "human-task-or").with_entry_criterion("sentry-or"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-bare-and", "human-task-bare-and")
                .with_entry_criterion("sentry-bare-and"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-bare-or", "human-task-bare-or")
                .with_entry_criterion("sentry-bare-or"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-and",
                CmmnPlanItemOnPart::new("on-a-complete-and", "plan-item-a", "complete"),
            )
            .with_if_part("(approved == true) && (amount != 0)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-or",
                CmmnPlanItemOnPart::new("on-a-complete-or", "plan-item-a", "complete"),
            )
            .with_if_part("(status == \"approved\") or (expedited == true) or (amount == 100)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-bare-and",
                CmmnPlanItemOnPart::new("on-a-complete-bare-and", "plan-item-a", "complete"),
            )
            .with_if_part("customer.active && caseFlags.ready && not(caseFlags.blocked)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-bare-or",
                CmmnPlanItemOnPart::new("on-a-complete-bare-or", "plan-item-a", "complete"),
            )
            .with_if_part("customer.active || caseFlags.ready"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-logical-if-part-entry-criterion",
        "logicalIfPartEntryCriterionCase",
        "Logical ifPart entry criterion case",
        plan_model,
    )])
}

fn grouped_not_if_part_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-not-and", "Not AND Task"))
        .with_human_task(CmmnHumanTask::new("human-task-not-or", "Not OR Task"))
        .with_human_task(CmmnHumanTask::new(
            "human-task-grouped-or",
            "Grouped OR Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-not-function-group",
            "Not Function Group Task",
        ))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-not-and", "human-task-not-and")
                .with_entry_criterion("sentry-not-and"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-not-or", "human-task-not-or")
                .with_entry_criterion("sentry-not-or"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-grouped-or", "human-task-grouped-or")
                .with_entry_criterion("sentry-grouped-or"),
        )
        .with_plan_item(
            CmmnPlanItem::new(
                "plan-item-not-function-group",
                "human-task-not-function-group",
            )
            .with_entry_criterion("sentry-not-function-group"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-not-and",
                CmmnPlanItemOnPart::new("on-a-complete-not-and", "plan-item-a", "complete"),
            )
            .with_if_part("not(customer.active && approved)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-not-or",
                CmmnPlanItemOnPart::new("on-a-complete-not-or", "plan-item-a", "complete"),
            )
            .with_if_part("not(customer.active || approved)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-grouped-or",
                CmmnPlanItemOnPart::new("on-a-complete-grouped-or", "plan-item-a", "complete"),
            )
            .with_if_part("(customer.active && approved) || fallback"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-not-function-group",
                CmmnPlanItemOnPart::new(
                    "on-a-complete-not-function-group",
                    "plan-item-a",
                    "complete",
                ),
            )
            .with_if_part("not(contains(tags, expectedTag) && size(items) >= minimumItemCount)"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-grouped-not-if-part-entry-criterion",
        "groupedNotIfPartEntryCriterionCase",
        "Grouped not ifPart entry criterion case",
        plan_model,
    )])
}

fn advanced_if_part_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-mixed", "Mixed Task"))
        .with_human_task(CmmnHumanTask::new("human-task-not", "Not Task"))
        .with_human_task(CmmnHumanTask::new("human-task-range", "Range Task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-mixed", "human-task-mixed")
                .with_entry_criterion("sentry-mixed"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-not", "human-task-not").with_entry_criterion("sentry-not"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-range", "human-task-range")
                .with_entry_criterion("sentry-range"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-mixed",
                CmmnPlanItemOnPart::new("on-a-complete-mixed", "plan-item-a", "complete"),
            )
            .with_if_part("(approved == true && amount > 100) || reviewer == 'lead'"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-not",
                CmmnPlanItemOnPart::new("on-a-complete-not", "plan-item-a", "complete"),
            )
            .with_if_part("not(rejected == true)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-range",
                CmmnPlanItemOnPart::new("on-a-complete-range", "plan-item-a", "complete"),
            )
            .with_if_part("amount >= 10 && amount <= 20"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-advanced-if-part-entry-criterion",
        "advancedIfPartEntryCriterionCase",
        "Advanced ifPart entry criterion case",
        plan_model,
    )])
}

fn null_empty_if_part_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new(
            "human-task-null-equal",
            "Null Equal Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-null-not-equal",
            "Null Not Equal Task",
        ))
        .with_human_task(CmmnHumanTask::new("human-task-empty", "Empty Task"))
        .with_human_task(CmmnHumanTask::new("human-task-not-empty", "Not Empty Task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-null-equal", "human-task-null-equal")
                .with_entry_criterion("sentry-null-equal"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-null-not-equal", "human-task-null-not-equal")
                .with_entry_criterion("sentry-null-not-equal"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-empty", "human-task-empty")
                .with_entry_criterion("sentry-empty"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-not-empty", "human-task-not-empty")
                .with_entry_criterion("sentry-not-empty"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-null-equal",
                CmmnPlanItemOnPart::new("on-a-complete-null-equal", "plan-item-a", "complete"),
            )
            .with_if_part("optionalValue == null"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-null-not-equal",
                CmmnPlanItemOnPart::new("on-a-complete-null-not-equal", "plan-item-a", "complete"),
            )
            .with_if_part("optionalValue != null"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-empty",
                CmmnPlanItemOnPart::new("on-a-complete-empty", "plan-item-a", "complete"),
            )
            .with_if_part("empty(comment)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-not-empty",
                CmmnPlanItemOnPart::new("on-a-complete-not-empty", "plan-item-a", "complete"),
            )
            .with_if_part("!empty(nonEmptyValue)"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-null-empty-if-part-entry-criterion",
        "nullEmptyIfPartEntryCriterionCase",
        "Null and empty ifPart entry criterion case",
        plan_model,
    )])
}

fn property_path_if_part_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-name", "Name Task"))
        .with_human_task(CmmnHumanTask::new("human-task-age", "Age Task"))
        .with_human_task(CmmnHumanTask::new(
            "human-task-first-item",
            "First Item Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-empty-email",
            "Empty Email Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-empty-items",
            "Empty Items Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-empty-metadata",
            "Empty Metadata Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-not-empty-items",
            "Not Empty Items Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-not-empty-customer",
            "Not Empty Customer Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-variable-age",
            "Variable Age Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-bracket-status",
            "Bracket Status Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-bracket-name",
            "Bracket Name Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-boolean-variable",
            "Boolean Variable Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-null-variable",
            "Null Variable Task",
        ))
        .with_human_task(CmmnHumanTask::new("human-task-size", "Size Task"))
        .with_human_task(CmmnHumanTask::new(
            "human-task-property-size",
            "Property Size Task",
        ))
        .with_human_task(CmmnHumanTask::new("human-task-length", "Length Task"))
        .with_human_task(CmmnHumanTask::new(
            "human-task-property-length",
            "Property Length Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-contains-string",
            "Contains String Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-contains-array",
            "Contains Array Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-contains-object",
            "Contains Object Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-contains-not-equal-false",
            "Contains Not Equal False Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-standalone-contains",
            "Standalone Contains Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-complex-contains",
            "Complex Contains Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-complex-length",
            "Complex Length Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-not-active-customer",
            "Not Active Customer Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-active-customer",
            "Active Customer Task",
        ))
        .with_human_task(CmmnHumanTask::new(
            "human-task-ready-flag",
            "Ready Flag Task",
        ))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-name", "human-task-name")
                .with_entry_criterion("sentry-name"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-age", "human-task-age").with_entry_criterion("sentry-age"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-first-item", "human-task-first-item")
                .with_entry_criterion("sentry-first-item"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-empty-email", "human-task-empty-email")
                .with_entry_criterion("sentry-empty-email"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-empty-items", "human-task-empty-items")
                .with_entry_criterion("sentry-empty-items"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-empty-metadata", "human-task-empty-metadata")
                .with_entry_criterion("sentry-empty-metadata"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-not-empty-items", "human-task-not-empty-items")
                .with_entry_criterion("sentry-not-empty-items"),
        )
        .with_plan_item(
            CmmnPlanItem::new(
                "plan-item-not-empty-customer",
                "human-task-not-empty-customer",
            )
            .with_entry_criterion("sentry-not-empty-customer"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-variable-age", "human-task-variable-age")
                .with_entry_criterion("sentry-variable-age"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-bracket-status", "human-task-bracket-status")
                .with_entry_criterion("sentry-bracket-status"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-bracket-name", "human-task-bracket-name")
                .with_entry_criterion("sentry-bracket-name"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-boolean-variable", "human-task-boolean-variable")
                .with_entry_criterion("sentry-boolean-variable"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-null-variable", "human-task-null-variable")
                .with_entry_criterion("sentry-null-variable"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-size", "human-task-size")
                .with_entry_criterion("sentry-size"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-property-size", "human-task-property-size")
                .with_entry_criterion("sentry-property-size"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-length", "human-task-length")
                .with_entry_criterion("sentry-length"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-property-length", "human-task-property-length")
                .with_entry_criterion("sentry-property-length"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-contains-string", "human-task-contains-string")
                .with_entry_criterion("sentry-contains-string"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-contains-array", "human-task-contains-array")
                .with_entry_criterion("sentry-contains-array"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-contains-object", "human-task-contains-object")
                .with_entry_criterion("sentry-contains-object"),
        )
        .with_plan_item(
            CmmnPlanItem::new(
                "plan-item-contains-not-equal-false",
                "human-task-contains-not-equal-false",
            )
            .with_entry_criterion("sentry-contains-not-equal-false"),
        )
        .with_plan_item(
            CmmnPlanItem::new(
                "plan-item-standalone-contains",
                "human-task-standalone-contains",
            )
            .with_entry_criterion("sentry-standalone-contains"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-complex-contains", "human-task-complex-contains")
                .with_entry_criterion("sentry-complex-contains"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-complex-length", "human-task-complex-length")
                .with_entry_criterion("sentry-complex-length"),
        )
        .with_plan_item(
            CmmnPlanItem::new(
                "plan-item-not-active-customer",
                "human-task-not-active-customer",
            )
            .with_entry_criterion("sentry-not-active-customer"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-active-customer", "human-task-active-customer")
                .with_entry_criterion("sentry-active-customer"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-ready-flag", "human-task-ready-flag")
                .with_entry_criterion("sentry-ready-flag"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-name",
                CmmnPlanItemOnPart::new("on-a-complete-name", "plan-item-a", "complete"),
            )
            .with_if_part("customer.name == 'Alice'"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-age",
                CmmnPlanItemOnPart::new("on-a-complete-age", "plan-item-a", "complete"),
            )
            .with_if_part("customer.age >= 18"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-first-item",
                CmmnPlanItemOnPart::new("on-a-complete-first-item", "plan-item-a", "complete"),
            )
            .with_if_part("items[0].status == 'open'"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-empty-email",
                CmmnPlanItemOnPart::new("on-a-complete-empty-email", "plan-item-a", "complete"),
            )
            .with_if_part("empty(customer.email)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-empty-items",
                CmmnPlanItemOnPart::new("on-a-complete-empty-items", "plan-item-a", "complete"),
            )
            .with_if_part("empty(emptyItems)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-empty-metadata",
                CmmnPlanItemOnPart::new("on-a-complete-empty-metadata", "plan-item-a", "complete"),
            )
            .with_if_part("empty(emptyMetadata)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-not-empty-items",
                CmmnPlanItemOnPart::new("on-a-complete-not-empty-items", "plan-item-a", "complete"),
            )
            .with_if_part("!empty(items)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-not-empty-customer",
                CmmnPlanItemOnPart::new(
                    "on-a-complete-not-empty-customer",
                    "plan-item-a",
                    "complete",
                ),
            )
            .with_if_part("!empty(customer)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-variable-age",
                CmmnPlanItemOnPart::new("on-a-complete-variable-age", "plan-item-a", "complete"),
            )
            .with_if_part("customer.age >= minAge"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-bracket-status",
                CmmnPlanItemOnPart::new("on-a-complete-bracket-status", "plan-item-a", "complete"),
            )
            .with_if_part("customer['status'] == expectedStatus"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-bracket-name",
                CmmnPlanItemOnPart::new("on-a-complete-bracket-name", "plan-item-a", "complete"),
            )
            .with_if_part("customer[\"name\"] == 'Alice'"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-boolean-variable",
                CmmnPlanItemOnPart::new(
                    "on-a-complete-boolean-variable",
                    "plan-item-a",
                    "complete",
                ),
            )
            .with_if_part("approved == expectedApproval"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-null-variable",
                CmmnPlanItemOnPart::new("on-a-complete-null-variable", "plan-item-a", "complete"),
            )
            .with_if_part("optionalValue == expectedNull"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-size",
                CmmnPlanItemOnPart::new("on-a-complete-size", "plan-item-a", "complete"),
            )
            .with_if_part("size(items) >= minimumItemCount"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-property-size",
                CmmnPlanItemOnPart::new("on-a-complete-property-size", "plan-item-a", "complete"),
            )
            .with_if_part("items.size() >= minimumItemCount"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-length",
                CmmnPlanItemOnPart::new("on-a-complete-length", "plan-item-a", "complete"),
            )
            .with_if_part("length(customer.name) >= minimumNameLength"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-property-length",
                CmmnPlanItemOnPart::new("on-a-complete-property-length", "plan-item-a", "complete"),
            )
            .with_if_part("customer.name.length() >= minimumNameLength"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-contains-string",
                CmmnPlanItemOnPart::new("on-a-complete-contains-string", "plan-item-a", "complete"),
            )
            .with_if_part("contains(customer.name, 'lic') == true"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-contains-array",
                CmmnPlanItemOnPart::new("on-a-complete-contains-array", "plan-item-a", "complete"),
            )
            .with_if_part("contains(tags, expectedTag) == true"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-contains-object",
                CmmnPlanItemOnPart::new("on-a-complete-contains-object", "plan-item-a", "complete"),
            )
            .with_if_part("contains(customer, 'name') == true"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-contains-not-equal-false",
                CmmnPlanItemOnPart::new(
                    "on-a-complete-contains-not-equal-false",
                    "plan-item-a",
                    "complete",
                ),
            )
            .with_if_part("contains(customer.name, 'lic') != false"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-standalone-contains",
                CmmnPlanItemOnPart::new(
                    "on-a-complete-standalone-contains",
                    "plan-item-a",
                    "complete",
                ),
            )
            .with_if_part("contains(tags, expectedTag)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-complex-contains",
                CmmnPlanItemOnPart::new(
                    "on-a-complete-complex-contains",
                    "plan-item-a",
                    "complete",
                ),
            )
            .with_if_part("contains(customer.name + suffix, expectedNeedle)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-complex-length",
                CmmnPlanItemOnPart::new("on-a-complete-complex-length", "plan-item-a", "complete"),
            )
            .with_if_part("length(customer.name + suffix) >= minimumFullNameLength"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-not-active-customer",
                CmmnPlanItemOnPart::new(
                    "on-a-complete-not-active-customer",
                    "plan-item-a",
                    "complete",
                ),
            )
            .with_if_part("not(customer.active)"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-active-customer",
                CmmnPlanItemOnPart::new("on-a-complete-active-customer", "plan-item-a", "complete"),
            )
            .with_if_part("customer.active"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-ready-flag",
                CmmnPlanItemOnPart::new("on-a-complete-ready-flag", "plan-item-a", "complete"),
            )
            .with_if_part("caseFlags.ready"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-property-path-if-part-entry-criterion",
        "propertyPathIfPartEntryCriterionCase",
        "Property path ifPart entry criterion case",
        plan_model,
    )])
}

fn multi_on_part_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_human_task(CmmnHumanTask::new("human-task-c", "Task C"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(CmmnPlanItem::new("plan-item-b", "human-task-b"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-c", "human-task-c")
                .with_entry_criterion("sentry-after-a-and-b"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-after-a-and-b",
                CmmnPlanItemOnPart::new("on-a-complete", "plan-item-a", "complete"),
            )
            .with_plan_item_on_part(CmmnPlanItemOnPart::new(
                "on-b-complete",
                "plan-item-b",
                "complete",
            )),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-multi-on-part-entry-criterion",
        "multiOnPartEntryCriterionCase",
        "Multi onPart entry criterion case",
        plan_model,
    )])
}

fn event_listener_occur_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("approval-event-listener", "message")
                .with_name("Wait for approval")
                .with_event_name("approvalReceived"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-approval", "Approve"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-approval-event",
            "approval-event-listener",
        ))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-approval-task", "human-task-approval")
                .with_entry_criterion("sentry-after-approval-event"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-approval-event",
            CmmnPlanItemOnPart::new(
                "on-approval-event-occur",
                "plan-item-approval-event",
                "occur",
            ),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-event-listener-occur-entry-criterion",
        "eventListenerOccurEntryCriterionCase",
        "Event listener occur entry criterion case",
        plan_model,
    )])
}

#[test]
fn queries_active_tasks_and_removes_completed_task_from_active_slice() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("human-tasks")
                .with_resource("human-task-case.cmmn", human_task_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("humanTaskCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 2);

    let first_task = &active_tasks[0];
    let completion = engine
        .complete_human_task(
            &first_task.id,
            CmmnHumanTaskCompletionRequest::new().with_completed_by("operator"),
        )
        .expect("completion");

    assert_eq!(completion.task.state, CmmnHumanTaskState::Completed);
    assert_eq!(completion.task.completed_by.as_deref(), Some("operator"));

    let remaining_active = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("remaining active tasks");

    assert_eq!(remaining_active.len(), 1);
    assert_ne!(remaining_active[0].id, first_task.id);
}

#[test]
fn rejects_duplicate_completion_of_same_human_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("human-tasks")
                .with_resource("human-task-case.cmmn", human_task_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("humanTaskCase", CmmnCaseInstanceStartRequest::new())
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
        .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("first completion");

    let error = engine
        .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
        .expect_err("duplicate completion should fail");

    assert!(error.to_string().contains("already completed"));
}

#[test]
fn activates_entry_criterion_task_after_source_task_completes() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("entry-criterion")
                .with_resource("entry-criterion-case.cmmn", entry_criterion_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("entryCriterionCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].task_definition_id, "human-task-a");

    engine
        .complete_human_task(&active_tasks[0].id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after completion");
    assert_eq!(active_after_completion.len(), 1);
    assert_eq!(
        active_after_completion[0].task_definition_id,
        "human-task-b"
    );
}

#[test]
fn activates_entry_criterion_tasks_after_source_task_is_disabled_and_enabled() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("enable-disable-entry-criterion").with_resource(
                "enable-disable-entry-criterion-case.cmmn",
                enable_disable_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "enableDisableEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let task_a = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Enabled)
        .single_result()
        .expect("task a query")
        .expect("task a");
    assert_eq!(task_a.task_definition_id, "human-task-a");

    engine
        .runtime_service()
        .disable_plan_item_instance(&task_a.id)
        .expect("disable task a");

    let task_b = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task b query")
        .expect("task b");
    assert_eq!(task_b.task_definition_id, "human-task-b");

    engine
        .runtime_service()
        .enable_plan_item_instance(&task_a.id)
        .expect("enable task a");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after enable");
    assert!(
        active_tasks
            .iter()
            .any(|task| task.task_definition_id == "human-task-c"),
        "human-task-c should activate after task a is enabled"
    );
}

#[test]
fn activates_entry_criterion_stage_after_source_task_completes() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("stage-entry-criterion").with_resource(
                "stage-entry-criterion-case.cmmn",
                stage_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "stageEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].task_definition_id, "human-task-a");

    let stage_overview_before_completion = engine
        .runtime_service()
        .get_stage_overview(&case_instance.id)
        .expect("runtime stage overview before completion");
    assert!(stage_overview_before_completion.is_empty());

    engine
        .complete_human_task(&active_tasks[0].id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after completion");
    assert_eq!(active_after_completion.len(), 1);
    assert_eq!(
        active_after_completion[0].task_definition_id,
        "human-task-b"
    );

    let stage_overview_after_completion = engine
        .runtime_service()
        .get_stage_overview(&case_instance.id)
        .expect("runtime stage overview after completion");
    assert_eq!(stage_overview_after_completion.len(), 1);
    assert_eq!(stage_overview_after_completion[0].id, "stage-review");
    assert!(stage_overview_after_completion[0].current);
    assert!(!stage_overview_after_completion[0].ended);
}

#[test]
fn activates_entry_criterion_task_after_source_task_terminates() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("terminate-entry-criterion").with_resource(
                "terminate-entry-criterion-case.cmmn",
                terminate_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "terminateEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].task_definition_id, "human-task-a");

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                terminate_plan_item_definition_ids: vec!["human-task-a".to_string()],
                ..Default::default()
            },
        )
        .expect("terminate task a");

    let active_after_termination = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after termination");
    assert_eq!(active_after_termination.len(), 1);
    assert_eq!(
        active_after_termination[0].task_definition_id,
        "human-task-b"
    );
}

#[test]
fn exits_active_human_task_after_source_task_completes() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("completion-exit-criterion").with_resource(
                "completion-exit-criterion-case.cmmn",
                completion_exit_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "completionExitCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 2);

    let task_a = active_tasks
        .iter()
        .find(|task| task.task_definition_id == "human-task-a")
        .expect("task a active");
    engine
        .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after completion");
    assert!(active_after_completion.is_empty());

    let terminated_history = engine
        .history_service()
        .create_historic_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Terminated)
        .single_result()
        .expect("historic task query")
        .expect("terminated task history");
    assert_eq!(terminated_history.task_definition_id, "human-task-b");
}

#[test]
fn exits_occurred_milestone_and_triggers_terminate_dependents_after_source_task_completes() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("milestone-exit-criterion").with_resource(
                "milestone-exit-criterion-case.cmmn",
                milestone_exit_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "milestoneExitCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let milestones = engine
        .history_service()
        .create_historic_milestone_query()
        .case_instance_id(&case_instance.id)
        .milestone_id("milestone-reviewed")
        .list()
        .expect("historic milestones");
    assert_eq!(milestones.len(), 1);

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].task_definition_id, "human-task-a");

    engine
        .complete_human_task(&active_tasks[0].id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after completion");
    assert_eq!(active_after_completion.len(), 1);
    assert_eq!(
        active_after_completion[0].task_definition_id,
        "human-task-b"
    );
}

#[test]
fn exits_human_task_only_when_exit_if_part_is_true() {
    for (approved, expected_b_active, expected_b_terminated) in
        [(true, false, true), (false, true, false)]
    {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        engine
            .deploy(
                CmmnDeploymentRequest::new("if-part-exit-criterion").with_resource(
                    "if-part-exit-criterion-case.cmmn",
                    if_part_exit_criterion_model(),
                ),
            )
            .expect("deployment");

        let case_instance = engine
            .start_case_instance_by_key(
                "ifPartExitCriterionCase",
                CmmnCaseInstanceStartRequest::new().with_variables(json!({
                    "customer": { "approved": approved }
                })),
            )
            .expect("case instance");

        let active_tasks = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks");
        let task_a = active_tasks
            .iter()
            .find(|task| task.task_definition_id == "human-task-a")
            .expect("task a active");

        engine
            .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
            .expect("complete task a");

        let active_after_completion = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks after completion");
        assert_eq!(
            active_after_completion
                .iter()
                .any(|task| task.task_definition_id == "human-task-b"),
            expected_b_active
        );

        let b_terminated = engine
            .history_service()
            .create_historic_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Terminated)
            .list()
            .expect("historic task query")
            .iter()
            .any(|task| task.task_definition_id == "human-task-b");
        assert_eq!(b_terminated, expected_b_terminated);
    }
}

#[test]
fn exits_active_human_task_after_if_part_only_exit_becomes_true() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("if-part-only-exit-criterion").with_resource(
                "if-part-only-exit-criterion-case.cmmn",
                if_part_only_exit_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "ifPartOnlyExitCriterionCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({
                "cancelReview": false
            })),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].task_definition_id, "human-task-review");

    engine
        .runtime_service()
        .set_case_instance_variables(
            &case_instance.id,
            vec![("cancelReview".to_string(), json!(true))],
        )
        .expect("set variables");

    let active_after_variable_update = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after variable update");
    assert!(active_after_variable_update.is_empty());

    let terminated_history = engine
        .history_service()
        .create_historic_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Terminated)
        .single_result()
        .expect("historic task query")
        .expect("terminated task history");
    assert_eq!(terminated_history.task_definition_id, "human-task-review");
}

#[test]
fn exits_stage_and_terminates_active_child_tasks_after_source_task_completes() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("stage-exit-criterion").with_resource(
                "stage-exit-criterion-case.cmmn",
                stage_exit_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "stageExitCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 2);

    let task_a = active_tasks
        .iter()
        .find(|task| task.task_definition_id == "human-task-a")
        .expect("task a active");
    engine
        .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after completion");
    assert!(active_after_completion.is_empty());

    let stage_overview = engine
        .history_service()
        .get_stage_overview(&case_instance.id)
        .expect("historic stage overview");
    assert_eq!(stage_overview.len(), 1);
    assert_eq!(stage_overview[0].id, "stage-review");
    assert!(stage_overview[0].ended);
    assert!(!stage_overview[0].current);
    assert!(stage_overview[0].end_time.is_some());

    let terminated_child = engine
        .history_service()
        .create_historic_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Terminated)
        .single_result()
        .expect("historic task query")
        .expect("terminated child task history");
    assert_eq!(terminated_child.task_definition_id, "human-task-b");
}

#[test]
fn exits_human_task_and_stage_after_event_listener_occurs() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("event-listener-occur-exit-criterion").with_resource(
                "event-listener-occur-exit-criterion-case.cmmn",
                event_listener_occur_exit_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "eventListenerOccurExitCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let subscriptions = engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("event subscriptions");
    assert_eq!(subscriptions.len(), 2);
    let cancel_subscription = subscriptions
        .iter()
        .find(|subscription| subscription.activity_id.as_deref() == Some("cancel-event-listener"))
        .expect("cancel event subscription");

    engine
        .runtime_service()
        .occur_event_subscription(&cancel_subscription.id)
        .expect("occur cancel event subscription");

    let active_after_occurrence = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after occurrence");
    assert!(active_after_occurrence.is_empty());

    let remaining_subscriptions = engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("remaining event subscriptions");
    assert!(remaining_subscriptions.is_empty());

    let terminated_tasks = engine
        .history_service()
        .create_historic_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Terminated)
        .list()
        .expect("terminated task history");
    assert_eq!(terminated_tasks.len(), 2);
    assert!(
        terminated_tasks
            .iter()
            .any(|task| task.task_definition_id == "human-task-review")
    );
    assert!(
        terminated_tasks
            .iter()
            .any(|task| task.task_definition_id == "human-task-inside-stage")
    );

    let stage_overview = engine
        .history_service()
        .get_stage_overview(&case_instance.id)
        .expect("historic stage overview");
    assert_eq!(stage_overview.len(), 1);
    assert_eq!(stage_overview[0].id, "stage-review");
    assert!(stage_overview[0].ended);
    assert!(!stage_overview[0].current);
}

#[test]
fn stage_exit_keeps_event_listener_stage_active_until_exit_and_cleans_subscription() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("stage-event-subscription-exit-cleanup").with_resource(
                "stage-event-subscription-exit-cleanup-case.cmmn",
                stage_event_subscription_exit_cleanup_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "stageEventSubscriptionExitCleanupCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let stage_overview_before_exit = engine
        .runtime_service()
        .get_stage_overview(&case_instance.id)
        .expect("runtime stage overview before exit");
    assert_eq!(stage_overview_before_exit.len(), 1);
    assert_eq!(stage_overview_before_exit[0].id, "stage-waiting");
    assert!(stage_overview_before_exit[0].current);
    assert!(!stage_overview_before_exit[0].ended);

    let subscriptions = engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("event subscriptions before exit");
    assert_eq!(subscriptions.len(), 2);
    let cancel_subscription = subscriptions
        .iter()
        .find(|subscription| subscription.activity_id.as_deref() == Some("cancel-event-listener"))
        .expect("cancel event subscription");

    engine
        .runtime_service()
        .occur_event_subscription(&cancel_subscription.id)
        .expect("occur cancel event subscription");

    let remaining_subscriptions = engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("remaining event subscriptions after exit");
    assert!(remaining_subscriptions.is_empty());

    let stage_overview_after_exit = engine
        .history_service()
        .get_stage_overview(&case_instance.id)
        .expect("historic stage overview after exit");
    assert_eq!(stage_overview_after_exit.len(), 1);
    assert_eq!(stage_overview_after_exit[0].id, "stage-waiting");
    assert!(stage_overview_after_exit[0].ended);
    assert!(!stage_overview_after_exit[0].current);
}

#[test]
fn activates_if_part_entry_criterion_task_only_when_condition_is_true() {
    for (approved, expected_task_count) in [(true, 1), (false, 0)] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        engine
            .deploy(
                CmmnDeploymentRequest::new("if-part-entry-criterion").with_resource(
                    "if-part-entry-criterion-case.cmmn",
                    if_part_entry_criterion_model(),
                ),
            )
            .expect("deployment");

        let case_instance = engine
            .start_case_instance_by_key(
                "ifPartEntryCriterionCase",
                CmmnCaseInstanceStartRequest::new().with_variables(json!({ "approved": approved })),
            )
            .expect("case instance");

        let active_tasks = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks");
        assert_eq!(active_tasks.len(), 1);
        assert_eq!(active_tasks[0].task_definition_id, "human-task-a");

        engine
            .complete_human_task(&active_tasks[0].id, CmmnHumanTaskCompletionRequest::new())
            .expect("complete task a");

        let active_after_completion = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks after completion");
        assert_eq!(active_after_completion.len(), expected_task_count);
        if approved {
            assert_eq!(
                active_after_completion[0].task_definition_id,
                "human-task-b"
            );
        }
    }
}

#[test]
fn activates_if_part_entry_criterion_task_for_string_number_and_not_equal_conditions() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("extended-if-part-entry-criterion").with_resource(
                "extended-if-part-entry-criterion-case.cmmn",
                extended_if_part_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "extendedIfPartEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({
                "status": "approved",
                "amount": 42.5,
                "decision": "needs-review"
            })),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].task_definition_id, "human-task-a");

    engine
        .complete_human_task(&active_tasks[0].id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let mut active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after completion")
        .into_iter()
        .map(|task| task.task_definition_id)
        .collect::<Vec<_>>();
    active_after_completion.sort();

    assert_eq!(
        active_after_completion,
        vec![
            "human-task-amount".to_string(),
            "human-task-decision".to_string(),
            "human-task-status".to_string(),
        ]
    );
}

#[test]
fn activates_logical_and_if_part_only_when_all_comparisons_are_true() {
    for (variables, expected_and_active, expected_bare_and_active) in [
        (
            json!({
                "approved": true,
                "amount": 42,
                "customer": { "active": true },
                "caseFlags": { "ready": true, "blocked": false }
            }),
            true,
            true,
        ),
        (
            json!({
                "approved": false,
                "amount": 42,
                "customer": { "active": true },
                "caseFlags": { "ready": true, "blocked": false }
            }),
            false,
            true,
        ),
        (
            json!({
                "approved": true,
                "amount": 0,
                "customer": { "active": true },
                "caseFlags": { "ready": false, "blocked": false }
            }),
            false,
            false,
        ),
        (
            json!({
                "approved": true,
                "amount": 42,
                "customer": { "active": true },
                "caseFlags": { "ready": true, "blocked": true }
            }),
            true,
            false,
        ),
    ] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        engine
            .deploy(
                CmmnDeploymentRequest::new("logical-if-part-entry-criterion").with_resource(
                    "logical-if-part-entry-criterion-case.cmmn",
                    logical_if_part_entry_criterion_model(),
                ),
            )
            .expect("deployment");

        let case_instance = engine
            .start_case_instance_by_key(
                "logicalIfPartEntryCriterionCase",
                CmmnCaseInstanceStartRequest::new().with_variables(variables),
            )
            .expect("case instance");

        let active_tasks = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks");

        let task_a = active_tasks
            .iter()
            .find(|task| task.task_definition_id == "human-task-a")
            .expect("task a active");
        engine
            .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
            .expect("complete task a");

        let active_after_completion = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks after completion");
        assert_eq!(
            active_after_completion
                .iter()
                .any(|task| task.task_definition_id == "human-task-and"),
            expected_and_active
        );
        assert_eq!(
            active_after_completion
                .iter()
                .any(|task| task.task_definition_id == "human-task-bare-and"),
            expected_bare_and_active
        );
    }
}

#[test]
fn activates_logical_or_if_part_when_any_comparison_is_true() {
    for (variables, expected_or_active, expected_bare_or_active) in [
        (
            json!({
                "status": "approved",
                "expedited": false,
                "amount": 0,
                "customer": { "active": true },
                "caseFlags": { "ready": false }
            }),
            true,
            true,
        ),
        (
            json!({
                "status": "pending",
                "expedited": true,
                "amount": 0,
                "customer": { "active": false },
                "caseFlags": { "ready": true }
            }),
            true,
            true,
        ),
        (
            json!({
                "status": "pending",
                "expedited": false,
                "amount": 100,
                "customer": { "active": false },
                "caseFlags": { "ready": false }
            }),
            true,
            false,
        ),
        (
            json!({
                "status": "pending",
                "expedited": false,
                "amount": 0,
                "customer": { "active": false },
                "caseFlags": { "ready": false }
            }),
            false,
            false,
        ),
    ] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        engine
            .deploy(
                CmmnDeploymentRequest::new("logical-if-part-entry-criterion").with_resource(
                    "logical-if-part-entry-criterion-case.cmmn",
                    logical_if_part_entry_criterion_model(),
                ),
            )
            .expect("deployment");

        let case_instance = engine
            .start_case_instance_by_key(
                "logicalIfPartEntryCriterionCase",
                CmmnCaseInstanceStartRequest::new().with_variables(variables),
            )
            .expect("case instance");

        let active_tasks = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks");

        let task_a = active_tasks
            .iter()
            .find(|task| task.task_definition_id == "human-task-a")
            .expect("task a active");
        engine
            .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
            .expect("complete task a");

        let active_after_completion = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks after completion");
        assert_eq!(
            active_after_completion
                .iter()
                .any(|task| task.task_definition_id == "human-task-or"),
            expected_or_active
        );
        assert_eq!(
            active_after_completion
                .iter()
                .any(|task| task.task_definition_id == "human-task-bare-or"),
            expected_bare_or_active
        );
    }
}

#[test]
fn activates_grouped_not_if_part_conditions_with_canonical_precedence() {
    for (variables, expected_active) in [
        (
            json!({
                "customer": { "active": true },
                "approved": true,
                "fallback": false,
                "tags": ["vip"],
                "expectedTag": "vip",
                "items": [{ "id": 1 }, { "id": 2 }],
                "minimumItemCount": 1
            }),
            vec!["human-task-grouped-or"],
        ),
        (
            json!({
                "customer": { "active": true },
                "approved": false,
                "fallback": false,
                "tags": ["vip"],
                "expectedTag": "vip",
                "items": [{ "id": 1 }],
                "minimumItemCount": 1
            }),
            vec!["human-task-not-and"],
        ),
        (
            json!({
                "customer": { "active": false },
                "approved": false,
                "fallback": true,
                "tags": ["standard"],
                "expectedTag": "vip",
                "items": [{ "id": 1 }],
                "minimumItemCount": 1
            }),
            vec![
                "human-task-grouped-or",
                "human-task-not-and",
                "human-task-not-function-group",
                "human-task-not-or",
            ],
        ),
    ] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        engine
            .deploy(
                CmmnDeploymentRequest::new("grouped-not-if-part-entry-criterion").with_resource(
                    "grouped-not-if-part-entry-criterion-case.cmmn",
                    grouped_not_if_part_entry_criterion_model(),
                ),
            )
            .expect("deployment");

        let case_instance = engine
            .start_case_instance_by_key(
                "groupedNotIfPartEntryCriterionCase",
                CmmnCaseInstanceStartRequest::new().with_variables(variables),
            )
            .expect("case instance");

        let active_tasks = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks");

        let task_a = active_tasks
            .iter()
            .find(|task| task.task_definition_id == "human-task-a")
            .expect("task a active");
        engine
            .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
            .expect("complete task a");

        let mut active_after_completion = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks after completion")
            .into_iter()
            .map(|task| task.task_definition_id)
            .collect::<Vec<_>>();
        active_after_completion.sort();

        let mut expected_active = expected_active
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        expected_active.sort();

        assert_eq!(active_after_completion, expected_active);
    }
}

#[test]
fn parses_nested_logical_if_part_expression_and_complex_contains_arguments() {
    CmmnSentryIfPartExpression::parse(
        "(approved == true) && (amount > 42 || decision != 'denied')",
    )
    .expect("nested logical ifPart expression");

    CmmnSentryIfPartExpression::parse("not(customer.active && approved)")
        .expect("not over and ifPart expression");
    CmmnSentryIfPartExpression::parse("not(customer.active || approved)")
        .expect("not over or ifPart expression");
    CmmnSentryIfPartExpression::parse("(customer.active && approved) || fallback")
        .expect("grouped boolean ifPart expression");
    CmmnSentryIfPartExpression::parse(
        "not(contains(tags, expectedTag) && size(items) >= minimumItemCount)",
    )
    .expect("not over function and sizing ifPart expression");
    CmmnSentryIfPartExpression::parse(
        "not(contains(tags, expectedTag) && items.size() >= minimumItemCount)",
    )
    .expect("not over function and property sizing ifPart expression");

    CmmnSentryIfPartExpression::parse(
        "optionalValue == null && (empty(comment) || !empty(nonEmptyValue))",
    )
    .expect("null and empty ifPart expression");

    let function = CmmnSentryIfPartExpression::parse("isApproved() == true")
        .expect("function call ifPart expression is now supported");
    assert_eq!(
        function,
        CmmnSentryIfPartExpression::Comparison(CmmnSentryIfPartCondition {
            variable_name: "isApproved()".to_string(),
            operator: CmmnSentryIfPartOperator::Equal,
            literal: CmmnSentryIfPartLiteral::Boolean(true),
        })
    );

    CmmnSentryIfPartExpression::parse("contains(customer.name, 'Ann') == true")
        .expect("contains string literal ifPart expression");
    CmmnSentryIfPartExpression::parse("contains(tags, expectedTag) == true")
        .expect("contains array variable ifPart expression");
    CmmnSentryIfPartExpression::parse("contains(tags, expectedTag)")
        .expect("standalone contains ifPart expression");
    CmmnSentryIfPartExpression::parse("contains(customer.name, 'Ann') != false")
        .expect("contains not-equal false ifPart expression");
    CmmnSentryIfPartExpression::parse("contains(customer.name, 'Bob') != true")
        .expect("contains not-equal true ifPart expression");

    let complex_contains =
        CmmnSentryIfPartExpression::parse("contains(customer.name + suffix, 'x') == true")
            .expect("complex contains arguments should parse");
    assert_eq!(
        complex_contains,
        CmmnSentryIfPartExpression::Contains {
            collection_variable_name: "customer.name + suffix".to_string(),
            value: CmmnSentryIfPartLiteral::String("x".to_string()),
            expected: true,
        }
    );

    let method_contains = CmmnSentryIfPartExpression::parse("customer.name.contains('x')")
        .expect("method-call contains is now supported");
    if let CmmnSentryIfPartExpression::MethodCall {
        ref object,
        ref method,
        ..
    } = method_contains
    {
        assert_eq!(object.as_deref(), Some("customer.name"));
        assert_eq!(method, "contains");
    } else {
        panic!("expected MethodCall variant, got: {method_contains:?}");
    }
}

#[test]
fn parses_property_path_if_part_expression_and_method_calls() {
    CmmnSentryIfPartExpression::parse("customer.name == 'Alice'")
        .expect("property path ifPart expression");

    CmmnSentryIfPartExpression::parse("customer.age >= 18")
        .expect("numeric property path ifPart expression");

    CmmnSentryIfPartExpression::parse("items[0].status == 'open'")
        .expect("indexed property path ifPart expression");

    CmmnSentryIfPartExpression::parse("empty(customer.email)")
        .expect("empty property path ifPart expression");

    CmmnSentryIfPartExpression::parse("items.size() >= minimumItemCount")
        .expect("property size method ifPart expression");
    CmmnSentryIfPartExpression::parse("customer.name.length() >= minimumNameLength")
        .expect("property length method ifPart expression");

    let method_call = CmmnSentryIfPartExpression::parse("customer.name() == 'Alice'")
        .expect("method calls are now supported");
    assert_eq!(
        method_call,
        CmmnSentryIfPartExpression::Comparison(CmmnSentryIfPartCondition {
            variable_name: "customer.name()".to_string(),
            operator: CmmnSentryIfPartOperator::Equal,
            literal: CmmnSentryIfPartLiteral::String("Alice".to_string()),
        })
    );

    let size_method_argument = CmmnSentryIfPartExpression::parse("items.size(extra) >= 1")
        .expect("property size method with arguments is now supported");
    assert_eq!(
        size_method_argument,
        CmmnSentryIfPartExpression::Comparison(CmmnSentryIfPartCondition {
            variable_name: "items.size(extra)".to_string(),
            operator: CmmnSentryIfPartOperator::GreaterThanOrEqual,
            literal: CmmnSentryIfPartLiteral::Number("1".to_string()),
        })
    );
}

#[test]
fn activates_nested_not_and_numeric_comparison_if_part_conditions() {
    for (variables, expected_active) in [
        (
            json!({
                "approved": true,
                "amount": 150,
                "reviewer": "member",
                "rejected": false
            }),
            vec!["human-task-mixed", "human-task-not"],
        ),
        (
            json!({
                "approved": false,
                "amount": 15,
                "reviewer": "lead",
                "rejected": true
            }),
            vec!["human-task-mixed", "human-task-range"],
        ),
        (
            json!({
                "approved": false,
                "amount": 5,
                "reviewer": "member",
                "rejected": false
            }),
            vec!["human-task-not"],
        ),
    ] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        engine
            .deploy(
                CmmnDeploymentRequest::new("advanced-if-part-entry-criterion").with_resource(
                    "advanced-if-part-entry-criterion-case.cmmn",
                    advanced_if_part_entry_criterion_model(),
                ),
            )
            .expect("deployment");

        let case_instance = engine
            .start_case_instance_by_key(
                "advancedIfPartEntryCriterionCase",
                CmmnCaseInstanceStartRequest::new().with_variables(variables),
            )
            .expect("case instance");

        let active_tasks = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks");

        let task_a = active_tasks
            .iter()
            .find(|task| task.task_definition_id == "human-task-a")
            .expect("task a active");
        engine
            .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
            .expect("complete task a");

        let mut active_after_completion = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks after completion")
            .into_iter()
            .map(|task| task.task_definition_id)
            .collect::<Vec<_>>();
        active_after_completion.sort();

        let mut expected_active = expected_active
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        expected_active.sort();

        assert_eq!(active_after_completion, expected_active);
    }
}

#[test]
fn activates_property_path_index_and_collection_empty_if_part_conditions() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("property-path-if-part-entry-criterion").with_resource(
                "property-path-if-part-entry-criterion-case.cmmn",
                property_path_if_part_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "propertyPathIfPartEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({
                "customer": {
                    "name": "Alice",
                    "age": 18,
                    "active": true,
                    "email": null,
                    "status": "vip"
                },
                "items": [
                    { "status": "open" }
                ],
                "tags": ["vip", "priority"],
                "emptyItems": [],
                "emptyMetadata": {},
                "caseFlags": {
                    "ready": true
                },
                "minAge": 18,
                "expectedStatus": "vip",
                "expectedTag": "priority",
                "approved": true,
                "expectedApproval": true,
                "optionalValue": null,
                "expectedNull": null,
                "minimumItemCount": 1,
                "minimumNameLength": 5,
                "suffix": " Smith",
                "expectedNeedle": "Smith",
                "minimumFullNameLength": 11
            })),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    let task_a = active_tasks
        .iter()
        .find(|task| task.task_definition_id == "human-task-a")
        .expect("task a active");
    engine
        .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let mut active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after completion")
        .into_iter()
        .map(|task| task.task_definition_id)
        .collect::<Vec<_>>();
    active_after_completion.sort();

    assert_eq!(
        active_after_completion,
        vec![
            "human-task-active-customer".to_string(),
            "human-task-age".to_string(),
            "human-task-boolean-variable".to_string(),
            "human-task-bracket-name".to_string(),
            "human-task-bracket-status".to_string(),
            "human-task-complex-contains".to_string(),
            "human-task-complex-length".to_string(),
            "human-task-contains-array".to_string(),
            "human-task-contains-not-equal-false".to_string(),
            "human-task-contains-object".to_string(),
            "human-task-contains-string".to_string(),
            "human-task-empty-email".to_string(),
            "human-task-empty-items".to_string(),
            "human-task-empty-metadata".to_string(),
            "human-task-first-item".to_string(),
            "human-task-length".to_string(),
            "human-task-name".to_string(),
            "human-task-not-empty-customer".to_string(),
            "human-task-not-empty-items".to_string(),
            "human-task-null-variable".to_string(),
            "human-task-property-length".to_string(),
            "human-task-property-size".to_string(),
            "human-task-ready-flag".to_string(),
            "human-task-size".to_string(),
            "human-task-standalone-contains".to_string(),
            "human-task-variable-age".to_string(),
        ]
    );
}

#[test]
fn does_not_activate_contains_if_part_conditions_when_contains_is_false() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("property-path-if-part-entry-criterion").with_resource(
                "property-path-if-part-entry-criterion-case.cmmn",
                property_path_if_part_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "propertyPathIfPartEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({
                "customer": {
                    "age": 18,
                    "active": false,
                    "email": null,
                    "status": "standard"
                },
                "items": [
                    { "status": "open" }
                ],
                "tags": ["standard"],
                "emptyItems": [],
                "emptyMetadata": {},
                "minAge": 18,
                "expectedStatus": "vip",
                "expectedTag": "priority",
                "approved": true,
                "expectedApproval": true,
                "optionalValue": null,
                "expectedNull": null,
                "minimumItemCount": 1,
                "minimumNameLength": 5
            })),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    let task_a = active_tasks
        .iter()
        .find(|task| task.task_definition_id == "human-task-a")
        .expect("task a active");
    engine
        .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after completion")
        .into_iter()
        .map(|task| task.task_definition_id)
        .collect::<Vec<_>>();

    for inactive in [
        "human-task-contains-array",
        "human-task-contains-not-equal-false",
        "human-task-contains-object",
        "human-task-contains-string",
        "human-task-standalone-contains",
    ] {
        assert!(
            !active_after_completion
                .iter()
                .any(|task_definition_id| task_definition_id == inactive),
            "{inactive} should not be active"
        );
    }
    assert!(
        active_after_completion
            .iter()
            .any(|task_definition_id| task_definition_id == "human-task-not-active-customer"),
        "human-task-not-active-customer should be active"
    );
}

#[test]
fn activates_null_and_empty_if_part_conditions() {
    for (variables, expected_active) in [
        (json!({}), vec!["human-task-empty", "human-task-null-equal"]),
        (
            json!({
                "optionalValue": null,
                "comment": "",
                "nonEmptyValue": ""
            }),
            vec!["human-task-empty", "human-task-null-equal"],
        ),
        (
            json!({
                "optionalValue": "set",
                "comment": "value",
                "nonEmptyValue": "value"
            }),
            vec!["human-task-not-empty", "human-task-null-not-equal"],
        ),
        (
            json!({
                "optionalValue": 42,
                "comment": 0,
                "nonEmptyValue": 42
            }),
            vec!["human-task-not-empty", "human-task-null-not-equal"],
        ),
        (
            json!({
                "optionalValue": false,
                "comment": false,
                "nonEmptyValue": false
            }),
            vec!["human-task-not-empty", "human-task-null-not-equal"],
        ),
    ] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        engine
            .deploy(
                CmmnDeploymentRequest::new("null-empty-if-part-entry-criterion").with_resource(
                    "null-empty-if-part-entry-criterion-case.cmmn",
                    null_empty_if_part_entry_criterion_model(),
                ),
            )
            .expect("deployment");

        let case_instance = engine
            .start_case_instance_by_key(
                "nullEmptyIfPartEntryCriterionCase",
                CmmnCaseInstanceStartRequest::new().with_variables(variables),
            )
            .expect("case instance");

        let active_tasks = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks");

        let task_a = active_tasks
            .iter()
            .find(|task| task.task_definition_id == "human-task-a")
            .expect("task a active");
        engine
            .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
            .expect("complete task a");

        let mut active_after_completion = engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .state(CmmnHumanTaskState::Active)
            .list()
            .expect("active tasks after completion")
            .into_iter()
            .map(|task| task.task_definition_id)
            .collect::<Vec<_>>();
        active_after_completion.sort();

        let mut expected_active = expected_active
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        expected_active.sort();

        assert_eq!(active_after_completion, expected_active);
    }
}

#[test]
fn activates_multi_on_part_entry_criterion_task_only_after_all_sources_complete() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("multi-on-part-entry-criterion").with_resource(
                "multi-on-part-entry-criterion-case.cmmn",
                multi_on_part_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "multiOnPartEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 2);
    assert!(
        active_tasks
            .iter()
            .any(|task| task.task_definition_id == "human-task-a")
    );
    assert!(
        active_tasks
            .iter()
            .any(|task| task.task_definition_id == "human-task-b")
    );

    let task_a = active_tasks
        .iter()
        .find(|task| task.task_definition_id == "human-task-a")
        .expect("task a");
    engine
        .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let active_after_first_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after first completion");
    assert_eq!(active_after_first_completion.len(), 1);
    assert_eq!(
        active_after_first_completion[0].task_definition_id,
        "human-task-b"
    );

    engine
        .complete_human_task(
            &active_after_first_completion[0].id,
            CmmnHumanTaskCompletionRequest::new(),
        )
        .expect("complete task b");

    let active_after_second_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after second completion");
    assert_eq!(active_after_second_completion.len(), 1);
    assert_eq!(
        active_after_second_completion[0].task_definition_id,
        "human-task-c"
    );
}

#[test]
fn activates_entry_criterion_task_after_event_listener_occurs() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("event-listener-occur-entry-criterion").with_resource(
                "event-listener-occur-entry-criterion-case.cmmn",
                event_listener_occur_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "eventListenerOccurEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert!(active_tasks.is_empty());

    let subscription = engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&case_instance.id)
        .single_result()
        .expect("subscription query")
        .expect("event subscription");

    engine
        .runtime_service()
        .occur_event_subscription(&subscription.id)
        .expect("occur event subscription");

    let active_after_occurrence = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after occurrence");
    assert_eq!(active_after_occurrence.len(), 1);
    assert_eq!(
        active_after_occurrence[0].task_definition_id,
        "human-task-approval"
    );

    let remaining_subscriptions = engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("remaining subscriptions");
    assert!(remaining_subscriptions.is_empty());
}

#[test]
fn entry_criterion_task_with_matching_manual_activation_rule_becomes_enabled_until_activated() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("manual-activation-entry").with_resource(
                "manual-activation-entry-case.cmmn",
                manual_activation_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "manualActivationEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "manualActivation": true })),
        )
        .expect("case instance");

    let task_a = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task a query")
        .expect("task a");
    assert_eq!(task_a.task_definition_id, "human-task-a");

    engine
        .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task a");

    let active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after completion");
    assert!(active_after_completion.is_empty());

    let enabled_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Enabled)
        .single_result()
        .expect("enabled task query")
        .expect("enabled task");
    assert_eq!(enabled_task.task_definition_id, "human-task-b");
    assert_eq!(enabled_task.plan_item_id, "plan-item-b");
    assert!(enabled_task.last_enabled_at.is_some());

    let premature_completion = engine
        .complete_human_task(&enabled_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect_err("enabled task must not complete before activation");
    assert!(
        premature_completion
            .to_string()
            .contains("must be active before it can be completed"),
        "unexpected error: {premature_completion}"
    );

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["human-task-b".to_string()],
                ..CmmnChangePlanItemStateRequest::default()
            },
        )
        .expect("activate enabled task");

    let active_task_b = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("active task b query")
        .expect("active task b");
    assert_eq!(active_task_b.id, enabled_task.id);
    assert_eq!(active_task_b.task_definition_id, "human-task-b");
}

#[test]
fn completing_task_with_matching_repetition_rule_creates_next_active_instance() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(CmmnDeploymentRequest::new("repetition-rule").with_resource(
            "repetition-rule-case.cmmn",
            repetition_rule_human_task_model(),
        ))
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "repetitionRuleCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "repeat": true })),
        )
        .expect("case instance");

    let first_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("first active task query")
        .expect("first active task");

    engine
        .complete_human_task(&first_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete first task");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after repetition");

    assert_eq!(active_tasks.len(), 1);
    assert_ne!(active_tasks[0].id, first_task.id);
    assert_eq!(active_tasks[0].plan_item_id, "plan-item-repeat");
    assert_eq!(active_tasks[0].task_definition_id, "human-task-repeat");
}

#[test]
fn change_state_add_and_remove_waiting_for_repetition_manages_repeatable_available_instance() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("repetition-rule-change-state").with_resource(
                "repetition-rule-case.cmmn",
                repetition_rule_human_task_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "repetitionRuleCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "repeat": false })),
        )
        .expect("case instance");

    let first_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("first active task query")
        .expect("first active task");
    engine
        .complete_human_task(&first_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete first task");

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                add_waiting_for_repetition_plan_item_definition_ids: vec![
                    "human-task-repeat".to_string(),
                ],
                ..CmmnChangePlanItemStateRequest::default()
            },
        )
        .expect("add waiting for repetition");

    let waiting_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Available)
        .single_result()
        .expect("available repetition task query")
        .expect("available repetition task");
    assert_eq!(waiting_task.plan_item_id, "plan-item-repeat");
    assert_eq!(waiting_task.task_definition_id, "human-task-repeat");

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                remove_waiting_for_repetition_plan_item_definition_ids: vec![
                    "human-task-repeat".to_string(),
                ],
                ..CmmnChangePlanItemStateRequest::default()
            },
        )
        .expect("remove waiting for repetition");

    let available_after_remove = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Available)
        .list()
        .expect("available tasks after remove");
    assert!(available_after_remove.is_empty());
}

fn start_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b").with_entry_criterion("sentry-a-start"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-a-start",
            CmmnPlanItemOnPart::new("on-a-start", "plan-item-a", "start"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-start-entry-criterion",
        "startEntryCriterionCase",
        "Start entry criterion case",
        plan_model,
    )])
}

#[test]
fn starting_human_task_fires_start_standard_event_and_activates_dependent_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("start-entry")
                .with_resource("start-entry-case.cmmn", start_entry_criterion_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "startEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(
        active_tasks.len(),
        2,
        "both tasks should be active: Task A activates immediately, Task B activates via start sentry"
    );
    assert!(active_tasks.iter().any(|t| t.name == "Task A"));
    assert!(active_tasks.iter().any(|t| t.name == "Task B"));
}
