//! C2: SentryPartInstance-equivalent persistence for the cumulative
//! multi-part sentry path.
//!
//! Java persists a `SentryPartInstanceEntity` per satisfied sentry part when
//! the sentry is in default trigger mode
//! (`AbstractEvaluationCriteriaOperation.createSentryPartInstanceEntity`,
//! AbstractEvaluationCriteriaOperation.java:679-715, insert gated on
//! `isDefaultTriggerMode` at :709-711). Satisfied parts are read back on the
//! next evaluation cycle (:515-525), so a satisfied ifPart "sticks" even if
//! the underlying variables later change. In onEvent trigger mode nothing is
//! persisted and the ifPart must hold at the moment the onParts complete
//! (Sentry.java:23-36, trigger-mode gate at
//! AbstractEvaluationCriteriaOperation.java:550-551).
//!
//! The Rust engine already accumulates onPart occurrences through the
//! `ACT_CMMN_PLAN_ITEM_EVENT` log; these tests pin down the C2 addition: the
//! sticky ifPart marker for default trigger mode and the strict
//! at-event-time evaluation for onEvent trigger mode.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnHumanTask, CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
    CmmnPlanItemOnPart, CmmnSentry,
};
use serde_json::json;

/// Task A completes -> entry sentry (onPart complete + ifPart
/// `approved == true`) guards task B. `trigger_mode` is left `None`
/// (= default, Sentry.java:30-32).
fn default_mode_entry_model(trigger_mode: Option<&str>) -> CmmnModel {
    let mut sentry = CmmnSentry::new(
        "sentry-b",
        CmmnPlanItemOnPart::new("on-a-complete", "plan-item-a", "complete"),
    )
    .with_if_part("approved == true");
    if let Some(mode) = trigger_mode {
        sentry = sentry.with_trigger_mode(mode);
    }

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b").with_entry_criterion("sentry-b"),
        )
        .with_sentry(sentry);

    CmmnModel::new(vec![CmmnCase::new(
        "case-c2-entry",
        "c2EntryCase",
        "C2 entry criterion case",
        plan_model,
    )])
}

/// Two onParts (A1 complete, A2 complete) + ifPart guard task B.
fn multi_on_part_entry_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a1", "Task A1"))
        .with_human_task(CmmnHumanTask::new("human-task-a2", "Task A2"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a1", "human-task-a1"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a2", "human-task-a2"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b").with_entry_criterion("sentry-b"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-b",
                CmmnPlanItemOnPart::new("on-a1-complete", "plan-item-a1", "complete"),
            )
            .with_plan_item_on_part(CmmnPlanItemOnPart::new(
                "on-a2-complete",
                "plan-item-a2",
                "complete",
            ))
            .with_if_part("approved == true"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-c2-multi-on-part",
        "c2MultiOnPartCase",
        "C2 multi onPart case",
        plan_model,
    )])
}

/// Exit sentry (onPart A complete + ifPart `cancel == true`) on task B.
fn default_mode_exit_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-a", "Task A"))
        .with_human_task(CmmnHumanTask::new("human-task-b", "Task B"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "human-task-a"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-b", "human-task-b").with_exit_criterion("sentry-exit-b"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-exit-b",
                CmmnPlanItemOnPart::new("on-a-complete-exit-b", "plan-item-a", "complete"),
            )
            .with_if_part("cancel == true"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-c2-exit",
        "c2ExitCase",
        "C2 exit criterion case",
        plan_model,
    )])
}

fn deploy(engine: &CmmnEngine, model: CmmnModel) {
    engine
        .deploy(CmmnDeploymentRequest::new("c2-sentry-part").with_resource("c2.cmmn", model))
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

fn set_variable(engine: &CmmnEngine, case_instance_id: &str, name: &str, value: bool) {
    engine
        .runtime_service()
        .set_case_instance_variables(case_instance_id, vec![(name.to_string(), json!(value))])
        .expect("set variables");
}

/// Default trigger mode: a satisfied ifPart is persisted
/// (AbstractEvaluationCriteriaOperation.java:558-566, :709-711) and read
/// back in later cycles (:515-525), so the sentry still fires after the
/// variable reverts to `false`.
#[test]
fn default_trigger_mode_persists_if_part_across_commands() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, default_mode_entry_model(None));

    let case_instance = engine
        .start_case_instance_by_key(
            "c2EntryCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "approved": false })),
        )
        .expect("case instance");

    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-a".to_string()]
    );

    // ifPart becomes true -> the evaluation sweep persists the marker.
    set_variable(&engine, &case_instance.id, "approved", true);
    // Variable falls back; in Java the SentryPartInstance survives this.
    set_variable(&engine, &case_instance.id, "approved", false);

    complete_task(&engine, &case_instance.id, "human-task-a");

    // Sentry fires from the persisted ifPart even though `approved` is
    // currently false.
    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-b".to_string()]
    );
}

/// Cumulative path: onParts accumulate across events (Java persists one
/// SentryPartInstance per onPart, AbstractEvaluationCriteriaOperation
/// .java:536) and the sticky ifPart joins them
/// (AbstractEvaluationCriteriaOperation.java:506-577).
#[test]
fn default_trigger_mode_multi_on_part_accumulates_with_sticky_if_part() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, multi_on_part_entry_model());

    let case_instance = engine
        .start_case_instance_by_key(
            "c2MultiOnPartCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "approved": false })),
        )
        .expect("case instance");

    complete_task(&engine, &case_instance.id, "human-task-a1");
    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-a2".to_string()]
    );

    set_variable(&engine, &case_instance.id, "approved", true);
    set_variable(&engine, &case_instance.id, "approved", false);

    complete_task(&engine, &case_instance.id, "human-task-a2");

    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-b".to_string()]
    );
}

/// onEvent trigger mode: nothing is persisted
/// (AbstractEvaluationCriteriaOperation.java:709-711 only inserts in
/// default mode), so the ifPart must hold at the moment the onPart fires
/// (Sentry.java:34-36).
#[test]
fn on_event_trigger_mode_requires_if_part_at_event_time() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        default_mode_entry_model(Some(CmmnSentry::TRIGGER_MODE_ON_EVENT)),
    );

    let case_instance = engine
        .start_case_instance_by_key(
            "c2EntryCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "approved": false })),
        )
        .expect("case instance");

    // The transient `approved == true` window is NOT remembered in
    // onEvent mode.
    set_variable(&engine, &case_instance.id, "approved", true);
    set_variable(&engine, &case_instance.id, "approved", false);

    complete_task(&engine, &case_instance.id, "human-task-a");

    // ifPart is false at event time -> sentry does not fire.
    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        Vec::<String>::new()
    );
}

/// onEvent trigger mode still fires when the ifPart holds at event time
/// (trigger-mode gate AbstractEvaluationCriteriaOperation.java:550-551).
#[test]
fn on_event_trigger_mode_fires_when_if_part_true_at_event() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        default_mode_entry_model(Some(CmmnSentry::TRIGGER_MODE_ON_EVENT)),
    );

    let case_instance = engine
        .start_case_instance_by_key(
            "c2EntryCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "approved": true })),
        )
        .expect("case instance");

    complete_task(&engine, &case_instance.id, "human-task-a");

    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-b".to_string()]
    );
}

/// Exit criteria run through the same cumulative machinery
/// (AbstractEvaluationCriteriaOperation.evaluateExitCriteria delegates to
/// evaluateCriteria), so the sticky ifPart also applies there.
#[test]
fn default_trigger_mode_exit_criterion_if_part_sticks() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, default_mode_exit_model());

    let case_instance = engine
        .start_case_instance_by_key(
            "c2ExitCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "cancel": false })),
        )
        .expect("case instance");

    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-a".to_string(), "human-task-b".to_string()]
    );

    set_variable(&engine, &case_instance.id, "cancel", true);
    set_variable(&engine, &case_instance.id, "cancel", false);

    complete_task(&engine, &case_instance.id, "human-task-a");

    // Task B was exited by the sticky ifPart.
    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        Vec::<String>::new()
    );
}

/// Single onPart without ifPart is not the cumulative path
/// (AbstractEvaluationCriteriaOperation.java:475-490 fast path); trigger
/// mode has no observable effect there.
#[test]
fn single_on_part_no_if_part_fast_path_unchanged() {
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
        "case-c2-fast-path",
        "c2FastPathCase",
        "C2 fast path case",
        plan_model,
    )]);

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, model);

    let case_instance = engine
        .start_case_instance_by_key("c2FastPathCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    complete_task(&engine, &case_instance.id, "human-task-a");

    assert_eq!(
        active_task_definitions(&engine, &case_instance.id),
        vec!["human-task-b".to_string()]
    );
}

/// Trigger-mode helpers mirror Sentry.java:23-36 exactly.
#[test]
fn trigger_mode_defaults_and_builders() {
    let base = CmmnSentry::new("s", CmmnPlanItemOnPart::new("on", "x", "complete"));
    // triggerMode == null -> default mode (Sentry.java:30-32).
    assert!(base.is_default_trigger_mode());
    assert!(!base.is_on_event_trigger_mode());
    // Single onPart, no ifPart -> fast path, not cumulative.
    assert!(!base.is_multi_part());

    let explicit_default = base.clone().with_trigger_mode(CmmnSentry::TRIGGER_MODE_DEFAULT);
    assert!(explicit_default.is_default_trigger_mode());
    assert!(!explicit_default.is_on_event_trigger_mode());

    let on_event = base.clone().with_trigger_mode(CmmnSentry::TRIGGER_MODE_ON_EVENT);
    assert!(!on_event.is_default_trigger_mode());
    assert!(on_event.is_on_event_trigger_mode());

    // onPart + ifPart -> cumulative multi-part evaluation
    // (AbstractEvaluationCriteriaOperation.java:506).
    assert!(base.clone().with_if_part("approved == true").is_multi_part());
    // Two onParts -> cumulative even without ifPart.
    assert!(base
        .with_plan_item_on_part(CmmnPlanItemOnPart::new("on-2", "y", "complete"))
        .is_multi_part());
}
