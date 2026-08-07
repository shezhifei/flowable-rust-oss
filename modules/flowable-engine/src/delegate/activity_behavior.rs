use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;

/// Superclass for all 'connectable' BPMN 2.0 process elements
pub trait ActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError>;
}

pub trait TriggerableActivityBehavior: ActivityBehavior {
    fn trigger(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
        signal_name: Option<String>,
        signal_data: Option<serde_json::Value>,
    ) -> Result<(), FlowableError>;
}
