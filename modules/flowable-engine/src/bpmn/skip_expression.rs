use crate::el::expression::{Expression, SimpleExpression};
use crate::error::FlowableError;
use crate::runtime::execution::Execution;
use serde_json::Value;

const ACTIVITI_SKIP_EXPRESSION_ENABLED: &str = "_ACTIVITI_SKIP_EXPRESSION_ENABLED";
const FLOWABLE_SKIP_EXPRESSION_ENABLED: &str = "_FLOWABLE_SKIP_EXPRESSION_ENABLED";

/// Java `TakeOutgoingSequenceFlowsOperation.java:215-228`: a sequence flow
/// whose skipExpression is enabled is selected directly, skipping condition
/// evaluation. With exactly one outgoing flow the flow is selected regardless
/// of the skip expression's value; otherwise the skip expression must evaluate
/// to true.
pub(crate) fn should_skip_sequence_flow(
    outgoing_flow_count: usize,
    skip_expression: Option<&str>,
    flow_id: Option<&str>,
    execution: &Execution,
) -> Result<bool, FlowableError> {
    let Some(skip_expression) = skip_expression else {
        return Ok(false);
    };
    // Java `SkipExpressionUtil.isSkipExpressionEnabled` (:30-46): the skip
    // machinery is inert unless `_ACTIVITI_SKIP_EXPRESSION_ENABLED` /
    // `_FLOWABLE_SKIP_EXPRESSION_ENABLED` resolves to true.
    if !is_skip_expression_enabled(flow_id, execution)? {
        return Ok(false);
    }
    if outgoing_flow_count == 1 {
        return Ok(true);
    }
    should_skip_flow_element(Some(skip_expression), "SequenceFlow", flow_id, execution)
}

pub(crate) fn should_skip_flow_element(
    skip_expression: Option<&str>,
    element_type: &str,
    activity_id: Option<&str>,
    execution: &Execution,
) -> Result<bool, FlowableError> {
    let Some(skip_expression) = skip_expression else {
        return Ok(false);
    };

    if !is_skip_expression_enabled(activity_id, execution)? {
        return Ok(false);
    }

    match SimpleExpression::new(skip_expression.to_string()).get_value(execution) {
        Some(Value::Bool(value)) => Ok(value),
        Some(value) => Err(FlowableError::ExecutionError(format!(
            "{element_type} '{}' skipExpression must evaluate to a boolean, got {value}",
            activity_id.unwrap_or("<unknown>")
        ))),
        None => Err(FlowableError::ExecutionError(format!(
            "{element_type} '{}' skipExpression did not resolve to a boolean: {skip_expression}",
            activity_id.unwrap_or("<unknown>")
        ))),
    }
}

pub(crate) fn is_skip_expression_enabled(
    activity_id: Option<&str>,
    execution: &Execution,
) -> Result<bool, FlowableError> {
    match execution.process_variable(ACTIVITI_SKIP_EXPRESSION_ENABLED) {
        Some(Value::Bool(value)) => return Ok(value),
        Some(value) => return Err(skip_expression_enabled_type_error(activity_id, value)),
        None => {}
    }

    match execution.process_variable(FLOWABLE_SKIP_EXPRESSION_ENABLED) {
        Some(Value::Bool(value)) => Ok(value),
        Some(value) => Err(skip_expression_enabled_type_error(activity_id, value)),
        None => Ok(false),
    }
}

fn skip_expression_enabled_type_error(activity_id: Option<&str>, value: Value) -> FlowableError {
    FlowableError::ExecutionError(format!(
        "Skip expression variable for activity '{}' does not resolve to a boolean: {value}",
        activity_id.unwrap_or("<unknown>")
    ))
}
