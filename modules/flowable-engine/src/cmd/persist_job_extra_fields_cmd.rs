use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::RuntimeTimerJobState;
use serde_json::Value;

pub struct PersistJobExtraFieldsCmd {
    job: RuntimeTimerJobState,
    fields: Vec<(String, Option<String>)>,
}

impl PersistJobExtraFieldsCmd {
    pub fn new(job: RuntimeTimerJobState, fields: Vec<(String, Option<String>)>) -> Self {
        Self { job, fields }
    }
}

impl Command<()> for PersistJobExtraFieldsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        if self.fields.iter().all(|(_, value)| value.is_none()) {
            return Ok(());
        }

        let mut data = serde_json::to_value(&self.job)
            .map_err(|error| crate::error::FlowableError::ExecutionError(error.to_string()))?;

        if let Value::Object(ref mut object) = data {
            for (field, value) in &self.fields {
                if let Some(value) = value {
                    object.insert(field.clone(), Value::String(value.clone()));
                }
            }
        }

        let data = serde_json::to_string(&data)
            .map_err(|error| crate::error::FlowableError::ExecutionError(error.to_string()))?;

        let session = command_context.session();
        session
            .cas_update(
                "timer_job_states",
                &self.job.timer_job_id,
                &data,
                &[
                    ("due_time".into(), self.job.due_time.map(|v| v.to_string())),
                    ("job_state".into(), self.job.job_state.clone()),
                ],
                &[],
            )
            .map_err(|error| crate::error::FlowableError::ExecutionError(error.to_string()))?;

        Ok(())
    }
}
