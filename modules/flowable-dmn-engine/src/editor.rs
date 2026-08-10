//! Public DMN editor capability and validation boundary.
//!
//! The modeler must not maintain a second FEEL or unary-test parser. This
//! module deliberately delegates to the same conversion, normalization,
//! structural validation, and expression evaluation code used by deployment
//! and runtime execution.

use std::collections::HashMap;

use flowable_dmn_model::DmnDefinition;
use serde::Serialize;
use serde_json::Value;

use crate::error::DmnError;
use crate::models::{DmnDeploymentRequest, DmnModel, FeelExpressionEngine};
use crate::repository::validate_and_normalize_deployment_request;

/// Hit policies offered when creating a new decision table in the first-party
/// editor. `COMPLETE` is intentionally excluded from creation but remains a
/// supported round-trip value for imported models.
pub const EDITOR_CREATABLE_HIT_POLICIES: &[&str] = &[
    "FIRST",
    "UNIQUE",
    "ANY",
    "COLLECT",
    "RULE_ORDER",
    "OUTPUT_ORDER",
    "PRIORITY",
];

/// Canonical hit policies that the editor can load, validate, and save without
/// changing their meaning.
pub const EDITOR_ROUND_TRIP_HIT_POLICIES: &[&str] = &[
    "FIRST",
    "UNIQUE",
    "ANY",
    "COLLECT",
    "RULE_ORDER",
    "OUTPUT_ORDER",
    "PRIORITY",
    "COMPLETE",
];

pub const EDITOR_COLLECT_OPERATORS: &[&str] = &["COUNT", "SUM", "MIN", "MAX"];

/// Operators accepted by the runtime expression boundary. Some are handled by
/// the typed parser and some (`%`) by its compatibility fallback.
pub const EDITOR_OUTPUT_EXPRESSION_OPERATORS: &[&str] = &[
    "+", "-", "*", "/", "**", "%", "and", "or", "=", "!=", "<", "<=", ">", ">=", "in",
];

/// Function spellings accepted by [`validate_editor_expression`]. This is an
/// editor hint catalogue, not a replacement grammar: validation still goes
/// through [`FeelExpressionEngine`].
pub const EDITOR_OUTPUT_EXPRESSION_FUNCTIONS: &[&str] = &[
    "abs",
    "ceiling",
    "ceil",
    "floor",
    "round",
    "sqrt",
    "modulo",
    "decimal",
    "even",
    "odd",
    "contains",
    "starts with",
    "ends with",
    "matches",
    "string length",
    "upper case",
    "lower case",
    "substring",
    "replace",
    "trim",
    "append",
    "concatenate",
    "count",
    "distinct values",
    "flatten",
    "reverse",
    "list contains",
    "index of",
    "sublist",
    "union",
    "intersect",
    "except",
    "sum",
    "mean",
    "min",
    "max",
    "now",
    "today",
    "fn_date",
    "fn_now",
    "fn_addDate",
    "fn_subtractDate",
    "date:toDate",
    "date:now",
    "date:addDate",
    "date:subtractDate",
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
];

/// Stable names for the input-cell unary-test forms recognized by canonical
/// model conversion. They are deliberately descriptive rather than a grammar;
/// the actual parser remains the authority.
pub const EDITOR_INPUT_UNARY_TEST_FORMS: &[&str] = &[
    "blank-or-dash",
    "literal-or-equality",
    "comparison",
    "open-or-closed-range",
    "comma-separated-alternatives",
    "not",
    "string-predicate",
    "string-transform",
    "substring",
    "replace",
    "list-contains",
    "in-list",
    "el-condition",
    "property-path",
    "temporal-or-duration",
    "date-alias-comparison",
];

pub const EDITOR_VALUE_TYPE_REFS: &[&str] = &[
    "string",
    "boolean",
    "integer",
    "long",
    "double",
    "number",
    "date",
    "time",
    "dateTime",
    "duration",
    "dayTimeDuration",
    "yearMonthDuration",
    "context",
    "list",
];

/// Machine-readable hints for building a constrained editor. Passing these
/// hints never substitutes for calling the validation functions below.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DmnEditorCapabilities {
    pub creatable_hit_policies: &'static [&'static str],
    pub round_trip_hit_policies: &'static [&'static str],
    pub collect_operators: &'static [&'static str],
    pub output_expression_operators: &'static [&'static str],
    pub output_expression_functions: &'static [&'static str],
    pub input_unary_test_forms: &'static [&'static str],
    pub value_type_refs: &'static [&'static str],
}

pub const fn editor_capabilities() -> DmnEditorCapabilities {
    DmnEditorCapabilities {
        creatable_hit_policies: EDITOR_CREATABLE_HIT_POLICIES,
        round_trip_hit_policies: EDITOR_ROUND_TRIP_HIT_POLICIES,
        collect_operators: EDITOR_COLLECT_OPERATORS,
        output_expression_operators: EDITOR_OUTPUT_EXPRESSION_OPERATORS,
        output_expression_functions: EDITOR_OUTPUT_EXPRESSION_FUNCTIONS,
        input_unary_test_forms: EDITOR_INPUT_UNARY_TEST_FORMS,
        value_type_refs: EDITOR_VALUE_TYPE_REFS,
    }
}

/// Validate a canonical editor definition with the exact parser,
/// type-normalization, and structural checks used before a deployment is
/// persisted. The input is cloned, so imported values such as `COMPLETE` are
/// never rewritten in the editor document.
pub fn validate_editor_definition(definition: &DmnDefinition) -> Result<(), DmnError> {
    let model = DmnModel::try_from(definition.clone())?;
    let mut request = DmnDeploymentRequest::new("modeler-validation")
        .with_resource("modeler-validation.dmn", model);
    validate_and_normalize_deployment_request(&mut request)
}

/// Validate an output expression against the actual typed-plus-compatibility
/// runtime evaluator and a caller-supplied context.
///
/// A context is required because expressions such as `mean(scores)` are valid
/// but cannot be evaluated meaningfully against an empty variable map. Model
/// validation therefore validates input unary tests and deployment structure;
/// interactive expression previews should call this function with sample
/// values for every referenced input.
pub fn validate_editor_expression(
    expression: &str,
    context: &HashMap<String, Value>,
) -> Result<(), DmnError> {
    FeelExpressionEngine::new()
        .evaluate(expression, context)
        .map(|_| ())
}

/// Evaluate an editor expression through the same runtime boundary. This is
/// useful for preview UIs that need the computed value as well as validity.
pub fn evaluate_editor_expression(
    expression: &str,
    context: &HashMap<String, Value>,
) -> Result<Value, DmnError> {
    FeelExpressionEngine::new().evaluate(expression, context)
}
