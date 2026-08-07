use crate::agenda::FlowableEngineAgenda;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;

pub struct StartEventActivityBehavior;

impl Default for StartEventActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl StartEventActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for StartEventActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // In real engine: find outgoing sequence flow and take it
        // By scheduling TakeOutgoingSequenceFlowsOperation, the engine will process conditions and sequences
        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(execution.clone());
        Ok(())
    }
}
