use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnError, DmnExecutionRequest, DmnHitPolicy,
    DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry,
    DmnStringFunction, DmnUnaryTest,
};
use flowable_dmn_model::{
    Decision, DecisionRule, DecisionTable, DmnDefinition, HitPolicy, InputClause,
    LiteralExpression, OutputClause, UnaryTests,
};
use serde_json::json;

fn numeric_unary_test_definition() -> DmnDefinition {
    DmnDefinition {
        id: Some("score-defs".to_string()),
        name: Some("Score Decisions".to_string()),
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
                rules: vec![
                    rule(1, ">= 90", "'excellent'"),
                    rule(2, "> 70", "'passing'"),
                    rule(3, "= 50", "'exact'"),
                    rule(4, "< 0", "'negative'"),
                    rule(5, "<= 10", "'low'"),
                    rule(6, "-", "'default'"),
                ],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

fn duration_unary_test_definition(type_ref: &str, input_test: &str) -> DmnDefinition {
    DmnDefinition {
        id: Some("duration-defs".to_string()),
        name: Some("Duration Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "durationDecision".to_string(),
            name: Some("Duration Decision".to_string()),
            decision_table: DecisionTable {
                id: "durationTable".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Value".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: Some(type_ref.to_string()),
                        text: Some("value".to_string()),
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
                rules: vec![rule(1, input_test, "'matched'"), rule(2, "-", "'default'")],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

fn temporal_unary_test_definition(type_ref: &str, input_test: &str) -> DmnDefinition {
    DmnDefinition {
        id: Some("temporal-defs".to_string()),
        name: Some("Temporal Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "temporalDecision".to_string(),
            name: Some("Temporal Decision".to_string()),
            decision_table: DecisionTable {
                id: "temporalTable".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Value".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: Some(type_ref.to_string()),
                        text: Some("value".to_string()),
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
                rules: vec![rule(1, input_test, "'matched'"), rule(2, "-", "'default'")],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

fn string_unary_test_definition() -> DmnDefinition {
    DmnDefinition {
        id: Some("string-defs".to_string()),
        name: Some("String Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "stringDecision".to_string(),
            name: Some("String Decision".to_string()),
            decision_table: DecisionTable {
                id: "stringTable".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Value".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: None,
                        text: Some("value".to_string()),
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
                rules: vec![
                    rule(1, "contains(?, \"vip\")", "'contains'"),
                    rule(2, "starts with(?, \"pre\")", "'starts'"),
                    rule(3, "ends with(?, \"suffix\")", "'ends'"),
                    rule(4, "lower case(?) = \"approved\"", "'lower'"),
                    rule(5, "upper case(?) = \"PENDING\"", "'upper'"),
                    rule(6, "-", "'default'"),
                ],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

fn string_literal_unary_test_definition() -> DmnDefinition {
    let mut definition = string_unary_test_definition();
    definition.decisions[0].decision_table.rules = vec![
        rule(1, "= \"exact\"", "'exact'"),
        rule(2, "!= \"blocked\"", "'not-blocked'"),
        rule(3, "-", "'default'"),
    ];
    definition
}

fn string_regex_unary_test_definition() -> DmnDefinition {
    let mut definition = string_unary_test_definition();
    definition.decisions[0].decision_table.rules = vec![
        rule(1, "matches(?, \"^[A-Z]{3}-[0-9]{2}$\")", "'regex'"),
        rule(2, "-", "'default'"),
    ];
    definition
}

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

fn execute_score(engine: &DmnEngine, score: serde_json::Value) -> serde_json::Value {
    let result = engine
        .decision_service()
        .execute_by_key(
            "scoreDecision",
            DmnExecutionRequest::new(json!({
                "score": score
            })),
        )
        .expect("execution");

    result.get_output("band").cloned().unwrap()
}

fn execute_duration(engine: &DmnEngine, value: serde_json::Value) -> serde_json::Value {
    let result = engine
        .decision_service()
        .execute_by_key(
            "durationDecision",
            DmnExecutionRequest::new(json!({
                "value": value
            })),
        )
        .expect("execution");

    result.get_output("result").cloned().unwrap()
}

fn execute_temporal(engine: &DmnEngine, value: serde_json::Value) -> serde_json::Value {
    let result = engine
        .decision_service()
        .execute_by_key(
            "temporalDecision",
            DmnExecutionRequest::new(json!({
                "value": value
            })),
        )
        .expect("execution");

    result.get_output("result").cloned().unwrap()
}

fn execute_string(engine: &DmnEngine, value: serde_json::Value) -> serde_json::Value {
    let result = engine
        .decision_service()
        .execute_by_key(
            "stringDecision",
            DmnExecutionRequest::new(json!({
                "value": value
            })),
        )
        .expect("execution");

    result.get_output("result").cloned().unwrap()
}

#[test]
fn evaluates_supported_numeric_comparison_unary_tests() {
    let model =
        DmnModel::try_from(numeric_unary_test_definition()).expect("numeric unary tests parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("numeric-unary-tests")
                .with_resource("score-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_score(&engine, json!(90)), json!("excellent"));
    assert_eq!(execute_score(&engine, json!(71)), json!("passing"));
    assert_eq!(execute_score(&engine, json!(50)), json!("exact"));
    assert_eq!(execute_score(&engine, json!(-1)), json!("negative"));
    assert_eq!(execute_score(&engine, json!(10)), json!("low"));
    assert_eq!(execute_score(&engine, json!(42)), json!("default"));
}

#[test]
fn evaluates_feel_numeric_range_unary_tests_with_open_and_closed_boundaries() {
    let mut definition = numeric_unary_test_definition();
    let rules = &mut definition.decisions[0].decision_table.rules;
    rules[0].input_entries[0].text = Some("[1..10]".to_string());
    rules[0].output_entries[0].text = Some("'closed'".to_string());
    rules[1].input_entries[0].text = Some("(10..20]".to_string());
    rules[1].output_entries[0].text = Some("'open-left'".to_string());
    rules[2].input_entries[0].text = Some("[20..30)".to_string());
    rules[2].output_entries[0].text = Some("'open-right'".to_string());
    rules[3].input_entries[0].text = Some("(30..40)".to_string());
    rules[3].output_entries[0].text = Some("'open-both'".to_string());
    rules[4].input_entries[0].text = Some("-".to_string());
    rules[4].output_entries[0].text = Some("'default'".to_string());
    rules.truncate(5);

    let model = DmnModel::try_from(definition).expect("range unary tests parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("range-unary-tests")
                .with_resource("score-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_score(&engine, json!(1)), json!("closed"));
    assert_eq!(execute_score(&engine, json!(10)), json!("closed"));
    assert_eq!(execute_score(&engine, json!(10.5)), json!("open-left"));
    assert_eq!(execute_score(&engine, json!(20)), json!("open-left"));
    assert_eq!(execute_score(&engine, json!(25)), json!("open-right"));
    assert_eq!(execute_score(&engine, json!(30)), json!("default"));
    assert_eq!(execute_score(&engine, json!(35)), json!("open-both"));
    assert_eq!(execute_score(&engine, json!(40)), json!("default"));
}

#[test]
fn evaluates_comma_separated_range_literal_and_comparison_as_or() {
    let mut definition = numeric_unary_test_definition();
    let rules = &mut definition.decisions[0].decision_table.rules;
    rules[0].input_entries[0].text = Some("[1..3], 7, > 10".to_string());
    rules[0].output_entries[0].text = Some("'matched'".to_string());
    rules[1].input_entries[0].text = Some("-".to_string());
    rules[1].output_entries[0].text = Some("'default'".to_string());
    rules.truncate(2);

    let model = DmnModel::try_from(definition).expect("combined unary tests parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("combined-unary-tests")
                .with_resource("score-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_score(&engine, json!(2)), json!("matched"));
    assert_eq!(execute_score(&engine, json!(7)), json!("matched"));
    assert_eq!(execute_score(&engine, json!(11)), json!("matched"));
    assert_eq!(execute_score(&engine, json!(5)), json!("default"));
}

#[test]
fn reports_malformed_ranges_as_structured_validation_errors() {
    let mut definition = numeric_unary_test_definition();
    definition.decisions[0].decision_table.rules[0].input_entries[0].text =
        Some("[..10]".to_string());

    let error = DmnModel::try_from(definition).expect_err("empty range endpoint is malformed");
    assert!(matches!(error, DmnError::Validation { .. }));
    assert!(
        error.to_string().contains("malformed unary range '[..10]'"),
        "unexpected error: {error}"
    );
}

#[test]
fn evaluates_string_literal_equality_and_inequality_unary_tests() {
    let model =
        DmnModel::try_from(string_literal_unary_test_definition()).expect("literal tests parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("string-literal-unary-tests")
                .with_resource("string-literal-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_string(&engine, json!("exact")), json!("exact"));
    assert_eq!(
        execute_string(&engine, json!("allowed")),
        json!("not-blocked")
    );
    assert_eq!(execute_string(&engine, json!("blocked")), json!("default"));
}

#[test]
fn evaluates_not_wrapped_literal_unary_test() {
    let mut definition = string_literal_unary_test_definition();
    definition.decisions[0].decision_table.rules = vec![
        rule(1, "not(\"blocked\")", "'allowed'"),
        rule(2, "-", "'default'"),
    ];

    let model = DmnModel::try_from(definition).expect("not literal unary test should parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("not-literal-unary-tests")
                .with_resource("string-literal-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_string(&engine, json!("allowed")), json!("allowed"));
    assert_eq!(execute_string(&engine, json!("blocked")), json!("default"));
}

#[test]
fn evaluates_string_function_unary_tests_against_current_input() {
    let model =
        DmnModel::try_from(string_unary_test_definition()).expect("string unary tests parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("string-unary-tests")
                .with_resource("string-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(
        execute_string(&engine, json!("gold-vip-tier")),
        json!("contains")
    );
    assert_eq!(
        execute_string(&engine, json!("pre-approved")),
        json!("starts")
    );
    assert_eq!(execute_string(&engine, json!("has-suffix")), json!("ends"));
    assert_eq!(
        execute_string(&engine, json!("suffix-free")),
        json!("default")
    );
    assert_eq!(execute_string(&engine, json!("Approved")), json!("lower"));
    assert_eq!(execute_string(&engine, json!("pending")), json!("upper"));
    assert_eq!(execute_string(&engine, json!("ordinary")), json!("default"));
    assert_eq!(execute_string(&engine, json!(42)), json!("default"));
}

#[test]
fn evaluates_matches_unary_test_against_current_string_input() {
    let model =
        DmnModel::try_from(string_regex_unary_test_definition()).expect("regex unary test parses");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("string-regex-unary-tests")
                .with_resource("string-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_string(&engine, json!("ABC-12")), json!("regex"));
    assert_eq!(execute_string(&engine, json!("abc-12")), json!("default"));
    assert_eq!(execute_string(&engine, json!(42)), json!("default"));
}

#[test]
fn evaluates_string_length_transform_comparison_unary_tests() {
    let mut definition = string_unary_test_definition();
    definition.decisions[0].decision_table.rules = vec![
        rule(1, "string length(?) > 8", "'long'"),
        rule(2, "string length(?) <= 3", "'short'"),
        rule(3, "-", "'default'"),
    ];

    let model = DmnModel::try_from(definition).expect("string length unary tests should parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("string-length-unary-tests")
                .with_resource("string-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_string(&engine, json!("enterprise")), json!("long"));
    assert_eq!(execute_string(&engine, json!("abc")), json!("short"));
    assert_eq!(execute_string(&engine, json!("medium")), json!("default"));
    assert_eq!(execute_string(&engine, json!(42)), json!("default"));
}

#[test]
fn evaluates_not_wrapped_string_function_unary_test() {
    let mut definition = string_unary_test_definition();
    definition.decisions[0].decision_table.rules = vec![
        rule(1, "not(contains(?, \"vip\"))", "'not-vip'"),
        rule(2, "-", "'default'"),
    ];

    let model =
        DmnModel::try_from(definition).expect("not string function unary test should parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("not-string-function-unary-tests")
                .with_resource("string-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_string(&engine, json!("ordinary")), json!("not-vip"));
    assert_eq!(
        execute_string(&engine, json!("gold-vip-tier")),
        json!("default")
    );
    assert_eq!(execute_string(&engine, json!(42)), json!("not-vip"));
}

#[test]
fn evaluates_not_wrapped_matches_unary_test() {
    let mut definition = string_regex_unary_test_definition();
    definition.decisions[0].decision_table.rules = vec![
        rule(1, "not(matches(?, \"^[A-Z]{3}-[0-9]{2}$\"))", "'not-regex'"),
        rule(2, "-", "'default'"),
    ];

    let model = DmnModel::try_from(definition).expect("not regex unary test should parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("not-regex-unary-tests")
                .with_resource("string-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_string(&engine, json!("abc-12")), json!("not-regex"));
    assert_eq!(execute_string(&engine, json!("ABC-12")), json!("default"));
    assert_eq!(execute_string(&engine, json!(42)), json!("not-regex"));
}

#[test]
fn evaluates_not_wrapped_range_unary_test_boundaries() {
    let mut definition = numeric_unary_test_definition();
    let rules = &mut definition.decisions[0].decision_table.rules;
    rules[0].input_entries[0].text = Some("not([1..5])".to_string());
    rules[0].output_entries[0].text = Some("'outside'".to_string());
    rules[1].input_entries[0].text = Some("-".to_string());
    rules[1].output_entries[0].text = Some("'inside'".to_string());
    rules.truncate(2);

    let model = DmnModel::try_from(definition).expect("not range unary test should parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("not-range-unary-tests")
                .with_resource("score-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_score(&engine, json!(0)), json!("outside"));
    assert_eq!(execute_score(&engine, json!(1)), json!("inside"));
    assert_eq!(execute_score(&engine, json!(5)), json!("inside"));
    assert_eq!(execute_score(&engine, json!(6)), json!("outside"));
}

#[test]
#[allow(clippy::single_element_loop)]
fn rejects_not_wrapping_more_than_one_or_unsupported_unary_test() {
    for input_test in ["not(score > 10)"] {
        let mut definition = string_unary_test_definition();
        definition.decisions[0].decision_table.rules[0].input_entries[0].text =
            Some(input_test.to_string());

        let error = DmnModel::try_from(definition)
            .expect_err("unsupported not unary test should be rejected");
        assert!(matches!(error, DmnError::UnsupportedModel { .. }));
        assert!(
            error.to_string().contains("unsupported unary test"),
            "unexpected error for {input_test}: {error}"
        );
    }
}

#[test]
fn evaluates_complex_nested_not_unary_tests() {
    let mut definition = string_unary_test_definition();
    definition.decisions[0].decision_table.rules = vec![
        rule(1, "not(not(\"vip\"))", "'vip-only'"),
        rule(2, "not(\"blocked\", \"closed\")", "'allowed'"),
        rule(3, "-", "'default'"),
    ];

    let model = DmnModel::try_from(definition).expect("complex not unary test should parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("complex-not").with_resource("string-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_string(&engine, json!("allowed")), json!("allowed"));
    assert_eq!(execute_string(&engine, json!("blocked")), json!("default"));
    assert_eq!(execute_string(&engine, json!("closed")), json!("default"));
    assert_eq!(execute_string(&engine, json!("vip")), json!("vip-only"));
    assert_eq!(execute_string(&engine, json!("ordinary")), json!("allowed"));
}

#[test]
fn rejects_string_unary_tests_with_complex_arguments() {
    for input_test in [
        "contains(value, \"vip\")",
        "contains(?, concat(\"v\", \"ip\"))",
        "matches(value, \"^[A-Z]+$\")",
        "matches(?, concat(\"^\", \"A\"))",
        "ends with(value, \"suffix\")",
        "ends with(?, concat(\"suf\", \"fix\"))",
        "lower case(value) = \"approved\"",
        "lower case(?) = concat(\"app\", \"roved\")",
        "upper case(value) = \"PENDING\"",
        "upper case(?) = concat(\"PEND\", \"ING\")",
        "string length(value) > 8",
        "string length(?) > \"8\"",
        "string length(?) = 8",
        "string length(?) > count(\"abc\")",
    ] {
        let mut definition = string_unary_test_definition();
        definition.decisions[0].decision_table.rules[0].input_entries[0].text =
            Some(input_test.to_string());

        let error = DmnModel::try_from(definition)
            .expect_err("complex string function unary test should be rejected");
        assert!(matches!(error, DmnError::UnsupportedModel { .. }));
        let error = error.to_string();
        assert!(
            error.contains("unsupported string function unary test")
                || error.contains("unsupported string transform unary test"),
            "unexpected error for {input_test}: {error}"
        );
    }
}

#[test]
fn reports_invalid_matches_regex_as_structured_validation_error() {
    let mut definition = string_regex_unary_test_definition();
    definition.decisions[0].decision_table.rules[0].input_entries[0].text =
        Some("matches(?, \"[unterminated\")".to_string());

    let error = DmnModel::try_from(definition).expect_err("invalid regex should fail parsing");
    assert!(matches!(error, DmnError::Validation { .. }));
    assert!(
        error
            .to_string()
            .contains("invalid matches regex '[unterminated'"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_programmatic_invalid_matches_regex_at_deployment() {
    let model = DmnModel::new(vec![DmnDecision::new(
        "regexDecision",
        "regexDecision",
        "Regex Decision",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input1", "value")],
        vec![DmnOutputClause::new("output1", "result")],
        vec![DmnRule::new(
            "rule1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::StringFunction {
                function: DmnStringFunction::Matches,
                needle: "[unterminated".to_string(),
            })],
            vec![DmnRuleOutputEntry::new(json!("matched"))],
        )],
    )]);
    let engine = DmnEngine::new_in_memory().expect("engine");

    let error = engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("invalid-programmatic-regex")
                .with_resource("regex-decision.dmn", model),
        )
        .expect_err("invalid regex should fail deployment");

    assert!(matches!(error, DmnError::Validation { .. }));
    assert!(
        error
            .to_string()
            .contains("invalid matches regex '[unterminated'"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_string_function_unary_tests_for_numeric_type_refs_at_deployment() {
    let mut definition = string_unary_test_definition();
    definition.decisions[0].decision_table.inputs[0]
        .input_expression
        .type_ref = Some("number".to_string());
    definition.decisions[0].decision_table.rules[0].input_entries[0].text =
        Some("ends with(?, \"suffix\")".to_string());

    let model = DmnModel::try_from(definition).expect("string function unary test parses");
    let engine = DmnEngine::new_in_memory().expect("engine");
    let error = engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("numeric-string-function-unary-tests")
                .with_resource("string-decision.dmn", model),
        )
        .expect_err("string function should be rejected for numeric typeRef");

    assert!(matches!(error, DmnError::UnsupportedModel { .. }));
    assert!(
        error
            .to_string()
            .contains("unsupported unary test 'ends with(?, 'suffix')'"),
        "unexpected error: {error}"
    );
}

#[test]
fn evaluates_duration_constructor_comparison_unary_tests_by_declared_type() {
    for (type_ref, input_test, matching_value) in [
        ("duration", "> duration(\"PT1H\")", json!("PT90M")),
        ("yearMonthDuration", ">= duration('P1Y')", json!("P12M")),
    ] {
        let model = DmnModel::try_from(duration_unary_test_definition(type_ref, input_test))
            .expect("duration unary tests parse");
        let engine = DmnEngine::new_in_memory().expect("engine");
        engine
            .repository_service()
            .deploy(
                DmnDeploymentRequest::new("duration-unary-tests")
                    .with_resource("duration-decision.dmn", model),
            )
            .expect("deployment");

        assert_eq!(
            execute_duration(&engine, matching_value),
            json!("matched"),
            "typeRef {type_ref}"
        );
    }
}

#[test]
fn evaluates_duration_constructor_comma_separated_unary_tests_as_or() {
    let model = DmnModel::try_from(duration_unary_test_definition(
        "duration",
        "duration(\"PT1H\"), duration('PT2H')",
    ))
    .expect("duration constructor unary tests parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("duration-unary-tests")
                .with_resource("duration-decision.dmn", model),
        )
        .expect("deployment");

    assert_eq!(execute_duration(&engine, json!("PT2H")), json!("matched"));
    assert_eq!(execute_duration(&engine, json!("PT3H")), json!("default"));
}

#[test]
fn evaluates_temporal_constructor_equality_unary_tests_by_declared_type() {
    for (type_ref, input_test, matching_value, default_value) in [
        (
            "date",
            "date(\"2024-01-31\")",
            json!("2024-01-31"),
            json!("2024-02-01"),
        ),
        (
            "time",
            "time('13:45:00')",
            json!("13:45:00"),
            json!("13:46:00"),
        ),
        (
            "dateTime",
            "date and time(\"2024-01-31T13:45:00\")",
            json!("2024-01-31T13:45:00"),
            json!("2024-01-31T13:46:00"),
        ),
        (
            "dateTime",
            "dateTime('2024-01-31T13:45:00Z')",
            json!("2024-01-31T13:45:00Z"),
            json!("2024-01-31T13:46:00Z"),
        ),
    ] {
        let model = DmnModel::try_from(temporal_unary_test_definition(type_ref, input_test))
            .expect("temporal constructor unary tests parse");
        let engine = DmnEngine::new_in_memory().expect("engine");
        engine
            .repository_service()
            .deploy(
                DmnDeploymentRequest::new("temporal-constructor-unary-tests")
                    .with_resource("temporal-decision.dmn", model),
            )
            .expect("deployment");

        assert_eq!(
            execute_temporal(&engine, matching_value),
            json!("matched"),
            "typeRef {type_ref}"
        );
        assert_eq!(
            execute_temporal(&engine, default_value),
            json!("default"),
            "typeRef {type_ref}"
        );
    }
}

#[test]
fn evaluates_temporal_constructor_range_unary_tests_by_declared_type() {
    for (type_ref, input_test, matching_value, default_value) in [
        (
            "date",
            "[date(\"2024-01-01\")..date(\"2024-12-31\")]",
            json!("2024-06-15"),
            json!("2025-01-01"),
        ),
        (
            "time",
            "(time(\"09:00:00\")..time(\"17:00:00\")]",
            json!("17:00:00"),
            json!("09:00:00"),
        ),
        (
            "dateTime",
            "[dateTime(\"2024-01-31T13:00:00Z\")..date and time(\"2024-01-31T14:00:00Z\"))",
            json!("2024-01-31T13:30:00Z"),
            json!("2024-01-31T14:00:00Z"),
        ),
    ] {
        let model = DmnModel::try_from(temporal_unary_test_definition(type_ref, input_test))
            .expect("temporal constructor range unary tests parse");
        let engine = DmnEngine::new_in_memory().expect("engine");
        engine
            .repository_service()
            .deploy(
                DmnDeploymentRequest::new("temporal-constructor-range-unary-tests")
                    .with_resource("temporal-decision.dmn", model),
            )
            .expect("deployment");

        assert_eq!(
            execute_temporal(&engine, matching_value),
            json!("matched"),
            "typeRef {type_ref}"
        );
        assert_eq!(
            execute_temporal(&engine, default_value),
            json!("default"),
            "typeRef {type_ref}"
        );
    }
}

#[test]
fn rejects_temporal_constructor_cross_type_unary_tests_at_deployment() {
    for (type_ref, input_test) in [
        ("date", "time(\"13:45:00\")"),
        ("time", "date(\"2024-01-31\")"),
        ("dateTime", "date(\"2024-01-31\")"),
    ] {
        let model = DmnModel::try_from(temporal_unary_test_definition(type_ref, input_test))
            .expect("temporal constructor unary tests parse");
        let engine = DmnEngine::new_in_memory().expect("engine");
        let error = engine
            .repository_service()
            .deploy(
                DmnDeploymentRequest::new("temporal-constructor-cross-type-unary-tests")
                    .with_resource("temporal-decision.dmn", model),
            )
            .expect_err("cross-type temporal constructor should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}
