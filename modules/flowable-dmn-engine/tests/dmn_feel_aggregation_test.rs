use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnHitPolicy,
    DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry,
    DmnUnaryTest, FeelExpressionEngine,
};
use serde_json::json;
use std::collections::HashMap;

fn score_decision() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "scoreDecision",
        "Score decision",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "scores")],
        vec![DmnOutputClause::new("output-1", "category")],
        vec![DmnRule::new(
            "rule-pass",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("evaluated"))],
        )],
    )])
}

fn deploy_and_collect_scores(scores: Vec<i64>) -> HashMap<String, serde_json::Value> {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("feel-aggregation")
                .with_resource("score.dmn", score_decision()),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "scoreDecision",
            DmnExecutionRequest::new(json!({
                "scores": scores
            })),
        )
        .expect("execution");

    let mut context = HashMap::new();
    for (key, value) in result.inputs {
        context.insert(key, value);
    }
    context.insert("scores".to_string(), json!(scores));
    context
}

#[test]
fn sum_function_aggregates_list_values_from_deployed_decision_context() {
    let context = deploy_and_collect_scores(vec![1, 2, 3, 4, 5]);
    let engine = FeelExpressionEngine::new();
    let result = engine
        .evaluate("sum(scores)", &context)
        .expect("sum should evaluate against deployed context");
    assert_eq!(result, json!(15.0));
}

#[test]
fn mean_function_averages_list_values_from_deployed_decision_context() {
    let context = deploy_and_collect_scores(vec![2, 4, 6, 8, 10]);
    let engine = FeelExpressionEngine::new();
    let result = engine
        .evaluate("mean(scores)", &context)
        .expect("mean should evaluate against deployed context");
    assert_eq!(result, json!(6.0));
}

#[test]
fn min_function_returns_smallest_value_from_deployed_decision_context() {
    let context = deploy_and_collect_scores(vec![5, 1, 3, 4, 2]);
    let engine = FeelExpressionEngine::new();
    let result = engine
        .evaluate("min(scores)", &context)
        .expect("min should evaluate against deployed context");
    assert_eq!(result, json!(1.0));
}

#[test]
fn max_function_returns_largest_value_from_deployed_decision_context() {
    let context = deploy_and_collect_scores(vec![5, 1, 9, 4, 2]);
    let engine = FeelExpressionEngine::new();
    let result = engine
        .evaluate("max(scores)", &context)
        .expect("max should evaluate against deployed context");
    assert_eq!(result, json!(9.0));
}

#[test]
fn sum_function_returns_zero_for_empty_list_in_deployed_context() {
    let context = deploy_and_collect_scores(vec![]);
    let engine = FeelExpressionEngine::new();
    let result = engine
        .evaluate("sum(scores)", &context)
        .expect("sum of empty list should return zero");
    assert_eq!(result, json!(0.0));
}

#[test]
fn mean_function_returns_structured_error_for_empty_list_in_deployed_context() {
    let context = deploy_and_collect_scores(vec![]);
    let engine = FeelExpressionEngine::new();
    let result = engine.evaluate("mean(scores)", &context);
    assert!(
        result.is_err(),
        "mean of empty list should return structured error"
    );
    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("mean requires at least one number"),
        "unexpected error message: {error}"
    );
}

#[test]
fn min_function_returns_structured_error_for_empty_list_in_deployed_context() {
    let context = deploy_and_collect_scores(vec![]);
    let engine = FeelExpressionEngine::new();
    let result = engine.evaluate("min(scores)", &context);
    assert!(
        result.is_err(),
        "min of empty list should return structured error"
    );
    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("min requires at least one number"),
        "unexpected error message: {error}"
    );
}

#[test]
fn max_function_returns_structured_error_for_empty_list_in_deployed_context() {
    let context = deploy_and_collect_scores(vec![]);
    let engine = FeelExpressionEngine::new();
    let result = engine.evaluate("max(scores)", &context);
    assert!(
        result.is_err(),
        "max of empty list should return structured error"
    );
    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("max requires at least one number"),
        "unexpected error message: {error}"
    );
}
