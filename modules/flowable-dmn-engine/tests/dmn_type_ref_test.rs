use flowable_dmn_engine::{
    DmnDeploymentRequest, DmnEngine, DmnError, DmnExecutionRequest, DmnModel,
};
use flowable_dmn_model::{
    CollectOperator, Decision, DecisionRule, DecisionTable, DmnDefinition, HitPolicy, InputClause,
    LiteralExpression, OutputClause, UnaryTests,
};
use serde_json::{Value, json};

fn single_input_definition(type_ref: &str, input_text: &str, output_text: &str) -> DmnDefinition {
    DmnDefinition {
        id: Some("typed-defs".to_string()),
        name: Some("Typed Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "typedDecision".to_string(),
            name: Some("Typed Decision".to_string()),
            decision_table: DecisionTable {
                id: "typedTable".to_string(),
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
                rules: vec![rule(1, input_text, output_text), rule(2, "-", "'default'")],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

fn single_output_definition(output_type_ref: &str, output_text: &str) -> DmnDefinition {
    typed_output_definition(
        HitPolicy::First,
        None,
        output_type_ref,
        None,
        vec![rule(1, "-", output_text)],
    )
}

fn typed_output_definition(
    hit_policy: HitPolicy,
    collect_operator: Option<CollectOperator>,
    output_type_ref: &str,
    output_values: Option<&str>,
    rules: Vec<DecisionRule>,
) -> DmnDefinition {
    DmnDefinition {
        id: Some("typed-output-defs".to_string()),
        name: Some("Typed Output Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "typedOutputDecision".to_string(),
            name: Some("Typed Output Decision".to_string()),
            decision_table: DecisionTable {
                id: "typedOutputTable".to_string(),
                hit_policy,
                collect_operator,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Value".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: Some("number".to_string()),
                        text: Some("value".to_string()),
                    },
                }],
                outputs: vec![OutputClause {
                    id: Some("output1".to_string()),
                    label: Some("Result".to_string()),
                    name: Some("result".to_string()),
                    type_ref: Some(output_type_ref.to_string()),
                    output_values: output_values.map(|text| UnaryTests {
                        id: Some("outputValues1".to_string()),
                        text: Some(text.to_string()),
                    }),
                    output_number: 1,
                }],
                rules,
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
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

fn deploy_definition(definition: DmnDefinition) -> DmnEngine {
    let model = DmnModel::try_from(definition).expect("typed definition parses");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("typed-inputs").with_resource("typed-decision.dmn", model),
        )
        .expect("deployment");
    engine
}

fn deploy_definition_result(definition: DmnDefinition) -> Result<DmnEngine, DmnError> {
    let model = DmnModel::try_from(definition).expect("typed definition parses");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine.repository_service().deploy(
        DmnDeploymentRequest::new("typed-outputs")
            .with_resource("typed-output-decision.dmn", model),
    )?;
    Ok(engine)
}

fn execute_value(engine: &DmnEngine, value: Value) -> Result<Value, DmnError> {
    engine
        .decision_service()
        .execute_by_key(
            "typedDecision",
            DmnExecutionRequest::new(json!({
                "value": value
            })),
        )
        .map(|result| result.get_output("result").cloned().unwrap())
}

fn execute_output_value(engine: &DmnEngine, value: Value) -> Result<Value, DmnError> {
    engine
        .decision_service()
        .execute_by_key(
            "typedOutputDecision",
            DmnExecutionRequest::new(json!({
                "value": value
            })),
        )
        .map(|result| result.get_output("result").cloned().unwrap())
}

fn execute_value_result(
    engine: &DmnEngine,
    value: Value,
) -> Result<flowable_dmn_engine::DmnExecutionResult, DmnError> {
    engine.decision_service().execute_by_key(
        "typedDecision",
        DmnExecutionRequest::new(json!({
            "value": value
        })),
    )
}

#[test]
fn numeric_type_ref_coerces_string_numbers_before_range_matching() {
    let engine = deploy_definition(single_input_definition("number", "[1..10]", "'matched'"));

    let output = execute_value(&engine, json!("7")).expect("execution");

    assert_eq!(output, json!("matched"));
}

#[test]
fn numeric_type_ref_recursively_normalizes_not_wrapped_range_boundaries() {
    let engine = deploy_definition(single_input_definition(
        "integer",
        "not([1..5])",
        "'outside'",
    ));

    let output = execute_value(&engine, json!(0)).expect("execution");
    assert_eq!(output, json!("outside"));

    let output = execute_value(&engine, json!(1)).expect("execution");
    assert_eq!(output, json!("default"));

    let output = execute_value(&engine, json!(5)).expect("execution");
    assert_eq!(output, json!("default"));

    let output = execute_value(&engine, json!(6)).expect("execution");
    assert_eq!(output, json!("outside"));
}

#[test]
fn numeric_type_ref_rejects_invalid_not_wrapped_numeric_operands_at_deployment() {
    for input_test in ["not(> 'high')", "not([1..'high'])", "not([1.5..5])"] {
        let error =
            deploy_definition_result(single_input_definition("integer", input_test, "'matched'"))
                .expect_err("invalid typed not unary test should fail deployment");

        assert!(matches!(error, DmnError::UnsupportedModel { .. }));
        assert!(
            error.to_string().contains("unsupported unary test"),
            "unexpected error for {input_test}: {error}"
        );
    }
}

#[test]
fn numeric_input_type_ref_rejects_string_comparison_operands_at_deployment() {
    let error = deploy_definition_result(single_input_definition("number", "> 'high'", "'high'"))
        .expect_err("string numeric comparison operand should fail deployment");

    assert!(matches!(error, DmnError::UnsupportedModel { .. }));
    assert!(
        error
            .to_string()
            .contains("unsupported unary test '> 'high''"),
        "unexpected error: {error}"
    );
}

#[test]
fn numeric_type_refs_accept_json_numbers_and_numeric_strings() {
    for (type_ref, value, expected) in [
        ("integer", json!(7), json!("integer")),
        ("integer", json!("7"), json!("integer")),
        ("long", json!(7), json!("long")),
        ("long", json!("7"), json!("long")),
        ("double", json!(7.5), json!("double")),
        ("double", json!("7.5"), json!("double")),
        ("number", json!(7.5), json!("number")),
        ("number", json!("7.5"), json!("number")),
    ] {
        let engine = deploy_definition(single_input_definition(
            type_ref,
            ">= 7",
            &format!("'{type_ref}'"),
        ));

        let output = execute_value(&engine, value).expect("execution");

        assert_eq!(output, expected, "typeRef {type_ref}");
    }
}

#[test]
fn integer_type_refs_reject_fractional_values_as_structured_errors() {
    let engine = deploy_definition(single_input_definition("integer", "-", "'any'"));

    let error = execute_value(&engine, json!(7.5)).expect_err("fractional integer is invalid");

    assert!(matches!(error, DmnError::Execution { .. }));
    assert!(
        error.to_string().contains("typeRef 'integer'"),
        "unexpected error: {error}"
    );
}

#[test]
fn string_and_boolean_type_refs_reject_incompatible_values_as_structured_errors() {
    for (type_ref, value) in [("string", json!(7)), ("boolean", json!("true"))] {
        let engine = deploy_definition(single_input_definition(type_ref, "-", "'any'"));

        let error = execute_value(&engine, value).expect_err("incompatible value is invalid");

        assert!(matches!(error, DmnError::Execution { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn numeric_output_type_refs_coerce_string_numbers_to_json_numbers() {
    // P88: number/double always yield f64 (Java ExecutionVariableFactory.java:60-69).
    // integer/long remain intentional Rust extensions (Java rejects them).
    for (type_ref, output_text, expected) in [
        ("integer", "'7'", json!(7)),
        (
            "long",
            "'9223372036854775807'",
            json!(9223372036854775807_i64),
        ),
        ("double", "'7'", json!(7.0)),
        ("double", "'7.5'", json!(7.5)),
        ("number", "'7'", json!(7.0)),
        ("number", "'7.5'", json!(7.5)),
    ] {
        let engine = deploy_definition(single_output_definition(type_ref, output_text));

        let output = execute_output_value(&engine, json!(0)).expect("execution");

        assert_eq!(output, expected, "typeRef {type_ref} text {output_text}");
    }
}

#[test]
fn integer_output_type_refs_reject_fractional_and_out_of_range_values_as_structured_errors() {
    for (type_ref, output_text) in [
        ("integer", "'7.5'"),
        ("integer", "'2147483648'"),
        ("long", "'7.5'"),
        ("long", "'9223372036854775808'"),
    ] {
        let error = deploy_definition_result(single_output_definition(type_ref, output_text))
            .expect_err("invalid typed output should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn collect_sum_aggregation_uses_numeric_outputs_coerced_from_strings() {
    let definition = typed_output_definition(
        HitPolicy::Collect,
        Some(CollectOperator::Sum),
        "number",
        None,
        vec![rule(1, ">= 0", "'1.5'"), rule(2, ">= 0", "'2.5'")],
    );
    let engine = deploy_definition(definition);

    let output = execute_output_value(&engine, json!(1)).expect("execution");

    assert_eq!(output, json!(4.0));
}

#[test]
fn output_order_compares_coerced_numeric_output_values_and_returns_numbers() {
    let definition = typed_output_definition(
        HitPolicy::OutputOrder,
        None,
        "integer",
        Some("'3','2','1'"),
        vec![
            rule(1, ">= 0", "'1'"),
            rule(2, ">= 0", "'2'"),
            rule(3, ">= 0", "'3'"),
        ],
    );
    let engine = deploy_definition(definition);

    let result = engine
        .decision_service()
        .execute_by_key(
            "typedOutputDecision",
            DmnExecutionRequest::new(json!({ "value": 1 })),
        )
        .expect("execution");

    // P79: OUTPUT_ORDER is multi-row (was columnar array under "result")
    assert!(result.multiple_results);
    assert_eq!(
        result
            .decision_result
            .iter()
            .map(|row| row.get("result").cloned().unwrap())
            .collect::<Vec<_>>(),
        vec![json!(3), json!(2), json!(1)]
    );
}

#[test]
fn priority_compares_coerced_numeric_output_values_and_returns_a_number() {
    let definition = typed_output_definition(
        HitPolicy::Priority,
        None,
        "integer",
        Some("'3','2','1'"),
        vec![
            rule(1, ">= 0", "'1'"),
            rule(2, ">= 0", "'2'"),
            rule(3, ">= 0", "'3'"),
        ],
    );
    let engine = deploy_definition(definition);

    let output = execute_output_value(&engine, json!(1)).expect("execution");

    assert_eq!(output, json!(3));
}

#[test]
fn temporal_input_type_refs_accept_iso_strings_and_compare_by_declared_type() {
    for (type_ref, input_test, value, expected) in [
        ("date", ">= 2024-01-31", json!("2024-02-01"), json!("date")),
        ("time", "< 13:45:00", json!("09:30:00"), json!("time")),
        (
            "dateTime",
            "> 2024-01-31T13:45:00",
            json!("2024-01-31T14:00:00"),
            json!("dateTime"),
        ),
    ] {
        let engine = deploy_definition(single_input_definition(
            type_ref,
            input_test,
            &format!("'{type_ref}'"),
        ));

        let output = execute_value(&engine, value).expect("temporal execution");

        assert_eq!(output, expected, "typeRef {type_ref}");
    }
}

#[test]
fn temporal_input_type_refs_accept_feel_constructors_in_comparisons() {
    for (type_ref, input_test, value, expected_normalized_input) in [
        (
            "date",
            ">= date(\"2024-01-31\")",
            json!("2024-02-01"),
            json!("2024-02-01"),
        ),
        (
            "time",
            "< time('13:45:00')",
            json!("09:30:00"),
            json!("09:30:00"),
        ),
        (
            "dateTime",
            "> date and time(\"2024-01-31T13:45:00\")",
            json!("2024-01-31T14:00:00"),
            json!("2024-01-31T14:00:00"),
        ),
        (
            "dateTime",
            "= dateTime('2024-01-31T13:45:00+08:00')",
            json!("2024-01-31T05:45:00Z"),
            json!("2024-01-31T05:45:00Z"),
        ),
    ] {
        let engine = deploy_definition(single_input_definition(type_ref, input_test, "'matched'"));

        let result = execute_value_result(&engine, value).expect("temporal constructor execution");

        assert_eq!(result.get_output("result"), Some(&json!("matched")),
            "typeRef {type_ref}"
        );
        assert_eq!(
            result.inputs["value"], expected_normalized_input,
            "typeRef {type_ref}"
        );
    }
}

#[test]
fn temporal_input_type_refs_evaluate_range_unary_tests_by_declared_type() {
    for (type_ref, input_test, matching_value, default_value, expected) in [
        (
            "date",
            "[2024-01-01..2024-12-31]",
            json!("2024-06-15"),
            json!("2025-01-01"),
            json!("date"),
        ),
        (
            "time",
            "(09:00:00..17:00:00]",
            json!("17:00:00"),
            json!("09:00:00"),
            json!("time"),
        ),
        (
            "dateTime",
            "[2024-01-31T13:00:00Z..2024-01-31T14:00:00Z)",
            json!("2024-01-31T13:30:00Z"),
            json!("2024-01-31T14:00:00Z"),
            json!("dateTime"),
        ),
    ] {
        let engine = deploy_definition(single_input_definition(
            type_ref,
            input_test,
            &format!("'{type_ref}'"),
        ));

        let output = execute_value(&engine, matching_value).expect("temporal range execution");
        assert_eq!(output, expected, "typeRef {type_ref} should match");

        let output = execute_value(&engine, default_value).expect("temporal range default");
        assert_eq!(
            output,
            json!("default"),
            "typeRef {type_ref} should not match"
        );
    }
}

#[test]
fn temporal_input_type_refs_accept_feel_constructors_in_ranges() {
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
        let engine = deploy_definition(single_input_definition(
            type_ref,
            input_test,
            &format!("'{type_ref}'"),
        ));

        let output = execute_value(&engine, matching_value).expect("temporal constructor range");
        assert_eq!(output, json!(type_ref), "typeRef {type_ref} should match");

        let output = execute_value(&engine, default_value).expect("temporal constructor default");
        assert_eq!(
            output,
            json!("default"),
            "typeRef {type_ref} should not match"
        );
    }
}

#[test]
fn temporal_input_type_refs_reject_cross_type_feel_constructors_at_deployment() {
    for (type_ref, input_test) in [
        ("date", "time(\"13:45:00\")"),
        ("time", "date(\"2024-01-31\")"),
        ("dateTime", "date(\"2024-01-31\")"),
    ] {
        let error =
            deploy_definition_result(single_input_definition(type_ref, input_test, "'matched'"))
                .expect_err("cross-type temporal constructor should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn temporal_input_type_refs_reject_invalid_range_unary_tests_at_deployment() {
    for (type_ref, input_test) in [
        ("date", "[2024-02-30..2024-12-31]"),
        ("time", "(09:00:00..25:00:00]"),
        ("dateTime", "[2024-01-31T13:00:00Z..2024-01-31 14:00:00)"),
    ] {
        let error =
            deploy_definition_result(single_input_definition(type_ref, input_test, "'matched'"))
                .expect_err("invalid temporal range should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn datetime_type_ref_accepts_offset_zulu_and_fractional_seconds() {
    for (input_test, value, expected_normalized_input) in [
        (
            "> 2024-01-31T13:45:00Z",
            json!("2024-01-31T13:45:00.001Z"),
            json!("2024-01-31T13:45:00.001Z"),
        ),
        (
            "= 2024-01-31T13:45:00+08:00",
            json!("2024-01-31T05:45:00Z"),
            json!("2024-01-31T05:45:00Z"),
        ),
        (
            "> 2024-01-31T13:45:00.122",
            json!("2024-01-31T13:45:00.123"),
            json!("2024-01-31T13:45:00.123"),
        ),
    ] {
        let engine =
            deploy_definition(single_input_definition("dateTime", input_test, "'matched'"));

        let result = execute_value_result(&engine, value).expect("dateTime execution");

        assert_eq!(result.get_output("result"), Some(&json!("matched")));
        assert_eq!(result.inputs["value"], expected_normalized_input);
    }
}

#[test]
fn datetime_output_type_ref_normalizes_offset_zulu_and_fractional_seconds() {
    for (output_text, expected) in [
        ("'2024-01-31T13:45:00Z'", json!("2024-01-31T13:45:00Z")),
        ("'2024-01-31T13:45:00+08:00'", json!("2024-01-31T05:45:00Z")),
        (
            "'2024-01-31T13:45:00.123'",
            json!("2024-01-31T13:45:00.123"),
        ),
    ] {
        let engine = deploy_definition(single_output_definition("dateTime", output_text));

        let output = execute_output_value(&engine, json!(0)).expect("execution");

        assert_eq!(output, expected);
    }
}

#[test]
fn temporal_input_type_refs_reject_invalid_runtime_values() {
    for (type_ref, value) in [
        ("date", json!("2024-02-30")),
        ("time", json!("25:00:00")),
        ("dateTime", json!("2024-01-31 13:45:00")),
    ] {
        let engine = deploy_definition(single_input_definition(type_ref, "-", "'any'"));

        let error = execute_value(&engine, value).expect_err("invalid temporal input");

        assert!(matches!(error, DmnError::Execution { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn temporal_input_type_refs_reject_invalid_unary_tests_at_deployment() {
    for (type_ref, input_test) in [
        ("date", ">= 2024-02-30"),
        ("time", "< 25:00:00"),
        ("dateTime", "> 2024-01-31 13:45:00"),
    ] {
        let error =
            deploy_definition_result(single_input_definition(type_ref, input_test, "'matched'"))
                .expect_err("invalid temporal unary test should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn temporal_output_type_refs_normalize_iso_strings_at_deployment_and_runtime() {
    for (type_ref, output_text, expected) in [
        ("date", "'2024-01-31'", json!("2024-01-31")),
        ("time", "'13:45:00'", json!("13:45:00")),
        (
            "dateTime",
            "'2024-01-31T13:45:00'",
            json!("2024-01-31T13:45:00"),
        ),
    ] {
        let engine = deploy_definition(single_output_definition(type_ref, output_text));

        let output = execute_output_value(&engine, json!(0)).expect("execution");

        assert_eq!(output, expected, "typeRef {type_ref}");
    }
}

#[test]
fn temporal_output_type_refs_reject_invalid_deployment_literals() {
    for (type_ref, output_text) in [
        ("date", "'2024-02-30'"),
        ("time", "'24:00:00'"),
        ("dateTime", "'2024-01-31 13:45:00'"),
    ] {
        let error = deploy_definition_result(single_output_definition(type_ref, output_text))
            .expect_err("invalid temporal output should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn temporal_output_priority_compares_normalized_values() {
    let definition = typed_output_definition(
        HitPolicy::Priority,
        None,
        "date",
        Some("'2024-02-01','2024-01-31','2024-01-30'"),
        vec![
            rule(1, ">= 0", "'2024-01-30'"),
            rule(2, ">= 0", "'2024-01-31'"),
            rule(3, ">= 0", "'2024-02-01'"),
        ],
    );
    let engine = deploy_definition(definition);

    let output = execute_output_value(&engine, json!(1)).expect("execution");

    assert_eq!(output, json!("2024-02-01"));
}

#[test]
fn temporal_output_order_compares_normalized_values() {
    let definition = typed_output_definition(
        HitPolicy::OutputOrder,
        None,
        "dateTime",
        Some("'2024-01-31T14:00:00','2024-01-31T13:45:00','2024-01-31T13:30:00'"),
        vec![
            rule(1, ">= 0", "'2024-01-31T13:30:00'"),
            rule(2, ">= 0", "'2024-01-31T13:45:00'"),
            rule(3, ">= 0", "'2024-01-31T14:00:00'"),
        ],
    );
    let engine = deploy_definition(definition);

    let result = engine
        .decision_service()
        .execute_by_key(
            "typedOutputDecision",
            DmnExecutionRequest::new(json!({ "value": 1 })),
        )
        .expect("execution");

    // P79: OUTPUT_ORDER multi-row ordered by outputValues priority
    assert!(result.multiple_results);
    assert_eq!(
        result
            .decision_result
            .iter()
            .map(|row| row.get("result").cloned().unwrap())
            .collect::<Vec<_>>(),
        vec![
            json!("2024-01-31T14:00:00"),
            json!("2024-01-31T13:45:00"),
            json!("2024-01-31T13:30:00")
        ]
    );
}

#[test]
fn duration_input_type_ref_normalizes_day_time_duration_strings_and_compares() {
    let engine = deploy_definition(single_input_definition("duration", "> PT2H", "'matched'"));

    let result = execute_value_result(&engine, json!("PT26H")).expect("duration execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("P1DT2H"));
}

#[test]
fn duration_input_type_ref_accepts_feel_duration_constructor_in_comparison() {
    let engine = deploy_definition(single_input_definition(
        "duration",
        "> duration(\"PT1H\")",
        "'matched'",
    ));

    let result = execute_value_result(&engine, json!("PT90M")).expect("duration execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("PT1H30M"));
}

#[test]
fn duration_input_type_ref_matches_equal_normalized_day_time_durations() {
    let engine = deploy_definition(single_input_definition("duration", "= P1D", "'matched'"));

    let result = execute_value_result(&engine, json!("PT24H")).expect("duration execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("P1D"));
}

#[test]
fn duration_input_type_ref_normalizes_fractional_seconds_and_compares() {
    let engine = deploy_definition(single_input_definition("duration", "> PT0.5S", "'matched'"));

    let result = execute_value_result(&engine, json!("PT1.250S")).expect("duration execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("PT1.25S"));
}

#[test]
fn duration_input_type_ref_normalizes_week_duration_to_days() {
    let engine = deploy_definition(single_input_definition("duration", "= P7D", "'matched'"));

    let result = execute_value_result(&engine, json!("P1W")).expect("duration execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("P7D"));
}

#[test]
fn duration_input_type_ref_accepts_negative_day_time_durations() {
    let engine = deploy_definition(single_input_definition("duration", "< PT0S", "'matched'"));

    let result = execute_value_result(&engine, json!("-PT30M")).expect("duration execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("-PT30M"));
}

#[test]
fn duration_input_type_ref_rejects_year_month_and_malformed_values() {
    for input_test in ["P1M", "P1Y", "PT", "P", "P1DT", "PT1.S"] {
        let error =
            deploy_definition_result(single_input_definition("duration", input_test, "'matched'"))
                .expect_err("invalid duration unary test should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains("typeRef 'duration'"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn duration_output_type_ref_normalizes_day_time_duration_literals() {
    for (output_text, expected) in [
        ("'P1D'", json!("P1D")),
        ("'PT26H'", json!("P1DT2H")),
        ("'PT1.250S'", json!("PT1.25S")),
        ("'P1W'", json!("P7D")),
        ("'-PT30M'", json!("-PT30M")),
    ] {
        let engine = deploy_definition(single_output_definition("duration", output_text));

        let output = execute_output_value(&engine, json!(0)).expect("execution");

        assert_eq!(output, expected);
    }
}

#[test]
fn duration_output_type_ref_rejects_invalid_deployment_literals() {
    for output_text in ["'P1M'", "'P1Y'", "'PT'", "'P'", "'P1DT'", "'PT1.S'"] {
        let error = deploy_definition_result(single_output_definition("duration", output_text))
            .expect_err("invalid duration output should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains("typeRef 'duration'"),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn duration_priority_compares_normalized_output_values() {
    let definition = typed_output_definition(
        HitPolicy::Priority,
        None,
        "duration",
        Some("'P1DT2H','P1D','PT2H'"),
        vec![
            rule(1, ">= 0", "'PT2H'"),
            rule(2, ">= 0", "'PT24H'"),
            rule(3, ">= 0", "'PT26H'"),
        ],
    );
    let engine = deploy_definition(definition);

    let output = execute_output_value(&engine, json!(1)).expect("execution");

    assert_eq!(output, json!("P1DT2H"));
}

#[test]
fn duration_year_month_input_type_ref_normalizes_total_months_and_compares() {
    let engine = deploy_definition(single_input_definition(
        "yearMonthDuration",
        "> P1Y",
        "'matched'",
    ));

    let result = execute_value_result(&engine, json!("P13M")).expect("duration execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("P13M"));
}

#[test]
fn duration_year_month_input_type_ref_accepts_feel_duration_constructor_in_comparison() {
    let engine = deploy_definition(single_input_definition(
        "yearMonthDuration",
        ">= duration('P1Y')",
        "'matched'",
    ));

    let result = execute_value_result(&engine, json!("P12M")).expect("duration execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("P12M"));
}

#[test]
fn duration_year_month_input_type_ref_matches_equal_normalized_total_months() {
    let engine = deploy_definition(single_input_definition(
        "yearMonthDuration",
        "= P1Y",
        "'matched'",
    ));

    let result = execute_value_result(&engine, json!("P12M")).expect("duration execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("P12M"));
}

#[test]
fn duration_year_month_output_type_ref_normalizes_total_months() {
    for (output_text, expected) in [
        ("'P1Y'", json!("P12M")),
        ("'P1Y2M'", json!("P14M")),
        ("'-P1Y2M'", json!("-P14M")),
    ] {
        let engine = deploy_definition(single_output_definition("yearMonthDuration", output_text));

        let output = execute_output_value(&engine, json!(0)).expect("execution");

        assert_eq!(output, expected);
    }
}

#[test]
fn duration_year_month_priority_compares_normalized_output_values() {
    let definition = typed_output_definition(
        HitPolicy::Priority,
        None,
        "yearMonthDuration",
        Some("'P14M','P12M','P2M'"),
        vec![
            rule(1, ">= 0", "'P2M'"),
            rule(2, ">= 0", "'P1Y'"),
            rule(3, ">= 0", "'P1Y2M'"),
        ],
    );
    let engine = deploy_definition(definition);

    let output = execute_output_value(&engine, json!(1)).expect("execution");

    assert_eq!(output, json!("P14M"));
}

#[test]
fn duration_year_month_type_ref_rejects_day_time_duration_values() {
    for input_test in ["P1D", "PT24H", "P1W", "P1YT1H", "P"] {
        let error = deploy_definition_result(single_input_definition(
            "yearMonthDuration",
            input_test,
            "'matched'",
        ))
        .expect_err("day-time duration unary test should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains("typeRef 'yearMonthDuration'"),
            "unexpected error: {error}"
        );
    }

    let engine = deploy_definition(single_input_definition("yearMonthDuration", "-", "'any'"));

    let error =
        execute_value(&engine, json!("P1D")).expect_err("day-time runtime value is invalid");

    assert!(matches!(error, DmnError::Execution { .. }));
    assert!(
        error.to_string().contains("typeRef 'yearMonthDuration'"),
        "unexpected error: {error}"
    );
}

#[test]
fn duration_type_refs_reject_feel_duration_constructor_cross_family_unary_tests() {
    for (type_ref, input_test) in [
        ("duration", "> duration(\"P1Y\")"),
        ("yearMonthDuration", "> duration('PT1H')"),
    ] {
        let error =
            deploy_definition_result(single_input_definition(type_ref, input_test, "'matched'"))
                .expect_err("cross-family duration constructor should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn context_and_list_input_type_refs_accept_json_object_and_array_values() {
    for (type_ref, value, expected) in [
        (
            "context",
            json!({"tier": "gold", "score": 7}),
            json!("context"),
        ),
        ("list", json!(["manual", "email"]), json!("list")),
    ] {
        let engine = deploy_definition(single_input_definition(
            type_ref,
            "-",
            &format!("'{type_ref}'"),
        ));

        let result = execute_value_result(&engine, value).expect("typed execution");

        assert_eq!(
            result.get_output("result"),
            Some(&expected),
            "typeRef {type_ref}"
        );
    }
}

#[test]
fn context_and_list_output_type_refs_preserve_json_object_and_array_values() {
    for (type_ref, output_text, expected) in [
        (
            "context",
            "{\"tier\":\"gold\",\"score\":7}",
            json!({"tier": "gold", "score": 7}),
        ),
        ("list", "[\"manual\",\"email\"]", json!(["manual", "email"])),
    ] {
        let engine = deploy_definition(single_output_definition(type_ref, output_text));

        let output = execute_output_value(&engine, json!(0)).expect("execution");

        assert_eq!(output, expected, "typeRef {type_ref}");
    }
}

#[test]
fn context_and_list_type_refs_reject_incompatible_values_as_structured_errors() {
    for (type_ref, value) in [
        ("context", json!(["not", "object"])),
        ("list", json!({"not": "array"})),
    ] {
        let engine = deploy_definition(single_input_definition(type_ref, "-", "'any'"));

        let error = execute_value(&engine, value).expect_err("incompatible value is invalid");

        assert!(matches!(error, DmnError::Execution { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }

    for (type_ref, output_text) in [
        ("context", "[\"not\",\"object\"]"),
        ("list", "{\"not\":\"array\"}"),
    ] {
        let error = deploy_definition_result(single_output_definition(type_ref, output_text))
            .expect_err("invalid typed output should fail deployment");

        assert!(matches!(error, DmnError::Validation { .. }));
        assert!(
            error.to_string().contains(&format!("typeRef '{type_ref}'")),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn date_and_time_type_ref_alias_uses_datetime_semantics() {
    let engine = deploy_definition(single_input_definition(
        "date and time",
        "> date and time(\"2024-01-31T13:45:00+08:00\")",
        "'matched'",
    ));

    let result =
        execute_value_result(&engine, json!("2024-01-31T05:45:01Z")).expect("dateTime execution");

    assert_eq!(result.get_output("result"), Some(&json!("matched")));
    assert_eq!(result.inputs["value"], json!("2024-01-31T05:45:01Z"));

    let engine = deploy_definition(single_output_definition(
        "date and time",
        "'2024-01-31T13:45:00+08:00'",
    ));

    let output = execute_output_value(&engine, json!(0)).expect("execution");

    assert_eq!(output, json!("2024-01-31T05:45:00Z"));
}

#[test]
fn spaced_duration_type_ref_aliases_use_existing_duration_semantics() {
    for (type_ref, input_test, runtime_value, expected_normalized_input) in [
        (
            "day time duration",
            "> duration(\"PT1H\")",
            json!("PT90M"),
            json!("PT1H30M"),
        ),
        (
            "year month duration",
            ">= duration('P1Y')",
            json!("P12M"),
            json!("P12M"),
        ),
    ] {
        let engine = deploy_definition(single_input_definition(type_ref, input_test, "'matched'"));

        let result = execute_value_result(&engine, runtime_value).expect("duration execution");

        assert_eq!(result.get_output("result"), Some(&json!("matched")));
        assert_eq!(result.inputs["value"], expected_normalized_input);
    }

    for (type_ref, output_text, expected) in [
        ("day time duration", "'PT26H'", json!("P1DT2H")),
        ("year month duration", "'P1Y2M'", json!("P14M")),
    ] {
        let engine = deploy_definition(single_output_definition(type_ref, output_text));

        let output = execute_output_value(&engine, json!(0)).expect("execution");

        assert_eq!(output, expected);
    }
}

#[test]
fn list_contains_unary_test_matches_list_inputs_containing_literal_values() {
    let definition = DmnDefinition {
        id: Some("list-contains-defs".to_string()),
        name: Some("List Contains Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "typedDecision".to_string(),
            name: Some("List Contains Decision".to_string()),
            decision_table: DecisionTable {
                id: "listContainsTable".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Tags".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: Some("list".to_string()),
                        text: Some("value".to_string()),
                    },
                }],
                outputs: vec![OutputClause {
                    id: Some("output1".to_string()),
                    label: Some("Category".to_string()),
                    name: Some("result".to_string()),
                    type_ref: Some("string".to_string()),
                    output_values: None,
                    output_number: 1,
                }],
                rules: vec![
                    rule(1, "list contains(?, \"urgent\")", "'priority'"),
                    rule(2, "list contains(?, \"review\")", "'pending'"),
                    rule(3, "-", "'standard'"),
                ],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    };

    let engine = deploy_definition(definition);

    let output = execute_value(&engine, json!(["urgent", "bug"])).expect("execution");
    assert_eq!(output, json!("priority"));

    let output = execute_value(&engine, json!(["review", "feature"])).expect("execution");
    assert_eq!(output, json!("pending"));

    let output = execute_value(&engine, json!(["feature", "enhancement"])).expect("execution");
    assert_eq!(output, json!("standard"));

    let output = execute_value(&engine, json!([])).expect("execution");
    assert_eq!(output, json!("standard"));
}

#[test]
fn list_contains_unary_test_rejects_non_list_inputs_at_runtime() {
    let definition = DmnDefinition {
        id: Some("list-contains-non-list-defs".to_string()),
        name: Some("List Contains Non-List Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "typedDecision".to_string(),
            name: Some("List Contains Non-List Decision".to_string()),
            decision_table: DecisionTable {
                id: "listContainsNonListTable".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Tags".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: Some("list".to_string()),
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
                    rule(1, "list contains(?, \"urgent\")", "'matched'"),
                    rule(2, "-", "'default'"),
                ],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    };

    let engine = deploy_definition(definition);

    let error =
        execute_value(&engine, json!("not-a-list")).expect_err("non-list input should fail");
    assert!(matches!(error, DmnError::Execution { .. }));
    assert!(
        error.to_string().contains("typeRef 'list'"),
        "unexpected error: {error}"
    );
}
