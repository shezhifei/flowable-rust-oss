//! P131/P132 — enable uses the explicit ENABLED lifecycle state.
//!
//! Java references:
//! - `EnablePlanItemInstanceOperation.java:39-51` stores ENABLED + lastEnabledTime.
//! - P132 reserves Rust AVAILABLE for sentry waiting, so the public re-enable
//!   command accepts DISABLED rather than bypassing an unsatisfied sentry.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnChangePlanItemStateRequest,
    CmmnDeploymentRequest, CmmnEngine, CmmnError, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskInstance, CmmnHumanTaskState, CmmnModel, CmmnPlanItem, CmmnPlanItemOnPart,
    CmmnSentry,
};

fn enable_semantics_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("p131-plan", "P131 plan")
        .with_human_task(CmmnHumanTask::new("source-task", "Source"))
        .with_human_task(CmmnHumanTask::new("enabled-target-task", "Enabled target"))
        .with_plan_item(
            CmmnPlanItem::new("source-plan-item", "source-task")
                .with_manual_activation_rule("true"),
        )
        .with_plan_item(
            CmmnPlanItem::new("enabled-target-plan-item", "enabled-target-task")
                .with_entry_criterion("after-source-enabled"),
        )
        .with_sentry(CmmnSentry::new(
            "after-source-enabled",
            CmmnPlanItemOnPart::new("on-source-enable", "source-plan-item", "enable"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "p131-case",
        "p131EnableCase",
        "P131 enable case",
        plan_model,
    )])
}

fn engine() -> CmmnEngine {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("p131-enable")
                .with_resource("p131-enable.cmmn", enable_semantics_model()),
        )
        .expect("deployment");
    engine
}

fn start_case(engine: &CmmnEngine) -> String {
    engine
        .start_case_instance_by_key("p131EnableCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn task(engine: &CmmnEngine, case_instance_id: &str, definition_id: &str) -> CmmnHumanTaskInstance {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .list()
        .expect("task query")
        .into_iter()
        .find(|task| task.task_definition_id == definition_id)
        .unwrap_or_else(|| panic!("task {definition_id} should exist"))
}

#[test]
fn enabled_plan_item_rejects_redundant_enable() {
    let engine = engine();
    let case_id = start_case(&engine);
    let source = task(&engine, &case_id, "source-task");
    assert_eq!(source.state, CmmnHumanTaskState::Enabled);
    assert!(source.last_enabled_at.is_some());

    // P132: manual activation already ran Java's enable operation
    // (ActivatePlanItemInstanceOperation.java:48-55), so a second enable is invalid.
    let error = engine
        .runtime_service()
        .enable_plan_item_instance(&source.id)
        .expect_err("enabled plan item must reject redundant enable");
    assert!(matches!(error, CmmnError::Conflict { .. }));
}

#[test]
fn disabled_plan_item_enable_returns_it_to_enabled() {
    let engine = engine();
    let case_id = start_case(&engine);
    let source = task(&engine, &case_id, "source-task");

    engine
        .runtime_service()
        .disable_plan_item_instance(&source.id)
        .expect("disable enabled plan item");
    assert_eq!(
        task(&engine, &case_id, "source-task").state,
        CmmnHumanTaskState::Disabled
    );

    // Java EnablePlanItemInstanceCmd.java:42-46 accepts DISABLED and the
    // operation stores ENABLED (EnablePlanItemInstanceOperation.java:39-51).
    engine
        .runtime_service()
        .enable_plan_item_instance(&source.id)
        .expect("disabled plan item should enable");
    assert_eq!(
        task(&engine, &case_id, "source-task").state,
        CmmnHumanTaskState::Enabled
    );
}

#[test]
fn active_and_completed_plan_items_still_reject_enable() {
    let engine = engine();

    let active_case_id = start_case(&engine);
    let active_source = task(&engine, &active_case_id, "source-task");
    engine
        .runtime_service()
        .start_plan_item_instance(&active_source.id)
        .expect("start source task");
    let active_error = engine
        .runtime_service()
        .enable_plan_item_instance(&active_source.id)
        .expect_err("active plan item must reject enable");
    assert!(matches!(active_error, CmmnError::Conflict { .. }));

    let completed_case_id = start_case(&engine);
    let completed_source = task(&engine, &completed_case_id, "source-task");
    engine
        .runtime_service()
        .start_plan_item_instance(&completed_source.id)
        .expect("start source task");
    engine
        .complete_human_task(&completed_source.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete source task");
    let completed_error = engine
        .runtime_service()
        .enable_plan_item_instance(&completed_source.id)
        .expect_err("completed plan item must reject enable");
    assert!(matches!(completed_error, CmmnError::Conflict { .. }));
}

#[test]
fn disable_and_start_accept_only_enabled() {
    // Java DisablePlanItemInstanceCmd.java:44-45 and
    // StartPlanItemInstanceCmd.java:54-55 both require ENABLED.
    let engine = engine();
    let case_id = start_case(&engine);
    let source = task(&engine, &case_id, "source-task");

    engine
        .runtime_service()
        .disable_plan_item_instance(&source.id)
        .expect("ENABLED -> DISABLED");
    assert!(matches!(
        engine
            .runtime_service()
            .disable_plan_item_instance(&source.id)
            .expect_err("DISABLED must reject disable"),
        CmmnError::Conflict { .. }
    ));
    assert!(matches!(
        engine
            .runtime_service()
            .start_plan_item_instance(&source.id)
            .expect_err("DISABLED must reject start"),
        CmmnError::Conflict { .. }
    ));

    engine
        .runtime_service()
        .enable_plan_item_instance(&source.id)
        .expect("DISABLED -> ENABLED");
    engine
        .runtime_service()
        .start_plan_item_instance(&source.id)
        .expect("ENABLED -> ACTIVE");
    assert!(matches!(
        engine
            .runtime_service()
            .start_plan_item_instance(&source.id)
            .expect_err("ACTIVE must reject start"),
        CmmnError::Conflict { .. }
    ));
    assert!(matches!(
        engine
            .runtime_service()
            .disable_plan_item_instance(&source.id)
            .expect_err("ACTIVE must reject disable"),
        CmmnError::Conflict { .. }
    ));

    engine
        .complete_human_task(&source.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete active source");
    assert!(matches!(
        engine
            .runtime_service()
            .start_plan_item_instance(&source.id)
            .expect_err("COMPLETED must reject start"),
        CmmnError::Conflict { .. }
    ));
    assert!(matches!(
        engine
            .runtime_service()
            .disable_plan_item_instance(&source.id)
            .expect_err("COMPLETED must reject disable"),
        CmmnError::Conflict { .. }
    ));
}

#[test]
fn available_waiting_for_repetition_rejects_control_commands() {
    // AVAILABLE now exclusively represents a plan item still waiting for its
    // lifecycle trigger. P132 therefore prevents enable/disable/start from
    // bypassing that wait.
    let plan_model = CmmnCasePlanModel::new("available-plan", "Available plan")
        .with_human_task(CmmnHumanTask::new("repeat-task", "Repeat task"))
        .with_human_task(CmmnHumanTask::new("keepalive-task", "Keep alive"))
        .with_plan_item(
            CmmnPlanItem::new("repeat-plan-item", "repeat-task")
                .with_repetition_rule("repeat == true"),
        )
        .with_plan_item(CmmnPlanItem::new("keepalive-plan-item", "keepalive-task"));
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("available-guards").with_resource(
                "available-guards.cmmn",
                CmmnModel::new(vec![CmmnCase::new(
                    "available-case",
                    "availableGuardCase",
                    "Available guard case",
                    plan_model,
                )]),
            ),
        )
        .expect("deployment");
    let case_id = engine
        .start_case_instance_by_key(
            "availableGuardCase",
            CmmnCaseInstanceStartRequest::new()
                .with_variables(serde_json::json!({ "repeat": false })),
        )
        .expect("case")
        .id;
    let repeat_task = task(&engine, &case_id, "repeat-task");
    engine
        .complete_human_task(&repeat_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete repeat task");
    engine
        .runtime_service()
        .change_plan_item_state(
            &case_id,
            CmmnChangePlanItemStateRequest {
                add_waiting_for_repetition_plan_item_definition_ids: vec![
                    "repeat-task".to_string(),
                ],
                ..Default::default()
            },
        )
        .expect("add waiting repetition instance");
    let available = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Available)
        .single_result()
        .expect("available query")
        .expect("available waiting instance");
    assert_eq!(available.state, CmmnHumanTaskState::Available);

    for result in [
        engine
            .runtime_service()
            .enable_plan_item_instance(&available.id),
        engine
            .runtime_service()
            .disable_plan_item_instance(&available.id),
        engine
            .runtime_service()
            .start_plan_item_instance(&available.id),
    ] {
        assert!(matches!(result, Err(CmmnError::Conflict { .. })));
    }
}
