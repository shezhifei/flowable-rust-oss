use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::runtime_store::RuntimeEventWaitState;
use std::sync::Arc;

pub struct GetEventSubscriptionsCmd {
    process_instance_id: String,
}

impl GetEventSubscriptionsCmd {
    pub fn new(process_instance_id: String) -> Self {
        Self {
            process_instance_id,
        }
    }
}

impl Command<Vec<RuntimeEventWaitState>> for GetEventSubscriptionsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<RuntimeEventWaitState>, crate::error::FlowableError> {
        Ok(command_context
            .runtime_store
            .find_event_wait_states_by_process_instance_id(
                &self.process_instance_id,
                &mut command_context.session,
            ))
    }
}

pub struct EventSubscriptionService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl EventSubscriptionService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    pub fn get_event_subscriptions_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Result<Vec<RuntimeEventWaitState>, crate::error::FlowableError> {
        let cmd = GetEventSubscriptionsCmd::new(process_instance_id);
        self.command_executor.execute(&cmd)
    }
}
