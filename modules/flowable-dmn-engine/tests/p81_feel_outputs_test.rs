//! P81 — DMN output entry runtime FEEL evaluation.
//!
//! Java truth: output entries evaluated at runtime via JUEL
//! (`RuleEngineExecutorImpl.java:248-289`, `ELExpressionExecutor.java:74-80`).
//! Rust wires `FeelExpressionEngine` (FEEL subset dialect).

use flowable_dmn_engine::{
    CollectOperator, DmnDecision, DmnDeploymentRequest, DmnEngine, DmnError, DmnExecutionRequest,
    DmnHitPolicy, DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry,
    DmnRuleOutputEntry, DmnUnaryTest,
};
use flowable_dmn_model::{
    Decision, DecisionRule, DecisionTable, DmnDefinition, HitPolicy, InputClause,
    LiteralExpression, OutputClause, UnaryTests,
};
use serde_json::{json, Value};

fn deploy(model: DmnModel) -> DmnEngine {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("p81").with_resource("p81.dmn", model))
        .expect("deployment");
    engine
}

fn execute(engine: &DmnEngine, key: &str, vars: Value) -> Result<flowable_dmn_engine::DmnExecutionResult, DmnError> {
    engine
        .decision_service()
        .execute_by_key(key, DmnExecutionRequest::new(vars))
}

/// Regression: pure literal outputs via `DmnRuleOutputEntry::new` unchanged.
#[test]
fn pure_literal_outputs_unchanged() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "lit-decision",
        "literalOut",
        "Literal outputs",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "tier")],
        vec![DmnOutputClause::new("out-1", "route")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("gold")))],
            vec![DmnRuleOutputEntry::new(json!("priority"))],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "literalOut", json!({"tier": "gold"})).expect("exec");
    assert_eq!(result.get_output("route"), Some(&json!("priority")));
}

/// Expression referencing input variable: arithmetic.
#[test]
fn output_expression_references_input_arithmetic() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "price-decision",
        "priceDouble",
        "Double price",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "price")],
        vec![DmnOutputClause::new("out-1", "total")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::from_expression("price * 2")],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "priceDouble", json!({"price": 21})).expect("exec");
    assert_eq!(result.get_output("total"), Some(&json!(42)));
}

/// Multi-word FEEL string function via compat path (`upper case`).
#[test]
fn output_expression_upper_case_input() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "name-decision",
        "upperName",
        "Uppercase name",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "name")],
        vec![DmnOutputClause::new("out-1", "display")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::from_expression("upper case(name)")],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "upperName", json!({"name": "alice"})).expect("exec");
    assert_eq!(result.get_output("display"), Some(&json!("ALICE")));
}

/// Multi-word FEEL function `starts with` via compat path.
#[test]
fn output_expression_starts_with_compat() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "starts-decision",
        "startsCheck",
        "Starts with check",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "name")],
        vec![DmnOutputClause::new("out-1", "ok")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::from_expression(
                "starts with(name, \"Al\")",
            )],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "startsCheck", json!({"name": "Alice"})).expect("exec");
    assert_eq!(result.get_output("ok"), Some(&json!(true)));
}

/// Same-rule prior output reference (clause order put into scope).
#[test]
fn same_rule_prior_output_reference() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "chain-decision",
        "outputChain",
        "Chained outputs",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "base")],
        vec![
            DmnOutputClause::new("out-1", "step1"),
            DmnOutputClause::new("out-2", "step2"),
        ],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![
                DmnRuleOutputEntry::from_expression("base + 1"),
                DmnRuleOutputEntry::from_expression("step1 * 10"),
            ],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "outputChain", json!({"base": 3})).expect("exec");
    assert_eq!(result.get_output("step1"), Some(&json!(4)));
    assert_eq!(result.get_output("step2"), Some(&json!(40)));
}

/// Output default for missing output var (number → 0) when referenced.
#[test]
fn output_default_number_when_referencing_unset_output() {
    // Java ELExecutionContextBuilder.java:105-114: number → 0D.
    // Second output references first output name before first is written —
    // actually within same rule first is evaluated first. Use a *different*
    // output name that is never provided as input and not yet evaluated:
    // e.g. third clause references second which has empty expression (skip),
    // so second keeps default 0 from scope... Wait: empty skip does not put
    // into scope; default is in scope from the start.
    // Reference an output that has empty expression: scope default remains.
    let model = DmnModel::new(vec![DmnDecision::new(
        "default-decision",
        "outputDefaults",
        "Output defaults",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "x")],
        vec![
            DmnOutputClause::new("out-1", "bonus").with_type_ref("number"),
            DmnOutputClause::new("out-2", "total"),
        ],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![
                // Empty expression → skip; `bonus` stays at default 0 in scope
                DmnRuleOutputEntry::from_expression(""),
                DmnRuleOutputEntry::from_expression("bonus + x"),
            ],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "outputDefaults", json!({"x": 5})).expect("exec");
    // bonus skipped (not in result map)
    assert_eq!(result.get_output("bonus"), None);
    let total = result.get_output("total").expect("total");
    assert_eq!(total.as_f64(), Some(5.0), "got {total}");
}

/// Evaluation failure (unknown function) fails whole decision execution.
#[test]
fn evaluation_failure_unknown_function_fails_execution() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "fail-decision",
        "badFn",
        "Bad function",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "x")],
        vec![DmnOutputClause::new("out-1", "y")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::from_expression("notARealFunction(x)")],
        )],
    )]);
    let engine = deploy(model);
    let err = execute(&engine, "badFn", json!({"x": 1})).expect_err("should fail");
    assert!(matches!(err, DmnError::Execution { .. }), "{err}");
    assert!(
        err.to_string().contains("failed to evaluate output expression")
            || err.to_string().contains("unknown")
            || err.to_string().contains("notARealFunction")
            || err.to_string().contains("Unsupported")
            || err.to_string().contains("error"),
        "unexpected error: {err}"
    );
}

/// Division by zero fails whole execution (no literal fallback).
#[test]
fn evaluation_failure_division_by_zero_fails_execution() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "div-decision",
        "divZero",
        "Div zero",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "n")],
        vec![DmnOutputClause::new("out-1", "q")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::from_expression("n / 0")],
        )],
    )]);
    let engine = deploy(model);
    let err = execute(&engine, "divZero", json!({"n": 10})).expect_err("should fail");
    assert!(matches!(err, DmnError::Execution { .. }), "{err}");
    assert!(
        err.to_string().to_lowercase().contains("zero")
            || err.to_string().contains("failed to evaluate"),
        "unexpected error: {err}"
    );
}

/// Coerce order: expression yields string "42", typeRef double → success.
#[test]
fn coerce_after_eval_string_number_to_double() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "coerce-decision",
        "stringToDouble",
        "Coerce string number",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "x")],
        vec![DmnOutputClause::new("out-1", "amount").with_type_ref("double")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            // Quoted string expression evaluates to "42"; typeRef coerces to number.
            vec![DmnRuleOutputEntry::from_expression("\"42\"")],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "stringToDouble", json!({"x": 0})).expect("exec");
    // P88: typeRef double always stores JSON f64 (ExecutionVariableFactory.java:60-69).
    assert_eq!(result.get_output("amount"), Some(&json!(42.0)));
}

/// Incompatible type after eval still errors.
#[test]
fn coerce_after_eval_incompatible_type_errors() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "bad-coerce-decision",
        "badCoerce",
        "Bad coerce",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "x")],
        vec![DmnOutputClause::new("out-1", "flag").with_type_ref("boolean")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            // Non-literal so deploy-time typeRef coerce is skipped; runtime
            // evaluates to a string then fails boolean coerce.
            vec![DmnRuleOutputEntry::from_expression("upper case(\"no\")")],
        )],
    )]);
    let engine = deploy(model);
    let err = execute(&engine, "badCoerce", json!({"x": 0})).expect_err("should fail");
    assert!(matches!(err, DmnError::Execution { .. }), "{err}");
    assert!(
        err.to_string().contains("typeRef") || err.to_string().contains("incompatible"),
        "unexpected error: {err}"
    );
}

/// COLLECT + SUM over expression outputs.
#[test]
fn collect_sum_aggregates_expression_outputs() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "collect-decision",
        "collectExpr",
        "Collect expressions",
        DmnHitPolicy::Collect,
        vec![DmnInputClause::new("in-1", "qty")],
        vec![DmnOutputClause::new("out-1", "score").with_type_ref("number")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(json!(1)))],
                vec![DmnRuleOutputEntry::from_expression("qty * 2")],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(json!(1)))],
                vec![DmnRuleOutputEntry::from_expression("qty + 5")],
            ),
        ],
    )
    .with_collect_operator(CollectOperator::Sum)]);
    let engine = deploy(model);
    // qty=3 → rule1: 6, rule2: 8 → sum 14
    let result = execute(&engine, "collectExpr", json!({"qty": 3})).expect("exec");
    assert_eq!(result.decision_result.len(), 1);
    let score = result.get_output("score").expect("score");
    assert_eq!(score.as_f64(), Some(14.0), "got {score}");
    assert!(!result.multiple_results);
}

/// Audit conclusion_results hold evaluated values (shared with decision result).
#[test]
fn audit_conclusion_results_are_evaluated_values() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "audit-decision",
        "auditExpr",
        "Audit expressions",
        DmnHitPolicy::Unique,
        vec![DmnInputClause::new("in-1", "n")],
        vec![DmnOutputClause::new("out-1", "doubled")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any).with_id("cond-1")],
            vec![DmnRuleOutputEntry::from_expression("n * 2").with_id("conc-1")],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "auditExpr", json!({"n": 7})).expect("exec");
    assert_eq!(result.get_output("doubled"), Some(&json!(14)));
    assert_eq!(result.rule_executions.len(), 1);
    assert!(result.rule_executions[0].valid);
    assert_eq!(result.rule_executions[0].conclusion_results.len(), 1);
    assert_eq!(result.rule_executions[0].conclusion_results[0].id, "conc-1");
    assert_eq!(
        result.rule_executions[0].conclusion_results[0].result,
        json!(14)
    );
}

/// Empty output entry is skipped; audit records null.
#[test]
fn empty_output_entry_skipped_audit_null() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "empty-decision",
        "emptyOut",
        "Empty output",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "x")],
        vec![
            DmnOutputClause::new("out-1", "a"),
            DmnOutputClause::new("out-2", "b"),
        ],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![
                DmnRuleOutputEntry::from_expression("").with_id("empty-conc"),
                DmnRuleOutputEntry::from_expression("\"kept\"").with_id("kept-conc"),
            ],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "emptyOut", json!({"x": 1})).expect("exec");
    assert_eq!(result.get_output("a"), None);
    assert_eq!(result.get_output("b"), Some(&json!("kept")));
    assert_eq!(result.rule_executions[0].conclusion_results.len(), 2);
    assert_eq!(
        result.rule_executions[0].conclusion_results[0].result,
        Value::Null
    );
    assert_eq!(
        result.rule_executions[0].conclusion_results[1].result,
        json!("kept")
    );
}

/// `#{...}` / `${...}` shells are peeled before FEEL evaluation.
#[test]
fn expression_shells_are_peeled() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "shell-decision",
        "shellExpr",
        "Shell peel",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "price")],
        vec![
            DmnOutputClause::new("out-1", "hash"),
            DmnOutputClause::new("out-2", "dollar"),
        ],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![
                DmnRuleOutputEntry::from_expression("#{price * 2}"),
                DmnRuleOutputEntry::from_expression("${price + 1}"),
            ],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "shellExpr", json!({"price": 10})).expect("exec");
    assert_eq!(result.get_output("hash"), Some(&json!(20)));
    assert_eq!(result.get_output("dollar"), Some(&json!(11)));
}

/// Old deployment JSON without `expression` field still deserializes and runs.
#[test]
fn legacy_json_without_expression_field_deserializes() {
    // Simulate pre-P81 stored definition JSON (no expression field).
    let legacy = r#"{
        "id": null,
        "value": "legacy-route"
    }"#;
    let entry: DmnRuleOutputEntry =
        serde_json::from_str(legacy).expect("legacy JSON deserializes");
    assert_eq!(entry.expression, "");
    assert_eq!(entry.value, json!("legacy-route"));

    let model = DmnModel::new(vec![DmnDecision::new(
        "legacy-decision",
        "legacyOut",
        "Legacy",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("in-1", "x")],
        vec![DmnOutputClause::new("out-1", "route")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![entry],
        )],
    )]);
    let engine = deploy(model);
    let result = execute(&engine, "legacyOut", json!({"x": 1})).expect("exec");
    assert_eq!(result.get_output("route"), Some(&json!("legacy-route")));
}

/// DMN model path: expression text from XML-like LiteralExpression is preserved
/// and evaluated (not frozen at parse).
#[test]
fn dmn_model_literal_expression_evaluated_at_runtime() {
    let definition = DmnDefinition {
        id: Some("p81-def".to_string()),
        name: Some("p81-model".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "modelExpr".to_string(),
            name: Some("Model Expression".to_string()),
            decision_table: DecisionTable {
                id: "table-1".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("in-1".to_string()),
                    label: None,
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("in-expr".to_string()),
                        type_ref: Some("number".to_string()),
                        text: Some("amount".to_string()),
                    },
                }],
                outputs: vec![OutputClause {
                    id: Some("out-1".to_string()),
                    name: Some("total".to_string()),
                    label: None,
                    type_ref: Some("number".to_string()),
                    output_values: None,
                    output_number: 1,
                }],
                rules: vec![DecisionRule {
                    id: Some("rule-1".to_string()),
                    rule_number: 1,
                    input_entries: vec![UnaryTests {
                        id: Some("ie-1".to_string()),
                        text: Some("-".to_string()),
                    }],
                    output_entries: vec![LiteralExpression {
                        id: Some("oe-1".to_string()),
                        type_ref: None,
                        text: Some("amount * 3".to_string()),
                    }],
                }],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    };

    let model = DmnModel::try_from(definition).expect("parse");
    // Expression text preserved on the rule output entry.
    assert_eq!(
        model.decisions[0].rules[0].output_entries[0].expression,
        "amount * 3"
    );

    let engine = deploy(model);
    let result = execute(&engine, "modelExpr", json!({"amount": 4})).expect("exec");
    let total = result.get_output("total").expect("total");
    assert_eq!(total.as_f64(), Some(12.0), "got {total}");
}
