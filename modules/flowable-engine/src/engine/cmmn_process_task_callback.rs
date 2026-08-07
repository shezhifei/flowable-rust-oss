use crate::interceptor::command_context::CommandContext;
use crate::runtime::process_instance::ProcessInstance;
use flowable_cmmn_engine::CMMN_PROCESS_TASK_CALLBACK_TYPE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmmnProcessTaskCallbackOutcome {
    Completed,
    Failed { failure_message: String },
}

pub fn is_cmmn_process_task_callback(process_instance: &ProcessInstance) -> bool {
    process_instance.callback_type.as_deref() == Some(CMMN_PROCESS_TASK_CALLBACK_TYPE)
}

pub fn notify_cmmn_process_task_callback(
    command_context: &CommandContext,
    process_instance_id: &str,
    callback_type: Option<&str>,
    outcome: CmmnProcessTaskCallbackOutcome,
) -> Result<(), crate::error::FlowableError> {
    if callback_type != Some(CMMN_PROCESS_TASK_CALLBACK_TYPE) {
        return Ok(());
    }
    let Some(cmmn_engine) = command_context.config.cmmn_engine.as_ref() else {
        return Ok(());
    };
    let runtime_service = cmmn_engine.runtime_service();
    match outcome {
        CmmnProcessTaskCallbackOutcome::Completed => runtime_service
            .notify_process_task_child_instance_completed(process_instance_id)
            .map(|_| ())
            .map_err(|error| {
                crate::error::FlowableError::ExecutionError(format!(
                    "Failed to notify CMMN processTask completion for process instance '{}': {}",
                    process_instance_id, error
                ))
            }),
        CmmnProcessTaskCallbackOutcome::Failed { failure_message } => runtime_service
            .fail_process_task_child_instance(process_instance_id, failure_message.clone())
            .map_err(|error| {
                crate::error::FlowableError::ExecutionError(format!(
                    "Failed to notify CMMN processTask failure for process instance '{}': {}",
                    process_instance_id, error
                ))
            }),
    }
}

pub fn notify_cmmn_process_task_callback_for_instance(
    command_context: &CommandContext,
    process_instance: &ProcessInstance,
    outcome: CmmnProcessTaskCallbackOutcome,
) -> Result<(), crate::error::FlowableError> {
    notify_cmmn_process_task_callback(
        command_context,
        &process_instance.id,
        process_instance.callback_type.as_deref(),
        outcome,
    )
}
