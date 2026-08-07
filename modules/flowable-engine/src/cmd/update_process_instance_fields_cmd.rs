use crate::cmd::process_instance_suspension::set_process_instance_suspension_state;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::process_instance::{ProcessInstance, ProcessInstanceUpdate};

pub struct UpdateProcessInstanceFieldsCmd {
    process_instance_id: String,
    updates: ProcessInstanceUpdate,
    suspended: Option<bool>,
}

impl UpdateProcessInstanceFieldsCmd {
    pub fn new(
        process_instance_id: String,
        updates: ProcessInstanceUpdate,
        suspended: Option<bool>,
    ) -> Self {
        Self {
            process_instance_id,
            updates,
            suspended,
        }
    }
}

impl Command<ProcessInstance> for UpdateProcessInstanceFieldsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();

        let mut process_instance = store
            .find_process_instance(&self.process_instance_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Process instance '{}' was not found",
                    self.process_instance_id
                ))
            })?;

        if process_instance.is_ended {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                format!(
                    "Cannot update ended process instance '{}'",
                    self.process_instance_id
                ),
            ));
        }
        if self
            .suspended
            .is_some_and(|suspended| process_instance.is_suspended == suspended)
        {
            let state = if process_instance.is_suspended {
                "suspended"
            } else {
                "active"
            };
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "Cannot set suspension state '{}' for process instance '{}': already in state '{}'.",
                state, self.process_instance_id, state
            )));
        }

        if let Some(name) = self.updates.name.clone() {
            process_instance.name = name;
        }
        if let Some(business_key) = self.updates.business_key.clone() {
            process_instance.business_key = business_key;
        }
        if let Some(business_status) = self.updates.business_status.clone() {
            process_instance.business_status = business_status;
        }
        if let Some(callback_id) = self.updates.callback_id.clone() {
            process_instance.callback_id = callback_id;
        }
        if let Some(callback_type) = self.updates.callback_type.clone() {
            process_instance.callback_type = callback_type;
        }
        if let Some(reference_id) = self.updates.reference_id.clone() {
            process_instance.reference_id = reference_id;
        }
        if let Some(reference_type) = self.updates.reference_type.clone() {
            process_instance.reference_type = reference_type;
        }
        if let Some(suspended) = self.suspended {
            return set_process_instance_suspension_state(
                command_context,
                process_instance,
                suspended,
            );
        }

        store.update_process_instance(&process_instance, &mut command_context.session);
        Ok(process_instance)
    }
}
