//! Bridges CMMN child-case completion back into BPMN caseServiceTask.
//! Java: ChildBpmnCaseInstanceStateChangeCallback + DefaultProcessInstanceService#triggerCaseTask.

use crate::cmd::trigger_case_task_cmd::TriggerCaseTaskCmd;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use flowable_cmmn_engine::{BpmnCaseTaskCallback, CmmnError};
use serde_json::{Map, Value};
use std::sync::{Arc, Weak};

/// Holds a `Weak` to the process-engine command executor so PE drop can release
/// the DB (config → cmmn → callback must not form a strong cycle).
pub struct ProcessEngineBpmnCaseTaskCallback {
    command_executor: Weak<DefaultCommandExecutor>,
}

impl ProcessEngineBpmnCaseTaskCallback {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            command_executor: Arc::downgrade(&command_executor),
        }
    }
}

impl BpmnCaseTaskCallback for ProcessEngineBpmnCaseTaskCallback {
    fn on_child_case_completed(
        &self,
        execution_id: &str,
        _case_instance_id: &str,
        variables: Map<String, Value>,
    ) -> Result<(), CmmnError> {
        let Some(command_executor) = self.command_executor.upgrade() else {
            return Err(CmmnError::execution(
                "BPMN process engine has been dropped; cannot trigger case service task",
            ));
        };
        let cmd = TriggerCaseTaskCmd::with_case_variable_mapping(execution_id, variables);
        command_executor
            .execute(&cmd)
            .map_err(|error| CmmnError::execution(error.to_string()))
    }

    fn on_child_case_terminated(
        &self,
        execution_id: &str,
        _case_instance_id: &str,
    ) -> Result<(), CmmnError> {
        let Some(command_executor) = self.command_executor.upgrade() else {
            return Err(CmmnError::execution(
                "BPMN process engine has been dropped; cannot trigger case service task",
            ));
        };
        let cmd = TriggerCaseTaskCmd::new(execution_id, Map::new());
        command_executor
            .execute(&cmd)
            .map_err(|error| CmmnError::execution(error.to_string()))
    }
}
