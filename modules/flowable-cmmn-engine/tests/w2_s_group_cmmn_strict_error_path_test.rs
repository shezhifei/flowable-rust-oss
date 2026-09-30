// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! W2 residual S-group (cmmn) — CMMN EL evaluation errors must fail the
//! command, never be disguised as null / unavailable / empty.
//!
//! Java evidence (research A.2 #1-3):
//! - `AbstractEvaluationCriteriaOperation.java:590` — `expression.getValue`
//!   has no catch; only a non-boolean/null *result* is treated as unavailable.
//! - `ExpressionPlanItemLifecycleListener` family — no catch.
//! - correlation-key getValue family.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnEventListener, CmmnHumanTask, CmmnModel, CmmnPlanItem,
};

fn deploy(engine: &CmmnEngine, key: &str, model: CmmnModel) {
    engine
        .deploy(CmmnDeploymentRequest::new(key).with_resource("case.cmmn", model))
        .expect("deployment");
}

fn single_case(case_key: &str, plan_model: CmmnCasePlanModel) -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        "w2-cmmn-case",
        case_key,
        "W2 cmmn S-group case",
        plan_model,
    )])
}

/// availableCondition `${undefinedAvailVar}` must fail the start command
/// (AbstractEvaluationCriteriaOperation:590 — no catch). Before W2 the lenient
/// path treated the evaluation error as "unavailable" (false).
#[test]
fn available_condition_undefined_variable_fails_start() {
    let plan = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("gated-el-listener", "message")
                .with_event_name("gatedEvent")
                .with_available_condition("${undefinedAvailVar}"),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-gated-listener",
            "gated-el-listener",
        ))
        .with_human_task(CmmnHumanTask::new("human-task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-keepalive",
            "human-task-keepalive",
        ));

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        "w2-cmmn-available",
        single_case("w2CmmnAvailableCase", plan),
    );

    let result = engine.start_case_instance_by_key(
        "w2CmmnAvailableCase",
        CmmnCaseInstanceStartRequest::new(),
    );
    assert!(
        result.is_err(),
        "start must fail when availableCondition evaluation errors (not silently unavailable)"
    );
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("undefinedAvailVar")
            || message.contains("availableCondition")
            || message.contains("Unknown property")
            || message.contains("failed"),
        "error should identify the availableCondition failure, got: {message}"
    );
}

/// Human-task `assignee="${undefinedAssigneeVar}"` must fail task creation
/// (ExpressionPlanItemLifecycleListener family — no catch).
#[test]
fn human_task_assignee_undefined_variable_fails_start() {
    let plan = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("review-task", "Review").with_assignee("${undefinedAssigneeVar}"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "review-task"));

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        "w2-cmmn-assignee",
        single_case("w2CmmnAssigneeCase", plan),
    );

    let result = engine.start_case_instance_by_key(
        "w2CmmnAssigneeCase",
        CmmnCaseInstanceStartRequest::new(),
    );
    assert!(
        result.is_err(),
        "start must fail when human-task assignee expression errors"
    );
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("undefinedAssigneeVar")
            || message.contains("assignee")
            || message.contains("Unknown property")
            || message.contains("failed"),
        "error should identify the assignee expression failure, got: {message}"
    );
}
