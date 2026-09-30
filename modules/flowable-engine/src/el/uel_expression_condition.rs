// Pre-existing `unwrap()` call(s), grandfathered by the workspace clippy ratchet
// (`[workspace.lints.clippy] unwrap_used = "warn"` in the root Cargo.toml). These
// sites predate the ratchet and were NOT individually audited against Java. The
// exemption is scoped with `cfg_attr(test, ...)`, so it covers only this file's
// `#[cfg(test)]` code; a NEW unwrap() in production code is still surfaced.
// Do not add more without an audit note.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use crate::el::condition::Condition;
use crate::el::expression::SimpleExpression;
use flowable_engine_common::el::VariableContainer;

pub struct UelExpressionCondition {
    expression: SimpleExpression,
}

impl UelExpressionCondition {
    pub fn new(expression: SimpleExpression) -> Self {
        Self { expression }
    }
}

impl Condition for UelExpressionCondition {
    /// Java `UelExpressionCondition.evaluate` (UelExpressionCondition.java:36-45)
    /// three-way classification (A.2 #43 / W6):
    /// 1. evaluation error → propagate (never disguised as null);
    /// 2. null result → `condition expression returns null (elementId: …)`;
    /// 3. non-Boolean → `condition expression returns non-Boolean (elementId: …)`.
    fn evaluate(
        &self,
        element_id: Option<&str>,
        scope: &dyn VariableContainer,
    ) -> Result<bool, crate::error::FlowableError> {
        match self.expression.get_value_strict(scope) {
            Err(error) => Err(crate::error::FlowableError::ExecutionError(format!(
                "condition expression failed (elementId: {element_id:?}): {error}"
            ))),
            Ok(Some(serde_json::Value::Bool(value))) => Ok(value),
            Ok(Some(serde_json::Value::Null)) | Ok(None) => {
                Err(crate::error::FlowableError::ExecutionError(format!(
                    "condition expression returns null (elementId: {element_id:?})"
                )))
            }
            Ok(Some(value)) => Err(crate::error::FlowableError::ExecutionError(format!(
                "condition expression returns non-Boolean (elementId: {element_id:?}): {value}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::el::expression::SimpleExpression;
    use crate::error::FlowableError;
    use crate::runtime::execution::Execution;
    use serde_json::json;

    fn condition(expression: &str) -> UelExpressionCondition {
        UelExpressionCondition::new(SimpleExpression::new(expression.to_string()))
    }

    #[test]
    fn boolean_result_is_returned() {
        let execution = Execution {
            variables: [("approved".to_string(), json!(true))].into(),
            ..Default::default()
        };

        assert!(
            condition("${approved}")
                .evaluate(Some("flow1"), &execution)
                .unwrap()
        );
    }

    #[test]
    fn defined_null_result_is_null_message() {
        let execution = Execution {
            variables: [("maybe".to_string(), json!(null))].into(),
            ..Default::default()
        };
        let error = condition("${maybe}")
            .evaluate(Some("flow1"), &execution)
            .expect_err("a null condition result must fail the command");

        assert!(matches!(
            error,
            FlowableError::ExecutionError(message)
                if message.contains("returns null")
                    && message.contains("flow1")
        ));
    }

    #[test]
    fn undefined_variable_is_eval_error_not_null() {
        // W1/W6: evaluation error must not be disguised as the null message.
        let error = condition("${missing}")
            .evaluate(Some("flow1"), &Execution::default())
            .expect_err("an undefined variable must fail the command");

        assert!(matches!(
            error,
            FlowableError::ExecutionError(message)
                if (message.contains("Unknown property") || message.contains("condition expression failed"))
                    && message.contains("flow1")
        ));
    }

    #[test]
    fn non_boolean_result_is_an_execution_error() {
        let execution = Execution {
            variables: [("decision".to_string(), json!("approve"))].into(),
            ..Default::default()
        };

        let error = condition("${decision}")
            .evaluate(Some("flow2"), &execution)
            .expect_err("a non-Boolean condition result must fail the command");

        assert!(matches!(
            error,
            FlowableError::ExecutionError(message)
                if message.contains("non-Boolean")
                    && message.contains("flow2")
                    && message.contains("approve")
        ));
    }
}
