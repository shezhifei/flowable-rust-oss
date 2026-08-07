use crate::el::condition::Condition;
use crate::el::expression::Expression;
use flowable_engine_common::el::VariableContainer;

pub struct UelExpressionCondition {
    expression: Box<dyn Expression>,
}

impl UelExpressionCondition {
    pub fn new(expression: Box<dyn Expression>) -> Self {
        Self { expression }
    }
}

impl Condition for UelExpressionCondition {
    fn evaluate(
        &self,
        element_id: Option<&str>,
        scope: &dyn VariableContainer,
    ) -> Result<bool, crate::error::FlowableError> {
        match self.expression.get_value(scope) {
            Some(serde_json::Value::Bool(value)) => Ok(value),
            Some(value) => Err(crate::error::FlowableError::ExecutionError(format!(
                "Condition expression returns non-Boolean (elementId: {:?}): {}",
                element_id, value
            ))),
            None => Err(crate::error::FlowableError::ExecutionError(format!(
                "Condition expression returns non-Boolean (elementId: {:?}): null",
                element_id
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
        UelExpressionCondition::new(Box::new(SimpleExpression::new(expression.to_string())))
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
    fn null_result_is_an_execution_error() {
        let error = condition("${missing}")
            .evaluate(Some("flow1"), &Execution::default())
            .expect_err("a null condition result must fail the command");

        assert!(matches!(
            error,
            FlowableError::ExecutionError(message)
                if message.contains("non-Boolean")
                    && message.contains("flow1")
                    && message.ends_with("null")
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
