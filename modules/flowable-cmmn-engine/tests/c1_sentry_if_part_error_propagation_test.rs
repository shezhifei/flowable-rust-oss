// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose:
// a failing fixture setup or assertion should abort loudly. Production code
// under `src/` is held to the lint; see the root Cargo.toml
// `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! P2 parity (Java Flowable 8): sentry `ifPart` expression evaluation errors
//! must propagate as case-operation errors. Java evaluates the ifPart inside
//! `AbstractEvaluationCriteriaOperation.evaluateSentryIfPart` with no catch: a
//! broken expression aborts the command; only a genuinely *missing* variable
//! (resolves to null) makes the ifPart evaluate to false.
//!
//! Before this contract the Rust engine swallowed evaluation errors
//! (`.ok()`), so a malformed ifPart silently behaved like `false` and the
//! case transition looked successful while the guarded plan item never
//! activated.
//!
//! End-to-end coverage here pins the two outcomes reachable through a
//! deployed model: the lenient missing-variable path stays `Ok(false)` and
//! the normal satisfied path still activates the guarded task. The actual
//! error branch (a runtime evaluation `Err`) is covered by the inline
//! `sentry_if_part_error_propagation_tests` module in `runtime.rs`: the only
//! evaluator error reachable from a parsed ifPart AST is the depth guard
//! (64 nested nodes), and such a model cannot round-trip deployment
//! hydration because serde_json's fixed 128-frame recursion cap rejects the
//! serialized model before the runtime ever evaluates it. The inline tests
//! construct the parsed AST directly, bypassing storage.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnModel, CmmnPlanItem, CmmnPlanItemOnPart, CmmnSentry,
};
use serde_json::json;

/// Deploy `Intake` (completed) -> sentry -> `Review` and return the started
/// case instance id.
fn deploy_and_start(
    engine: &CmmnEngine,
    case_key: &str,
    if_part: &str,
    variables: serde_json::Value,
) -> String {
    let sentry = CmmnSentry::new(
        "sentry-intake-complete",
        CmmnPlanItemOnPart::new("on-intake-complete", "plan-item-intake", "complete"),
    )
    .with_if_part(if_part);

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review", "human-task-review")
                .with_entry_criterion("sentry-intake-complete"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        case_key,
        case_key,
        "IfPart error propagation case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new(case_key).with_resource(format!("{case_key}.cmmn"), model),
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

fn active_task_names(engine: &CmmnEngine, case_instance_id: &str) -> Vec<String> {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .map(|task| task.name)
        .collect()
}

#[test]
fn missing_if_part_variable_stays_lenient_false() {
    // Java/UEL parity: an unresolved variable is null and the ifPart is
    // simply not satisfied; this must remain Ok(false) and must not be
    // mistaken for an evaluation error.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        "ifPartMissingVarCase",
        "approved == true",
        json!({}),
    );

    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    let intake_task_id = tasks
        .iter()
        .find(|task| task.name == "Intake")
        .expect("intake task active")
        .id
        .clone();

    engine
        .runtime_service()
        .complete_human_task(&intake_task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("missing variable is lenient false, not an error");

    // Intake completed; Review is not activated (sentry unsatisfied).
    assert_eq!(
        active_task_names(&engine, &case_instance_id),
        Vec::<String>::new()
    );
}

#[test]
fn satisfied_if_part_still_activates_guarded_task() {
    // Non-regression: the error-propagation change must not affect the
    // normal true path.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(
        &engine,
        "ifPartSatisfiedCase",
        "approved == true",
        json!({ "approved": true }),
    );

    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    let intake_task_id = tasks
        .iter()
        .find(|task| task.name == "Intake")
        .expect("intake task active")
        .id
        .clone();

    engine
        .runtime_service()
        .complete_human_task(&intake_task_id, CmmnHumanTaskCompletionRequest::new())
        .expect("satisfied ifPart completes the transition");

    assert_eq!(
        active_task_names(&engine, &case_instance_id),
        vec!["Review".to_string()]
    );

    // Case instance stays active while Review is open.
    let case_instance = engine
        .runtime_service()
        .get_case_instance(&case_instance_id)
        .expect("case instance");
    assert_eq!(case_instance.state, CmmnCaseInstanceState::Active);
}
