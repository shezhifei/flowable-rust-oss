use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnHitPolicy,
    DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry,
    DmnRuleOutputEntry, DmnUnaryTest,
};
use serde_json::json;

fn audit_model() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "p63-decision",
        "p63Routing",
        "P63 routing",
        DmnHitPolicy::Unique,
        vec![DmnInputClause::new("tier-input", "tier")],
        vec![DmnOutputClause::new("route-output", "route")],
        vec![
            DmnRule::new(
                "gold-rule",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("gold")))
                    .with_id("gold-condition")],
                vec![DmnRuleOutputEntry::new(json!("priority")).with_id("gold-conclusion")],
            ),
            DmnRule::new(
                "standard-rule",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("standard")))
                    .with_id("standard-condition")],
                vec![DmnRuleOutputEntry::new(json!("normal")).with_id("standard-conclusion")],
            ),
        ],
    )])
}

#[test]
fn runtime_and_history_expose_rule_condition_and_conclusion_results() {
    let engine = DmnEngine::new_in_memory().unwrap();
    engine
        .deploy(
            DmnDeploymentRequest::new("P63 audit")
                .with_resource("p63-audit.dmn", audit_model()),
        )
        .unwrap();

    let result = engine
        .execute_by_key(
            "p63Routing",
            DmnExecutionRequest::new(json!({"tier": "gold"})),
        )
        .unwrap();

    assert_eq!(result.inputs["tier"], json!("gold"));
    assert_eq!(result.rule_executions.len(), 2);
    assert_eq!(result.rule_executions[0].rule_number, 1);
    assert_eq!(result.rule_executions[0].rule_id, "gold-rule");
    assert!(result.rule_executions[0].valid);
    assert_eq!(
        result.rule_executions[0].condition_results[0].id,
        "gold-condition"
    );
    assert_eq!(
        result.rule_executions[0].condition_results[0].result,
        json!(true)
    );
    assert_eq!(
        result.rule_executions[0].conclusion_results[0].id,
        "gold-conclusion"
    );
    assert_eq!(
        result.rule_executions[0].conclusion_results[0].result,
        json!("priority")
    );
    assert!(!result.rule_executions[1].valid);
    assert_eq!(
        result.rule_executions[1].condition_results[0].result,
        json!(false)
    );
    assert!(result.rule_executions[1].conclusion_results.is_empty());

    let historic = engine
        .history_service()
        .create_execution_history_query()
        .execution_id(&result.execution_id)
        .single_result()
        .unwrap()
        .unwrap();
    assert_eq!(historic.inputs["tier"], json!("gold"));
    assert_eq!(historic.rule_executions, result.rule_executions);
}
