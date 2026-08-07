use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::runtime_store::RuntimeTimerJobState;
use std::sync::Arc;

pub struct GetTimerJobsCmd {
    process_instance_id: String,
}

impl GetTimerJobsCmd {
    pub fn new(process_instance_id: String) -> Self {
        Self {
            process_instance_id,
        }
    }
}

impl Command<Vec<RuntimeTimerJobState>> for GetTimerJobsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<RuntimeTimerJobState>, crate::error::FlowableError> {
        Ok(command_context
            .runtime_store
            .find_timer_job_states_by_process_instance_id(
                &self.process_instance_id,
                &mut command_context.session,
            ))
    }
}

pub struct JobService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl JobService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    pub fn get_timer_jobs_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Result<Vec<RuntimeTimerJobState>, crate::error::FlowableError> {
        let cmd = GetTimerJobsCmd::new(process_instance_id);
        self.command_executor.execute(&cmd)
    }
}
