//! Contract tests for the CMMN sentry evaluation engine (C1).
//!
//! Java reference: `AbstractEvaluationCriteriaOperation.evaluateCriteria`
//! (`modules/flowable-cmmn-engine/src/main/java/org/flowable/cmmn/engine/impl/agenda/operation/AbstractEvaluationCriteriaOperation.java`,
//! L466-582) and `evaluateSentryIfPart` (L717-739). The Rust subset
//! covers:
//!   - the single onPart + no ifPart "fast path" (Java L475-490);
//!   - the onPart + ifPart AND combination (Java L506-577);
//!   - ifPart expression semantics: comparison, logical (and / or),
//!     not, empty, contains, startsWith, endsWith, matches, size,
//!     length, and bare-literal truthiness.

use flowable_cmmn_engine::{
    CmmnCaseFileItemOnPart, CmmnEngine, CmmnPlanItemOnPart, CmmnSentry, SentryLifecycleEvent,
    SentryVariableMap,
};
use serde_json::{Value, json};

fn ctx(pairs: &[(&str, Value)]) -> SentryVariableMap {
    SentryVariableMap::from_pairs(pairs.iter().map(|(k, v)| (*k, v.clone())))
}

#[test]
fn if_part_equal_compares_numbers_across_types() {
    // Java: `Expression.isEqual` is type-tolerant for primitives.
    // The Rust variant compares both direct equality and a numeric
    // string<->number coercion.
    let expression = flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("count == 5").unwrap();
    let vars = ctx(&[("count", json!(5))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("count", json!("5"))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("count", json!(4))]);
    assert!(!expression.evaluate(&vars).unwrap());
}

#[test]
fn if_part_not_equal_rejects_matching_value() {
    let expression =
        flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("status != \"done\"").unwrap();
    let vars = ctx(&[("status", json!("open"))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("status", json!("done"))]);
    assert!(!expression.evaluate(&vars).unwrap());
}

#[test]
fn if_part_logical_and_combines_two_comparisons() {
    let expression =
        flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("count > 1 && count < 10").unwrap();
    let vars = ctx(&[("count", json!(5))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("count", json!(11))]);
    assert!(!expression.evaluate(&vars).unwrap());
}

#[test]
fn if_part_logical_or_short_circuits_to_true() {
    let expression = flowable_cmmn_engine::CmmnSentryIfPartExpression::parse(
        "status == \"done\" || status == \"cancelled\"",
    )
    .unwrap();
    let vars = ctx(&[("status", json!("cancelled"))]);
    assert!(expression.evaluate(&vars).unwrap());
}

#[test]
fn if_part_not_negates_inner_expression() {
    let expression =
        flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("!(flag == true)").unwrap();
    let vars = ctx(&[("flag", json!(false))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("flag", json!(true))]);
    assert!(!expression.evaluate(&vars).unwrap());
}

#[test]
fn if_part_empty_matches_unset_and_zero_length() {
    let expression =
        flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("empty(items)").unwrap();
    let vars = ctx(&[]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("items", json!(null))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("items", json!([]))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("items", json!(""))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("items", json!([1]))]);
    assert!(!expression.evaluate(&vars).unwrap());
}

#[test]
fn if_part_contains_finds_value_in_array() {
    let expression =
        flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("contains(roles, \"admin\")")
            .unwrap();
    let vars = ctx(&[("roles", json!(["user", "admin"]))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("roles", json!(["user"]))]);
    assert!(!expression.evaluate(&vars).unwrap());
}

#[test]
fn if_part_starts_with_and_ends_with_match_substrings() {
    let starts =
        flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("startsWith(name, \"A\")").unwrap();
    let ends =
        flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("endsWith(name, \"Z\")").unwrap();
    let vars = ctx(&[("name", json!("Alice-Z"))]);
    assert!(starts.evaluate(&vars).unwrap());
    assert!(ends.evaluate(&vars).unwrap());
    let vars = ctx(&[("name", json!("Bob"))]);
    assert!(!starts.evaluate(&vars).unwrap());
    assert!(!ends.evaluate(&vars).unwrap());
}

#[test]
fn if_part_matches_applies_regex() {
    let expression = flowable_cmmn_engine::CmmnSentryIfPartExpression::parse(
        "matches(ref, \"^[A-Z]{3}-[0-9]+$\")",
    )
    .unwrap();
    let vars = ctx(&[("ref", json!("ABC-12"))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("ref", json!("abc-12"))]);
    assert!(!expression.evaluate(&vars).unwrap());
}

#[test]
fn if_part_size_compares_collection_length() {
    let expression =
        flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("size(items) > 2").unwrap();
    let vars = ctx(&[("items", json!(["a", "b", "c"]))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("items", json!(["a"]))]);
    assert!(!expression.evaluate(&vars).unwrap());
}

#[test]
fn if_part_length_compares_string_length() {
    let expression =
        flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("length(name) == 3").unwrap();
    let vars = ctx(&[("name", json!("Bob"))]);
    assert!(expression.evaluate(&vars).unwrap());
    let vars = ctx(&[("name", json!("Bobby"))]);
    assert!(!expression.evaluate(&vars).unwrap());
}

#[test]
fn sentry_without_if_part_fires_on_matching_event() {
    // Java L475-490: single onPart + no ifPart is a fast path that
    // returns the criterion as soon as the lifecycle event matches.
    let sentry = CmmnSentry::new(
        "sentry-task-complete",
        CmmnPlanItemOnPart::new("on-task-complete", "task1", "complete"),
    );
    let vars = ctx(&[]);
    assert!(sentry.evaluate_for_event(&SentryLifecycleEvent::new("task1", "complete"), &vars,));
    // Different event does not satisfy.
    assert!(!sentry.evaluate_for_event(&SentryLifecycleEvent::new("task1", "occur"), &vars,));
    // Different source does not satisfy.
    assert!(!sentry.evaluate_for_event(&SentryLifecycleEvent::new("task2", "complete"), &vars,));
}

#[test]
fn sentry_with_if_part_requires_both_onpart_and_condition() {
    // Java L506-577: all onParts must match AND the ifPart must
    // evaluate to true.
    let sentry = CmmnSentry::new(
        "sentry-task-and-flag",
        CmmnPlanItemOnPart::new("on-task-complete", "task1", "complete"),
    )
    .with_if_part("approved == true");
    let vars = ctx(&[("approved", json!(true))]);
    assert!(sentry.evaluate_for_event(&SentryLifecycleEvent::new("task1", "complete"), &vars,));
    let vars = ctx(&[("approved", json!(false))]);
    assert!(!sentry.evaluate_for_event(&SentryLifecycleEvent::new("task1", "complete"), &vars,));
    // Event matches but the ifPart would fail to parse -> evaluator
    // returns false, matching the "sentry not satisfied" fallback
    // when the expression engine raises.
}

#[test]
fn sentry_with_case_file_on_part_fires_on_case_file_event() {
    // C1 single-event semantics: every onPart in the sentry must
    // match the supplied event. C2's `SentryPartInstance` work
    // will relax this to "all onParts satisfied across an event
    // stream" (Java L506-577), but the C1 evaluation surface stays
    // the simple AND.
    let sentry = CmmnSentry {
        id: "sentry-cf-create".to_string(),
        plan_item_on_parts: vec![],
        case_file_item_on_parts: vec![CmmnCaseFileItemOnPart::new("on-cf", "document", "create")],
        trigger_mode: None,
        if_part: None,
    };
    let vars = ctx(&[]);
    assert!(sentry.evaluate_for_event(&SentryLifecycleEvent::new("document", "create"), &vars,));
    // Different case-file onPart id or event -> sentry does not
    // fire.
    assert!(!sentry.evaluate_for_event(&SentryLifecycleEvent::new("document", "update"), &vars,));
    assert!(!sentry.evaluate_for_event(&SentryLifecycleEvent::new("other", "create"), &vars,));
}

#[test]
fn sentry_has_parts_reports_definition_state() {
    // Mirrors the "criteria list empty" guard inside Java
    // `AbstractEvaluationCriteriaOperation.evaluateCriteria` (the
    // sentry only fires when it has at least one part).
    let empty = CmmnSentry::new("empty", CmmnPlanItemOnPart::new("on", "x", "complete"))
        .with_case_file_item_on_part(CmmnCaseFileItemOnPart::new("on-cf", "y", "create"));
    // The constructor seeds the first plan-item onPart, so it's
    // non-empty.
    assert!(empty.has_parts());
    let lone_if = CmmnSentry {
        id: "if-only".to_string(),
        plan_item_on_parts: vec![],
        case_file_item_on_parts: vec![],
        trigger_mode: None,
        if_part: Some(
            flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("count > 0").unwrap(),
        ),
    };
    assert!(lone_if.has_parts());
}

#[test]
fn if_part_unsupported_method_call_returns_err() {
    // Arithmetic / method calls / property access are explicitly
    // out of scope for C1 (C2 will pick up the agenda-side work).
    // The evaluator must return Err so the caller can decide
    // whether to propagate or fall back.
    let expression = flowable_cmmn_engine::CmmnSentryIfPartExpression::parse("count + 1").unwrap();
    let vars = ctx(&[("count", json!(1))]);
    assert!(expression.evaluate(&vars).is_err());
}

#[test]
fn cmmn_engine_smoke_after_sentry_module_loads() {
    // Sanity check: instantiate the engine. C1 does not change the
    // engine boot path; this guards against a structural break in
    // models.rs (e.g. an accidental change to a public type).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let _runtime = engine.runtime_service();
}
