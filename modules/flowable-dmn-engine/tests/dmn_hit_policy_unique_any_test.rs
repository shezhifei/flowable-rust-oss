use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnError, DmnExecutionRequest, DmnHitPolicy,
    DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry,
    DmnUnaryTest,
};
use serde_json::json;

fn overlapping_model(hit_policy: DmnHitPolicy, outputs: (&str, &str)) -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "routingDecision",
        "Routing decision",
        hit_policy,
        vec![DmnInputClause::new("input-1", "channel")],
        vec![DmnOutputClause::new("output-1", "route")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!(outputs.0))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
                vec![DmnRuleOutputEntry::new(json!(outputs.1))],
            ),
        ],
    )])
}

#[test]
fn unique_hit_policy_executes_single_match() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("unique").with_resource(
            "routing.dmn",
            overlapping_model(DmnHitPolicy::Unique, ("manual", "email-queue")),
        ))
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({
                "channel": "phone"
            })),
        )
        .expect("execution");

    assert_eq!(result.hit_policy, DmnHitPolicy::Unique);
    assert_eq!(result.get_output("route"), Some(&json!("manual")));
    assert_eq!(result.matched_rule_id.as_deref(), Some("rule-1"));
}

#[test]
fn unique_hit_policy_rejects_overlapping_matches() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("unique").with_resource(
            "routing.dmn",
            overlapping_model(DmnHitPolicy::Unique, ("manual", "email-queue")),
        ))
        .expect("deployment");

    let error = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({
                "channel": "email"
            })),
        )
        .expect_err("UNIQUE must fail when more than one rule matches");

    assert!(matches!(error, DmnError::Execution { .. }));
    assert!(
        error.to_string().contains("UNIQUE") && error.to_string().contains("rule-1"),
        "unexpected error: {error}"
    );
}

#[test]
fn any_hit_policy_accepts_overlapping_matches_with_identical_outputs() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("any").with_resource(
            "routing.dmn",
            overlapping_model(DmnHitPolicy::Any, ("manual", "manual")),
        ))
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({
                "channel": "email"
            })),
        )
        .expect("execution");

    assert_eq!(result.hit_policy, DmnHitPolicy::Any);
    assert_eq!(result.get_output("route"), Some(&json!("manual")));
    assert_eq!(result.matched_rule_id.as_deref(), Some("rule-1"));
}

#[test]
fn any_hit_policy_rejects_overlapping_matches_with_different_outputs() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("any").with_resource(
            "routing.dmn",
            overlapping_model(DmnHitPolicy::Any, ("manual", "email-queue")),
        ))
        .expect("deployment");

    let error = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({
                "channel": "email"
            })),
        )
        .expect_err("ANY must fail when matching rules produce different outputs");

    assert!(matches!(error, DmnError::Execution { .. }));
    assert!(
        error.to_string().contains("ANY") && error.to_string().contains("rule-2"),
        "unexpected error: {error}"
    );
}
