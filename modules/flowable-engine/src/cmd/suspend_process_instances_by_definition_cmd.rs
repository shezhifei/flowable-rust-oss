use crate::cmd::process_instance_suspension::set_process_instance_suspension_state;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;

pub struct SuspendProcessInstancesByDefinitionCmd {
    process_definition_id: String,
    suspended: bool,
}

impl SuspendProcessInstancesByDefinitionCmd {
    pub fn new(process_definition_id: String, suspended: bool) -> Self {
        Self {
            process_definition_id,
            suspended,
        }
    }
}

impl Command<usize> for SuspendProcessInstancesByDefinitionCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<usize, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let process_instances = store
            .snapshot_process_instances(&mut command_context.session)
            .into_values()
            .filter(|instance| {
                instance.process_definition_id == self.process_definition_id
                    && instance.is_suspended != self.suspended
            })
            .collect::<Vec<_>>();

        for process_instance in &process_instances {
            set_process_instance_suspension_state(
                command_context,
                process_instance.clone(),
                self.suspended,
            )?;
        }

        Ok(process_instances.len())
    }
}
