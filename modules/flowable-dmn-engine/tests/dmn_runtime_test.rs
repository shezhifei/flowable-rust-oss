use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnHitPolicy,
    DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry,
    DmnUnaryTest,
};
use serde_json::json;

fn sample_model() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "dishDecision",
        "Dish decision",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "dishType")],
        vec![DmnOutputClause::new("output-1", "dish")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("salad")))],
                vec![DmnRuleOutputEntry::new(json!("light"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!("default"))],
            ),
        ],
    )])
}

#[test]
fn executes_latest_decision_by_key_and_returns_structured_outputs() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("runtime").with_resource("dish-decision.dmn", sample_model()),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({
                "dishType": "salad"
            })),
        )
        .expect("execution result");

    assert_eq!(result.decision_key, "dishDecision");
    assert_eq!(result.get_output("dish"), Some(&json!("light")));
    assert_eq!(result.matched_rule_id.as_deref(), Some("rule-1"));
}

#[test]
fn executes_latest_definition_version_when_multiple_versions_exist() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let repository = engine.repository_service();

    repository
        .deploy(
            DmnDeploymentRequest::new("runtime-v1")
                .with_resource("dish-decision-v1.dmn", sample_model()),
        )
        .expect("deployment v1");
    repository
        .deploy(
            DmnDeploymentRequest::new("runtime-v2")
                .with_resource("dish-decision-v2.dmn", sample_model()),
        )
        .expect("deployment v2");

    let result = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({
                "dishType": "unknown"
            })),
        )
        .expect("execution result");

    assert_eq!(result.decision_version, 2);
    assert_eq!(result.get_output("dish"), Some(&json!("default")));
}

#[test]
fn parent_deployment_id_scopes_runtime_execution_to_matching_deployment() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let repository = engine.repository_service();

    repository
        .deploy(
            DmnDeploymentRequest::new("parent-a")
                .with_parent_deployment_id("case-parent-a")
                .with_resource("dish-decision-a.dmn", sample_model()),
        )
        .expect("deployment a");
    repository
        .deploy(
            DmnDeploymentRequest::new("parent-b")
                .with_parent_deployment_id("case-parent-b")
                .with_resource("dish-decision-b.dmn", sample_model()),
        )
        .expect("deployment b");

    let result = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest {
                variables: json!({"dishType": "salad"}),
                parent_deployment_id: Some("case-parent-b".to_string()),
                ..DmnExecutionRequest::new(json!({}))
            },
        )
        .expect("execution result");

    assert_eq!(result.decision_version, 2);
    assert!(result.deployment_id.starts_with("dmn-deployment:"));
}
