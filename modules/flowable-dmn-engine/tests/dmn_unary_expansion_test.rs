//! M77 FEEL unary expansion: nested `not`, `not` around comparisons/ranges,
//! and `list contains(?, <variable>)` variable-ref needles.

use flowable_dmn_engine::{
    DmnDeploymentRequest, DmnEngine, DmnError, DmnExecutionRequest, DmnListContainsNeedle,
    DmnModel, DmnUnaryTest,
};
use flowable_dmn_model::{
    Decision, DecisionRule, DecisionTable, DmnDefinition, HitPolicy, InputClause,
    LiteralExpression, OutputClause, UnaryTests,
};
use serde_json::{Map, Value, json};

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

fn multi_input_rule(rule_number: usize, inputs: &[&str], output: &str) -> DecisionRule {
    DecisionRule {
        id: Some(format!("rule{rule_number}")),
        rule_number,
        input_entries: inputs
            .iter()
            .enumerate()
            .map(|(index, input)| UnaryTests {
                id: Some(format!("inputEntry{rule_number}_{index}")),
                text: Some((*input).to_string()),
            })
            .collect(),
        output_entries: vec![LiteralExpression {
            id: Some(format!("outputEntry{rule_number}")),
            type_ref: None,
            text: Some(output.to_string()),
        }],
    }
}

fn numeric_definition(input_tests: &[(&str, &str)]) -> DmnDefinition {
    DmnDefinition {
        id: Some("expansion-numeric-defs".to_string()),
        name: Some("Expansion Numeric Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "scoreDecision".to_string(),
            name: Some("Score Decision".to_string()),
            decision_table: DecisionTable {
                id: "scoreTable".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Score".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: Some("number".to_string()),
                        text: Some("score".to_string()),
                    },
                }],
                outputs: vec![OutputClause {
                    id: Some("output1".to_string()),
                    label: Some("Band".to_string()),
                    name: Some("band".to_string()),
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

fn list_contains_variable_definition() -> DmnDefinition {
    DmnDefinition {
        id: Some("list-contains-var-defs".to_string()),
        name: Some("List Contains Variable Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "tagsDecision".to_string(),
            name: Some("Tags Decision".to_string()),
            decision_table: DecisionTable {
                id: "tagsTable".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![
                    InputClause {
                        id: Some("input1".to_string()),
                        label: Some("Tags".to_string()),
                        input_number: 1,
                        input_expression: LiteralExpression {
                            id: Some("inputExpression1".to_string()),
                            type_ref: Some("list".to_string()),
                            text: Some("tags".to_string()),
                        },
                    },
                    InputClause {
                        id: Some("input2".to_string()),
                        label: Some("Required Tag".to_string()),
                        input_number: 2,
                        input_expression: LiteralExpression {
                            id: Some("inputExpression2".to_string()),
                            type_ref: Some("string".to_string()),
                            text: Some("requiredTag".to_string()),
                        },
                    },
                ],
                outputs: vec![OutputClause {
                    id: Some("output1".to_string()),
                    label: Some("Result".to_string()),
                    name: Some("result".to_string()),
                    type_ref: Some("string".to_string()),
                    output_values: None,
                    output_number: 1,
                }],
                rules: vec![
                    multi_input_rule(1, &["list contains(?, requiredTag)", "-"], "'matched'"),
                    multi_input_rule(2, &["-", "-"], "'default'"),
                ],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

fn deploy(key: &str, definition: DmnDefinition) -> DmnEngine {
    let model = DmnModel::try_from(definition).expect("definition parses");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new(key).with_resource(format!("{key}.dmn"), model))
        .expect("deployment");
    engine
}

fn execute_score(engine: &DmnEngine, score: Value) -> Value {
    engine
        .decision_service()
        .execute_by_key(
            "scoreDecision",
            DmnExecutionRequest::new(json!({ "score": score })),
        )
        .expect("execution")
        .get_output("band").cloned().unwrap()
}

fn execute_tags(engine: &DmnEngine, tags: Value, required_tag: Value) -> Value {
    engine
        .decision_service()
        .execute_by_key(
            "tagsDecision",
            DmnExecutionRequest::new(json!({
                "tags": tags,
                "requiredTag": required_tag,
            })),
        )
        .expect("execution")
        .get_output("result").cloned().unwrap()
}

/// Nested double-negation over a comparison: `not(not(> 5))` ≡ `> 5`.
#[test]
fn deploys_and_evaluates_nested_not_around_comparison() {
    let engine = deploy(
        "nested-not-comparison",
        numeric_definition(&[("not(not(> 5))", "'high'"), ("-", "'low'")]),
    );

    assert_eq!(execute_score(&engine, json!(6)), json!("high"));
    assert_eq!(execute_score(&engine, json!(5)), json!("low"));
    assert_eq!(execute_score(&engine, json!(0)), json!("low"));
}

/// Nested double-negation over a closed range: `not(not([1..5]))` ≡ `[1..5]`.
#[test]
fn deploys_and_evaluates_nested_not_around_range() {
    let engine = deploy(
        "nested-not-range",
        numeric_definition(&[("not(not([1..5]))", "'inside'"), ("-", "'outside'")]),
    );

    assert_eq!(execute_score(&engine, json!(1)), json!("inside"));
    assert_eq!(execute_score(&engine, json!(3)), json!("inside"));
    assert_eq!(execute_score(&engine, json!(5)), json!("inside"));
    assert_eq!(execute_score(&engine, json!(0)), json!("outside"));
    assert_eq!(execute_score(&engine, json!(6)), json!("outside"));
}

/// Single `not` around a comparison already supported by M15 — keep deploy+execute coverage.
#[test]
fn deploys_and_evaluates_not_around_comparison() {
    let engine = deploy(
        "not-comparison",
        numeric_definition(&[("not(> 90)", "'not-excellent'"), ("-", "'excellent'")]),
    );

    assert_eq!(execute_score(&engine, json!(90)), json!("not-excellent"));
    assert_eq!(execute_score(&engine, json!(50)), json!("not-excellent"));
    assert_eq!(execute_score(&engine, json!(91)), json!("excellent"));
}

/// `list contains(?, requiredTag)` resolves the unquoted identifier from inputs.
#[test]
fn deploys_and_evaluates_list_contains_with_variable_needle() {
    let engine = deploy(
        "list-contains-variable",
        list_contains_variable_definition(),
    );

    assert_eq!(
        execute_tags(&engine, json!(["urgent", "bug"]), json!("urgent")),
        json!("matched")
    );
    assert_eq!(
        execute_tags(&engine, json!(["urgent", "bug"]), json!("review")),
        json!("default")
    );
    assert_eq!(
        execute_tags(&engine, json!([]), json!("urgent")),
        json!("default")
    );
    assert_eq!(
        execute_tags(&engine, json!(["review"]), json!("review")),
        json!("matched")
    );
}

/// `not(list contains(?, requiredTag))` combines nesting with variable resolution.
#[test]
fn deploys_and_evaluates_not_list_contains_with_variable_needle() {
    let mut definition = list_contains_variable_definition();
    definition.decisions[0].decision_table.rules = vec![
        multi_input_rule(1, &["not(list contains(?, requiredTag))", "-"], "'missing'"),
        multi_input_rule(2, &["-", "-"], "'present'"),
    ];

    let engine = deploy("not-list-contains-variable", definition);

    assert_eq!(
        execute_tags(&engine, json!(["urgent", "bug"]), json!("review")),
        json!("missing")
    );
    assert_eq!(
        execute_tags(&engine, json!(["urgent", "bug"]), json!("urgent")),
        json!("present")
    );
    assert_eq!(
        execute_tags(&engine, json!([]), json!("urgent")),
        json!("missing")
    );
}

/// Missing variable needle resolves to null; list does not contain null unless present.
#[test]
fn list_contains_variable_missing_input_resolves_to_null() {
    let engine = deploy(
        "list-contains-missing-var",
        list_contains_variable_definition(),
    );

    let mut inputs = Map::new();
    inputs.insert("tags".to_string(), json!([null, "x"]));
    // requiredTag intentionally omitted → Null

    let result = engine
        .decision_service()
        .execute_by_key(
            "tagsDecision",
            DmnExecutionRequest::new(Value::Object(inputs)),
        )
        .expect("execution");
    assert_eq!(result.get_output("result"), Some(&json!("matched")));

    let mut inputs = Map::new();
    inputs.insert("tags".to_string(), json!(["x"]));
    let result = engine
        .decision_service()
        .execute_by_key(
            "tagsDecision",
            DmnExecutionRequest::new(Value::Object(inputs)),
        )
        .expect("execution");
    assert_eq!(result.get_output("result"), Some(&json!("default")));
}

#[test]
fn parses_list_contains_literal_and_variable_needles() {
    let literal = DmnModel::try_from({
        let mut def = list_contains_variable_definition();
        def.decisions[0].decision_table.rules = vec![multi_input_rule(
            1,
            &["list contains(?, \"urgent\")", "-"],
            "'ok'",
        )];
        def
    })
    .expect("literal list contains parses");
    assert!(matches!(
        &literal.decisions[0].rules[0].input_entries[0].expression,
        DmnUnaryTest::ListContains {
            needle: DmnListContainsNeedle::Literal(Value::String(s))
        } if s == "urgent"
    ));

    let variable = DmnModel::try_from(list_contains_variable_definition())
        .expect("variable list contains parses");
    assert!(matches!(
        &variable.decisions[0].rules[0].input_entries[0].expression,
        DmnUnaryTest::ListContains {
            needle: DmnListContainsNeedle::Variable(name)
        } if name == "requiredTag"
    ));
}

#[test]
fn rejects_unsupported_list_contains_forms_as_structured_error() {
    for input_test in [
        "list contains(tags, requiredTag)",
        "list contains(?, foo.bar)",
        "list contains(?, requiredTag, extra)",
        "list contains(?)",
    ] {
        let mut definition = list_contains_variable_definition();
        definition.decisions[0].decision_table.rules =
            vec![multi_input_rule(1, &[input_test, "-"], "'ok'")];

        let error =
            DmnModel::try_from(definition).expect_err("unsupported list contains form should fail");
        assert!(
            matches!(error, DmnError::UnsupportedModel { .. }),
            "expected UnsupportedModel for {input_test}, got {error}"
        );
        assert!(
            error
                .to_string()
                .contains("unsupported list contains unary test"),
            "unexpected error for {input_test}: {error}"
        );
    }
}
