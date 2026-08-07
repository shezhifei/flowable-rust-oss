use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;

/// A command that can be executed by the command executor
pub trait Command<T> {
    fn execute(&self, command_context: &mut CommandContext) -> Result<T, FlowableError>;
}
