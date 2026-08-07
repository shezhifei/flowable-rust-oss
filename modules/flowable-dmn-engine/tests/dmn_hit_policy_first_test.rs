use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnHitPolicy,
    DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry,
    DmnUnaryTest,
};
use serde_json::json;

fn overlapping_first_model() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "routingDecision",
        "Routing decision",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "channel")],
        vec![DmnOutputClause::new("output-1", "route")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!("manual"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
                vec![DmnRuleOutputEntry::new(json!("email-queue"))],
            ),
        ],
    )])
}

#[test]
fn first_hit_policy_stops_at_first_matching_rule() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("hit-policy")
                .with_resource("routing.dmn", overlapping_first_model()),
        )
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

    assert_eq!(result.get_output("route"), Some(&json!("manual")));
    assert_eq!(result.matched_rule_id.as_deref(), Some("rule-1"));
}

#[test]
fn accepts_collect_hit_policy_at_deployment_time() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let collect = DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "collectDecision",
        "Collect decision",
        DmnHitPolicy::Collect,
        vec![DmnInputClause::new("input-1", "value")],
        vec![DmnOutputClause::new("output-1", "result")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("ok"))],
        )],
    )]);

    let deployment = engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("collect").with_resource("collect.dmn", collect))
        .expect("COLLECT without aggregation should deploy");

    assert_eq!(deployment.resource_names, vec!["collect.dmn"]);
}
