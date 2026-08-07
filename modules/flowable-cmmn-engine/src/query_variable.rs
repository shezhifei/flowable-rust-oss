//! CMMN variable-condition matching for runtime queries.
//!
//! Java truth:
//! - `QueryVariable.java:74-96` — operation enum (friendly names)
//! - `BaseCaseInstanceResource.java:292-376` / `PlanItemInstanceBaseResource.java:141+`
//!   / `TaskBaseResource.java:360-444` — validation and operator dispatch
//! - `AbstractVariableQueryImpl.java:299-331` — bool/null banned from comparison ops;
//!   ignoreCase/like require string query values
//!
//! Incomparable-type policy (Rust, documented): when a comparison operator
//! (`greaterThan` / `lessThan` / …) is applied to values that are not both
//! numbers or both strings, the condition evaluates to **false** (no match).
//! Java routes this through typed SQL columns, so mixed-type rows simply do
//! not join; we mirror that "no match" outcome without raising.

use serde_json::{Map, Value};
use std::cmp::Ordering;

/// Java `QueryVariable.QueryVariableOperation` (QueryVariable.java:74-76).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryVariableOperation {
    Equals,
    NotEquals,
    EqualsIgnoreCase,
    NotEqualsIgnoreCase,
    GreaterThan,
    GreaterThanOrEquals,
    LessThan,
    LessThanOrEquals,
    Like,
    LikeIgnoreCase,
}

/// One AND-ed variable condition applied against a variable map.
///
/// Validation of illegal operations / nameLess / non-string ignoreCase is the
/// REST layer's job (→ 400). The engine assumes a well-formed condition.
#[derive(Debug, Clone)]
pub struct QueryVariableCondition {
    pub name: Option<String>,
    pub operation: QueryVariableOperation,
    pub value: Value,
}

/// Returns true when **every** condition matches the given variable map
/// (AND semantics, Java multi-variable query).
pub fn variables_match_conditions(
    variables: &Map<String, Value>,
    conditions: &[QueryVariableCondition],
) -> bool {
    conditions
        .iter()
        .all(|condition| condition_matches(variables, condition))
}

fn condition_matches(variables: &Map<String, Value>, condition: &QueryVariableCondition) -> bool {
    match condition.name.as_deref() {
        // nameLess equals only (validated at REST): any variable value equals.
        // Java CaseInstanceQuery.variableValueEquals(Object) (BaseCaseInstanceResource.java:312-314).
        None => variables
            .values()
            .any(|actual| value_matches(actual, condition.operation, &condition.value)),
        Some(name) => match variables.get(name) {
            // Missing named variable → no match for every operator (SQL INNER-JOIN
            // style; notEquals also requires the row to exist).
            None => false,
            Some(actual) => value_matches(actual, condition.operation, &condition.value),
        },
    }
}

fn value_matches(actual: &Value, operation: QueryVariableOperation, expected: &Value) -> bool {
    match operation {
        QueryVariableOperation::Equals => actual == expected,
        QueryVariableOperation::NotEquals => actual != expected,
        QueryVariableOperation::EqualsIgnoreCase => {
            string_eq_ignore_case(actual, expected)
        }
        QueryVariableOperation::NotEqualsIgnoreCase => {
            // Only defined when both sides are strings; non-string actual → false
            // (does not equal ignore-case, so "not equals ignore case" is true only
            // when both are strings and differ ignoring case).
            match (actual.as_str(), expected.as_str()) {
                (Some(a), Some(e)) => !a.eq_ignore_ascii_case(e),
                _ => false,
            }
        }
        QueryVariableOperation::Like => match (actual.as_str(), expected.as_str()) {
            (Some(a), Some(e)) => like_match(e, a),
            _ => false,
        },
        QueryVariableOperation::LikeIgnoreCase => match (actual.as_str(), expected.as_str()) {
            (Some(a), Some(e)) => like_match(&e.to_lowercase(), &a.to_lowercase()),
            _ => false,
        },
        QueryVariableOperation::GreaterThan => {
            compare_values(actual, expected) == Some(Ordering::Greater)
        }
        QueryVariableOperation::GreaterThanOrEquals => {
            matches!(
                compare_values(actual, expected),
                Some(Ordering::Greater | Ordering::Equal)
            )
        }
        QueryVariableOperation::LessThan => {
            compare_values(actual, expected) == Some(Ordering::Less)
        }
        QueryVariableOperation::LessThanOrEquals => {
            matches!(
                compare_values(actual, expected),
                Some(Ordering::Less | Ordering::Equal)
            )
        }
    }
}

fn string_eq_ignore_case(actual: &Value, expected: &Value) -> bool {
    match (actual.as_str(), expected.as_str()) {
        (Some(a), Some(e)) => a.eq_ignore_ascii_case(e),
        _ => false,
    }
}

/// Numeric (as f64) or string lexicographic comparison; otherwise `None`
/// (incomparable → comparison ops do not match).
fn compare_values(actual: &Value, expected: &Value) -> Option<Ordering> {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            let a = a.as_f64()?;
            let b = b.as_f64()?;
            a.partial_cmp(&b)
        }
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// SQL `LIKE` (`%` any sequence, `_` single char). Same algorithm as
/// `runtime::like_match` (runtime.rs:9339-9354).
fn like_match(pattern: &str, haystack: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, haystack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    fn cond(name: Option<&str>, op: QueryVariableOperation, value: Value) -> QueryVariableCondition {
        QueryVariableCondition {
            name: name.map(str::to_string),
            operation: op,
            value,
        }
    }

    #[test]
    fn equals_and_not_equals_numeric_string_bool() {
        let map = vars(&[
            ("n", json!(10)),
            ("s", json!("Hello")),
            ("b", json!(true)),
        ]);
        assert!(variables_match_conditions(
            &map,
            &[cond(Some("n"), QueryVariableOperation::Equals, json!(10))]
        ));
        assert!(!variables_match_conditions(
            &map,
            &[cond(Some("n"), QueryVariableOperation::Equals, json!(11))]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(
                Some("n"),
                QueryVariableOperation::NotEquals,
                json!(11)
            )]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(Some("s"), QueryVariableOperation::Equals, json!("Hello"))]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(Some("b"), QueryVariableOperation::Equals, json!(true))]
        ));
        assert!(!variables_match_conditions(
            &map,
            &[cond(Some("b"), QueryVariableOperation::Equals, json!(false))]
        ));
        assert!(!variables_match_conditions(
            &map,
            &[cond(
                Some("missing"),
                QueryVariableOperation::Equals,
                json!(1)
            )]
        ));
        // Missing named var also fails notEquals (no join row).
        assert!(!variables_match_conditions(
            &map,
            &[cond(
                Some("missing"),
                QueryVariableOperation::NotEquals,
                json!(1)
            )]
        ));
    }

    #[test]
    fn equals_ignore_case_and_not_equals_ignore_case() {
        let map = vars(&[("s", json!("Hello")), ("n", json!(1))]);
        assert!(variables_match_conditions(
            &map,
            &[cond(
                Some("s"),
                QueryVariableOperation::EqualsIgnoreCase,
                json!("hello")
            )]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(
                Some("s"),
                QueryVariableOperation::NotEqualsIgnoreCase,
                json!("world")
            )]
        ));
        assert!(!variables_match_conditions(
            &map,
            &[cond(
                Some("s"),
                QueryVariableOperation::NotEqualsIgnoreCase,
                json!("HELLO")
            )]
        ));
        // Non-string actual → ignoreCase equals false.
        assert!(!variables_match_conditions(
            &map,
            &[cond(
                Some("n"),
                QueryVariableOperation::EqualsIgnoreCase,
                json!("1")
            )]
        ));
    }

    #[test]
    fn greater_less_numeric_and_string() {
        let map = vars(&[("n", json!(10)), ("s", json!("m"))]);
        assert!(variables_match_conditions(
            &map,
            &[cond(
                Some("n"),
                QueryVariableOperation::GreaterThan,
                json!(5)
            )]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(
                Some("n"),
                QueryVariableOperation::GreaterThanOrEquals,
                json!(10)
            )]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(Some("n"), QueryVariableOperation::LessThan, json!(20))]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(
                Some("n"),
                QueryVariableOperation::LessThanOrEquals,
                json!(10)
            )]
        ));
        assert!(!variables_match_conditions(
            &map,
            &[cond(
                Some("n"),
                QueryVariableOperation::GreaterThan,
                json!(10)
            )]
        ));
        // String lexicographic.
        assert!(variables_match_conditions(
            &map,
            &[cond(
                Some("s"),
                QueryVariableOperation::GreaterThan,
                json!("a")
            )]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(Some("s"), QueryVariableOperation::LessThan, json!("z"))]
        ));
        // Incomparable (number vs string) → false.
        assert!(!variables_match_conditions(
            &map,
            &[cond(
                Some("n"),
                QueryVariableOperation::GreaterThan,
                json!("5")
            )]
        ));
        // Bool with comparison → false (incomparable).
        let map_bool = vars(&[("b", json!(true))]);
        assert!(!variables_match_conditions(
            &map_bool,
            &[cond(
                Some("b"),
                QueryVariableOperation::GreaterThan,
                json!(false)
            )]
        ));
    }

    #[test]
    fn like_and_like_ignore_case() {
        let map = vars(&[("s", json!("HelloWorld"))]);
        assert!(variables_match_conditions(
            &map,
            &[cond(Some("s"), QueryVariableOperation::Like, json!("Hello%"))]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(
                Some("s"),
                QueryVariableOperation::LikeIgnoreCase,
                json!("hello%")
            )]
        ));
        assert!(!variables_match_conditions(
            &map,
            &[cond(Some("s"), QueryVariableOperation::Like, json!("Nope%"))]
        ));
        // Single-char wildcard.
        assert!(variables_match_conditions(
            &map,
            &[cond(
                Some("s"),
                QueryVariableOperation::Like,
                json!("Hello_orld")
            )]
        ));
    }

    #[test]
    fn name_less_equals_any_value() {
        let map = vars(&[("a", json!(1)), ("b", json!("x"))]);
        assert!(variables_match_conditions(
            &map,
            &[cond(None, QueryVariableOperation::Equals, json!(1))]
        ));
        assert!(variables_match_conditions(
            &map,
            &[cond(None, QueryVariableOperation::Equals, json!("x"))]
        ));
        assert!(!variables_match_conditions(
            &map,
            &[cond(None, QueryVariableOperation::Equals, json!(99))]
        ));
    }

    #[test]
    fn and_semantics_across_multiple_conditions() {
        let map = vars(&[("n", json!(10)), ("s", json!("ok"))]);
        assert!(variables_match_conditions(
            &map,
            &[
                cond(Some("n"), QueryVariableOperation::Equals, json!(10)),
                cond(Some("s"), QueryVariableOperation::Equals, json!("ok")),
            ]
        ));
        assert!(!variables_match_conditions(
            &map,
            &[
                cond(Some("n"), QueryVariableOperation::Equals, json!(10)),
                cond(Some("s"), QueryVariableOperation::Equals, json!("no")),
            ]
        ));
    }
}
