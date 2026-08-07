//! BPMN `caseServiceTask` behavior.
//!
//! Java parity:
//! - `CaseTaskActivityBehavior.java` (execute / triggerCaseTaskAndLeave)
//! - `CaseServiceTaskParseHandler.java:30-31`
//! - `DefaultCaseInstanceService.java:54-95` (startCaseInstanceByKey)
//! - `ChildBpmnCaseInstanceStateChangeCallback.java:50-88` (completion)

use crate::agenda::FlowableEngineAgenda;
use crate::el::expression::{Expression, SimpleExpression};
use crate::engine::variable_service::variable_type_name;
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use crate::delegate::activity_behavior::ActivityBehavior;
use flowable_bpmn_model::model::{CaseServiceTask, FlowElementEnum, IOParameter};
use flowable_cmmn_engine::{
    CMMN_EXECUTION_CHILD_CASE_CALLBACK_TYPE, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState,
};
use serde_json::{Map, Value};
use uuid::Uuid;

/// Java `ReferenceTypes.EXECUTION_CHILD_CASE`.
pub const EXECUTION_CHILD_CASE_REFERENCE_TYPE: &str = "bpmn-2.0-to-cmmn-1.1-child-case";

/// Process variable used when `caseInstanceIdVariableName` is not set — stores the
/// started case instance id on the parent execution for diagnostics / tests.
pub const CASE_INSTANCE_ID_VARIABLE: &str = "caseInstanceId";

pub struct CaseTaskActivityBehavior;

impl Default for CaseTaskActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl CaseTaskActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

fn find_case_service_task<'a>(
    process: &'a flowable_bpmn_model::model::Process,
    activity_id: &str,
) -> Option<&'a CaseServiceTask> {
    match process.flow_element_map.get(activity_id) {
        Some(FlowElementEnum::CaseServiceTask(task)) => Some(task),
        _ => process.flow_elements.iter().find_map(|element| {
            if let FlowElementEnum::CaseServiceTask(task) = element
                && task.activity_id() == Some(activity_id)
            {
                Some(task)
            } else {
                None
            }
        }),
    }
}

fn expression_or_literal_coerced(
    text: &str,
    execution: &Execution,
    field_name: &str,
) -> Result<String, FlowableError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(FlowableError::ExecutionError(format!(
            "Case service task {field_name} must not be empty"
        )));
    }
    if trimmed.starts_with("${") && trimmed.ends_with('}') {
        let resolved = match SimpleExpression::new(trimmed.to_string()).get_value(execution) {
            Some(Value::String(value)) => value,
            Some(Value::Bool(b)) => b.to_string(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Null) | None => {
                return Err(FlowableError::ExecutionError(format!(
                    "Case service task {field_name} expression '{trimmed}' could not be resolved"
                )));
            }
            Some(other) => other.to_string().trim_matches('"').to_string(),
        };
        if resolved.trim().is_empty() {
            return Err(FlowableError::ExecutionError(format!(
                "Case service task {field_name} expression '{trimmed}' resolved to empty"
            )));
        }
        return Ok(resolved);
    }
    Ok(trimmed.to_string())
}

/// Java `CaseTaskActivityBehavior#getCaseDefinitionKey` (:122-127).
fn resolve_case_definition_key(
    case_definition_key: Option<&str>,
    execution: &Execution,
) -> Result<String, FlowableError> {
    let key = case_definition_key.ok_or_else(|| {
        FlowableError::ExecutionError(
            "Case service task requires flowable:caseDefinitionKey".into(),
        )
    })?;
    expression_or_literal_coerced(key, execution, "caseDefinitionKey")
}

/// Java `CaseTaskActivityBehavior#execute` businessKey / inheritBusinessKey (:70-78).
fn resolve_business_key(
    case_task: &CaseServiceTask,
    execution: &Execution,
    command_context: &mut CommandContext,
) -> Result<Option<String>, FlowableError> {
    if let Some(business_key) = case_task.business_key.as_deref() {
        let resolved = expression_or_literal_coerced(business_key, execution, "businessKey")?;
        return Ok(Some(resolved));
    }
    if case_task.inherit_business_key
        && let Some(process_instance_id) = execution.process_instance_id.as_deref()
        && let Some(process_instance) = command_context
            .runtime_store
            .find_process_instance(process_instance_id, &mut command_context.session)
        && let Some(business_key) = process_instance.business_key
    {
        return Ok(Some(business_key));
    }
    Ok(None)
}

/// Java `IOParameterUtil.processInParameters` (:89) — build child variable map from declared in-params.
fn map_in_parameters(case_task: &CaseServiceTask, execution: &Execution) -> Map<String, Value> {
    let mut mapped = Map::new();
    for parameter in case_task.in_parameters() {
        let Some(target) = parameter_target_name(parameter, execution) else {
            continue;
        };
        let value = parameter_value(parameter, execution);
        mapped.insert(target, value);
    }
    mapped
}

fn parameter_target_name(parameter: &IOParameter, execution: &Execution) -> Option<String> {
    if let Some(target) = parameter.target.as_deref() {
        let trimmed = target.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(target_expression) = parameter.target_expression.as_deref() {
        return expression_or_literal_coerced(target_expression, execution, "targetExpression").ok();
    }
    None
}

fn parameter_value(parameter: &IOParameter, execution: &Execution) -> Value {
    if let Some(source_expression) = parameter.source_expression.as_deref() {
        return SimpleExpression::new(source_expression.to_string())
            .get_value(execution)
            .unwrap_or(Value::Null);
    }
    if let Some(source) = parameter.source.as_deref() {
        return execution
            .process_variable(source.trim())
            .unwrap_or(Value::Null);
    }
    Value::Null
}

fn set_process_variable_with_history(
    command_context: &mut CommandContext,
    execution: &mut Execution,
    name: String,
    value: Value,
) {
    execution.set_process_variable(name.clone(), value.clone());
    let historic_variable_id = format!("{}:{}", execution.id, name);
    if command_context
        .runtime_store
        .get_historic_variable_instance(&historic_variable_id, &mut command_context.session)
        .is_some()
    {
        command_context.history_manager.record_variable_updated(
            &historic_variable_id,
            value,
            &mut command_context.session,
        );
    } else {
        let process_instance_id = execution
            .process_instance_id
            .as_deref()
            .unwrap_or(&execution.id);
        command_context.history_manager.record_variable_created(
            &historic_variable_id,
            &name,
            variable_type_name(&value),
            value,
            process_instance_id,
            Some(&execution.id),
            None,
            &mut command_context.session,
        );
    }
}

/// Java `CaseTaskActivityBehavior#triggerCaseTask` (:145-156) + leave.
pub fn trigger_case_task_and_leave(
    command_context: &mut CommandContext,
    execution_id: &str,
    variables: Map<String, Value>,
) -> Result<(), FlowableError> {
    let mut execution = command_context
        .runtime_store
        .find_execution(execution_id, &mut command_context.session)
        .ok_or_else(|| {
            FlowableError::NotFound(format!("No execution could be found for id {execution_id}"))
        })?;

    if execution.is_suspended {
        return Err(FlowableError::ExecutionError(format!(
            "Cannot complete case task. Parent process instance {} is suspended",
            execution.id
        )));
    }

    // Apply out-parameter variables (already mapped by callback / cmd).
    for (name, value) in variables {
        set_process_variable_with_history(command_context, &mut execution, name, value);
    }

    // Clear reference (Java CaseTaskActivityBehavior.java:154-155).
    execution.reference_id = None;
    execution.reference_type = None;

    command_context
        .execution_entity_manager
        .update(&execution, &mut command_context.session);

    command_context
        .agenda
        .plan_take_outgoing_sequence_flows_operation(execution);
    Ok(())
}

/// Map case variables through declared out-parameters (ChildBpmnCaseInstanceStateChangeCallback.java:59-85).
pub fn map_out_parameters_from_case_variables(
    case_task: &CaseServiceTask,
    case_variables: &Map<String, Value>,
) -> Map<String, Value> {
    let mut mapped = Map::new();
    // Minimal VariableContainer for target/source expressions over case vars.
    let case_exec = Execution {
        variables: case_variables
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        ..Default::default()
    };
    for parameter in case_task.out_parameters() {
        let Some(target) = parameter_target_name(parameter, &case_exec) else {
            continue;
        };
        let value = if let Some(source_expression) = parameter.source_expression.as_deref() {
            SimpleExpression::new(source_expression.to_string())
                .get_value(&case_exec)
                .unwrap_or(Value::Null)
        } else if let Some(source) = parameter.source.as_deref() {
            case_variables
                .get(source.trim())
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        mapped.insert(target, value);
    }
    mapped
}

impl ActivityBehavior for CaseTaskActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        let process_definition_id = execution.process_definition_id.clone().ok_or_else(|| {
            FlowableError::ExecutionError(
                "Case service task missing process definition id".into(),
            )
        })?;
        let activity_id = execution.activity_id.clone().ok_or_else(|| {
            FlowableError::ExecutionError("Case service task missing activity id".into())
        })?;

        let case_task = {
            let model = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id)
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
            find_case_service_task(process, &activity_id)
                .cloned()
                .ok_or_else(|| {
                    FlowableError::NotFound(format!(
                        "Case service task '{activity_id}' not found in process definition '{process_definition_id}'"
                    ))
                })?
        };

        let cmmn_engine = command_context.config.cmmn_engine.clone().ok_or_else(|| {
            // Java CaseTaskActivityBehavior.java:66-68
            FlowableError::ExecutionError(
                "To use the case service task a CMMN engine needs to be available in the process engine configuration"
                    .into(),
            )
        })?;

        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);

        let case_definition_key =
            resolve_case_definition_key(case_task.case_definition_key.as_deref(), &evaluation_execution)?;

        let business_key =
            resolve_business_key(&case_task, &evaluation_execution, command_context)?;

        let case_instance_name = case_task
            .case_instance_name
            .as_deref()
            .map(|name| expression_or_literal_coerced(name, &evaluation_execution, "caseInstanceName"))
            .transpose()?;

        let in_parameters = map_in_parameters(&case_task, &evaluation_execution);

        // Java generates id first then starts with predefined id (:91, :113-115).
        let case_instance_id = format!("cmmn-case-instance:{}", Uuid::new_v4());

        // Java CaseTaskActivityBehavior.java:93-98 — store id variable on parent.
        if let Some(id_var_name) = case_task.case_instance_id_variable_name.as_deref() {
            let resolved =
                expression_or_literal_coerced(id_var_name, &evaluation_execution, "caseInstanceIdVariableName")?;
            if !resolved.trim().is_empty() {
                set_process_variable_with_history(
                    command_context,
                    execution,
                    resolved,
                    Value::String(case_instance_id.clone()),
                );
            }
        } else {
            set_process_variable_with_history(
                command_context,
                execution,
                CASE_INSTANCE_ID_VARIABLE.to_string(),
                Value::String(case_instance_id.clone()),
            );
        }

        // Entity links (Java :101-104) — out of scope when disabled / large; honor flag.
        // (No-op here when enable_entity_links is false; full EntityLinkUtil is P76 out-of-scope.)

        let mut request = CmmnCaseInstanceStartRequest::new()
            .with_variables(Value::Object(in_parameters))
            .with_predefined_case_instance_id(case_instance_id.clone())
            .with_callback(
                execution.id.clone(),
                CMMN_EXECUTION_CHILD_CASE_CALLBACK_TYPE,
            );
        if let Some(business_key) = business_key {
            request = request.with_business_key(business_key);
        }
        if let Some(name) = case_instance_name {
            request = request.with_name(name);
        }
        if let Some(tenant_id) = execution.tenant_id.clone() {
            request = request.with_tenant_id(tenant_id);
        }
        // sameDeployment / fallbackToDefaultTenant: case definition resolution on CMMN
        // currently uses latest-by-key (+ tenant). Full same-deployment parentDeploymentId
        // is tracked as a follow-up (Java DefaultCaseInstanceService.java:61-63).

        let case_instance = cmmn_engine
            .start_case_instance_by_key(&case_definition_key, request)
            .map_err(|error| {
                FlowableError::ExecutionError(format!(
                    "Failed to start CMMN case '{case_definition_key}' from case service task '{activity_id}': {error}"
                ))
            })?;

        // Java CaseTaskActivityBehavior.java:118-119 — bidirectional reference.
        execution.reference_id = Some(case_instance.id.clone());
        execution.reference_type = Some(EXECUTION_CHILD_CASE_REFERENCE_TYPE.to_string());
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        // Non-blocking is not modeled on BPMN CaseServiceTask in Java (always waits).
        // If the child case already completed synchronously, map outs and leave now.
        if case_instance.state == CmmnCaseInstanceState::Completed {
            let out_vars =
                map_out_parameters_from_case_variables(&case_task, &case_instance.variables);
            for (name, value) in out_vars {
                set_process_variable_with_history(command_context, execution, name, value);
            }
            execution.reference_id = None;
            execution.reference_type = None;
            command_context
                .execution_entity_manager
                .update(execution, &mut command_context.session);
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
        }

        Ok(())
    }
}
