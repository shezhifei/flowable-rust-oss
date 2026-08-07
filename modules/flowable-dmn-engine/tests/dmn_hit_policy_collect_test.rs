use flowable_dmn_engine::{
    CollectOperator, DmnDecision, DmnDeploymentRequest, DmnEngine, DmnError, DmnExecutionRequest,
    DmnHitPolicy, DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry,
    DmnRuleOutputEntry, DmnUnaryTest,
};
use serde_json::json;

fn collect_model() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "routingDecision",
        "Routing decision",
        DmnHitPolicy::Collect,
        vec![DmnInputClause::new("input-1", "channel")],
        vec![
            DmnOutputClause::new("output-1", "route"),
            DmnOutputClause::new("output-2", "priority"),
        ],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![
                    DmnRuleOutputEntry::new(json!("manual")),
                    DmnRuleOutputEntry::new(json!(10)),
                ],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
                vec![
                    DmnRuleOutputEntry::new(json!("email-queue")),
                    DmnRuleOutputEntry::new(json!(20)),
                ],
            ),
        ],
    )])
}

fn aggregate_model(operator: CollectOperator) -> DmnModel {
    DmnModel::new(vec![
        DmnDecision::new(
            "decision-1",
            "scoreDecision",
            "Score decision",
            DmnHitPolicy::Collect,
            vec![DmnInputClause::new("input-1", "score")],
            // P82c: COLLECT+aggregation requires typeRef=number (Java RuleEngineExecutorImpl:329)
            vec![DmnOutputClause::new("output-1", "points").with_type_ref("number")],
            vec![
                DmnRule::new(
                    "rule-1",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                        json!(0),
                    ))],
                    vec![DmnRuleOutputEntry::new(json!(10))],
                ),
                DmnRule::new(
                    "rule-2",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                        json!(50),
                    ))],
                    vec![DmnRuleOutputEntry::new(json!(20))],
                ),
                DmnRule::new(
                    "rule-3",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                        json!(80),
                    ))],
                    vec![DmnRuleOutputEntry::new(json!(5))],
                ),
            ],
        )
        .with_collect_operator(operator),
    ])
}

fn execute_aggregate(operator: CollectOperator) -> serde_json::Value {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("collect aggregate")
                .with_resource("score.dmn", aggregate_model(operator)),
        )
        .expect("deployment");

    engine
        .decision_service()
        .execute_by_key(
            "scoreDecision",
            DmnExecutionRequest::new(json!({
                "score": 90
            })),
        )
        .expect("execution")
        .get_output("points").cloned().unwrap()
}

#[test]
fn collect_hit_policy_returns_all_matching_outputs_in_rule_order() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("collect").with_resource("routing.dmn", collect_model()))
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

    assert_eq!(result.hit_policy, DmnHitPolicy::Collect);
    // P79: row-shaped multi-hit (was columnar Map<name, Vec>)
    assert!(result.multiple_results);
    assert_eq!(
        result.decision_result,
        vec![
            serde_json::Map::from_iter([
                ("route".to_string(), json!("manual")),
                ("priority".to_string(), json!(10)),
            ]),
            serde_json::Map::from_iter([
                ("route".to_string(), json!("email-queue")),
                ("priority".to_string(), json!(20)),
            ]),
        ]
    );
    assert_eq!(result.matched_rule_id, None);
    assert_eq!(result.matched_rule_count, 2);
}

#[test]
fn collect_hit_policy_returns_empty_outputs_when_no_rule_matches() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let model = DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "routingDecision",
        "Routing decision",
        DmnHitPolicy::Collect,
        vec![DmnInputClause::new("input-1", "channel")],
        vec![DmnOutputClause::new("output-1", "route")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
            vec![DmnRuleOutputEntry::new(json!("email-queue"))],
        )],
    )]);
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("collect").with_resource("routing.dmn", model))
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

    assert!(result.decision_result.is_empty());
    assert_eq!(result.matched_rule_id, None);
    assert_eq!(result.matched_rule_count, 0);
}

#[test]
fn collect_count_returns_number_of_matching_rules() {
    // P88: COLLECT Count via typeRef=number → f64 (HitPolicyCollect.java:134-136).
    assert_eq!(execute_aggregate(CollectOperator::Count), json!(3.0));
}

#[test]
fn collect_sum_returns_sum_of_matching_single_output_values() {
    assert_eq!(execute_aggregate(CollectOperator::Sum), json!(35.0));
}

#[test]
fn collect_min_returns_minimum_matching_single_output_value() {
    assert_eq!(execute_aggregate(CollectOperator::Min), json!(5.0));
}

#[test]
fn collect_max_returns_maximum_matching_single_output_value() {
    assert_eq!(execute_aggregate(CollectOperator::Max), json!(20.0));
}

#[test]
fn collect_aggregation_returns_zero_count_when_no_rule_matches() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("collect aggregate")
                .with_resource("score.dmn", aggregate_model(CollectOperator::Count)),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "scoreDecision",
            DmnExecutionRequest::new(json!({
                "score": -1
            })),
        )
        .expect("execution");

    // P88: Count 0 with typeRef=number → 0.0 (HitPolicyCollect.java:134-136).
    assert_eq!(result.get_output("points"), Some(&json!(0.0)));
    assert_eq!(result.matched_rule_count, 0);
}

/// P82c: COLLECT+aggregation with multiple outputs is rejected at deploy time
/// (Java RuleEngineExecutorImpl.java:325-326 — runtime; Rust uses deploy-time).
#[test]
fn p82c_rejects_collect_aggregation_with_multiple_outputs_at_deployment() {
    let model = DmnModel::new(vec![
        DmnDecision::new(
            "decision-1",
            "scoreDecision",
            "Score decision",
            DmnHitPolicy::Collect,
            vec![DmnInputClause::new("input-1", "score")],
            vec![
                DmnOutputClause::new("output-1", "points").with_type_ref("number"),
                DmnOutputClause::new("output-2", "extra").with_type_ref("number"),
            ],
            vec![
                DmnRule::new(
                    "rule-1",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                    vec![
                        DmnRuleOutputEntry::new(json!(10)),
                        DmnRuleOutputEntry::new(json!(5)),
                    ],
                ),
                DmnRule::new(
                    "rule-2",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                    vec![
                        DmnRuleOutputEntry::new(json!(20)),
                        DmnRuleOutputEntry::new(json!(15)),
                    ],
                ),
            ],
        )
        .with_collect_operator(CollectOperator::Sum),
    ]);

    let engine = DmnEngine::new_in_memory().expect("engine");
    let error = engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("invalid").with_resource("score.dmn", model))
        .expect_err("multi-output COLLECT aggregation must be rejected");

    assert!(
        matches!(error, DmnError::Validation { .. }),
        "unexpected error kind: {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("multiple outputs") && message.contains("not supported"),
        "unexpected error: {message}"
    );
}

/// P82c: single output with typeRef=string rejected (needs number). COUNT included.
#[test]
fn p82c_rejects_collect_aggregation_with_non_number_type_ref() {
    for operator in [
        CollectOperator::Sum,
        CollectOperator::Count,
        CollectOperator::Min,
        CollectOperator::Max,
    ] {
        let model = DmnModel::new(vec![
            DmnDecision::new(
                "decision-1",
                "scoreDecision",
                "Score decision",
                DmnHitPolicy::Collect,
                vec![DmnInputClause::new("input-1", "score")],
                vec![DmnOutputClause::new("output-1", "points").with_type_ref("string")],
                vec![DmnRule::new(
                    "rule-1",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                    // string value so typeRef coerce does not fail first
                    vec![DmnRuleOutputEntry::new(json!("ten"))],
                )],
            )
            .with_collect_operator(operator.clone()),
        ]);
        let engine = DmnEngine::new_in_memory().expect("engine");
        let error = engine
            .repository_service()
            .deploy(DmnDeploymentRequest::new("invalid").with_resource("score.dmn", model))
            .expect_err("non-number typeRef must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("needs output type number"),
            "operator={operator:?} unexpected error: {message}"
        );
    }
}

/// P82c: COUNT with multiple outputs also rejected.
#[test]
fn p82c_rejects_count_aggregation_with_multiple_outputs() {
    let model = DmnModel::new(vec![
        DmnDecision::new(
            "decision-1",
            "scoreDecision",
            "Score decision",
            DmnHitPolicy::Collect,
            vec![DmnInputClause::new("input-1", "score")],
            vec![
                DmnOutputClause::new("output-1", "points").with_type_ref("number"),
                DmnOutputClause::new("output-2", "extra").with_type_ref("number"),
            ],
            vec![DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![
                    DmnRuleOutputEntry::new(json!(10)),
                    DmnRuleOutputEntry::new(json!(5)),
                ],
            )],
        )
        .with_collect_operator(CollectOperator::Count),
    ]);
    let engine = DmnEngine::new_in_memory().expect("engine");
    let error = engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("invalid").with_resource("score.dmn", model))
        .expect_err("COUNT multi-output must be rejected");
    assert!(
        error.to_string().contains("multiple outputs"),
        "unexpected: {error}"
    );
}

/// P82c regression: SUM single output number typeRef deploys and executes.
#[test]
fn p82c_sum_single_number_output_still_allowed() {
    assert_eq!(execute_aggregate(CollectOperator::Sum), json!(35.0));
}

/// P82c: COLLECT without aggregation may have multiple outputs.
#[test]
fn p82c_collect_without_aggregation_allows_multiple_outputs() {
    // collect_model() has two outputs and no collect_operator — already covered by
    // collect_hit_policy_returns_all_matching_outputs_in_rule_order; assert deploy succeeds.
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("collect").with_resource("routing.dmn", collect_model()))
        .expect("COLLECT without aggregation allows multiple outputs");
}

#[test]
fn rejects_sum_min_max_aggregation_with_non_numeric_outputs_at_deployment() {
    for operator in [
        CollectOperator::Sum,
        CollectOperator::Min,
        CollectOperator::Max,
    ] {
        let model = DmnModel::new(vec![
            DmnDecision::new(
                "decision-1",
                "scoreDecision",
                "Score decision",
                DmnHitPolicy::Collect,
                vec![DmnInputClause::new("input-1", "score")],
                vec![DmnOutputClause::new("output-1", "points").with_type_ref("number")],
                vec![DmnRule::new(
                    "rule-1",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                    vec![DmnRuleOutputEntry::new(json!("not-a-number"))],
                )],
            )
            .with_collect_operator(operator.clone()),
        ]);
        let engine = DmnEngine::new_in_memory().expect("engine");
        let error = engine
            .repository_service()
            .deploy(DmnDeploymentRequest::new("invalid").with_resource("score.dmn", model))
            .expect_err("numeric aggregation should reject non-numeric outputs");

        assert!(
            matches!(
                error,
                DmnError::Validation { .. } | DmnError::UnsupportedModel { .. }
            ),
            "unexpected error kind: {error:?}"
        );
        let message = error.to_string();
        // May fail at typeRef coerce (incompatible value) or collect numeric check.
        assert!(
            message.contains("numeric")
                || message.contains("incompatible value")
                || message.contains("typeRef"),
            "unexpected error: {message}"
        );
    }
}
