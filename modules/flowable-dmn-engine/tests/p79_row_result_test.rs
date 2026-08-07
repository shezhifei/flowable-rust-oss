//! P79 — DMN row-shaped result model (`List<Map>` + `multipleResults`).
//!
//! Java reference:
//! - `DecisionExecutionAuditContainer.java:48-49` decisionResult / multipleResults
//! - `AbstractHitPolicy.java:64-71` composeDecisionResults
//! - `HitPolicyRuleOrder.java:23` / `HitPolicyOutputOrder.java:32` multipleResults=true
//! - `HitPolicyCollect.java:75-80` aggregator-dependent multipleResults
//! - Rust extensions Complete / Batch retained as multi-row

use flowable_dmn_engine::{
    CollectOperator, DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest,
    DmnHitPolicy, DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry,
    DmnRuleOutputEntry, DmnUnaryTest, HistoricDecisionExecution, columnar_outputs_to_rows,
};
use serde_json::{Map, Value, json};

fn row(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

fn multi_hit_model(hit_policy: DmnHitPolicy) -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "routingDecision",
        "Routing",
        hit_policy,
        vec![DmnInputClause::new("input-1", "channel")],
        vec![
            DmnOutputClause::new("output-1", "route"),
            DmnOutputClause::new("output-2", "priority"),
        ],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
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

fn first_model() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "loanEligibility",
        "Loan",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "creditScore")],
        vec![
            DmnOutputClause::new("output-1", "approved"),
            DmnOutputClause::new("output-2", "riskBand"),
        ],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![
                DmnRuleOutputEntry::new(json!(true)),
                DmnRuleOutputEntry::new(json!("LOW")),
            ],
        )],
    )])
}

fn collect_sum_model() -> DmnModel {
    DmnModel::new(vec![
        DmnDecision::new(
            "decision-1",
            "scoreDecision",
            "Score",
            DmnHitPolicy::Collect,
            vec![DmnInputClause::new("input-1", "score")],
            vec![DmnOutputClause::new("output-1", "points").with_type_ref("number")],
            vec![
                DmnRule::new(
                    "rule-1",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                    vec![DmnRuleOutputEntry::new(json!(10))],
                ),
                DmnRule::new(
                    "rule-2",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThan(json!(50)))],
                    vec![DmnRuleOutputEntry::new(json!(25))],
                ),
            ],
        )
        .with_collect_operator(CollectOperator::Sum),
    ])
}

#[test]
fn first_hit_returns_single_row_not_multiple() {
    let engine = DmnEngine::new_in_memory().unwrap();
    engine
        .deploy(DmnDeploymentRequest::new("p79-first").with_resource("loan.dmn", first_model()))
        .unwrap();

    let result = engine
        .execute_by_key(
            "loanEligibility",
            DmnExecutionRequest::new(json!({"creditScore": 730})),
        )
        .unwrap();

    assert!(!result.multiple_results);
    assert_eq!(result.decision_result.len(), 1);
    assert_eq!(result.get_output("approved"), Some(&json!(true)));
    assert_eq!(result.get_output("riskBand"), Some(&json!("LOW")));
}

#[test]
fn rule_order_returns_rows_and_multiple_results_flag() {
    let engine = DmnEngine::new_in_memory().unwrap();
    engine
        .deploy(
            DmnDeploymentRequest::new("p79-rule-order")
                .with_resource("routing.dmn", multi_hit_model(DmnHitPolicy::RuleOrder)),
        )
        .unwrap();

    let result = engine
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({"channel": "email"})),
        )
        .unwrap();

    assert!(result.multiple_results);
    assert_eq!(
        result.decision_result,
        vec![
            row(&[("route", json!("manual")), ("priority", json!(10))]),
            row(&[("route", json!("email-queue")), ("priority", json!(20))]),
        ]
    );
}

#[test]
fn collect_without_aggregator_is_multi_row() {
    let engine = DmnEngine::new_in_memory().unwrap();
    engine
        .deploy(
            DmnDeploymentRequest::new("p79-collect")
                .with_resource("routing.dmn", multi_hit_model(DmnHitPolicy::Collect)),
        )
        .unwrap();

    let result = engine
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({"channel": "email"})),
        )
        .unwrap();

    assert!(result.multiple_results);
    assert_eq!(result.decision_result.len(), 2);
    assert_eq!(result.decision_result[0]["route"], json!("manual"));
    assert_eq!(result.decision_result[1]["priority"], json!(20));
}

#[test]
fn collect_sum_aggregation_is_single_row_not_multiple() {
    let engine = DmnEngine::new_in_memory().unwrap();
    engine
        .deploy(
            DmnDeploymentRequest::new("p79-collect-sum")
                .with_resource("score.dmn", collect_sum_model()),
        )
        .unwrap();

    let result = engine
        .execute_by_key(
            "scoreDecision",
            DmnExecutionRequest::new(json!({"score": 90})),
        )
        .unwrap();

    assert!(!result.multiple_results);
    assert_eq!(result.decision_result.len(), 1);
    assert_eq!(result.get_output("points"), Some(&json!(35.0)));
}

#[test]
fn complete_and_batch_extensions_remain_multi_row() {
    for hit_policy in [DmnHitPolicy::Complete, DmnHitPolicy::Batch] {
        let engine = DmnEngine::new_in_memory().unwrap();
        engine
            .deploy(
                DmnDeploymentRequest::new(format!("p79-{hit_policy:?}"))
                    .with_resource("routing.dmn", multi_hit_model(hit_policy.clone())),
            )
            .unwrap();

        let result = engine
            .execute_by_key(
                "routingDecision",
                DmnExecutionRequest::new(json!({"channel": "email"})),
            )
            .unwrap();

        assert!(
            result.multiple_results,
            "{hit_policy:?} must keep multi-row flag"
        );
        assert_eq!(result.decision_result.len(), 2, "{hit_policy:?}");
        assert_eq!(result.hit_policy, hit_policy);
    }
}

#[test]
fn history_persists_row_shape_and_reads_legacy_columnar() {
    let engine = DmnEngine::new_in_memory().unwrap();
    engine
        .deploy(
            DmnDeploymentRequest::new("p79-history")
                .with_resource("routing.dmn", multi_hit_model(DmnHitPolicy::RuleOrder)),
        )
        .unwrap();

    let result = engine
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({"channel": "email"})),
        )
        .unwrap();

    let historic = engine
        .history_service()
        .create_execution_history_query()
        .execution_id(&result.execution_id)
        .single_result()
        .unwrap()
        .expect("historic row");

    assert!(historic.multiple_results);
    assert_eq!(historic.decision_result.len(), 2);
    assert_eq!(historic.get_output("route"), Some(&json!("manual")));

    // Legacy columnar JSON (pre-P79) must still deserialize.
    let legacy_json = json!({
        "execution_id": "legacy-1",
        "decision_definition_id": "def-1",
        "deployment_id": "dep-1",
        "decision_key": "routingDecision",
        "decision_name": "Routing",
        "decision_version": 1,
        "hit_policy": "RuleOrder",
        "matched_rule_id": null,
        "matched_rule_count": 2,
        "rule_executions": [],
        "business_key": null,
        "tenant_id": null,
        "executed_at": "2026-01-01T00:00:00Z",
        "inputs": {"channel": "email"},
        "outputs": {
            "route": ["manual", "email-queue"],
            "priority": [10, 20]
        }
    });
    let legacy: HistoricDecisionExecution = serde_json::from_value(legacy_json).unwrap();
    assert_eq!(legacy.decision_result.len(), 2);
    assert_eq!(legacy.decision_result[0]["route"], json!("manual"));
    assert_eq!(legacy.decision_result[1]["priority"], json!(20));

    // Helper used by the compatibility path.
    let rows = columnar_outputs_to_rows(Map::from_iter([
        ("route".to_string(), json!(["a", "b"])),
        ("n".to_string(), json!([1, 2])),
    ]));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["route"], json!("b"));
}

#[test]
fn stack_variables_use_last_row_wins() {
    let engine = DmnEngine::new_in_memory().unwrap();
    engine
        .deploy(
            DmnDeploymentRequest::new("p79-stack")
                .with_resource("routing.dmn", multi_hit_model(DmnHitPolicy::RuleOrder)),
        )
        .unwrap();

    let result = engine
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({"channel": "email"})),
        )
        .unwrap();

    let stack = result.stack_variables();
    // Java AbstractHitPolicy.updateStackWithDecisionResults — last row wins.
    assert_eq!(stack.get("route"), Some(&json!("email-queue")));
    assert_eq!(stack.get("priority"), Some(&json!(20)));
}
