use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnHitPolicy,
    DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry,
    DmnUnaryTest,
};
use serde_json::json;

fn route_model(hit_policy: DmnHitPolicy) -> DmnModel {
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

fn risk_model(hit_policy: DmnHitPolicy) -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "riskDecision",
        "Risk decision",
        hit_policy,
        vec![DmnInputClause::new("input-1", "score")],
        vec![
            DmnOutputClause::new("output-1", "riskBand").with_output_values(vec![
                json!("HIGH"),
                json!("MEDIUM"),
                json!("LOW"),
            ]),
        ],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                    json!(0),
                ))],
                vec![DmnRuleOutputEntry::new(json!("LOW"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                    json!(50),
                ))],
                vec![DmnRuleOutputEntry::new(json!("MEDIUM"))],
            ),
            DmnRule::new(
                "rule-3",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                    json!(80),
                ))],
                vec![DmnRuleOutputEntry::new(json!("HIGH"))],
            ),
        ],
    )])
}

#[test]
fn rule_order_returns_all_matching_outputs_in_rule_definition_order() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("rule order")
                .with_resource("routing.dmn", route_model(DmnHitPolicy::RuleOrder)),
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

    assert_eq!(result.hit_policy, DmnHitPolicy::RuleOrder);
    // P79: row-shaped multi-hit
    assert!(result.multiple_results);
    assert_eq!(
        result.decision_result,
        vec![
            serde_json::Map::from_iter([("route".to_string(), json!("manual"))]),
            serde_json::Map::from_iter([("route".to_string(), json!("email-queue"))]),
        ]
    );
    assert_eq!(result.matched_rule_id, None);
    assert_eq!(result.matched_rule_count, 2);
}

#[test]
fn output_order_returns_all_matching_outputs_by_output_values_priority() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("output order")
                .with_resource("risk.dmn", risk_model(DmnHitPolicy::OutputOrder)),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "riskDecision",
            DmnExecutionRequest::new(json!({
                "score": 90
            })),
        )
        .expect("execution");

    assert_eq!(result.hit_policy, DmnHitPolicy::OutputOrder);
    // P79: row-shaped multi-hit, ordered by outputValues priority
    assert!(result.multiple_results);
    assert_eq!(
        result.decision_result,
        vec![
            serde_json::Map::from_iter([("riskBand".to_string(), json!("HIGH"))]),
            serde_json::Map::from_iter([("riskBand".to_string(), json!("MEDIUM"))]),
            serde_json::Map::from_iter([("riskBand".to_string(), json!("LOW"))]),
        ]
    );
    assert_eq!(result.matched_rule_count, 3);
}

#[test]
fn priority_returns_highest_priority_matching_output() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("priority")
                .with_resource("risk.dmn", risk_model(DmnHitPolicy::Priority)),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "riskDecision",
            DmnExecutionRequest::new(json!({
                "score": 90
            })),
        )
        .expect("execution");

    assert_eq!(result.hit_policy, DmnHitPolicy::Priority);
    assert_eq!(result.get_output("riskBand"), Some(&json!("HIGH")));
    assert_eq!(result.matched_rule_id.as_deref(), Some("rule-3"));
    assert_eq!(result.matched_rule_count, 3);
}
