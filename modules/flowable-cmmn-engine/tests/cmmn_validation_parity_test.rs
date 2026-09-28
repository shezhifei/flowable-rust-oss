// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

use flowable_cmmn_engine::{
    CmmnCase, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask, CmmnModel,
    CmmnPlanItem,
};

fn make_valid_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));

    CmmnModel::new(vec![CmmnCase::new(
        "case-review",
        "reviewCase",
        "Review case",
        plan_model,
    )])
}

#[test]
fn cmmn_validation_accepts_valid_model() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let model = make_valid_model();
    let request = CmmnDeploymentRequest::new("Test Deployment").with_resource("test.cmmn", model);
    let result = engine.repository_service().deploy(request);
    assert!(result.is_ok(), "Valid CMMN model should deploy");
}

#[test]
fn cmmn_validation_accepts_blank_deployment_name_as_absent() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let model = make_valid_model();
    let request = CmmnDeploymentRequest::new("  ").with_resource("test.cmmn", model);
    let deployment = engine
        .repository_service()
        .deploy(request)
        .expect("a resource-only deployment should be valid");
    assert_eq!(deployment.name, None);
}

#[test]
fn cmmn_validation_rejects_empty_resources() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let request = CmmnDeploymentRequest::new("Test Deployment");
    let result = engine.repository_service().deploy(request);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("resources are required")
    );
}

#[test]
fn cmmn_validation_rejects_non_cmmn_resource_name() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let model = make_valid_model();
    let request = CmmnDeploymentRequest::new("Test Deployment").with_resource("test.bpmn", model);
    let result = engine.repository_service().deploy(request);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("at least one CMMN model resource"),
        "deployment with only non-.cmmn resources must be rejected"
    );
}
