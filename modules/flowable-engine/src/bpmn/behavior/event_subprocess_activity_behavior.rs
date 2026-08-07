use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;

pub struct EventSubprocessActivityBehavior;

impl Default for EventSubprocessActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSubprocessActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for EventSubprocessActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        _command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // Event Subprocesses are triggered by event subscriptions on their inner start
        // event (message, signal, timer, error, etc.). When a matching event fires,
        // the start event subscription handler activates the subprocess and injects
        // a new child execution. This behavior is a no-op because the event-trigger
        // path (activate_interrupting_event_subprocess / activate_non_interrupting_event_subprocess
        // in trigger_start_event_subscription_cmd.rs) handles the actual activation.
        // The behavior is registered so the factory can identify event subprocess nodes.
        let _ = execution;
        Ok(())
    }
}
