//! P87: bare leading / trailing decimal points in DMN unary tests (`.5`, `5.`).
//!
//! Java routes a number-typed input entry through the pre-parser, which inserts
//! the implicit `==` and hands `#{input == .5}` to JUEL
//! (`ELInputEntryExpressionPreParser.java:39-40,53-59`). The vendored JUEL
//! scanner accepts both shapes as FLOAT tokens: a `.` followed by a digit is not
//! a DOT token but the start of a number (`Scanner.java:390-394,429-430`), and a
//! trailing point with zero fraction digits is also a FLOAT (`:332-345`). So
//! both `.5` and `5.` are numeric literals in Java.
//!
//! Dialect notes:
//!   * Rust evaluates a FEEL subset, not JUEL (P81 deviation). FEEL proper does
//!     not allow a leading point, so this parity lives in the compat literal
//!     parser, not in the FEEL lexer.
//!   * For a non-number typeRef Java builds `#{input.5}`, a property access that
//!     effectively never matches (`ELInputEntryExpressionPreParser.java:42-46`).
//!     Rust parses `.5` as the number 0.5 regardless of typeRef. Under a string
//!     typeRef that still never matches, so the observable outcome agrees with
//!     Java; only an untyped input exposes the difference, where Rust matches a
//!     numeric 0.5. That is a harmless superset, asserted below.
//!   * `5.` normalizes to `5.0` and so inherits the engine's pre-existing
//!     int/float `Equals` identity (a JSON integer 5 does not equal 5.0). That
//!     predates P87 and is asserted against plain `5.0` for comparison.

use flowable_dmn_engine::{DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnModel};
use flowable_dmn_model::{
    Decision, DecisionRule, DecisionTable, DmnDefinition, HitPolicy, InputClause,
    LiteralExpression, OutputClause, UnaryTests,
};
use serde_json::{Value, json};

fn rule(rule_number: usize, input: &str, output: &str) -> DecisionRule {
    DecisionRule {
        id: Some(format!("rule{rule_number}")),
        rule_number,
        input_entries: vec![UnaryTests {
            id: Some(format!("inputEntry{rule_number}")),
            text: Some(input.to_string()),
        }],
        output_entries: vec![LiteralExpression {
            id: Some(format!("outputEntry{rule_number}")),
            type_ref: None,
            text: Some(output.to_string()),
        }],
    }
}

/// Single-input decision table over `variable` typed `type_ref`.
fn definition(variable: &str, type_ref: Option<&str>, input_tests: &[(&str, &str)]) -> DmnDefinition {
    DmnDefinition {
        id: Some("p87-defs".to_string()),
        name: Some("P87 Leading Decimal Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "p87Decision".to_string(),
            name: Some("P87 Decision".to_string()),
            decision_table: DecisionTable {
                id: "p87Table".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Input".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: type_ref.map(str::to_string),
                        text: Some(variable.to_string()),
                    },
                }],
                outputs: vec![OutputClause {
                    id: Some("output1".to_string()),
                    label: Some("Result".to_string()),
                    name: Some("result".to_string()),
                    type_ref: Some("string".to_string()),
                    output_values: None,
                    output_number: 1,
                }],
                rules: input_tests
                    .iter()
                    .enumerate()
                    .map(|(index, (input, output))| rule(index + 1, input, output))
                    .collect(),
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

fn deploy(definition: DmnDefinition) -> DmnEngine {
    let model = DmnModel::try_from(definition).expect("definition parses");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("p87").with_resource("p87.dmn", model))
        .expect("deployment");
    engine
}

fn deploy_err(definition: DmnDefinition) -> String {
    let model = DmnModel::try_from(definition).expect("definition parses");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("p87").with_resource("p87.dmn", model))
        .expect_err("deployment should fail")
        .to_string()
}

fn run(engine: &DmnEngine, variables: Value) -> Option<Value> {
    engine
        .decision_service()
        .execute_by_key("p87Decision", DmnExecutionRequest::new(variables))
        .expect("execution")
        .get_output("result")
        .cloned()
}

/// Number-typed table with a single rule, evaluated against `score`.
fn number_engine(input_entry: &str) -> DmnEngine {
    deploy(definition(
        "score",
        Some("number"),
        &[(input_entry, "\"hit\"")],
    ))
}

// ---------------------------------------------------------------------------
// Leading decimal point: `.5` is the number 0.5
// ---------------------------------------------------------------------------

#[test]
fn bare_leading_decimal_equality_matches_numeric_value() {
    let engine = number_engine(".5");

    assert_eq!(
        run(&engine, json!({ "score": 0.5 })),
        Some(json!("hit")),
        "`.5` is the implicit `== .5` of the Java pre-parser, matching 0.5"
    );
    assert_eq!(
        run(&engine, json!({ "score": 5 })),
        None,
        "`.5` must not be read as the integer 5"
    );
    assert_eq!(run(&engine, json!({ "score": 0.6 })), None);
}

#[test]
fn leading_decimal_less_than_compares_numerically() {
    let engine = number_engine("< .5");

    assert_eq!(run(&engine, json!({ "score": 0.25 })), Some(json!("hit")));
    assert_eq!(
        run(&engine, json!({ "score": 0.5 })),
        None,
        "strict `<` excludes the boundary"
    );
    assert_eq!(run(&engine, json!({ "score": 2 })), None);
}

#[test]
fn leading_decimal_greater_or_equal_includes_boundary() {
    let engine = number_engine(">= .5");

    assert_eq!(
        run(&engine, json!({ "score": 0.5 })),
        Some(json!("hit")),
        "`>=` includes the boundary, which requires a numeric operand"
    );
    assert_eq!(run(&engine, json!({ "score": 0.75 })), Some(json!("hit")));
    assert_eq!(run(&engine, json!({ "score": 0.49 })), None);
}

#[test]
fn leading_decimal_range_endpoint_matches() {
    let engine = number_engine("[.5..1]");

    assert_eq!(
        run(&engine, json!({ "score": 0.5 })),
        Some(json!("hit")),
        "an inclusive range start of `.5` must be the number 0.5"
    );
    assert_eq!(run(&engine, json!({ "score": 0.9 })), Some(json!("hit")));
    assert_eq!(run(&engine, json!({ "score": 1 })), Some(json!("hit")));
    assert_eq!(run(&engine, json!({ "score": 0.4 })), None);
    assert_eq!(run(&engine, json!({ "score": 1.5 })), None);
}

#[test]
fn leading_decimal_comma_list_matches_each_entry() {
    let engine = number_engine(".5, .7");

    assert_eq!(run(&engine, json!({ "score": 0.5 })), Some(json!("hit")));
    assert_eq!(run(&engine, json!({ "score": 0.7 })), Some(json!("hit")));
    assert_eq!(run(&engine, json!({ "score": 0.6 })), None);
}

#[test]
fn negative_leading_decimal_matches_numeric_value() {
    let engine = number_engine("-.5");

    assert_eq!(run(&engine, json!({ "score": -0.5 })), Some(json!("hit")));
    assert_eq!(run(&engine, json!({ "score": 0.5 })), None);
}

// ---------------------------------------------------------------------------
// Trailing decimal point: `5.` is the number 5.0
// ---------------------------------------------------------------------------

#[test]
fn trailing_decimal_point_equality_matches_numeric_value() {
    let engine = number_engine("5.");

    assert_eq!(
        run(&engine, json!({ "score": 5.0 })),
        Some(json!("hit")),
        "zero fraction digits after the point is still a FLOAT in JUEL"
    );
    assert_eq!(run(&engine, json!({ "score": 5.5 })), None);
    assert_eq!(run(&engine, json!({ "score": 0.5 })), None);

    // `5.` normalizes to `5.0`, so it inherits the engine's pre-existing
    // int/float `Equals` identity: a JSON integer 5 does not equal 5.0. Plain
    // `5.0` behaves the same way, so this is not specific to the trailing dot.
    assert_eq!(run(&engine, json!({ "score": 5 })), None);
    assert_eq!(
        run(&number_engine("5.0"), json!({ "score": 5 })),
        None,
        "`5.` matches whatever the already-supported `5.0` matches"
    );
}

#[test]
fn trailing_decimal_point_comparison_compares_numerically() {
    let engine = number_engine(">= 5.");

    assert_eq!(run(&engine, json!({ "score": 5 })), Some(json!("hit")));
    assert_eq!(run(&engine, json!({ "score": 7.5 })), Some(json!("hit")));
    assert_eq!(run(&engine, json!({ "score": 4.9 })), None);
}

// ---------------------------------------------------------------------------
// Negative cases: the normalization must not widen into a float parser
// ---------------------------------------------------------------------------

#[test]
fn non_numeric_float_spellings_are_not_treated_as_numbers() {
    // `parse::<f64>()` accepts every one of these, which is exactly why the
    // normalization is shape-only. Under a number typeRef they stay strings, and
    // a string operand against a numeric input is rejected at deploy time.
    for entry in ["nan", "inf", "infinity", "-inf", "NaN"] {
        let message = deploy_err(definition("score", Some("number"), &[(entry, "\"hit\"")]));
        assert!(
            message.contains("unsupported unary test"),
            "'{entry}' must not become a float; got: {message}"
        );
    }
}

#[test]
fn non_numeric_float_spellings_keep_string_equality() {
    for entry in ["nan", "inf", "infinity", "NaN"] {
        let engine = deploy(definition("label", Some("string"), &[(entry, "\"hit\"")]));

        assert_eq!(
            run(&engine, json!({ "label": entry })),
            Some(json!("hit")),
            "'{entry}' keeps plain string equality semantics"
        );
        assert_eq!(run(&engine, json!({ "label": "other" })), None);
    }
}

#[test]
fn near_miss_decimal_shapes_stay_strings() {
    // None of these is a number shape, so none may be padded into one. Each is
    // matched as the literal string it is. (`5..` / `..` are excluded: `..` is
    // the range delimiter and is claimed by the range parser well before the
    // literal path, which predates P87.)
    for entry in [".", ".5x", "5.5.5", "5.x", "-.x"] {
        let engine = deploy(definition("label", Some("string"), &[(entry, "\"hit\"")]));

        assert_eq!(
            run(&engine, json!({ "label": entry })),
            Some(json!("hit")),
            "'{entry}' is a string literal, not a number"
        );
    }
}

#[test]
fn leading_plus_decimal_is_not_a_number() {
    // JSON rejects a leading `+`, matching how this parser already declines the
    // plain `+5`, so `+.5` is deliberately left alone rather than padded.
    let message = deploy_err(definition("score", Some("number"), &[("+.5", "\"hit\"")]));
    assert!(
        message.contains("unsupported unary test"),
        "`+.5` follows `+5`, which is not a supported numeric literal; got: {message}"
    );
}

#[test]
fn string_typed_leading_decimal_never_matches_like_java() {
    // Java builds `#{input.5}` for a non-number typeRef — a property access that
    // effectively never matches. Rust parses `.5` as the number 0.5, which under
    // a string typeRef also never matches, so the outcome agrees.
    let engine = deploy(definition("label", Some("string"), &[(".5", "\"hit\"")]));

    assert_eq!(run(&engine, json!({ "label": "abc" })), None);
    assert_eq!(run(&engine, json!({ "label": "0.5" })), None);
    assert_eq!(run(&engine, json!({ "label": ".5" })), None);
}

#[test]
fn untyped_leading_decimal_matches_numeric_input() {
    // The one place Rust is observably wider than Java: with no typeRef there is
    // no coercion, so the numeric literal 0.5 matches a numeric input directly.
    let engine = deploy(definition("score", None, &[(".5", "\"hit\"")]));

    assert_eq!(run(&engine, json!({ "score": 0.5 })), Some(json!("hit")));
    assert_eq!(run(&engine, json!({ "score": "x" })), None);
}
