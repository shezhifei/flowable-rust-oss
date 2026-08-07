use crate::agenda::FlowableEngineAgenda;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{FlowElementEnum, SendTask};

/// Behavior for the BPMN `<sendTask>` element.
///
/// Java reference: `SendTaskParseHandler.java:37-56` assigns a behavior by
/// `type`: mail → `MailActivityBehavior`, dmn → `DmnActivityBehavior`, and for
/// a missing/unknown `type` leaves the behavior `null`. A `null` activity
/// behavior is then executed by `ContinueProcessOperation.java:172-181` as a
/// plain pass-through (`planTakeOutgoingSequenceFlowsOperation`). The Rust port
/// reuses the service-task mail / dmn execution helpers (`DmnActivityBehavior.java:58-195`,
/// `BaseMailActivityDelegate`); the webservice form is rejected at deployment
/// validation, and camel flows through unchanged (parity with `serviceTask
/// flowable:type="camel"`).
pub struct SendTaskActivityBehavior;

impl Default for SendTaskActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl SendTaskActivityBehavior {
    pub fn new() -> Self {
        Self
    }

    fn resolve_send_task(
        &self,
        execution: &Execution,
        command_context: &mut CommandContext,
    ) -> Result<SendTask, FlowableError> {
        let activity_id = execution.activity_id.as_ref().ok_or_else(|| {
            FlowableError::ExecutionError("Send task execution has no activity_id".to_string())
        })?;
        let process_def_id = execution.process_definition_id.as_ref().ok_or_else(|| {
            FlowableError::ExecutionError(
                "Send task execution has no process_definition_id".to_string(),
            )
        })?;

        let model = command_context
            .deployment_manager
            .get_bpmn_model(process_def_id)
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "No BPMN model found for process definition: {}",
                    process_def_id
                ))
            })?;
        let process = model.main_process.as_ref().ok_or_else(|| {
            FlowableError::ExecutionError("No main process in BPMN model".to_string())
        })?;

        let flow_element =
            crate::agenda::continue_process_operation::find_flow_element(process, activity_id)
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "SendTask element '{}' not found in process model",
                        activity_id
                    ))
                })?;

        match flow_element {
            FlowElementEnum::SendTask(send_task) => Ok(send_task.clone()),
            _ => Err(FlowableError::ExecutionError(
                "Activity is not a SendTask element".to_string(),
            )),
        }
    }
}

impl ActivityBehavior for SendTaskActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        let send_task = self.resolve_send_task(execution, command_context)?;
        let task_type = send_task
            .service_task
            .task_type
            .as_deref()
            .map(str::to_lowercase)
            .unwrap_or_default();

        match task_type.as_str() {
            // P138: already discarded the send payload (no resultVariable write).
            // Aligns with Java DefaultActivityBehaviorFactory.java:242-244
            // (sendTask type=mail → BpmnMailActivityDelegate only) and with the
            // serviceTask mail branch after the super-set cut.
            "mail" => {
                let _ = crate::bpmn::behavior::service_task_activity_behavior::execute_mail_service_task(
                    &send_task.service_task,
                    execution,
                    command_context,
                )?;
            }
            "dmn" => {
                crate::bpmn::behavior::service_task_activity_behavior::execute_dmn_service_task(
                    &send_task.service_task,
                    execution,
                    command_context,
                )?;
            }
            // No / unknown type → Java leaves the behavior null and
            // `ContinueProcessOperation.java:178-180` plans take outgoing.
            _ => {}
        }

        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(execution.clone());
        Ok(())
    }
}
