//! M81 FEEL unary expansion: membership tests `? in (...)` / `in (...)`.

use flowable_dmn_engine::{
    DmnDeploymentRequest, DmnEngine, DmnError, DmnExecutionRequest, DmnModel, DmnUnaryTest,
};
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

fn string_status_definition(input_tests: &[(&str, &str)]) -> DmnDefinition {
    DmnDefinition {
        id: Some("in-list-status-defs".to_string()),
        name: Some("In-list Status Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "statusDecision".to_string(),
            name: Some("Status Decision".to_string()),
            decision_table: DecisionTable {
                id: "statusTable".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Status".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: Some("string".to_string()),
                        text: Some("status".to_string()),
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

fn numeric_code_definition(input_tests: &[(&str, &str)]) -> DmnDefinition {
    DmnDefinition {
        id: Some("in-list-code-defs".to_string()),
        name: Some("In-list Code Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "codeDecision".to_string(),
            name: Some("Code Decision".to_string()),
            decision_table: DecisionTable {
                id: "codeTable".to_string(),
                hit_policy: HitPolicy::First,
                collect_operator: None,
                inputs: vec![InputClause {
                    id: Some("input1".to_string()),
                    label: Some("Code".to_string()),
                    input_number: 1,
                    input_expression: LiteralExpression {
                        id: Some("inputExpression1".to_string()),
                        type_ref: Some("number".to_string()),
                        text: Some("code".to_string()),
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

fn deploy(key: &str, definition: DmnDefinition) -> DmnEngine {
    let model = DmnModel::try_from(definition).expect("definition parses");
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new(key).with_resource(format!("{key}.dmn"), model))
        .expect("deployment");
    engine
}

fn execute_status(engine: &DmnEngine, status: &str) -> Value {
    engine
        .decision_service()
        .execute_by_key(
            "statusDecision",
            DmnExecutionRequest::new(json!({ "status": status })),
        )
        .expect("execution")
        .get_output("band").cloned().unwrap()
}

fn execute_code(engine: &DmnEngine, code: Value) -> Value {
    engine
        .decision_service()
        .execute_by_key(
            "codeDecision",
            DmnExecutionRequest::new(json!({ "code": code })),
        )
        .expect("execution")
        .get_output("result").cloned().unwrap()
}

#[test]
fn deploys_and_evaluates_question_mark_in_list_string_membership() {
    let engine = deploy(
        "in-list-status",
        string_status_definition(&[
            (r#"? in ("open", "pending")"#, "'active'"),
            (r#"? in ("closed", "cancelled")"#, "'done'"),
            ("-", "'other'"),
        ]),
    );

    assert_eq!(execute_status(&engine, "open"), json!("active"));
    assert_eq!(execute_status(&engine, "pending"), json!("active"));
    assert_eq!(execute_status(&engine, "closed"), json!("done"));
    assert_eq!(execute_status(&engine, "cancelled"), json!("done"));
    assert_eq!(execute_status(&engine, "draft"), json!("other"));
}

#[test]
fn deploys_and_evaluates_bare_in_list_numeric_membership() {
    let engine = deploy(
        "in-list-code",
        numeric_code_definition(&[("in (200, 201, 204)", "'success'"), ("-", "'other'")]),
    );

    assert_eq!(execute_code(&engine, json!(200)), json!("success"));
    assert_eq!(execute_code(&engine, json!(201)), json!("success"));
    assert_eq!(execute_code(&engine, json!(204)), json!("success"));
    assert_eq!(execute_code(&engine, json!(404)), json!("other"));
    assert_eq!(execute_code(&engine, json!(500)), json!("other"));
}

#[test]
fn deploys_and_evaluates_not_around_in_list_membership() {
    let engine = deploy(
        "not-in-list-status",
        string_status_definition(&[
            (r#"not(? in ("blocked", "closed"))"#, "'allowed'"),
            ("-", "'denied'"),
        ]),
    );

    assert_eq!(execute_status(&engine, "open"), json!("allowed"));
    assert_eq!(execute_status(&engine, "pending"), json!("allowed"));
    assert_eq!(execute_status(&engine, "blocked"), json!("denied"));
    assert_eq!(execute_status(&engine, "closed"), json!("denied"));
}

#[test]
fn parses_in_list_unary_tests_as_in_list_variant() {
    let model = DmnModel::try_from(string_status_definition(&[(
        r#"? in ("a", "b")"#,
        "'ok'",
    )]))
    .expect("in-list parses");

    match &model.decisions[0].rules[0].input_entries[0].expression {
        DmnUnaryTest::InList { values } => {
            assert_eq!(values, &vec![json!("a"), json!("b")]);
        }
        other => panic!("expected InList, got {other:?}"),
    }
}

#[test]
fn rejects_empty_in_list_as_structured_error() {
    let error = DmnModel::try_from(string_status_definition(&[("? in ()", "'ok'")]))
        .expect_err("empty in-list should fail");
    assert!(
        matches!(error, DmnError::UnsupportedModel { .. }),
        "expected UnsupportedModel, got {error}"
    );
    assert!(
        error.to_string().contains("unsupported in-list unary test"),
        "unexpected error: {error}"
    );
}
