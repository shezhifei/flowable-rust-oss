//! Java `TriggerCaseTaskCmd` (TriggerCaseTaskCmd.java:34-80).

use crate::bpmn::behavior::case_task_activity_behavior::{
    map_out_parameters_from_case_variables, trigger_case_task_and_leave,
};
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use flowable_bpmn_model::model::FlowElementEnum;
use serde_json::{Map, Value};

pub struct TriggerCaseTaskCmd {
    execution_id: String,
    /// Pre-mapped process variables (out-parameters already applied), or raw case variables
    /// when `map_from_case_task` is true.
    variables: Map<String, Value>,
    map_from_case_task: bool,
}

impl TriggerCaseTaskCmd {
    pub fn new(execution_id: impl Into<String>, variables: Map<String, Value>) -> Self {
        Self {
            execution_id: execution_id.into(),
            variables,
            map_from_case_task: false,
        }
    }

    /// Map `variables` through the current CaseServiceTask out-parameters before applying.
    pub fn with_case_variable_mapping(
        execution_id: impl Into<String>,
        case_variables: Map<String, Value>,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            variables: case_variables,
            map_from_case_task: true,
        }
    }
}

impl Command<()> for TriggerCaseTaskCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        let execution = command_context
            .runtime_store
            .find_execution(&self.execution_id, &mut command_context.session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "No execution could be found for id {}",
                    self.execution_id
                ))
            })?;

        let mapped = if self.map_from_case_task {
            let process_definition_id = execution.process_definition_id.as_deref().ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "Execution {} has no process definition for case task trigger",
                    self.execution_id
                ))
            })?;
            let activity_id = execution.activity_id.as_deref().ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "Execution {} has no activity id for case task trigger",
                    self.execution_id
                ))
            })?;
            let model = command_context
                .deployment_manager
                .get_bpmn_model(process_definition_id)
                .ok_or_else(|| {
                    FlowableError::NotFound(format!(
                        "BPMN model not found for process definition '{process_definition_id}'"
                    ))
                })?;
            let process = model.main_process.as_ref().ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "No main process on definition '{process_definition_id}'"
                ))
            })?;
            let case_task = match process.flow_element_map.get(activity_id) {
                Some(FlowElementEnum::CaseServiceTask(task)) => task,
                _ => {
                    // Java DefaultProcessInstanceService.getOutputParametersOfCaseTask:128-131 —
                    // empty when flow element is no longer a CaseServiceTask.
                    return trigger_case_task_and_leave(
                        command_context,
                        &self.execution_id,
                        Map::new(),
                    );
                }
            };
            map_out_parameters_from_case_variables(case_task, &self.variables)
        } else {
            self.variables.clone()
        };

        trigger_case_task_and_leave(command_context, &self.execution_id, mapped)
    }
}
