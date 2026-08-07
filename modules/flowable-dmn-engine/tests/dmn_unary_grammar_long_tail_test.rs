use flowable_dmn_engine::{DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnModel};
use flowable_dmn_model::{
    Decision, DecisionRule, DecisionTable, DmnDefinition, HitPolicy, InputClause,
    LiteralExpression, OutputClause, UnaryTests,
};
use serde_json::json;

fn rule(rule_number: usize, input: &str, output: &str) -> DecisionRule {
    DecisionRule {
        id: Some(format!("rule-{rule_number}")),
        rule_number,
        input_entries: vec![UnaryTests {
            id: Some(format!("input-entry-{rule_number}")),
            text: Some(input.to_string()),
        }],
        output_entries: vec![LiteralExpression {
            id: Some(format!("output-entry-{rule_number}")),
            type_ref: None,
            text: Some(output.to_string()),
        }],
    }
}

fn substring_unary_test_definition() -> DmnDefinition {
    DmnDefinition {
        id: Some("substring-defs".to_string()),
        name: Some("Substring Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "substringDecision".to_string(),
            name: Some("Substring Decision".to_string()),
            decision_table: DecisionTable {
                id: "substringTable".to_string(),
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
                    rule(1, "substring(?, 1, 3) = \"abc\"", "'first-three'"),
                    rule(2, "substring(?, 5) = \"efgh\"", "'from-five'"),
                    rule(3, "substring(?, -3) = \"xyz\"", "'last-three'"),
                    rule(4, "-", "'default'"),
                ],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

fn replace_unary_test_definition() -> DmnDefinition {
    DmnDefinition {
        id: Some("replace-defs".to_string()),
        name: Some("Replace Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "replaceDecision".to_string(),
            name: Some("Replace Decision".to_string()),
            decision_table: DecisionTable {
                id: "replaceTable".to_string(),
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
                    rule(
                        1,
                        "replace(?, \"foo\", \"bar\") = \"barbaz\"",
                        "'simple-replace'",
                    ),
                    rule(
                        2,
                        "replace(?, \"[0-9]+\", \"#\", \"g\") = \"abc-#-#-#-def\"",
                        "'regex-replace'",
                    ),
                    rule(3, "-", "'default'"),
                ],
            },
            required_decisions: Vec::new(),
        }],
        ..DmnDefinition::default()
    }
}

#[test]
fn deploys_and_evaluates_substring_unary_tests() {
    let model =
        DmnModel::try_from(substring_unary_test_definition()).expect("substring tests parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("substring-tests")
                .with_resource("substring-decision.dmn", model),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "substringDecision",
            DmnExecutionRequest::new(json!({ "value": "abcdefghi" })),
        )
        .expect("execution");
    assert_eq!(result.get_output("result"), Some(&json!("first-three")));

    let result = engine
        .decision_service()
        .execute_by_key(
            "substringDecision",
            DmnExecutionRequest::new(json!({ "value": "1234efgh" })),
        )
        .expect("execution");
    assert_eq!(result.get_output("result"), Some(&json!("from-five")));

    let result = engine
        .decision_service()
        .execute_by_key(
            "substringDecision",
            DmnExecutionRequest::new(json!({ "value": "Xbcxyz" })),
        )
        .expect("execution");
    assert_eq!(result.get_output("result"), Some(&json!("last-three")));

    let result = engine
        .decision_service()
        .execute_by_key(
            "substringDecision",
            DmnExecutionRequest::new(json!({ "value": "nomatch" })),
        )
        .expect("execution");
    assert_eq!(result.get_output("result"), Some(&json!("default")));
}

#[test]
fn deploys_and_evaluates_replace_unary_tests() {
    let model = DmnModel::try_from(replace_unary_test_definition()).expect("replace tests parse");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("replace-tests").with_resource("replace-decision.dmn", model),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "replaceDecision",
            DmnExecutionRequest::new(json!({ "value": "foobaz" })),
        )
        .expect("execution");
    assert_eq!(result.get_output("result"), Some(&json!("simple-replace")));

    let result = engine
        .decision_service()
        .execute_by_key(
            "replaceDecision",
            DmnExecutionRequest::new(json!({ "value": "abc-123-45-678-def" })),
        )
        .expect("execution");
    assert_eq!(result.get_output("result"), Some(&json!("regex-replace")));

    let result = engine
        .decision_service()
        .execute_by_key(
            "replaceDecision",
            DmnExecutionRequest::new(json!({ "value": "nomatch" })),
        )
        .expect("execution");
    assert_eq!(result.get_output("result"), Some(&json!("default")));
}
