use crate::agenda::FlowableEngineAgenda;
use crate::bpmn::behavior::dmn_result_writeback::write_dmn_result_to_execution;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::FlowElementEnum;
use flowable_dmn_engine::DmnExecutionRequest;
use serde_json::{Map, Value};

pub struct BusinessRuleTaskActivityBehavior;

impl Default for BusinessRuleTaskActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl BusinessRuleTaskActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for BusinessRuleTaskActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        let activity_id = execution.activity_id.clone().ok_or_else(|| {
            FlowableError::ExecutionError(
                "Business rule task execution has no activity_id".to_string(),
            )
        })?;
        let process_definition_id = execution.process_definition_id.clone().ok_or_else(|| {
            FlowableError::ExecutionError(
                "Business rule task execution has no process_definition_id".to_string(),
            )
        })?;

        let (decision_ref, input_variables, result_variable_name) = {
            let model = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id)
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "No BPMN model found for process definition: {}",
                        process_definition_id
                    ))
                })?;
            let process = model.main_process.as_ref().ok_or_else(|| {
                FlowableError::ExecutionError("No main process in BPMN model".to_string())
            })?;
            let flow_element =
                crate::agenda::continue_process_operation::find_flow_element(process, &activity_id)
                    .ok_or_else(|| {
                        FlowableError::ExecutionError(format!(
                            "BusinessRuleTask element '{}' not found in process model",
                            activity_id
                        ))
                    })?;
            match flow_element {
                FlowElementEnum::BusinessRuleTask(task) => (
                    task.decision_ref.clone(),
                    task.input_variables.clone(),
                    task.result_variable_name.clone(),
                ),
                _ => {
                    return Err(FlowableError::ExecutionError(
                        "Activity is not a BusinessRuleTask element".to_string(),
                    ));
                }
            }
        };

        let decision_ref = decision_ref.ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "BusinessRuleTask '{}' missing decisionRef for owned M15 DMN path",
                activity_id
            ))
        })?;

        let mut inputs = Map::new();
        // P6-B: DMN input variables must walk the parent scope chain. The
        // business rule task may run on a forked child execution whose
        // variable maps were emptied by P4-7b, so we resolve inputs against
        // `evaluation_execution` which merges the parent chain and the PI
        // scope row. Results are still written back to the real `execution`.
        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);
        if input_variables.is_empty() {
            for (key, value) in evaluation_execution.process_variables() {
                inputs.insert(key, value);
            }
        } else {
            for name in input_variables {
                inputs.insert(
                    name.clone(),
                    evaluation_execution
                        .process_variable(&name)
                        .unwrap_or(Value::Null),
                );
            }
        }

        let dmn_engine = command_context.config.dmn_engine.clone().ok_or_else(|| {
            FlowableError::ExecutionError(
                "DMN engine is not configured for BusinessRuleTask execution".to_string(),
            )
        })?;

        let execution_result = dmn_engine
            .decision_service()
            .execute_by_key(
                &decision_ref,
                // Java `DmnActivityBehavior.java:99-103` — audit correlation.
                // BusinessRuleTask has no `fallbackToDefaultTenant` field in
                // Java, so the tenant fallback stays off on this path.
                DmnExecutionRequest::new(Value::Object(inputs)).with_audit_correlation(
                    execution.process_instance_id.clone(),
                    Some(execution.id.clone()),
                    Some(activity_id.clone()),
                ),
            )
            .map_err(|error| FlowableError::ExecutionError(error.to_string()))?;

        // Java `DmnActivityBehavior.execute` (:153-161):
        // multipleResults = audit.isMultipleResults() && alwaysUseArraysForDmnMultiHitPolicies
        // (default true — ProcessEngineConfiguration.java:133).
        let always_use_arrays = command_context
            .config
            .always_use_arrays_for_dmn_multi_hit_policies;
        let multiple_results = execution_result.multiple_results && always_use_arrays;

        write_dmn_result_to_execution(
            execution,
            &decision_ref,
            &execution_result,
            result_variable_name.as_deref(),
            multiple_results,
        );

        let details = format!(
            "Business rule task '{}' executed decision '{}'",
            activity_id, decision_ref
        );
        command_context.history_manager.record_audit_event(
            "business-rule-task-executed",
            execution.process_instance_id.as_deref(),
            execution.process_definition_id.as_deref(),
            Some(&details),
            &mut command_context.session,
        );

        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(execution.clone());
        Ok(())
    }
}
