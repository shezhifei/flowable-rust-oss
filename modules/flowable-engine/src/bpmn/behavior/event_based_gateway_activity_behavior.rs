use crate::agenda::FlowableEngineAgenda;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;

pub struct EventBasedGatewayActivityBehavior;

impl Default for EventBasedGatewayActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBasedGatewayActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for EventBasedGatewayActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // Standard Event-based gateway: flow to all outgoing intermediate catch events.
        // Each catch event will register its own subscription and wait.
        // We mark the gateway execution as a scope (if not already) to manage children.
        execution.is_scope = true;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(execution.clone());

        Ok(())
    }
}
