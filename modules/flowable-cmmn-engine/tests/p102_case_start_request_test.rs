// P102: CMMN case start request additions — transient variables, outcome,
// overrideDefinitionTenantId.
//
// Java references:
// - CaseInstanceCreateRequest.java:37-48 (request fields)
// - CaseInstanceCollectionResource.java:357-401 (parsing + builder wiring)
// - CaseInstanceHelperImpl.java:275 (transient variables), :325-326 (override tenant)
//
// Intentional cuts (P102 acceptance): startFormVariables (no form engine) and
// fallbackToDefaultTenant (CMMN has no such flag) are not implemented.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest,
    CmmnEngine, CmmnHumanTask, CmmnModel, CmmnPlanItem,
};
use serde_json::json;

fn expression_model(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("task-express", "Express task").with_assignee("${assignee}"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-express", "task-express"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "P102 expression case",
        plan_model,
    )])
}

#[test]
fn transient_variables_visible_during_start_but_not_persisted() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("p102Transient-deployment").with_resource(
                "p102Transient.cmmn",
                expression_model("p102Transient"),
            ),
        )
        .expect("deployment");

    let instance = engine
        .start_case_instance_by_key(
            "p102Transient",
            CmmnCaseInstanceStartRequest::new()
                .with_variables(json!({ "real": "kept" }))
                .with_transient_variables(json!({ "assignee": "alice", "temp": "gone" })),
        )
        .expect("case instance");

    // The expression-resolved assignee saw the transient variable.
    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&instance.id)
        .single_result()
        .expect("query")
        .expect("task");
    assert_eq!(task.assignee.as_deref(), Some("alice"));

    // Transient variables are NOT persisted on the case instance.
    let refreshed = engine
        .runtime_service()
        .get_case_instance(&instance.id)
        .expect("case");
    let variable_names = refreshed
        .variables
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(variable_names, vec!["real".to_string()]);
    assert!(
        !variable_names.contains(&"temp".to_string()),
        "transient variable must not be persisted"
    );
}

#[test]
fn transient_variables_do_not_override_persisted_variables() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("p102TransientOverride-deployment").with_resource(
                "p102TransientOverride.cmmn",
                expression_model("p102TransientOverride"),
            ),
        )
        .expect("deployment");

    let instance = engine
        .start_case_instance_by_key(
            "p102TransientOverride",
            CmmnCaseInstanceStartRequest::new()
                .with_variables(json!({ "assignee": "real-owner" }))
                .with_transient_variables(json!({ "assignee": "transient-owner" })),
        )
        .expect("case instance");

    // Transient wins during activation (Java merges transient last).
    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&instance.id)
        .single_result()
        .expect("query")
        .expect("task");
    assert_eq!(task.assignee.as_deref(), Some("transient-owner"));

    // The persisted value keeps the real variable.
    let refreshed = engine
        .runtime_service()
        .get_case_instance(&instance.id)
        .expect("case");
    assert_eq!(refreshed.variables.get("assignee"), Some(&json!("real-owner")));
}

#[test]
fn override_definition_tenant_id_sets_case_instance_tenant() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("p102Override-deployment").with_resource(
                "p102Override.cmmn",
                expression_model("p102Override"),
            ),
        )
        .expect("deployment");

    // Java CaseInstanceHelperImpl.java:325-326 — the override replaces the case
    // instance tenant; the definition lookup still uses the request tenant.
    let instance = engine
        .start_case_instance_by_key(
            "p102Override",
            CmmnCaseInstanceStartRequest::new()
                .with_override_definition_tenant_id("tenant-override"),
        )
        .expect("case instance");
    assert_eq!(instance.tenant_id.as_deref(), Some("tenant-override"));

    let refreshed = engine
        .runtime_service()
        .get_case_instance(&instance.id)
        .expect("case");
    assert_eq!(refreshed.tenant_id.as_deref(), Some("tenant-override"));
}

#[test]
fn outcome_is_accepted_and_dropped() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("p102Outcome-deployment").with_resource(
                "p102Outcome.cmmn",
                expression_model("p102Outcome"),
            ),
        )
        .expect("deployment");

    let instance = engine
        .start_case_instance_by_key(
            "p102Outcome",
            CmmnCaseInstanceStartRequest::new().with_outcome("approved"),
        )
        .expect("case instance");
    assert_eq!(instance.state, flowable_cmmn_engine::CmmnCaseInstanceState::Active);
}
