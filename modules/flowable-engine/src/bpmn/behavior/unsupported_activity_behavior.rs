use crate::delegate::activity_behavior::ActivityBehavior;
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;

pub struct UnsupportedActivityBehavior {
    element_type: String,
}

impl UnsupportedActivityBehavior {
    pub fn new(element_type: String) -> Self {
        Self { element_type }
    }
}

impl ActivityBehavior for UnsupportedActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        _command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        let activity_id = execution.activity_id.as_deref().unwrap_or("<unknown>");
        Err(FlowableError::UnsupportedElement {
            element_type: self.element_type.clone(),
            activity_id: activity_id.to_string(),
        })
    }
}
