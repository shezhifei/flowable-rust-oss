//! C9: cumulative multi-part sentry onPart accumulation is command-scoped in
//! `onEvent` trigger mode.
//!
//! Java only inserts a `SentryPartInstanceEntity` for a satisfied onPart in
//! default trigger mode (AbstractEvaluationCriteriaOperation.java:709-711).
//! In `onEvent` trigger mode the satisfied part lives only in the in-memory
//! `getSatisfiedSentryPartInstances()` collection for the current command
//! (:707-713) and is never persisted, so a multi-onPart sentry fires only when
//! every onPart is triggered inside the same command/transaction. Default mode
//! accumulates onPart occurrences across commands via the persisted log.
//!
//! These tests pin down the C9 addition: onPart satisfaction for `onEvent`
//! sentries is read from the command-scoped event set instead of the permanent
//! `ACT_CMMN_PLAN_ITEM_EVENT` log, so completions in separate commands do not
//! accumulate; default mode keeps its cross-command accumulation.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnHumanTask, CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
    CmmnPlanItemOnPart, CmmnSentry,
};

/// Two onParts (A1 complete, A2 complete), no ifPart, guarding task B. The
/// trigger mode is parameterised so the two modes can be contrasted with an
/// otherwise identical model.
fn multi_on_part_model(trigger_mode: Option<&str>) -> CmmnModel {
    let mut sentry = CmmnSentry::new(
        "sentry-b",
        CmmnPlanItemOnPart::new("on-a1-complete", "plan-item-a1", "complete"),
    )
    .with_plan_item_on_part(CmmnPlanItemOnPart::new(
        "on-a2-complete",
        "plan-item-a2",
        "complete",
    ));
    if let Some(mode) = trigger_mode {
        sentry = sentry.with_trigger_mode(mode);
    }

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a1", "Task A1"))
        .with_human_task(CmmnHumanTask::new("human-task-a2", "Task A2"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a1", "human-task-a1"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a2", "human-task-a2"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b").with_entry_criterion("sentry-b"),
        )
        .with_sentry(sentry);

    CmmnModel::new(vec![CmmnCase::new(
        "case-c9-multi-on-part",
        "c9MultiOnPartCase",
        "C9 multi onPart case",
        plan_model,
    )])
}

fn deploy(engine: &CmmnEngine, model: CmmnModel) {
    engine
        .deploy(CmmnDeploymentRequest::new("c9-on-event-scope").with_resource("c9.cmmn", model))
        .expect("deployment");
}

fn active_task_definitions(engine: &CmmnEngine, case_instance_id: &str) -> Vec<String> {
    let mut definitions: Vec<String> = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .map(|task| task.task_definition_id)
        .collect();
    definitions.sort();
    definitions
}

fn complete_task(engine: &CmmnEngine, case_instance_id: &str, task_definition_id: &str) {
    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .find(|task| task.task_definition_id == task_definition_id)
        .expect("task by definition id");
    engine
        .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task");
}

/// Default trigger mode: onPart occurrences accumulate across separate
/// commands via the persisted SentryPartInstance log
/// (AbstractEvaluationCriteriaOperation.java:709-711), so completing A1 and A2
/// in different commands still fires the sentry.
#[test]
fn default_trigger_mode_multi_on_part_accumulates_across_commands() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, multi_on_part_model(None));

    let case_instance = engine
        .start_case_instance_by_key("c9MultiOnPartCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    // Command 1: complete A1. B stays gated (only one onPart satisfied).
    complete_task(&engine, &case_instance.id, "human-task-a1");
    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-a2".to_string()]
    );

    // Command 2: complete A2. The default-mode log still carries A1's
    // occurrence, so both onParts are satisfied and B activates.
    complete_task(&engine, &case_instance.id, "human-task-a2");
    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-b".to_string()]
    );
}

/// onEvent trigger mode: satisfied onParts are not persisted
/// (AbstractEvaluationCriteriaOperation.java:707-713), so a multi-onPart
/// sentry only fires when every onPart occurs inside one command. Completing
/// A1 and A2 in separate commands does NOT fire the sentry.
#[test]
fn on_event_trigger_mode_multi_on_part_does_not_accumulate_across_commands() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        multi_on_part_model(Some(CmmnSentry::TRIGGER_MODE_ON_EVENT)),
    );

    let case_instance = engine
        .start_case_instance_by_key("c9MultiOnPartCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    // Command 1: complete A1. Only A1's event is in this command's scope.
    complete_task(&engine, &case_instance.id, "human-task-a1");
    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-a2".to_string()]
    );

    // Command 2: complete A2. A1's earlier occurrence is not in this command's
    // scope and is never read from the permanent log in onEvent mode, so the
    // second onPart is satisfied alone -> sentry does not fire, B stays gated.
    complete_task(&engine, &case_instance.id, "human-task-a2");
    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        Vec::<String>::new()
    );
}

/// A single-onPart onEvent sentry still fires: the sole onPart's event is
/// recorded within the completing command, so command-scoped satisfaction is
/// equivalent to the previous behaviour for the fast path.
#[test]
fn on_event_trigger_mode_single_on_part_still_fires_within_command() {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b").with_entry_criterion("sentry-b"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-b",
                CmmnPlanItemOnPart::new("on-a-complete", "plan-item-a", "complete"),
            )
            .with_trigger_mode(CmmnSentry::TRIGGER_MODE_ON_EVENT),
        );
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-c9-single-on-part",
        "c9SingleOnPartCase",
        "C9 single onPart case",
        plan_model,
    )]);

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, model);

    let case_instance = engine
        .start_case_instance_by_key("c9SingleOnPartCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    complete_task(&engine, &case_instance.id, "human-task-a");

    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-b".to_string()]
    );
}
