//! P88 — output typeRef number/double normalize to f64 (Java Double).
//!
//! Java truth:
//! - `RuleEngineExecutorImpl.java:246,253-254` → `ExecutionVariableFactory.getExecutionVariable`
//! - `ExecutionVariableFactory.java:60-69` — typeRef number always yields `Double` (7 → 7.0)
//! - `HitPolicyCollect.java:100,103,118-141` — COLLECT aggregates are Double; Count is `(double) size`

use flowable_dmn_engine::{
    CollectOperator, DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest,
    DmnHitPolicy, DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry,
    DmnRuleOutputEntry, DmnUnaryTest,
};
use serde_json::{json, Value};

fn deploy(model: DmnModel) -> DmnEngine {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("p88").with_resource("p88.dmn", model))
        .expect("deployment");
    engine
}

fn execute(engine: &DmnEngine, key: &str, vars: Value) -> flowable_dmn_engine::DmnExecutionResult {
    engine
        .decision_service()
        .execute_by_key(key, DmnExecutionRequest::new(vars))
        .expect("execution")
}

fn single_output_model(type_ref: &str, output_entry: DmnRuleOutputEntry) -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "p88Out",
        "P88 output normalize",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "x")],
        vec![DmnOutputClause::new("out-1", "amount").with_type_ref(type_ref)],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![output_entry],
        )],
    )])
}

/// Integer JSON literal + typeRef number → f64 7.0.
#[test]
fn number_type_ref_normalizes_integer_literal_to_f64() {
    let engine = deploy(single_output_model(
        "number",
        DmnRuleOutputEntry::new(json!(7)),
    ));
    let result = execute(&engine, "p88Out", json!({"x": 0}));
    assert_eq!(result.get_output("amount"), Some(&json!(7.0)));
    assert!(result.get_output("amount").unwrap().as_f64().is_some());
    assert!(
        result
            .get_output("amount")
            .unwrap()
            .as_i64()
            .is_none(),
        "must not remain a JSON integer"
    );
}

/// Integer JSON literal + typeRef double → f64 7.0.
#[test]
fn double_type_ref_normalizes_integer_literal_to_f64() {
    let engine = deploy(single_output_model(
        "double",
        DmnRuleOutputEntry::new(json!(7)),
    ));
    let result = execute(&engine, "p88Out", json!({"x": 0}));
    assert_eq!(result.get_output("amount"), Some(&json!(7.0)));
}

/// Numeric string output + typeRef number → f64.
#[test]
fn number_type_ref_coerces_integer_string_to_f64() {
    let engine = deploy(single_output_model(
        "number",
        DmnRuleOutputEntry::from_expression("\"42\""),
    ));
    let result = execute(&engine, "p88Out", json!({"x": 0}));
    assert_eq!(result.get_output("amount"), Some(&json!(42.0)));
}

/// FEEL arithmetic producing whole number + typeRef double → f64.
#[test]
fn double_type_ref_normalizes_feel_arithmetic_to_f64() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "p88Arith",
        "P88 arithmetic",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "price")],
        vec![DmnOutputClause::new("out-1", "total").with_type_ref("double")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::from_expression("price * 2")],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "p88Arith", json!({"price": 21}));
    assert_eq!(result.get_output("total"), Some(&json!(42.0)));
}

/// COLLECT Count with typeRef=number yields f64 (Java HitPolicyCollect.java:134-136).
#[test]
fn collect_count_with_number_type_ref_yields_f64() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "p88Count",
        "P88 count",
        DmnHitPolicy::Collect,
        vec![DmnInputClause::new("in-1", "score")],
        vec![DmnOutputClause::new("out-1", "hits").with_type_ref("number")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!(1))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!(2))],
            ),
        ],
    )
    .with_collect_operator(CollectOperator::Count)]);
    let engine = deploy(model);
    let result = execute(&engine, "p88Count", json!({"score": 1}));
    assert_eq!(result.get_output("hits"), Some(&json!(2.0)));
    assert!(
        result.get_output("hits").unwrap().as_i64().is_none(),
        "Count must be f64 after number typeRef coerce"
    );
}

/// typeRef integer remains integer (intentional Rust extension; Java rejects).
#[test]
fn integer_type_ref_extension_keeps_integer_json() {
    let engine = deploy(single_output_model(
        "integer",
        DmnRuleOutputEntry::new(json!(7)),
    ));
    let result = execute(&engine, "p88Out", json!({"x": 0}));
    assert_eq!(result.get_output("amount"), Some(&json!(7)));
}

/// Missing typeRef is pass-through (integer stays integer).
#[test]
fn missing_type_ref_passes_integer_through() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "p88Pass",
        "P88 pass-through",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "x")],
        vec![DmnOutputClause::new("out-1", "amount")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!(7))],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "p88Pass", json!({"x": 0}));
    assert_eq!(result.get_output("amount"), Some(&json!(7)));
}
