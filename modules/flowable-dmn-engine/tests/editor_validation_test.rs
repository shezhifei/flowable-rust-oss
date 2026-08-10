use std::collections::HashMap;

use flowable_dmn_engine::{
    EDITOR_CREATABLE_HIT_POLICIES, EDITOR_ROUND_TRIP_HIT_POLICIES, editor_capabilities,
    evaluate_editor_expression, validate_editor_definition, validate_editor_expression,
};
use flowable_dmn_model::{
    CollectOperator, Decision, DecisionRule, DecisionTable, DmnDefinition, HitPolicy, InputClause,
    LiteralExpression, OutputClause, UnaryTests,
};
use serde_json::json;

fn definition(
    hit_policy: HitPolicy,
    collect_operator: Option<CollectOperator>,
    input_test: &str,
    outputs: Vec<(&str, &str, &str)>,
) -> DmnDefinition {
    DmnDefinition {
        id: Some("editorDefinitions".to_string()),
        name: Some("Editor validation".to_string()),
        namespace: Some("https://flowable.org/modeler/tests".to_string()),
        decisions: vec![Decision {
            id: "eligibility".to_string(),
            name: Some("Eligibility".to_string()),
            decision_table: DecisionTable {
                id: "eligibilityTable".to_string(),
                hit_policy,
                collect_operator,
                inputs: vec![InputClause {
                    id: Some("ageInput".to_string()),
                    label: Some("Age".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("ageExpression".to_string()),
                        type_ref: Some("integer".to_string()),
                        text: Some("age".to_string()),
                    },
                }],
                outputs: outputs
                    .iter()
                    .enumerate()
                    .map(|(index, (name, type_ref, _))| OutputClause {
                        id: Some(format!("output{}", index + 1)),
                        label: Some((*name).to_string()),
                        name: Some((*name).to_string()),
                        type_ref: Some((*type_ref).to_string()),
                        output_values: None,
                        output_number: index + 1,
                    })
                    .collect(),
                rules: vec![DecisionRule {
                    id: Some("rule1".to_string()),
                    rule_number: 1,
                    input_entries: vec![UnaryTests {
                        id: Some("rule1Input".to_string()),
                        text: Some(input_test.to_string()),
                    }],
                    output_entries: outputs
                        .iter()
                        .enumerate()
                        .map(|(index, (_, _, expression))| LiteralExpression {
                            id: Some(format!("rule1Output{}", index + 1)),
                            type_ref: None,
                            text: Some((*expression).to_string()),
                        })
                        .collect(),
                }],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

#[test]
fn capabilities_separate_new_model_choices_from_imported_complete() {
    let capabilities = editor_capabilities();

    assert_eq!(
        capabilities.creatable_hit_policies,
        EDITOR_CREATABLE_HIT_POLICIES
    );
    assert!(!EDITOR_CREATABLE_HIT_POLICIES.contains(&"COMPLETE"));
    assert!(EDITOR_ROUND_TRIP_HIT_POLICIES.contains(&"COMPLETE"));
    assert_eq!(
        capabilities.collect_operators,
        ["COUNT", "SUM", "MIN", "MAX"]
    );
    assert!(capabilities.output_expression_functions.contains(&"mean"));
    assert!(
        capabilities
            .input_unary_test_forms
            .contains(&"open-or-closed-range")
    );
}

#[test]
fn definition_validation_reuses_unary_parser_and_typed_normalization() {
    let valid = definition(
        HitPolicy::First,
        None,
        ">= 18",
        vec![("result", "string", "\"adult\"")],
    );
    validate_editor_definition(&valid).expect("valid editor definition");

    let unsupported = definition(
        HitPolicy::First,
        None,
        "starts with(\"missing-placeholder\")",
        vec![("result", "string", "\"invalid\"")],
    );
    let error = validate_editor_definition(&unsupported).expect_err("unary test must fail");
    assert_eq!(
        error.to_string(),
        "Unsupported DMN unary test: unsupported string function unary test \
         'starts with(\"missing-placeholder\")'; only contains(?, \"literal\"), starts with(?, \
         \"literal\"), ends with(?, \"literal\"), and matches(?, \"regex\") are supported"
    );
}

#[test]
fn definition_validation_rejects_collect_shape_and_output_type_ref() {
    let multiple_outputs = definition(
        HitPolicy::Collect,
        Some(CollectOperator::Sum),
        "-",
        vec![("score", "number", "1"), ("reason", "number", "2")],
    );
    let error = validate_editor_definition(&multiple_outputs).expect_err("shape must fail");
    assert!(
        error.to_string().contains("multiple outputs")
            && error.to_string().contains("not supported"),
        "unexpected error: {error}"
    );

    let wrong_type = definition(
        HitPolicy::Collect,
        Some(CollectOperator::Count),
        "-",
        vec![("result", "string", "\"one\"")],
    );
    let error = validate_editor_definition(&wrong_type).expect_err("typeRef must fail");
    assert!(
        error.to_string().contains("needs output type number"),
        "unexpected error: {error}"
    );
}

#[test]
fn imported_complete_is_validated_without_rewriting_the_document() {
    let imported = definition(
        HitPolicy::Complete,
        None,
        "-",
        vec![("result", "string", "\"kept\"")],
    );
    let before = imported.clone();

    validate_editor_definition(&imported).expect("COMPLETE remains deployable");

    assert_eq!(imported, before);
    assert_eq!(
        imported.decisions[0].decision_table.hit_policy,
        HitPolicy::Complete
    );
}

#[test]
fn expression_validation_uses_typed_and_compatibility_runtime_paths() {
    let context = HashMap::from([("scores".to_string(), json!([2, 4, 6]))]);

    validate_editor_expression("for x in [1, 2] return x * 2", &context).expect("typed FEEL path");
    assert_eq!(
        evaluate_editor_expression("mean(scores)", &context).expect("compatibility function"),
        json!(4.0)
    );

    let error = validate_editor_expression("unsupported_editor_function()", &context)
        .expect_err("unknown function must fail");
    assert_eq!(
        error.to_string(),
        "Unsupported DMN FEEL function: unsupported FEEL function 'unsupported_editor_function'"
    );
}
