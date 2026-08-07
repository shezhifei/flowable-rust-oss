use crate::agenda::FlowableEngineAgenda;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;

pub struct ManualTaskActivityBehavior;

impl Default for ManualTaskActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl ManualTaskActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for ManualTaskActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(execution.clone());
        Ok(())
    }
}
