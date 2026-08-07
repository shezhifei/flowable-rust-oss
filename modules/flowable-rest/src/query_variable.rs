//! Shared BPMN REST variable-condition matching for runtime and history
//! queries.
//!
//! Java truth:
//! - `QueryVariable.java:74-96` — operation enum (friendly names); `type`
//!   field (QueryVariable.java:66-71)
//! - `TaskBaseResource.java:384-468` / `BaseProcessInstanceResource.java:272+`
//!   / `ExecutionBaseResource.java:125+` / `HistoricProcessInstanceBaseResource.
//!   java:304+` / `HistoricTaskInstanceBaseResource.java:340+` — dispatch
//! - `AbstractVariableQueryImpl.java:299-329` — validation rules
//!
//! P108 choice (why shared here, not reused from the CMMN crate): the P103
//! `flowable-cmmn-engine::query_variable` offers `variables_match_conditions`
//! over a whole variable map, but the BPMN REST filters evaluate one candidate
//! variable at a time across task-local / execution / historic scopes.
//! flowable-rest already depends on flowable-cmmn-engine (Cargo.toml:18), yet
//! coupling BPMN REST to a CMMN crate's internals for this is semantically
//! awkward, so the helper is replicated inside flowable-rest next to the route
//! modules that own the per-file `QueryVariable` structs.
//!
//! `type` field note: accepted for JSON parity but does not drive value
//! conversion (same documented deviation as the CMMN side, cmmn.rs:1508-1512).
//! Java converts the query value through `RestResponseFactory.getVariableValue`
//! (RestResponseFactory.java:406-434) when `type` is set; here matching is
//! driven by the raw JSON value shape.

use crate::error::ApiError;
use serde_json::Value;
use std::cmp::Ordering;

/// Java `QueryVariable.QueryVariableOperation` (QueryVariable.java:74-76).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryVariableOperation {
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

impl QueryVariableOperation {
    /// Java `QueryVariableOperation.forFriendlyName` (QueryVariable.java:88-95).
    /// Returns `None` for unknown names; callers turn that into a 400.
    pub(crate) fn from_friendly_name(name: &str) -> Option<Self> {
        Some(match name {
            "equals" => Self::Equals,
            "notEquals" => Self::NotEquals,
            "equalsIgnoreCase" => Self::EqualsIgnoreCase,
            "notEqualsIgnoreCase" => Self::NotEqualsIgnoreCase,
            "greaterThan" => Self::GreaterThan,
            "greaterThanOrEquals" => Self::GreaterThanOrEquals,
            "lessThan" => Self::LessThan,
            "lessThanOrEquals" => Self::LessThanOrEquals,
            "like" => Self::Like,
            "likeIgnoreCase" => Self::LikeIgnoreCase,
            _ => return None,
        })
    }

    /// Java per-operator message clause for comparison ops
    /// (AbstractVariableQueryImpl.java:306-313).
    fn comparison_clause(self) -> &'static str {
        match self {
            Self::GreaterThan => "greater than",
            Self::GreaterThanOrEquals => "greater than or equal",
            Self::LessThan => "less than",
            Self::LessThanOrEquals => "less than or equal",
            _ => unreachable!("comparison_clause is only called for comparison ops"),
        }
    }
}

/// True when `actual` matches `expected` under `operation`.
///
/// Incomparable-type policy: when a comparison operator (`greaterThan` /
/// `lessThan` / …) is applied to values that are not both numbers or both
/// strings, the condition evaluates to **false** (no match). Java routes this
/// through typed SQL columns, so mixed-type rows simply do not join; we mirror
/// that "no match" outcome without raising (P103 query_variable.rs:10-14).
pub(crate) fn value_matches(
    actual: &Value,
    operation: QueryVariableOperation,
    expected: &Value,
) -> bool {
    match operation {
        QueryVariableOperation::Equals => actual == expected,
        QueryVariableOperation::NotEquals => actual != expected,
        QueryVariableOperation::EqualsIgnoreCase => string_eq_ignore_case(actual, expected),
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
        QueryVariableOperation::LessThan => compare_values(actual, expected) == Some(Ordering::Less),
        QueryVariableOperation::LessThanOrEquals => {
            matches!(
                compare_values(actual, expected),
                Some(Ordering::Less | Ordering::Equal)
            )
        }
    }
}

/// Name-less queries (value-only, no variable name) are only allowed for
/// `equals` (TaskBaseResource.java:399-401). The historic task query rejects
/// name-less filters outright and enforces that separately.
pub(crate) fn validate_name_less_equals(
    name: Option<&str>,
    operation: QueryVariableOperation,
) -> Result<(), ApiError> {
    if name.is_none() && operation != QueryVariableOperation::Equals {
        return Err(ApiError::bad_request(
            "Value-only query (without a variable-name) is only supported when using 'equals' operation.",
        ));
    }
    Ok(())
}

/// REST-layer value validation shared by every query endpoint.
///
/// - ignoreCase / like require string query values
///   (TaskBaseResource.java:413-462).
/// - Booleans and null are banned from comparison ops
///   (AbstractVariableQueryImpl.java:303-316).
pub(crate) fn validate_operation_value(
    operation: QueryVariableOperation,
    value: &Value,
) -> Result<(), ApiError> {
    if matches!(
        operation,
        QueryVariableOperation::EqualsIgnoreCase | QueryVariableOperation::NotEqualsIgnoreCase
    ) && !value.is_string()
    {
        return Err(ApiError::bad_request(format!(
            "Only string variable values are supported when ignoring casing, but was: {}",
            json_value_type_name(value)
        )));
    }
    if matches!(
        operation,
        QueryVariableOperation::Like | QueryVariableOperation::LikeIgnoreCase
    ) && !value.is_string()
    {
        return Err(ApiError::bad_request(format!(
            "Only string variable values are supported using like, but was: {}",
            json_value_type_name(value)
        )));
    }
    if matches!(
        operation,
        QueryVariableOperation::GreaterThan
            | QueryVariableOperation::GreaterThanOrEquals
            | QueryVariableOperation::LessThan
            | QueryVariableOperation::LessThanOrEquals
    ) && (value.is_null() || value.is_boolean())
    {
        return Err(ApiError::bad_request(format!(
            "Booleans and null cannot be used in '{}' condition",
            operation.comparison_clause()
        )));
    }
    Ok(())
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

/// SQL `LIKE` (`%` any sequence, `_` single char). Delegates to the shared
/// O(pattern × value) implementation with the 512-char input cap
/// (`routes::tasks::sql_like_matches`); the former recursive matcher here had
/// exponential worst cases on `%`-heavy patterns.
fn like_match(pattern: &str, haystack: &str) -> bool {
    crate::routes::tasks::sql_like_matches(pattern, haystack)
}

/// Java `RestVariable.type`-style name for an error message value
/// (RestVariableConverter.getRestTypeName parity).
pub(crate) fn json_value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "double",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(name: &str) -> Option<QueryVariableOperation> {
        QueryVariableOperation::from_friendly_name(name)
    }

    #[test]
    fn friendly_names_cover_all_ten_operations() {
        assert_eq!(parse("equals"), Some(QueryVariableOperation::Equals));
        assert_eq!(parse("notEquals"), Some(QueryVariableOperation::NotEquals));
        assert_eq!(
            parse("equalsIgnoreCase"),
            Some(QueryVariableOperation::EqualsIgnoreCase)
        );
        assert_eq!(
            parse("notEqualsIgnoreCase"),
            Some(QueryVariableOperation::NotEqualsIgnoreCase)
        );
        assert_eq!(parse("like"), Some(QueryVariableOperation::Like));
        assert_eq!(
            parse("likeIgnoreCase"),
            Some(QueryVariableOperation::LikeIgnoreCase)
        );
        assert_eq!(parse("greaterThan"), Some(QueryVariableOperation::GreaterThan));
        assert_eq!(
            parse("greaterThanOrEquals"),
            Some(QueryVariableOperation::GreaterThanOrEquals)
        );
        assert_eq!(parse("lessThan"), Some(QueryVariableOperation::LessThan));
        assert_eq!(
            parse("lessThanOrEquals"),
            Some(QueryVariableOperation::LessThanOrEquals)
        );
        assert_eq!(parse("contains"), None);
        assert_eq!(parse("LIKE"), None);
    }

    // Typed comparison matrix — ported from the P103 CMMN engine
    // (flowable-cmmn-engine/src/query_variable.rs:181-410).

    #[test]
    fn equals_and_not_equals_numeric_string_bool() {
        assert!(value_matches(&json!(10), QueryVariableOperation::Equals, &json!(10)));
        assert!(!value_matches(&json!(10), QueryVariableOperation::Equals, &json!(11)));
        assert!(value_matches(&json!(10), QueryVariableOperation::NotEquals, &json!(11)));
        assert!(!value_matches(&json!(10), QueryVariableOperation::NotEquals, &json!(10)));
        assert!(value_matches(&json!("Hello"), QueryVariableOperation::Equals, &json!("Hello")));
        assert!(value_matches(&json!(true), QueryVariableOperation::Equals, &json!(true)));
        assert!(!value_matches(&json!(true), QueryVariableOperation::Equals, &json!(false)));
        // Number vs string never equals.
        assert!(!value_matches(&json!(5), QueryVariableOperation::Equals, &json!("5")));
    }

    #[test]
    fn equals_ignore_case_and_not_equals_ignore_case() {
        assert!(value_matches(
            &json!("Hello"),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("hello")
        ));
        assert!(value_matches(
            &json!("Hello"),
            QueryVariableOperation::NotEqualsIgnoreCase,
            &json!("world")
        ));
        assert!(!value_matches(
            &json!("Hello"),
            QueryVariableOperation::NotEqualsIgnoreCase,
            &json!("HELLO")
        ));
        // Non-string actual → ignoreCase equals false.
        assert!(!value_matches(
            &json!(1),
            QueryVariableOperation::EqualsIgnoreCase,
            &json!("1")
        ));
        // Non-string actual → notEqualsIgnoreCase also false (both-sides rule).
        assert!(!value_matches(
            &json!(1),
            QueryVariableOperation::NotEqualsIgnoreCase,
            &json!("1")
        ));
    }

    #[test]
    fn greater_less_numeric_and_string() {
        assert!(value_matches(&json!(10), QueryVariableOperation::GreaterThan, &json!(5)));
        assert!(value_matches(
            &json!(10),
            QueryVariableOperation::GreaterThanOrEquals,
            &json!(10)
        ));
        assert!(value_matches(&json!(10), QueryVariableOperation::LessThan, &json!(20)));
        assert!(value_matches(
            &json!(10),
            QueryVariableOperation::LessThanOrEquals,
            &json!(10)
        ));
        assert!(!value_matches(&json!(10), QueryVariableOperation::GreaterThan, &json!(10)));
        assert!(!value_matches(&json!(10), QueryVariableOperation::LessThan, &json!(10)));
        // String lexicographic.
        assert!(value_matches(&json!("m"), QueryVariableOperation::GreaterThan, &json!("a")));
        assert!(value_matches(&json!("m"), QueryVariableOperation::LessThan, &json!("z")));
        // Incomparable (number vs string) → false.
        assert!(!value_matches(&json!(10), QueryVariableOperation::GreaterThan, &json!("5")));
        // Bool with comparison → false (incomparable).
        assert!(!value_matches(&json!(true), QueryVariableOperation::GreaterThan, &json!(false)));
    }

    #[test]
    fn like_and_like_ignore_case() {
        assert!(value_matches(&json!("HelloWorld"), QueryVariableOperation::Like, &json!("Hello%")));
        assert!(value_matches(
            &json!("HelloWorld"),
            QueryVariableOperation::LikeIgnoreCase,
            &json!("hello%")
        ));
        assert!(!value_matches(&json!("HelloWorld"), QueryVariableOperation::Like, &json!("Nope%")));
        // Single-char wildcard.
        assert!(value_matches(&json!("HelloWorld"), QueryVariableOperation::Like, &json!("Hello_orld")));
        // `%` in the middle.
        assert!(value_matches(&json!("abcXdef"), QueryVariableOperation::Like, &json!("abc%def")));
        // Non-string actual → like false.
        assert!(!value_matches(&json!(7), QueryVariableOperation::Like, &json!("%")));
    }

    #[test]
    fn name_less_equals_any_value_uses_same_matrix() {
        // nameLess equals is implemented by callers as "any variable equals";
        // the value matrix itself is unchanged.
        assert!(value_matches(&json!(1), QueryVariableOperation::Equals, &json!(1)));
        assert!(!value_matches(&json!(1), QueryVariableOperation::Equals, &json!(99)));
    }

    #[test]
    fn string_ops_reject_non_string_query_values() {
        for (operation, detail) in [
            (QueryVariableOperation::EqualsIgnoreCase, "when ignoring casing"),
            (QueryVariableOperation::NotEqualsIgnoreCase, "when ignoring casing"),
            (QueryVariableOperation::Like, "using like"),
            (QueryVariableOperation::LikeIgnoreCase, "using like"),
        ] {
            let error = validate_operation_value(operation, &json!(7)).unwrap_err();
            assert!(
                matches!(error, ApiError::BadRequest(message) if message.contains(detail)),
                "op {operation:?} detail was: {detail}"
            );
        }
        assert!(validate_operation_value(QueryVariableOperation::EqualsIgnoreCase, &json!("ok")).is_ok());
    }

    #[test]
    fn bool_and_null_rejected_for_comparison_ops() {
        for (operation, clause) in [
            (QueryVariableOperation::GreaterThan, "greater than"),
            (QueryVariableOperation::GreaterThanOrEquals, "greater than or equal"),
            (QueryVariableOperation::LessThan, "less than"),
            (QueryVariableOperation::LessThanOrEquals, "less than or equal"),
        ] {
            for value in [Value::Bool(true), Value::Null] {
                let error = validate_operation_value(operation, &value).unwrap_err();
                assert!(
                    matches!(error, ApiError::BadRequest(message) if message == format!("Booleans and null cannot be used in '{clause}' condition")),
                    "op {operation:?} value {value:?}"
                );
            }
        }
        // Numeric comparison values are fine.
        assert!(validate_operation_value(QueryVariableOperation::GreaterThan, &json!(5)).is_ok());
        // equals/notEquals accept booleans.
        assert!(validate_operation_value(QueryVariableOperation::Equals, &json!(true)).is_ok());
    }
}
