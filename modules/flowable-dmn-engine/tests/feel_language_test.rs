use flowable_dmn_engine::FeelExpressionEngine;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn evaluates_context_paths_lists_and_conditionals() {
    let engine = FeelExpressionEngine::new();
    let context = HashMap::new();
    assert_eq!(
        engine
            .evaluate("{customer: {age: 42}}.customer.age", &context)
            .unwrap(),
        json!(42)
    );
    assert_eq!(
        engine
            .evaluate("if 2 + 2 = 4 then [1, 2, 3] else []", &context)
            .unwrap(),
        json!([1, 2, 3])
    );
}

#[test]
fn evaluates_for_and_quantified_expressions() {
    let engine = FeelExpressionEngine::new();
    let context = HashMap::new();
    assert_eq!(
        engine
            .evaluate("for x in [1, 2, 3] return x * 2", &context)
            .unwrap(),
        json!([2, 4, 6])
    );
    assert_eq!(
        engine
            .evaluate("some x in [1, 2, 3] satisfies x > 2", &context)
            .unwrap(),
        json!(true)
    );
    assert_eq!(
        engine
            .evaluate("every x in [1, 2, 3] satisfies x > 0", &context)
            .unwrap(),
        json!(true)
    );
}

#[test]
fn evaluates_ranges_and_membership() {
    let engine = FeelExpressionEngine::new();
    let context = HashMap::new();
    assert_eq!(
        engine.evaluate("2 in [1, 2, 3]", &context).unwrap(),
        json!(true)
    );
    assert_eq!(
        engine.evaluate("[1..5]", &context).unwrap(),
        json!({"start":1,"end":5,"startInclusive":true,"endInclusive":true})
    );
}
