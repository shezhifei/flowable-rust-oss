use crate::agenda::FlowableEngineAgenda;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;

pub struct ExclusiveGatewayActivityBehavior;

impl Default for ExclusiveGatewayActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl ExclusiveGatewayActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for ExclusiveGatewayActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // In real engine:
        // 1. Evaluate conditions on outgoing sequence flows.
        // 2. Select the first sequence flow whose condition evaluates to true, or the default flow.
        // 3. Plan TakeOutgoingSequenceFlowsOperation with the selected sequence flow.

        // For simulation, we just plan the take outgoing operation.
        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(execution.clone());
        Ok(())
    }
}
