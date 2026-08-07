//! P84: DMN input-entry pre-parser parity with Java
//! `ELInputEntryExpressionPreParser.java`.
//!
//! Covers the three things the Java pre-parser does before handing an input
//! entry to the expression manager:
//!   1. date function aliases      (`ELInputEntryExpressionPreParser.java:26-29`)
//!   2. EL pass-through            (`:31-33`)
//!   3. `.property` shorthand      (`:42-46`)
//! plus the implicit `== ` it inserts for bare operands (`:53-62`).
//!
//! Dialect note: Rust evaluates a FEEL subset, not JUEL (P81 deviation). The
//! aliases are registered natively as `fn_*` instead of being textually
//! rewritten to the JUEL-prefixed `date:toDate` / `date:now` names, because
//! `prefix:name` is not lexable FEEL.

use chrono::{Duration, Utc};
use flowable_dmn_engine::{
    DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnModel, DmnUnaryTest,
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

/// Single-input decision table over `variable` typed `type_ref`.
fn definition(
    variable: &str,
    type_ref: Option<&str>,
    input_tests: &[(&str, &str)],
) -> DmnDefinition {
    DmnDefinition {
        id: Some("p84-defs".to_string()),
        name: Some("P84 Pre-parser Decisions".to_string()),
        namespace: Some("http://flowable.org/dmn".to_string()),
        decisions: vec![Decision {
            id: "p84Decision".to_string(),
            name: Some("P84 Decision".to_string()),
            decision_table: DecisionTable {
                id: "p84Table".to_string(),
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
        .deploy(DmnDeploymentRequest::new("p84").with_resource("p84.dmn", model))
        .expect("deployment");
    engine
}

fn run(engine: &DmnEngine, variables: Value) -> Option<Value> {
    engine
        .decision_service()
        .execute_by_key("p84Decision", DmnExecutionRequest::new(variables))
        .expect("execution")
        .get_output("result")
        .cloned()
}

fn run_err(engine: &DmnEngine, variables: Value) -> String {
    engine
        .decision_service()
        .execute_by_key("p84Decision", DmnExecutionRequest::new(variables))
        .expect_err("execution should fail")
        .to_string()
}

fn today() -> chrono::NaiveDate {
    Utc::now().date_naive()
}

fn iso(date: chrono::NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

// ---------------------------------------------------------------------------
// 1. Date function aliases (ELInputEntryExpressionPreParser.java:26-29)
// ---------------------------------------------------------------------------

#[test]
fn fn_date_comparison_matches_and_rejects() {
    let engine = deploy(definition(
        "hired",
        Some("date"),
        &[(">= fn_date(\"2024-06-01\")", "\"recent\"")],
    ));

    assert_eq!(
        run(&engine, json!({ "hired": "2024-07-15" })),
        Some(json!("recent")),
        "date after the alias-produced boundary matches"
    );
    assert_eq!(
        run(&engine, json!({ "hired": "2024-06-01" })),
        Some(json!("recent")),
        ">= is inclusive at the boundary"
    );
    assert_eq!(
        run(&engine, json!({ "hired": "2024-05-31" })),
        None,
        "date before the boundary does not match"
    );
}

#[test]
fn fn_date_bare_operand_gets_implicit_equals() {
    // Java inserts "== " for operands that do not start with an operator
    // (ELInputEntryExpressionPreParser.java:53-62).
    let engine = deploy(definition(
        "hired",
        Some("date"),
        &[("fn_date(\"2024-06-01\")", "\"exact\"")],
    ));

    assert_eq!(
        run(&engine, json!({ "hired": "2024-06-01" })),
        Some(json!("exact"))
    );
    assert_eq!(run(&engine, json!({ "hired": "2024-06-02" })), None);
}

#[test]
fn fn_now_is_evaluated_per_row_not_at_deploy_time() {
    // DateUtil.now() is `new LocalDate().toDate()` — midnight today
    // (DateUtil.java:69-71), so today itself satisfies `<= fn_now()`.
    let engine = deploy(definition(
        "seen",
        Some("date"),
        &[("<= fn_now()", "\"past\"")],
    ));

    assert_eq!(
        run(&engine, json!({ "seen": iso(today()) })),
        Some(json!("past")),
        "today is not in the future"
    );
    assert_eq!(
        run(&engine, json!({ "seen": iso(today() - Duration::days(1)) })),
        Some(json!("past"))
    );
    assert_eq!(
        run(&engine, json!({ "seen": iso(today() + Duration::days(1)) })),
        None,
        "tomorrow is after fn_now()"
    );
}

#[test]
fn fn_add_date_shifts_forward() {
    let engine = deploy(definition(
        "due",
        Some("date"),
        &[("<= fn_addDate(fn_now(), 0, 0, 7)", "\"soon\"")],
    ));

    assert_eq!(
        run(&engine, json!({ "due": iso(today() + Duration::days(3)) })),
        Some(json!("soon"))
    );
    assert_eq!(
        run(&engine, json!({ "due": iso(today() + Duration::days(7)) })),
        Some(json!("soon")),
        "boundary is inclusive"
    );
    assert_eq!(
        run(&engine, json!({ "due": iso(today() + Duration::days(8)) })),
        None
    );
}

#[test]
fn fn_subtract_date_shifts_backward() {
    let engine = deploy(definition(
        "seen",
        Some("date"),
        &[(">= fn_subtractDate(fn_now(), 0, 0, 30)", "\"active\"")],
    ));

    assert_eq!(
        run(
            &engine,
            json!({ "seen": iso(today() - Duration::days(10)) })
        ),
        Some(json!("active"))
    );
    assert_eq!(
        run(
            &engine,
            json!({ "seen": iso(today() - Duration::days(31)) })
        ),
        None
    );
}

#[test]
fn add_date_applies_years_months_days_and_clamps_month_end() {
    // Joda plusMonths clamps to the target month's last day
    // (DateUtil.java:52-54), e.g. 2024-01-31 + 1 month = 2024-02-29.
    let engine = deploy(definition(
        "d",
        Some("date"),
        &[(
            "fn_addDate(fn_date(\"2024-01-31\"), 0, 1, 0)",
            "\"clamped\"",
        )],
    ));

    assert_eq!(
        run(&engine, json!({ "d": "2024-02-29" })),
        Some(json!("clamped")),
        "Jan 31 + 1 month clamps to Feb 29 in a leap year"
    );
    assert_eq!(run(&engine, json!({ "d": "2024-03-02" })), None);
}

#[test]
fn add_date_combines_years_months_and_days() {
    let engine = deploy(definition(
        "d",
        Some("date"),
        &[(
            "fn_addDate(fn_date(\"2020-01-15\"), 2, 3, 10)",
            "\"shifted\"",
        )],
    ));

    assert_eq!(
        run(&engine, json!({ "d": "2022-04-25" })),
        Some(json!("shifted"))
    );
}

#[test]
fn fn_date_rejects_unparseable_argument_at_runtime() {
    // Java's Joda parseLocalDate throws, which ELExpressionExecutor wraps in a
    // FlowableDmnExpressionException and rethrows (ELExpressionExecutor.java:57-60).
    let engine = deploy(definition(
        "d",
        Some("date"),
        &[("fn_date(\"not-a-date\")", "\"never\"")],
    ));

    let message = run_err(&engine, json!({ "d": "2024-01-01" }));
    assert!(
        message.contains("not-a-date"),
        "error should name the bad argument, got: {message}"
    );
}

#[test]
fn identifier_prefixed_with_alias_name_is_not_treated_as_a_date_call() {
    // `fn_dateish` must not be mistaken for the `fn_date` alias.
    let expression = flowable_dmn_model::UnaryTests {
        id: None,
        text: Some("\"fn_dateish\"".to_string()),
    };
    let entry = flowable_dmn_engine::DmnRuleInputEntry::try_from(expression).expect("parses");
    assert_eq!(entry.expression, DmnUnaryTest::Equals(json!("fn_dateish")));
}

// ---------------------------------------------------------------------------
// 2. EL pass-through (ELInputEntryExpressionPreParser.java:31-33)
// ---------------------------------------------------------------------------

#[test]
fn el_passthrough_evaluates_whole_expression_as_boolean() {
    // Java returns the text untouched and evaluates it through
    // RuleExpressionCondition, which requires a Boolean
    // (RuleExpressionCondition.java:36-50). The input value is NOT compared
    // against the result — the expression carries its own comparison.
    let engine = deploy(definition(
        "age",
        Some("number"),
        &[("${age > 18}", "\"adult\"")],
    ));

    assert_eq!(run(&engine, json!({ "age": 21 })), Some(json!("adult")));
    assert_eq!(run(&engine, json!({ "age": 18 })), None);
}

#[test]
fn el_passthrough_accepts_hash_shell_and_other_variables() {
    // `#{...}` is the shell Java itself generates, and the expression may
    // reference variables other than the input variable.
    let engine = deploy(definition(
        "age",
        Some("number"),
        &[("#{age > 18 and country = \"NL\"}", "\"eligible\"")],
    ));

    assert_eq!(
        run(&engine, json!({ "age": 21, "country": "NL" })),
        Some(json!("eligible"))
    );
    assert_eq!(
        run(&engine, json!({ "age": 21, "country": "BE" })),
        None,
        "second conjunct fails"
    );
}

#[test]
fn el_passthrough_can_call_date_aliases() {
    let engine = deploy(definition(
        "due",
        Some("date"),
        &[("${due < fn_addDate(fn_now(), 0, 0, 2)}", "\"urgent\"")],
    ));

    assert_eq!(
        run(&engine, json!({ "due": iso(today()) })),
        Some(json!("urgent"))
    );
    assert_eq!(
        run(&engine, json!({ "due": iso(today() + Duration::days(5)) })),
        None
    );
}

#[test]
fn el_passthrough_non_boolean_result_fails_execution() {
    // RuleExpressionCondition throws on a non-Boolean result
    // (RuleExpressionCondition.java:44-47).
    let engine = deploy(definition(
        "age",
        Some("number"),
        &[("${age + 1}", "\"never\"")],
    ));

    let message = run_err(&engine, json!({ "age": 21 }));
    assert!(
        message.contains("non-Boolean"),
        "error should report the non-Boolean result, got: {message}"
    );
}

#[test]
fn el_passthrough_unknown_function_fails_execution() {
    // JUEL-only syntax has no FEEL equivalent and is a hard error, matching the
    // P81 dialect deviation rather than silently not matching.
    let engine = deploy(definition(
        "age",
        Some("number"),
        &[("${juelOnlyFunction(age)}", "\"never\"")],
    ));

    let message = run_err(&engine, json!({ "age": 21 }));
    assert!(
        message.contains("failed to evaluate input entry expression"),
        "error should identify the input entry, got: {message}"
    );
}

#[test]
fn embedded_el_template_is_rejected_at_deploy_time() {
    // Java hands `prefix ${x} suffix` to JUEL, which treats it as a template.
    // The FEEL subset has no template concept, so this is a deploy-time error
    // instead of a silent mismatch.
    let model = DmnModel::try_from(definition(
        "name",
        Some("string"),
        &[("prefix ${name} suffix", "\"never\"")],
    ));

    let error = model.expect_err("embedded EL should not parse").to_string();
    assert!(
        error.contains("embedded or templated EL"),
        "error should explain the limitation, got: {error}"
    );
}

// ---------------------------------------------------------------------------
// 3. `.property` shorthand (ELInputEntryExpressionPreParser.java:42-46)
// ---------------------------------------------------------------------------

#[test]
fn property_shorthand_compares_nested_value() {
    // Java builds `#{customer.tier == "gold"}`; Rust applies the path to the
    // resolved input value, so the input variable name is never needed.
    // The input carries no typeRef: a structured value would not survive the
    // scalar coercion in `coerce_input_value`.
    let engine = deploy(definition(
        "customer",
        None,
        &[(".tier == \"gold\"", "\"vip\"")],
    ));

    assert_eq!(
        run(&engine, json!({ "customer": { "tier": "gold" } })),
        Some(json!("vip"))
    );
    assert_eq!(
        run(&engine, json!({ "customer": { "tier": "silver" } })),
        None
    );
}

#[test]
fn property_shorthand_supports_multi_level_paths_and_operators() {
    let engine = deploy(definition(
        "customer",
        None,
        &[(".address.zip == \"1011\"", "\"amsterdam\"")],
    ));

    assert_eq!(
        run(
            &engine,
            json!({ "customer": { "address": { "zip": "1011" } } })
        ),
        Some(json!("amsterdam"))
    );
    assert_eq!(
        run(
            &engine,
            json!({ "customer": { "address": { "zip": "3011" } } })
        ),
        None
    );
}

#[test]
fn bare_property_shorthand_tests_boolean_property() {
    // `.active` becomes `#{customer.active}` in Java — a boolean property, not
    // a comparison (ELInputEntryExpressionPreParser.java:42-44).
    let engine = deploy(definition("customer", None, &[(".active", "\"on\"")]));

    assert_eq!(
        run(&engine, json!({ "customer": { "active": true } })),
        Some(json!("on"))
    );
    assert_eq!(
        run(&engine, json!({ "customer": { "active": false } })),
        None
    );
}

#[test]
fn property_shorthand_does_not_match_when_property_is_absent() {
    let engine = deploy(definition(
        "customer",
        None,
        &[(".tier == \"gold\"", "\"vip\"")],
    ));

    assert_eq!(run(&engine, json!({ "customer": { "name": "ann" } })), None);
    assert_eq!(
        run(&engine, json!({ "customer": "not-an-object" })),
        None,
        "scalar input has no properties"
    );
}

#[test]
fn leading_dot_decimal_is_a_number_not_a_property_path() {
    // Java routes number typeRefs through the operator branch, so `.5` is a
    // decimal literal, not a property (ELInputEntryExpressionPreParser.java:39-41).
    //
    // The P84 property-path parser declines `.5` — it requires an alphabetic or
    // `_` character after the dot — so the entry falls through to the ordinary
    // literal parser. P84 recorded a gap there (`.5` is not valid JSON, so the
    // operand degraded to the string ".5" and deploy-time validation rejected it
    // against the `double` typeRef); P87 closed it by padding the bare decimal
    // point before the JSON parse. This test now pins the property-path parser's
    // half of the contract: `.5` still is not a property path, and the literal
    // parser reads it as the number 0.5.
    let engine = deploy(definition(
        "ratio",
        Some("double"),
        &[(">= .5", "\"high\"")],
    ));
    assert_eq!(run(&engine, json!({ "ratio": 0.75 })), Some(json!("high")));
    assert_eq!(run(&engine, json!({ "ratio": 0.5 })), Some(json!("high")));
    assert_eq!(run(&engine, json!({ "ratio": 0.25 })), None);

    // The equivalent zero-prefixed literal behaves identically, confirming the
    // leading dot is purely a lexing detail and not a comparison difference.
    let engine = deploy(definition(
        "ratio",
        Some("double"),
        &[(">= 0.5", "\"high\"")],
    ));
    assert_eq!(run(&engine, json!({ "ratio": 0.75 })), Some(json!("high")));
    assert_eq!(run(&engine, json!({ "ratio": 0.25 })), None);
}

#[test]
fn property_shorthand_is_skipped_for_numeric_type_ref() {
    // Java only takes the `.` branch for non-date/number typeRefs
    // (ELInputEntryExpressionPreParser.java:39-47).
    let engine = deploy(definition(
        "score",
        Some("number"),
        &[(".tier == \"x\"", "\"never\"")],
    ));

    assert_eq!(run(&engine, json!({ "score": 10 })), None);
}

// ---------------------------------------------------------------------------
// Implicit `== ` for bare operands — already the Rust default; asserted here so
// the parity claim is covered by a test (ELInputEntryExpressionPreParser.java:53-62).
// ---------------------------------------------------------------------------

#[test]
fn bare_literal_still_parses_as_equality() {
    let engine = deploy(definition(
        "status",
        Some("string"),
        &[("\"manual\"", "\"m\"")],
    ));

    assert_eq!(
        run(&engine, json!({ "status": "manual" })),
        Some(json!("m"))
    );
    assert_eq!(run(&engine, json!({ "status": "auto" })), None);
}

#[test]
fn explicit_operators_are_preserved() {
    let engine = deploy(definition("code", Some("number"), &[("!= 7", "\"other\"")]));

    assert_eq!(run(&engine, json!({ "code": 8 })), Some(json!("other")));
    assert_eq!(run(&engine, json!({ "code": 7 })), None);
}
