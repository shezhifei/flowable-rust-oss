use crate::bpmn::behavior::boundary_event_activity_behavior::{
    resolve_boundary_event_subscription, runtime_cancel_activity,
};
use crate::bpmn::job_category::resolve_job_category;
use crate::cmd::start_process_instance_cmd::StartProcessInstanceCmd;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::el::expression::{Expression, SimpleExpression};
use crate::engine::variable_service::variable_type_name;
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{RuntimeBoundaryEventState, RuntimeTimerJobState};
use crate::repository::process_definition::ProcessDefinition;
use crate::runtime::execution::Execution;
use crate::runtime::process_instance::ProcessInstance;
use crate::runtime::process_instance_builder::ProcessInstanceBuilder;
use flowable_bpmn_model::model::{CallActivity, EventDefinitionEnum, FlowElementEnum, IOParameter};
use serde_json::Value;
use uuid::Uuid;

/// Java `CallActivityBehavior.CALLED_ELEMENT_TYPE_KEY`.
const CALLED_ELEMENT_TYPE_KEY: &str = "key";
/// Java `CallActivityBehavior.CALLED_ELEMENT_TYPE_ID`.
const CALLED_ELEMENT_TYPE_ID: &str = "id";

pub struct CallActivityBehavior;

impl Default for CallActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl CallActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

fn expression_or_literal_string(
    text: &str,
    execution: &Execution,
    field_name: &str,
) -> Result<String, FlowableError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(FlowableError::ExecutionError(format!(
            "Call activity {} must not be empty",
            field_name
        )));
    }

    if trimmed.starts_with("${") && trimmed.ends_with('}') {
        return match SimpleExpression::new(trimmed.to_string()).get_value(execution) {
            Some(Value::String(value)) if !value.trim().is_empty() => Ok(value),
            Some(value) => Err(FlowableError::ExecutionError(format!(
                "Call activity {} expression '{}' resolved to a non-string value: {}",
                field_name, trimmed, value
            ))),
            None => Err(FlowableError::ExecutionError(format!(
                "Call activity {} expression '{}' could not be resolved",
                field_name, trimmed
            ))),
        };
    }

    Ok(trimmed.to_string())
}

/// Like `expression_or_literal_string` but allows non-string expression results
/// (coerced via Display) for names / id variable names (Java `toString()`).
fn expression_or_literal_coerced(
    text: &str,
    execution: &Execution,
    field_name: &str,
) -> Result<String, FlowableError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(FlowableError::ExecutionError(format!(
            "Call activity {} must not be empty",
            field_name
        )));
    }

    if trimmed.starts_with("${") && trimmed.ends_with('}') {
        let resolved = match SimpleExpression::new(trimmed.to_string()).get_value(execution) {
            Some(Value::String(value)) => value,
            Some(Value::Bool(b)) => b.to_string(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Null) | None => {
                return Err(FlowableError::ExecutionError(format!(
                    "Call activity {} expression '{}' could not be resolved",
                    field_name, trimmed
                )));
            }
            Some(other) => other.to_string().trim_matches('"').to_string(),
        };
        if resolved.trim().is_empty() {
            return Err(FlowableError::ExecutionError(format!(
                "Call activity {} expression '{}' resolved to empty",
                field_name, trimmed
            )));
        }
        return Ok(resolved);
    }

    Ok(trimmed.to_string())
}

fn call_activity_id(call_activity: &CallActivity) -> String {
    call_activity
        .activity
        .flow_node
        .flow_element
        .base_element
        .id
        .clone()
        .unwrap_or_default()
}

fn resolve_called_element_value(
    call_activity: &CallActivity,
    execution: &Execution,
) -> Result<String, FlowableError> {
    let called_element = call_activity.called_element.as_deref().ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "Call activity '{}' requires calledElement",
            call_activity_id(call_activity)
        ))
    })?;
    expression_or_literal_string(called_element, execution, "calledElement")
}

fn called_element_type(call_activity: &CallActivity) -> &str {
    call_activity
        .called_element_type
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or(CALLED_ELEMENT_TYPE_KEY)
}

fn find_latest_definition_by_key(
    definitions: &std::collections::HashMap<String, ProcessDefinition>,
    key: &str,
    tenant_id: Option<&str>,
) -> Option<ProcessDefinition> {
    definitions
        .values()
        .filter(|definition| definition.key == key)
        .filter(|definition| definition.tenant_id.as_deref() == tenant_id)
        .max_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then(left.id.cmp(&right.id))
        })
        .cloned()
}

/// Java `CallActivityBehavior#getProcessDefinitionByKey` same-deployment branch
/// (`:299-310`): lookup by deployment+key(+tenant). On miss returns `None` so
/// the caller falls through to latest-by-key (`:312-326`).
fn resolve_same_deployment_definition(
    command_context: &mut CommandContext,
    call_activity: &CallActivity,
    execution: &Execution,
    called_element_key: &str,
) -> Result<Option<ProcessDefinition>, FlowableError> {
    if !call_activity.same_deployment {
        return Ok(None);
    }

    let parent_definition_id = execution.process_definition_id.as_deref().ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "Call activity '{}' cannot resolve same deployment without parent process definition id",
            call_activity_id(call_activity)
        ))
    })?;

    let definitions = command_context
        .deployment_manager
        .get_process_definitions(&mut command_context.session);
    let parent_definition = definitions.get(parent_definition_id).ok_or_else(|| {
        FlowableError::NotFound(format!(
            "Parent process definition '{}' was not found for call activity '{}'",
            parent_definition_id,
            call_activity_id(call_activity)
        ))
    })?;
    let parent_deployment_id = parent_definition.deployment_id.as_deref().ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "Parent process definition '{}' has no deployment id for same-deployment call activity '{}'",
            parent_definition_id,
            call_activity_id(call_activity)
        ))
    })?;

    let mut matches = definitions
        .values()
        .filter(|definition| definition.key == called_element_key)
        .filter(|definition| definition.deployment_id.as_deref() == Some(parent_deployment_id))
        .filter(|definition| definition.tenant_id.as_deref() == execution.tenant_id.as_deref())
        .cloned()
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        left.version
            .cmp(&right.version)
            .then(left.id.cmp(&right.id))
    });

    // Miss → Ok(None): fall back to latest-by-key (Java `:307-313`).
    Ok(matches.into_iter().last())
}

/// Java `CallActivityBehavior#getProcessDefinition` + by-key/id branches
/// (`:242-249`, `:287-333`).
fn resolve_called_process_definition(
    command_context: &mut CommandContext,
    call_activity: &CallActivity,
    execution: &Execution,
    called_element: &str,
) -> Result<ProcessDefinition, FlowableError> {
    let element_type = called_element_type(call_activity);
    let definitions = command_context
        .deployment_manager
        .get_process_definitions(&mut command_context.session);

    match element_type {
        CALLED_ELEMENT_TYPE_ID => {
            // Java `:287-290` findDeployedProcessDefinitionById
            definitions
                .get(called_element)
                .cloned()
                .ok_or_else(|| {
                    FlowableError::NotFound(format!(
                        "Process definition id '{}' was not found for call activity '{}'",
                        called_element,
                        call_activity_id(call_activity)
                    ))
                })
        }
        CALLED_ELEMENT_TYPE_KEY => {
            // sameDeployment first (miss falls through)
            if let Some(definition) = resolve_same_deployment_definition(
                command_context,
                call_activity,
                execution,
                called_element,
            )? {
                return Ok(definition);
            }

            let tenant_id = execution.tenant_id.as_deref();
            let definitions = command_context
                .deployment_manager
                .get_process_definitions(&mut command_context.session);

            if tenant_id.is_none() {
                return find_latest_definition_by_key(&definitions, called_element, None).ok_or_else(
                    || {
                        FlowableError::NotFound(format!(
                            "Process definition {} was not found in sameDeployment[{}] tenantId[None] fallbackToDefaultTenant[{:?}]",
                            called_element,
                            call_activity.same_deployment,
                            call_activity.fallback_to_default_tenant
                        ))
                    },
                );
            }

            if let Some(definition) =
                find_latest_definition_by_key(&definitions, called_element, tenant_id)
            {
                return Ok(definition);
            }

            // Java `:316-325` fallbackToDefaultTenant on the activity (or engine-wide).
            let fallback = call_activity.fallback_to_default_tenant.unwrap_or(false)
                || command_context.config.fallback_to_default_tenant;
            if fallback {
                // Default-tenant provider not configured → empty default tenant →
                // findLatestProcessDefinitionByKey (no tenant). Java `:322-324`.
                if let Some(definition) =
                    find_latest_definition_by_key(&definitions, called_element, None)
                {
                    return Ok(definition);
                }
            }

            Err(FlowableError::NotFound(format!(
                "Process definition {} was not found in sameDeployment[{}] tenantId[{:?}] fallbackToDefaultTenant[{}]",
                called_element,
                call_activity.same_deployment,
                tenant_id,
                fallback
            )))
        }
        other => Err(FlowableError::ExecutionError(format!(
            "Unrecognized calledElementType [{}] in call activity '{}'",
            other,
            call_activity_id(call_activity)
        ))),
    }
}

/// Java `CallActivityBehavior#execute` businessKey priority (`:122-130`):
/// explicit `businessKey` expression wins over `inheritBusinessKey`.
fn business_key_from_call_activity(
    call_activity: &CallActivity,
    execution: &Execution,
    command_context: &mut CommandContext,
) -> Result<Option<String>, FlowableError> {
    if let Some(business_key) = call_activity.business_key.as_deref() {
        let resolved = expression_or_literal_string(business_key, execution, "businessKey")?;
        return Ok(Some(resolved));
    }

    if call_activity.inherit_business_key
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

fn parameter_target(
    parameter: &IOParameter,
    // Java IOParameterUtil: targetExpression is evaluated against sourceContainer
    source_execution: &Execution,
) -> Result<Option<String>, FlowableError> {
    if let Some(target) = parameter.target.as_deref() {
        return Ok(Some(target.trim().to_string()).filter(|target| !target.is_empty()));
    }

    parameter
        .target_expression
        .as_deref()
        .map(|target_expression| {
            // May be a bare name (no ${}) — treat as literal, matching existing in-param usage.
            expression_or_literal_string(target_expression, source_execution, "targetExpression")
        })
        .transpose()
}

fn parameter_value(parameter: &IOParameter, execution: &Execution) -> Value {
    if let Some(source_expression) = parameter.source_expression.as_deref() {
        if let Some(value) =
            SimpleExpression::new(source_expression.to_string()).get_value(execution)
        {
            return value;
        }
        return Value::Null;
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

fn set_local_variable_with_history(
    command_context: &mut CommandContext,
    execution: &mut Execution,
    name: String,
    value: Value,
) {
    execution.set_local_variable(name.clone(), value.clone());
    // Local vars are still historicized against the execution (best-effort parity).
    let historic_variable_id = format!("{}:local:{}", execution.id, name);
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

/// Persists variable writes made on the execution row. The row is the single
/// process-level variable store; nothing is mirrored onto the process instance.
fn persist_execution(command_context: &mut CommandContext, execution: &Execution) {
    command_context
        .execution_entity_manager
        .update(execution, &mut command_context.session);
}

fn expression_property_name(expression: &str) -> Option<&str> {
    expression
        .trim()
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn find_call_activity<'a>(
    process: &'a flowable_bpmn_model::model::Process,
    activity_id: &str,
) -> Option<&'a CallActivity> {
    match process.flow_element_map.get(activity_id) {
        Some(FlowElementEnum::CallActivity(call_activity)) => Some(call_activity),
        _ => process.flow_elements.iter().find_map(|element| {
            if let FlowElementEnum::CallActivity(call_activity) = element
                && call_activity
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id)
            {
                Some(call_activity)
            } else {
                None
            }
        }),
    }
}

/// Java `CallActivityBehavior#completed` (`:279-285`): refuse to continue the
/// parent when the call-activity execution or its process definition is suspended.
pub fn ensure_call_activity_parent_not_suspended(
    command_context: &mut CommandContext,
    super_execution: &Execution,
) -> Result<(), FlowableError> {
    if super_execution.is_suspended {
        return Err(FlowableError::ExecutionError(format!(
            "Cannot complete process instance. Parent process instance {} is suspended",
            super_execution.id
        )));
    }

    if let Some(process_instance_id) = super_execution.process_instance_id.as_deref()
        && let Some(pi) = command_context
            .runtime_store
            .find_process_instance(process_instance_id, &mut command_context.session)
        && pi.is_suspended
    {
        return Err(FlowableError::ExecutionError(format!(
            "Cannot complete process instance. Parent process instance {} is suspended",
            process_instance_id
        )));
    }

    if let Some(definition_id) = super_execution.process_definition_id.as_deref() {
        let definitions = command_context
            .deployment_manager
            .get_process_definitions(&mut command_context.session);
        if definitions
            .get(definition_id)
            .map(|d| d.is_suspended)
            .unwrap_or(false)
        {
            return Err(FlowableError::ExecutionError(format!(
                "Cannot complete process instance. Parent process instance {} is suspended",
                super_execution.id
            )));
        }
    }

    Ok(())
}

pub fn apply_call_activity_out_parameters(
    command_context: &mut CommandContext,
    completed_process_instance: &ProcessInstance,
    super_execution: &mut Execution,
) -> Result<(), crate::error::FlowableError> {
    let (out_parameters, use_local_scope) = {
        let Some(process_definition_id) = super_execution.process_definition_id.as_deref() else {
            return Ok(());
        };
        let Some(activity_id) = super_execution.activity_id.as_deref() else {
            return Ok(());
        };

        let Some(model) = command_context
            .deployment_manager
            .get_bpmn_model(process_definition_id)
        else {
            return Ok(());
        };
        let Some(process) = model.main_process.as_ref() else {
            return Ok(());
        };

        match find_call_activity(process, activity_id) {
            Some(call_activity) => (
                call_activity.out_parameters.clone(),
                call_activity.use_local_scope_for_out_parameters,
            ),
            None => (Vec::new(), false),
        }
    };

    if out_parameters.is_empty() {
        return Ok(());
    }

    // Single storage: the completed child instance's scope execution row is the
    // only process-level variable store (the row survives completion as a
    // soft-ended row), so out parameters resolve from it alone.
    let child_expression_execution = command_context
        .runtime_store
        .find_execution(&completed_process_instance.id, &mut command_context.session)
        .unwrap_or_else(|| Execution {
            id: completed_process_instance.id.clone(),
            process_instance_id: Some(completed_process_instance.id.clone()),
            process_definition_id: Some(completed_process_instance.process_definition_id.clone()),
            process_definition_key: Some(completed_process_instance.process_definition_key.clone()),
            tenant_id: completed_process_instance.tenant_id.clone(),
            ..Default::default()
        });
    let child_variables = child_expression_execution.process_variables();

    let mut updated = false;
    for out_param in out_parameters {
        // Java IOParameterUtil.processOutParameters: targetExpression evaluated
        // against sourceContainer = child (subProcessInstance), not parent.
        let Some(target) = parameter_target(&out_param, &child_expression_execution)? else {
            continue;
        };
        let value = if let Some(source_expression) = out_param.source_expression.as_deref() {
            if let Some(value) = SimpleExpression::new(source_expression.to_string())
                .get_value(&child_expression_execution)
            {
                value
            } else if let Some(property_name) = expression_property_name(source_expression) {
                child_variables
                    .get(property_name)
                    .cloned()
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        } else if let Some(source) = out_param.source.as_deref() {
            child_variables
                .get(source.trim())
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };

        // Java IOParameterUtil `:100-104` + CallActivityBehavior `:263-269`.
        if out_param.transient {
            super_execution.set_transient_variable(target, value);
        } else if use_local_scope {
            set_local_variable_with_history(command_context, super_execution, target, value);
        } else {
            set_process_variable_with_history(command_context, super_execution, target, value);
        }
        updated = true;
    }

    if updated {
        persist_execution(command_context, super_execution);
    }

    Ok(())
}

impl ActivityBehavior for CallActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let process_definition_id = execution
            .process_definition_id
            .clone()
            .ok_or_else(|| {
                FlowableError::ExecutionError("Call activity missing process definition id".into())
            })?;
        let activity_id = execution.activity_id.clone().ok_or_else(|| {
            FlowableError::ExecutionError("Call activity missing activity id".into())
        })?;

        let (call_activity, bpmn_model) = {
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
            let call_activity = find_call_activity(process, &activity_id)
                .cloned()
                .ok_or_else(|| {
                    FlowableError::NotFound(format!(
                        "Call activity '{activity_id}' not found in process definition '{process_definition_id}'"
                    ))
                })?;
            (call_activity, model)
        };

        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);
        let called_element =
            resolve_called_element_value(&call_activity, &evaluation_execution)?;
        let child_definition = resolve_called_process_definition(
            command_context,
            &call_activity,
            execution,
            &called_element,
        )?;

        let mut builder = ProcessInstanceBuilder::new()
            .process_definition_id(child_definition.id.clone())
            .process_definition_key(child_definition.key.clone())
            .super_execution_id(execution.id.clone());

        // Preserve child tenant from the resolved definition (fallback may
        // pick a no-tenant definition while parent has a tenant).
        if let Some(tenant_id) = child_definition
            .tenant_id
            .clone()
            .or_else(|| execution.tenant_id.clone())
        {
            builder = builder.tenant_id(tenant_id);
        }

        if let Some(business_key) =
            business_key_from_call_activity(&call_activity, &evaluation_execution, command_context)?
        {
            builder = builder.business_key(business_key);
        }

        let process_instance_id = execution
            .process_instance_id
            .clone()
            .unwrap_or_else(|| execution.id.clone());

        // Java `:154-172,185-187` inheritVariables preserves transient vs durable.
        // evaluation_execution flattens transient into `variables` for EL, so the
        // durable/transient split is recovered from the process-instance root row
        // (and the host execution) which still keep separate maps.
        if call_activity.inherit_variables {
            let mut transient_keys = std::collections::HashSet::new();
            for key in evaluation_execution.transient_variables.keys() {
                transient_keys.insert(key.clone());
            }
            for key in execution.transient_variables.keys() {
                transient_keys.insert(key.clone());
            }
            if let Some(root) = command_context.runtime_store.find_execution(
                &process_instance_id,
                &mut command_context.session,
            ) {
                for key in root.transient_variables.keys() {
                    transient_keys.insert(key.clone());
                }
                // Prefer root's durable + transient maps as the source of truth.
                for (k, v) in root.variables {
                    if transient_keys.contains(&k) {
                        builder = builder.transient_variable(k, v);
                    } else {
                        builder = builder.variable(k, v);
                    }
                }
                for (k, v) in root.transient_variables {
                    if !builder.variables.contains_key(&k)
                        && !builder.transient_variables.contains_key(&k)
                    {
                        builder = builder.transient_variable(k, v);
                    }
                }
            } else {
                for (k, v) in evaluation_execution.process_variables() {
                    if transient_keys.contains(&k) {
                        builder = builder.transient_variable(k, v);
                    } else {
                        builder = builder.variable(k, v);
                    }
                }
            }
        }

        for in_param in &call_activity.in_parameters {
            // In params: sourceContainer is parent (evaluation_execution).
            if let Some(target) = parameter_target(in_param, &evaluation_execution)? {
                let value = parameter_value(in_param, &evaluation_execution);
                if in_param.transient {
                    builder = builder.transient_variable(target, value);
                } else {
                    builder = builder.variable(target, value);
                }
            }
        }

        // processInstanceName: evaluate against child vars after mapping
        // (Java `:189-195` evaluates on subProcessInstance post-variable init).
        if let Some(name_expr) = call_activity.process_instance_name.as_deref() {
            let mut name_scope = evaluation_execution.clone();
            // Overlay variables that will be on the child so expressions can
            // reference in-mapped / inherited names.
            for (k, v) in &builder.variables {
                name_scope.set_process_variable(k.clone(), v.clone());
            }
            for (k, v) in &builder.transient_variables {
                name_scope.set_transient_variable(k.clone(), v.clone());
            }
            if let Ok(name) =
                expression_or_literal_coerced(name_expr, &name_scope, "processInstanceName")
            {
                builder = builder.name(name);
            }
        }

        // Register boundary events (incl. error) on the call activity host
        // before starting the child so parent catches are available when the
        // child throws (P19 / Java ErrorPropagation across call activities).
        let boundary_host_id =
            crate::bpmn::behavior::multi_instance_support::boundary_host_execution_id(
                command_context,
                execution,
            );
        for boundary_event in &call_activity.activity.boundary_events {
            let Some(boundary_event_id) = boundary_event
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .as_ref()
            else {
                continue;
            };

            if let [EventDefinitionEnum::TimerEventDefinition(timer_def)] =
                boundary_event.event.event_definitions.as_slice()
            {
                let already_registered = command_context
                    .runtime_store
                    .find_timer_job_states_by_process_instance_id(
                        &process_instance_id,
                        &mut command_context.session,
                    )
                    .iter()
                    .any(|state| {
                        state.is_boundary
                            && state.activity_id == *boundary_event_id
                            && state.execution_id == boundary_host_id
                    });
                if already_registered {
                    continue;
                }
                let now = command_context.runtime_store.time_source().now();
                let schedule = crate::bpmn::timer_util::resolve_timer_schedule(
                    timer_def.time_date.as_ref(),
                    timer_def.time_duration.as_ref(),
                    timer_def.time_cycle.as_ref(),
                    timer_def.end_date.as_ref(),
                    timer_def.calendar_name.as_ref(),
                    &evaluation_execution,
                    &command_context.config.business_calendar_registry,
                    now,
                )?;
                command_context.runtime_store.insert_timer_job_state(
                    &RuntimeTimerJobState {
                        timer_job_id: Uuid::new_v4().to_string(),
                        process_instance_id: process_instance_id.clone(),
                        execution_id: boundary_host_id.clone(),
                        activity_id: boundary_event_id.clone(),
                        job_state: Some("timer".to_string()),
                        is_boundary: true,
                        attached_activity_id: Some(activity_id.clone()),
                        cancel_activity: boundary_event.cancel_activity,
                        time_duration: schedule.time_duration,
                        time_date: schedule.time_date,
                        time_cycle: schedule.time_cycle,
                        end_date: schedule.end_date,
                        calendar_name: schedule.calendar_name,
                        due_time: schedule.due_time,
                        lock_owner: None,
                        lock_time: None,
                        lock_expiration_time: None,
                        retries: crate::bpmn::timer_util::default_timer_retries(command_context),
                        error_message: None,
                        error_details: None,
                        category: resolve_job_category(
                            &boundary_event.event.flow_node.flow_element.base_element,
                            &evaluation_execution,
                        ),
                        ..Default::default()
                    },
                    &mut command_context.session,
                );
                continue;
            }

            let event_sub = match resolve_boundary_event_subscription(
                boundary_event,
                Some(bpmn_model.as_ref()),
            ) {
                Some(sub) => sub,
                None => {
                    return Err(FlowableError::UnsupportedElement {
                        element_type: "BoundaryEvent".to_string(),
                        activity_id: boundary_event_id.clone(),
                    });
                }
            };

            let configuration =
                crate::bpmn::behavior::boundary_event_activity_behavior::resolve_boundary_configuration(
                    boundary_event,
                    Some(execution),
                );
            crate::bpmn::behavior::boundary_event_activity_behavior::insert_boundary_event_state_with_waiting(
                command_context,
                RuntimeBoundaryEventState {
                    boundary_event_id: boundary_event_id.clone(),
                    attached_activity_id: activity_id.clone(),
                    process_instance_id: process_instance_id.clone(),
                    host_execution_id: boundary_host_id.clone(),
                    cancel_activity: runtime_cancel_activity(boundary_event, &event_sub),
                    event_subscription: event_sub,
                    configuration,
                },
                execution.process_definition_id.as_deref(),
            );
        }

        // Call activity is a wait state until the child process instance ends
        // (or a parent boundary interrupts). Required so interrupting error
        // boundaries can fire (`execute_boundary_trigger` rejects active hosts
        // that are not scopes).
        execution.is_active = false;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        let start_cmd = StartProcessInstanceCmd::new(builder);
        let child_process_instance = start_cmd.execute(command_context)?;

        // Entity links: Java `:202-205` only when enableEntityLinks (default false).
        if command_context.config.enable_entity_links {
            let parent_pi_id = process_instance_id.clone();
            let link = crate::identity::entities::EntityLink {
                id: Uuid::new_v4().to_string(),
                link_type: "child".to_string(),
                scope_id: Some(parent_pi_id),
                scope_type: Some("bpmn".to_string()),
                reference_scope_id: Some(child_process_instance.id.clone()),
                reference_scope_type: Some("bpmn".to_string()),
                hierarchy_type: Some("parent".to_string()),
            };
            command_context
                .runtime_store
                .insert_entity_link(link, &mut command_context.session);
        }

        if let Some(variable_name_expr) = call_activity
            .process_instance_id_variable_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            // Java `:207-213`: evaluate expression against parent execution.
            let variable_name = expression_or_literal_coerced(
                variable_name_expr,
                &evaluation_execution,
                "processInstanceIdVariableName",
            )?;
            // Re-load: child start may have nested commands; keep vars on host.
            if let Some(mut host) = command_context
                .runtime_store
                .find_execution(&execution.id, &mut command_context.session)
            {
                set_process_variable_with_history(
                    command_context,
                    &mut host,
                    variable_name,
                    Value::String(child_process_instance.id),
                );
                persist_execution(command_context, &host);
                *execution = host;
            }
        }

        Ok(())
    }
}
