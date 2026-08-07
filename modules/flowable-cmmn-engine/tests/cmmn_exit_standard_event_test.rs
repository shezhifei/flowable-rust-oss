use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnChangePlanItemStateRequest,
    CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnModel, CmmnPlanItem, CmmnPlanItemOnPart, CmmnSentry, CmmnStage,
};

fn exit_entry_criterion_model(case_key: &str, case_name: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b")
                .with_entry_criterion("sentry-after-a-exit"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-a-exit",
            CmmnPlanItemOnPart::new("on-a-exit", "plan-item-a", "exit"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-exit-entry-criterion",
        case_key,
        case_name,
        plan_model,
    )])
}

#[test]
fn completing_human_task_fires_exit_standard_event_and_activates_dependent() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("exit-on-complete").with_resource(
                "exit-on-complete-case.cmmn",
                exit_entry_criterion_model("exitOnCompleteCase", "Exit on complete case"),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("exitOnCompleteCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let active_before = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks before complete");
    assert_eq!(active_before.len(), 1);
    assert_eq!(active_before[0].task_definition_id, "human-task-a");
    let task_a_id = active_before[0].id.clone();

    engine
        .runtime_service()
        .complete_human_task(&task_a_id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task A");

    let active_after = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after complete");
    assert_eq!(
        active_after.len(),
        1,
        "task B should activate via planItemA.exit after A completes"
    );
    assert_eq!(active_after[0].task_definition_id, "human-task-b");
    assert_eq!(active_after[0].name, "Task B");
}

#[test]
fn terminating_human_task_fires_exit_standard_event_and_activates_dependent() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("exit-on-terminate").with_resource(
                "exit-on-terminate-case.cmmn",
                exit_entry_criterion_model("exitOnTerminateCase", "Exit on terminate case"),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("exitOnTerminateCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let active_before = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks before terminate");
    assert_eq!(active_before.len(), 1);
    assert_eq!(active_before[0].task_definition_id, "human-task-a");

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                terminate_plan_item_definition_ids: vec!["human-task-a".to_string()],
                ..Default::default()
            },
        )
        .expect("terminate task A");

    let active_after = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after terminate");
    assert_eq!(
        active_after.len(),
        1,
        "task B should activate via planItemA.exit after A terminates"
    );
    assert_eq!(active_after[0].task_definition_id, "human-task-b");
    assert_eq!(active_after[0].name, "Task B");
}

#[test]
fn exit_standard_event_is_supported_by_engine_model() {
    assert!(CmmnPlanItemOnPart::is_supported_standard_event(
        CmmnPlanItemOnPart::STANDARD_EVENT_EXIT
    ));
    assert!(CmmnPlanItemOnPart::is_supported_standard_event("exit"));
    // Still-unsupported events remain rejected by the supported set.
    assert!(!CmmnPlanItemOnPart::is_supported_standard_event("resume"));
    assert!(!CmmnPlanItemOnPart::is_supported_standard_event("suspend"));
    assert!(!CmmnPlanItemOnPart::is_supported_standard_event("reenable"));
    assert!(!CmmnPlanItemOnPart::is_supported_standard_event("fault"));
}

#[test]
fn complete_sentry_does_not_fire_only_exit_dependent_and_exit_does_not_require_complete() {
    // Guard against accidental mapping of exit → complete or exit → terminate.
    // Model A: sentry on complete activates B.
    // Model B: sentry on exit activates C.
    // Completing A should activate both B (complete) and C (derived exit).
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_human_task(CmmnHumanTask::new("human-task-c", "Task C"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b")
                .with_entry_criterion("sentry-after-a-complete"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-c", "human-task-c")
                .with_entry_criterion("sentry-after-a-exit"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-a-complete",
            CmmnPlanItemOnPart::new("on-a-complete", "plan-item-a", "complete"),
        ))
        .with_sentry(CmmnSentry::new(
            "sentry-after-a-exit",
            CmmnPlanItemOnPart::new("on-a-exit", "plan-item-a", "exit"),
        ));

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-complete-and-exit",
        "completeAndExitCase",
        "Complete and exit case",
        plan_model,
    )]);

    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("complete-and-exit")
                .with_resource("complete-and-exit-case.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("completeAndExitCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let task_a = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .find(|t| t.task_definition_id == "human-task-a")
        .expect("task A");

    engine
        .runtime_service()
        .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task A");

    let active_after = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after complete");
    assert_eq!(active_after.len(), 2);
    assert!(
        active_after
            .iter()
            .any(|t| t.task_definition_id == "human-task-b")
    );
    assert!(
        active_after
            .iter()
            .any(|t| t.task_definition_id == "human-task-c")
    );
}

fn stage_exit_entry_criterion_model() -> CmmnModel {
    // Stage S starts with child task B. Completing task A exits the stage.
    // Stage.exit activates task C outside the stage.
    let stage = CmmnStage::new("stage-review", "Review stage")
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-b", "human-task-b"));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-c", "Task C"))
        .with_stage(stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-stage", "stage-review")
                .with_exit_criterion("sentry-exit-stage-after-a"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-c", "human-task-c")
                .with_entry_criterion("sentry-after-stage-exit"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-exit-stage-after-a",
            CmmnPlanItemOnPart::new("on-a-complete", "plan-item-a", "complete"),
        ))
        .with_sentry(CmmnSentry::new(
            "sentry-after-stage-exit",
            CmmnPlanItemOnPart::new("on-stage-exit", "plan-item-stage", "exit"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-stage-exit",
        "stageExitEventCase",
        "Stage exit event case",
        plan_model,
    )])
}

#[test]
fn terminating_stage_fires_exit_standard_event_and_activates_dependent() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("stage-exit-event").with_resource(
                "stage-exit-event-case.cmmn",
                stage_exit_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("stageExitEventCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let active_before = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks before");
    // Task A + Task B (inside stage). Task C waits on stage.exit.
    assert_eq!(active_before.len(), 2);
    assert!(
        active_before
            .iter()
            .any(|t| t.task_definition_id == "human-task-a")
    );
    assert!(
        active_before
            .iter()
            .any(|t| t.task_definition_id == "human-task-b")
    );
    assert!(
        !active_before
            .iter()
            .any(|t| t.task_definition_id == "human-task-c")
    );

    let task_a = active_before
        .iter()
        .find(|t| t.task_definition_id == "human-task-a")
        .expect("task A");
    engine
        .runtime_service()
        .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task A (exits stage)");

    let active_after = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after stage exit");
    assert_eq!(
        active_after.len(),
        1,
        "task C should activate via planItemStage.exit; A completed and B terminated with stage"
    );
    assert_eq!(active_after[0].task_definition_id, "human-task-c");
    assert_eq!(active_after[0].name, "Task C");
}

fn stage_start_entry_criterion_model() -> CmmnModel {
    // Stage S activates on start. Task C has entry criterion on stage.start.
    let stage = CmmnStage::new("stage-review", "Review stage")
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-b", "human-task-b"));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-c", "Task C"))
        .with_stage(stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-stage", "stage-review"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-c", "human-task-c")
                .with_entry_criterion("sentry-after-stage-start"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-stage-start",
            CmmnPlanItemOnPart::new("on-stage-start", "plan-item-stage", "start"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-stage-start",
        "stageStartEventCase",
        "Stage start event case",
        plan_model,
    )])
}

#[test]
fn starting_stage_fires_start_standard_event_and_activates_dependent() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("stage-start-event").with_resource(
                "stage-start-event-case.cmmn",
                stage_start_entry_criterion_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("stageStartEventCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let active = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(
        active.len(),
        2,
        "stage child task B and start-sentry task C should both be active"
    );
    assert!(
        active
            .iter()
            .any(|t| t.task_definition_id == "human-task-b")
    );
    assert!(
        active
            .iter()
            .any(|t| t.task_definition_id == "human-task-c")
    );
}
