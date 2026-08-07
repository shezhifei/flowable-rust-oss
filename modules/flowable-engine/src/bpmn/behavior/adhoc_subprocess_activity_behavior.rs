use crate::agenda::FlowableEngineAgenda;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::FlowElementEnum;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivationCondition {
    pub expression: String,
    pub variables: HashMap<String, serde_json::Value>,
}

impl ActivationCondition {
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            variables: HashMap::new(),
        }
    }

    pub fn with_variables(mut self, variables: HashMap<String, serde_json::Value>) -> Self {
        self.variables = variables;
        self
    }

    pub fn evaluate(
        &self,
        context: &HashMap<String, serde_json::Value>,
    ) -> Result<bool, crate::error::FlowableError> {
        let expr = self.expression.trim();

        if expr.is_empty() {
            return Ok(true);
        }

        let mut combined = context.clone();
        combined.extend(self.variables.clone());

        evaluate_simple_expression(expr, &combined)
    }
}

fn evaluate_simple_expression(
    expr: &str,
    variables: &HashMap<String, serde_json::Value>,
) -> Result<bool, crate::error::FlowableError> {
    let expr = expr.trim();

    // Handle boolean literals
    if expr == "true" {
        return Ok(true);
    }
    if expr == "false" {
        return Ok(false);
    }

    // Handle negation
    if let Some(inner) = expr.strip_prefix('!') {
        let inner_result = evaluate_simple_expression(inner.trim(), variables)?;
        return Ok(!inner_result);
    }

    // Handle parenthesized expressions
    if expr.starts_with('(') && expr.ends_with(')') {
        let inner = &expr[1..expr.len() - 1];
        return evaluate_simple_expression(inner, variables);
    }

    // Handle AND/OR operators
    if let Some((left, right)) = split_binary_op(expr, "&&") {
        let left_val = evaluate_simple_expression(left.trim(), variables)?;
        let right_val = evaluate_simple_expression(right.trim(), variables)?;
        return Ok(left_val && right_val);
    }

    if let Some((left, right)) = split_binary_op(expr, "||") {
        let left_val = evaluate_simple_expression(left.trim(), variables)?;
        let right_val = evaluate_simple_expression(right.trim(), variables)?;
        return Ok(left_val || right_val);
    }

    // Handle comparison operators
    if let Some((left, right)) = split_binary_op(expr, "==") {
        let left_val = resolve_value(left.trim(), variables)?;
        let right_val = resolve_value(right.trim(), variables)?;
        return Ok(left_val == right_val);
    }

    if let Some((left, right)) = split_binary_op(expr, "!=") {
        let left_val = resolve_value(left.trim(), variables)?;
        let right_val = resolve_value(right.trim(), variables)?;
        return Ok(left_val != right_val);
    }

    // Try to resolve as a boolean variable
    let val = resolve_value(expr, variables)?;
    match val {
        serde_json::Value::Bool(b) => Ok(b),
        _ => Err(crate::error::FlowableError::ExecutionError(format!(
            "Expression '{}' evaluated to non-boolean value",
            expr
        ))),
    }
}

fn resolve_value(
    token: &str,
    variables: &HashMap<String, serde_json::Value>,
) -> Result<serde_json::Value, crate::error::FlowableError> {
    let token = token.trim();

    // String literal
    if (token.starts_with('"') && token.ends_with('"'))
        || (token.starts_with('\'') && token.ends_with('\''))
    {
        return Ok(serde_json::Value::String(
            token[1..token.len() - 1].to_string(),
        ));
    }

    // Number literal
    if let Ok(n) = token.parse::<i64>() {
        return Ok(serde_json::Value::Number(n.into()));
    }
    if let Ok(n) = token.parse::<f64>()
        && let Some(num) = serde_json::Number::from_f64(n)
    {
        return Ok(serde_json::Value::Number(num));
    }

    // Boolean literals
    if token == "true" {
        return Ok(serde_json::Value::Bool(true));
    }
    if token == "false" {
        return Ok(serde_json::Value::Bool(false));
    }

    // Null literal
    if token == "null" {
        return Ok(serde_json::Value::Null);
    }

    // Variable lookup
    variables.get(token).cloned().ok_or_else(|| {
        crate::error::FlowableError::ExecutionError(format!("Variable '{}' not found", token))
    })
}

fn split_binary_op<'a>(expr: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    // Find the operator outside of parentheses and quotes
    let mut depth = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let bytes = expr.as_bytes();
    let op_bytes = op.as_bytes();

    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' if !in_single_quote && !in_double_quote => depth += 1,
            b')' if !in_single_quote && !in_double_quote => depth -= 1,
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            _ => {}
        }

        if depth == 0
            && !in_single_quote
            && !in_double_quote
            && i + op_bytes.len() <= bytes.len()
            && &bytes[i..i + op_bytes.len()] == op_bytes
        {
            // Make sure we're not matching part of a longer operator (e.g., != inside ==)
            let after_op = i + op_bytes.len();
            if after_op < bytes.len() {
                let next_char = bytes[after_op];
                // For ==, don't match if followed by another = (that would be === or similar)
                if op == "==" && next_char == b'=' {
                    continue;
                }
            }
            return Some((&expr[..i], &expr[i + op_bytes.len()..]));
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdhocTaskState {
    pub task_id: String,
    pub activity_id: String,
    pub is_activated: bool,
    pub is_completed: bool,
    pub activation_condition: Option<ActivationCondition>,
}

pub struct AdhocSubProcessActivityBehavior;

impl Default for AdhocSubProcessActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl AdhocSubProcessActivityBehavior {
    pub fn new() -> Self {
        Self
    }

    pub fn activate_task(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
        task_id: &str,
    ) -> Result<(), crate::error::FlowableError> {
        let activity_id = execution.activity_id.clone().ok_or_else(|| {
            crate::error::FlowableError::ExecutionError("Execution has no activity id".to_string())
        })?;

        let process_definition_id = execution.process_definition_id.clone().ok_or_else(|| {
            crate::error::FlowableError::ExecutionError(
                "Execution has no process definition id".to_string(),
            )
        })?;

        // Verify the task exists in the ad-hoc subprocess
        let mut found_task = None;
        {
            if let Some(bpmn_model) = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id)
                && let Some(process) = bpmn_model.main_process.as_ref()
            {
                for flow_element in &process.flow_elements {
                    if let FlowElementEnum::AdhocSubProcess(adhoc) = flow_element
                        && adhoc
                            .sub_process
                            .activity
                            .flow_node
                            .flow_element
                            .base_element
                            .id
                            .as_deref()
                            == Some(&activity_id)
                    {
                        for inner_element in &adhoc.sub_process.flow_elements {
                            if let Some(id) = get_flow_element_id(inner_element)
                                && id == task_id
                            {
                                found_task = Some(id.clone());
                                break;
                            }
                        }
                    }
                }
            }
        }

        let _ = found_task.ok_or_else(|| {
            crate::error::FlowableError::NotFound(format!(
                "Task '{}' not found in ad-hoc subprocess '{}'",
                task_id, activity_id
            ))
        })?;

        // Create a child execution for the activated task
        let child_execution = Execution {
            id: Uuid::new_v4().to_string(),
            parent_id: Some(execution.id.clone()),
            super_execution_id: None,
            root_process_instance_id: execution.root_process_instance_id.clone(),
            process_instance_id: execution.process_instance_id.clone(),
            process_definition_id: execution.process_definition_id.clone(),
            process_definition_key: execution.process_definition_key.clone(),
            process_definition_name: execution.process_definition_name.clone(),
            process_definition_version: execution.process_definition_version,
            activity_id: Some(task_id.to_string()),
            activity_name: None,
            name: None,
            description: None,
            is_suspended: false,
            is_ended: false,
            is_active: true,
            is_concurrent: false,
            is_scope: false,
            is_multi_instance_root: false,
            tenant_id: execution.tenant_id.clone(),
            ..Default::default()
        };

        command_context
            .execution_entity_manager
            .insert(&child_execution, &mut command_context.session);
        command_context
            .agenda
            .plan_continue_process_operation(child_execution);

        Ok(())
    }

    pub fn complete_task(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
        task_id: &str,
    ) -> Result<(), crate::error::FlowableError> {
        // Find child execution for the task
        let child_executions = command_context
            .execution_entity_manager
            .find_child_executions_by_parent_execution_id(
                &execution.id,
                &mut command_context.session,
            );

        let task_execution = child_executions
            .iter()
            .find(|e| e.activity_id.as_deref() == Some(task_id) && !e.is_ended)
            .cloned()
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Open execution for task '{}' not found",
                    task_id
                ))
            })?;

        // End the task execution
        let mut ended_execution = task_execution;
        ended_execution.is_ended = true;
        ended_execution.is_active = false;
        command_context
            .execution_entity_manager
            .update(&ended_execution, &mut command_context.session);

        // Java TakeOutgoingSequenceFlowsOperation.handleAdhocSubProcess
        // (:293-326): leaf tasks with no outgoing flows also evaluate the
        // completion condition (and honour cancelRemainingInstances) after the
        // child ends. Shared with the take-outgoing path.
        try_auto_complete_adhoc_after_child_leave(&ended_execution, command_context)
    }

    /// Java `GetEnabledActivitiesForAdhocSubProcessCmd`: activities with no
    /// incoming flows (and sequential: none when any child is already active).
    pub fn get_enabled_activities(
        &self,
        execution: &Execution,
        command_context: &mut CommandContext,
    ) -> Result<Vec<EnabledAdhocActivity>, crate::error::FlowableError> {
        let adhoc = load_adhoc_subprocess(execution, command_context)?;

        if is_sequential_ordering(&adhoc.ordering) {
            let children = command_context
                .execution_entity_manager
                .find_child_executions_by_parent_execution_id(
                    &execution.id,
                    &mut command_context.session,
                );
            // User tasks wait with is_active=false; treat any non-ended child as
            // "currently running" for sequential ordering (Java: getExecutions).
            if children.iter().any(|c| !c.is_ended) {
                return Ok(Vec::new());
            }
        }

        let mut enabled = Vec::new();
        for element in &adhoc.sub_process.flow_elements {
            if let Some(id) = flow_node_id_if_no_incoming(element) {
                let name = flow_node_name(element);
                enabled.push(EnabledAdhocActivity {
                    id,
                    name,
                    element_type: flow_node_type_name(element),
                });
            }
        }
        Ok(enabled)
    }

    /// Java `ExecuteActivityForAdhocSubProcessCmd`.
    pub fn execute_activity(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
        activity_id: &str,
    ) -> Result<Execution, crate::error::FlowableError> {
        let adhoc = load_adhoc_subprocess(execution, command_context)?;

        if is_sequential_ordering(&adhoc.ordering) {
            let children = command_context
                .execution_entity_manager
                .find_child_executions_by_parent_execution_id(
                    &execution.id,
                    &mut command_context.session,
                );
            if children.iter().any(|c| !c.is_ended) {
                return Err(crate::error::FlowableError::ExecutionError(format!(
                    "Sequential ad-hoc sub process in execution '{}' already has an active execution",
                    execution.id
                )));
            }
        }

        let found = adhoc.sub_process.flow_elements.iter().find(|element| {
            flow_node_id_if_no_incoming(element).as_deref() == Some(activity_id)
        });
        if found.is_none() {
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "The requested activity with id {} can not be enabled in execution '{}'",
                activity_id, execution.id
            )));
        }

        let child_execution = Execution {
            id: Uuid::new_v4().to_string(),
            parent_id: Some(execution.id.clone()),
            super_execution_id: None,
            root_process_instance_id: execution.root_process_instance_id.clone(),
            process_instance_id: execution.process_instance_id.clone(),
            process_definition_id: execution.process_definition_id.clone(),
            process_definition_key: execution.process_definition_key.clone(),
            process_definition_name: execution.process_definition_name.clone(),
            process_definition_version: execution.process_definition_version,
            activity_id: Some(activity_id.to_string()),
            activity_name: None,
            name: None,
            description: None,
            is_suspended: false,
            is_ended: false,
            is_active: true,
            is_concurrent: false,
            is_scope: false,
            is_multi_instance_root: false,
            tenant_id: execution.tenant_id.clone(),
            ..Default::default()
        };

        command_context
            .execution_entity_manager
            .insert(&child_execution, &mut command_context.session);
        command_context
            .agenda
            .plan_continue_process_operation(child_execution.clone());

        Ok(child_execution)
    }

    /// Java `CompleteAdhocSubProcessCmd`: requires no active children, then leave.
    pub fn complete_adhoc_subprocess(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let _ = load_adhoc_subprocess(execution, command_context)?;

        let children = command_context
            .execution_entity_manager
            .find_child_executions_by_parent_execution_id(
                &execution.id,
                &mut command_context.session,
            );
        if children.iter().any(|c| !c.is_ended) {
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "Ad-hoc sub process has running child executions that need to be completed first. execution '{}'",
                execution.id
            )));
        }

        // Leave the ad-hoc: take its outgoing sequence flows.
        //
        // Java creates a child of the parent with the adhoc as current flow
        // element, then deletes the adhoc scope. Rust often keeps the adhoc on
        // the process-instance row itself (`parent_id == None`); in that case
        // leave directly on the same execution.
        if let Some(parent_id) = execution.parent_id.clone() {
            execution.is_active = false;
            execution.is_ended = true;
            command_context
                .execution_entity_manager
                .update(execution, &mut command_context.session);

            let parent = command_context
                .runtime_store
                .find_execution(&parent_id, &mut command_context.session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Parent execution '{}' was not found",
                        parent_id
                    ))
                })?;

            let outgoing = Execution {
                id: Uuid::new_v4().to_string(),
                parent_id: Some(parent.id.clone()),
                process_instance_id: parent.process_instance_id.clone(),
                process_definition_id: parent.process_definition_id.clone(),
                process_definition_key: parent.process_definition_key.clone(),
                process_definition_name: parent.process_definition_name.clone(),
                process_definition_version: parent.process_definition_version,
                root_process_instance_id: parent.root_process_instance_id.clone(),
                activity_id: execution.activity_id.clone(),
                is_active: true,
                is_scope: false,
                tenant_id: parent.tenant_id.clone(),
                ..Default::default()
            };
            command_context
                .execution_entity_manager
                .insert(&outgoing, &mut command_context.session);

            command_context
                .execution_entity_manager
                .delete(&execution.id, &mut command_context.session);

            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(outgoing);
        } else {
            execution.is_active = true;
            execution.is_ended = false;
            execution.is_scope = false;
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

/// Lightweight description of an enabled ad-hoc activity (Java returns `FlowNode`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnabledAdhocActivity {
    pub id: String,
    pub name: Option<String>,
    pub element_type: String,
}

/// Java `TakeOutgoingSequenceFlowsOperation.handleAdhocSubProcess`
/// (`TakeOutgoingSequenceFlowsOperation.java:293-326`).
///
/// After a child inside an ad-hoc subprocess leaves (take-outgoing or leaf
/// complete), evaluate optional `completionCondition`. When true:
/// - `cancelRemainingInstances=true` (default): delete sibling children and end
///   the ad-hoc (take its outgoing flows).
/// - `cancelRemainingInstances=false`: end the ad-hoc only when no other
///   non-ended siblings remain.
///
/// Explicit API `completeAdhocSubProcess` is separate and always errors when
/// any non-ended child exists (`CompleteAdhocSubProcessCmd.java:53-56`).
pub fn try_auto_complete_adhoc_after_child_leave(
    child_execution: &Execution,
    command_context: &mut CommandContext,
) -> Result<(), crate::error::FlowableError> {
    let Some(parent_id) = child_execution.parent_id.as_deref() else {
        return Ok(());
    };
    let Some(mut parent_execution) = command_context
        .runtime_store
        .find_execution(parent_id, &mut command_context.session)
    else {
        return Ok(());
    };

    let Ok(adhoc) = load_adhoc_subprocess(&parent_execution, command_context) else {
        return Ok(());
    };

    let Some(condition_text) = adhoc.completion_condition.as_deref() else {
        return Ok(());
    };

    let adhoc_id = adhoc
        .sub_process
        .activity
        .flow_node
        .flow_element
        .base_element
        .id
        .clone();

    // Java: Condition.evaluate(adhocSubProcess.getId(), execution) on the
    // leaving child execution (parent-chain variable resolution via
    // evaluation_execution).
    let eval_exec = crate::engine::variable_service::evaluation_execution(
        command_context,
        child_execution,
    );
    let expression = crate::el::expression::SimpleExpression::new(condition_text.to_string());
    let condition = crate::el::uel_expression_condition::UelExpressionCondition::new(Box::new(
        expression,
    ));
    use crate::el::condition::Condition;
    let complete_adhoc = condition.evaluate(adhoc_id.as_deref(), &eval_exec)?;
    if !complete_adhoc {
        return Ok(());
    }

    // Java :311-320 — when cancelRemainingInstances is false, only end if the
    // leaving child is the sole remaining child under the ad-hoc parent.
    let mut end_adhoc = true;
    if !adhoc.cancel_remaining_instances {
        let siblings = command_context
            .execution_entity_manager
            .find_child_executions_by_parent_execution_id(
                parent_id,
                &mut command_context.session,
            );
        for sibling in &siblings {
            if sibling.id != child_execution.id && !sibling.is_ended {
                end_adhoc = false;
                break;
            }
        }
    }

    if !end_adhoc {
        return Ok(());
    }

    // Java EndExecutionOperation on the ad-hoc scope deletes all children first
    // (cancelRemainingInstances=true path, and the last-sibling path where only
    // ended/leaving children remain). Drop remaining children so
    // complete_adhoc_subprocess can leave.
    let remaining_children: Vec<String> = command_context
        .execution_entity_manager
        .find_child_executions_by_parent_execution_id(parent_id, &mut command_context.session)
        .into_iter()
        .map(|c| c.id)
        .collect();
    for child_id in remaining_children {
        crate::bpmn::behavior::multi_instance_support::delete_execution_tree(
            command_context,
            &child_id,
        );
    }

    AdhocSubProcessActivityBehavior::new()
        .complete_adhoc_subprocess(&mut parent_execution, command_context)
}

fn is_sequential_ordering(ordering: &Option<String>) -> bool {
    ordering
        .as_deref()
        .is_some_and(|o| o.eq_ignore_ascii_case("Sequential"))
}

fn load_adhoc_subprocess(
    execution: &Execution,
    command_context: &CommandContext,
) -> Result<flowable_bpmn_model::model::AdhocSubProcess, crate::error::FlowableError> {
    let activity_id = execution.activity_id.as_deref().ok_or_else(|| {
        crate::error::FlowableError::ExecutionError(
            "The current flow element of the requested execution is not an ad-hoc sub process"
                .to_string(),
        )
    })?;
    let process_definition_id = execution.process_definition_id.as_deref().ok_or_else(|| {
        crate::error::FlowableError::ExecutionError(
            "Execution has no process definition id".to_string(),
        )
    })?;
    let bpmn_model = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)
        .ok_or_else(|| {
            crate::error::FlowableError::NotFound(format!(
                "BPMN model for process definition '{}' was not found",
                process_definition_id
            ))
        })?;
    let process = bpmn_model.main_process.as_ref().ok_or_else(|| {
        crate::error::FlowableError::ExecutionError("Process has no main process".to_string())
    })?;

    for flow_element in &process.flow_elements {
        if let FlowElementEnum::AdhocSubProcess(adhoc) = flow_element
            && adhoc
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref()
                == Some(activity_id)
        {
            return Ok(adhoc.clone());
        }
    }
    Err(crate::error::FlowableError::ExecutionError(format!(
        "The current flow element of the requested execution '{}' is not an ad-hoc sub process",
        execution.id
    )))
}

fn flow_node_id_if_no_incoming(element: &FlowElementEnum) -> Option<String> {
    let (id, incoming_empty) = match element {
        FlowElementEnum::UserTask(t) => (
            t.task.activity.flow_node.flow_element.base_element.id.clone(),
            t.task.activity.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::ServiceTask(t) => (
            t.task.activity.flow_node.flow_element.base_element.id.clone(),
            t.task.activity.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::CaseServiceTask(t) => (
            t.service_task.task.activity.flow_node.flow_element.base_element.id.clone(),
            t.service_task.task.activity.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::ManualTask(t) => (
            t.task.activity.flow_node.flow_element.base_element.id.clone(),
            t.task.activity.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::ScriptTask(t) => (
            t.task.activity.flow_node.flow_element.base_element.id.clone(),
            t.task.activity.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::ReceiveTask(t) => (
            t.task.activity.flow_node.flow_element.base_element.id.clone(),
            t.task.activity.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::BusinessRuleTask(t) => (
            t.task.activity.flow_node.flow_element.base_element.id.clone(),
            t.task.activity.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::Task(t) => (
            t.activity.flow_node.flow_element.base_element.id.clone(),
            t.activity.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::SubProcess(s) => (
            s.activity.flow_node.flow_element.base_element.id.clone(),
            s.activity.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::StartEvent(e) => (
            e.event.flow_node.flow_element.base_element.id.clone(),
            e.event.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::ExclusiveGateway(g) => (
            g.gateway.flow_node.flow_element.base_element.id.clone(),
            g.gateway.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::ParallelGateway(g) => (
            g.gateway.flow_node.flow_element.base_element.id.clone(),
            g.gateway.flow_node.incoming_flows.is_empty(),
        ),
        FlowElementEnum::InclusiveGateway(g) => (
            g.gateway.flow_node.flow_element.base_element.id.clone(),
            g.gateway.flow_node.incoming_flows.is_empty(),
        ),
        _ => return None,
    };
    if incoming_empty {
        id
    } else {
        None
    }
}

fn flow_node_name(element: &FlowElementEnum) -> Option<String> {
    match element {
        FlowElementEnum::UserTask(t) => t.task.activity.flow_node.flow_element.name.clone(),
        FlowElementEnum::ServiceTask(t) => t.task.activity.flow_node.flow_element.name.clone(),
        FlowElementEnum::ManualTask(t) => t.task.activity.flow_node.flow_element.name.clone(),
        FlowElementEnum::ScriptTask(t) => t.task.activity.flow_node.flow_element.name.clone(),
        FlowElementEnum::ReceiveTask(t) => t.task.activity.flow_node.flow_element.name.clone(),
        FlowElementEnum::BusinessRuleTask(t) => t.task.activity.flow_node.flow_element.name.clone(),
        FlowElementEnum::Task(t) => t.activity.flow_node.flow_element.name.clone(),
        FlowElementEnum::SubProcess(s) => s.activity.flow_node.flow_element.name.clone(),
        FlowElementEnum::StartEvent(e) => e.event.flow_node.flow_element.name.clone(),
        _ => None,
    }
}

fn flow_node_type_name(element: &FlowElementEnum) -> String {
    match element {
        FlowElementEnum::UserTask(_) => "userTask",
        FlowElementEnum::ServiceTask(_) => "serviceTask",
        FlowElementEnum::ManualTask(_) => "manualTask",
        FlowElementEnum::ScriptTask(_) => "scriptTask",
        FlowElementEnum::ReceiveTask(_) => "receiveTask",
        FlowElementEnum::BusinessRuleTask(_) => "businessRuleTask",
        FlowElementEnum::Task(_) => "task",
        FlowElementEnum::SubProcess(_) => "subProcess",
        FlowElementEnum::StartEvent(_) => "startEvent",
        FlowElementEnum::ExclusiveGateway(_) => "exclusiveGateway",
        FlowElementEnum::ParallelGateway(_) => "parallelGateway",
        FlowElementEnum::InclusiveGateway(_) => "inclusiveGateway",
        _ => "flowNode",
    }
    .to_string()
}

fn get_flow_element_id(element: &FlowElementEnum) -> Option<String> {
    match element {
        FlowElementEnum::UserTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        FlowElementEnum::ServiceTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        FlowElementEnum::ManualTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        FlowElementEnum::ScriptTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        FlowElementEnum::BusinessRuleTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        FlowElementEnum::ReceiveTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        FlowElementEnum::SubProcess(sub_process) => sub_process
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        _ => None,
    }
}

impl ActivityBehavior for AdhocSubProcessActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let activity_id = match execution.activity_id.clone() {
            Some(id) => id,
            None => return Ok(()),
        };

        let process_definition_id = match execution.process_definition_id.clone() {
            Some(id) => id,
            None => return Ok(()),
        };

        // AdhocSubProcess doesn't necessarily have a StartEvent.
        // For minimal M4 Task 3, let's just act like a SubProcess and try to find a StartEvent
        // or just wait.
        let mut start_event_id = None;
        {
            if let Some(bpmn_model) = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id)
                && let Some(process) = bpmn_model.main_process.as_ref()
            {
                for flow_element in &process.flow_elements {
                    if let FlowElementEnum::AdhocSubProcess(adhoc) = flow_element
                        && adhoc
                            .sub_process
                            .activity
                            .flow_node
                            .flow_element
                            .base_element
                            .id
                            .as_deref()
                            == Some(&activity_id)
                    {
                        for inner_element in &adhoc.sub_process.flow_elements {
                            if let FlowElementEnum::StartEvent(start_event) = inner_element
                                && start_event.event.event_definitions.is_empty()
                            {
                                start_event_id = start_event
                                    .event
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .clone();
                                break;
                            }
                        }
                    }
                }
            }
        }

        if let Some(start_id) = start_event_id {
            execution.is_scope = true;
            command_context
                .execution_entity_manager
                .update(execution, &mut command_context.session);

            let child_execution = Execution {
                id: Uuid::new_v4().to_string(),
                parent_id: Some(execution.id.clone()),
                super_execution_id: None,
                root_process_instance_id: execution.root_process_instance_id.clone(),
                process_instance_id: execution.process_instance_id.clone(),
                process_definition_id: execution.process_definition_id.clone(),
                process_definition_key: execution.process_definition_key.clone(),
                process_definition_name: execution.process_definition_name.clone(),
                process_definition_version: execution.process_definition_version,
                activity_id: Some(start_id),
                activity_name: None,
                name: None,
                description: None,
                is_suspended: false,
                is_ended: false,
                is_active: true,
                is_concurrent: false,
                is_scope: false,
                is_multi_instance_root: false,
                tenant_id: execution.tenant_id.clone(),
                ..Default::default()
            };

            command_context
                .execution_entity_manager
                .insert(&child_execution, &mut command_context.session);
            command_context
                .agenda
                .plan_continue_process_operation(child_execution);
        } else {
            // For ad-hoc without start event, set up as a scope and wait for manual
            // activation via AdhocSubProcessActivityBehavior::activate_task().
            // The execution is marked inactive so it does not auto-progress; child
            // executions created by activate_task() will run activities inside this scope.
            execution.is_scope = true;
            execution.is_active = false;
            command_context
                .execution_entity_manager
                .update(execution, &mut command_context.session);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_activation_condition_empty_always_true() {
        let condition = ActivationCondition::new("");
        let context = HashMap::new();
        assert!(condition.evaluate(&context).unwrap());
    }

    #[test]
    fn test_activation_condition_true_literal() {
        let condition = ActivationCondition::new("true");
        let context = HashMap::new();
        assert!(condition.evaluate(&context).unwrap());
    }

    #[test]
    fn test_activation_condition_false_literal() {
        let condition = ActivationCondition::new("false");
        let context = HashMap::new();
        assert!(!condition.evaluate(&context).unwrap());
    }

    #[test]
    fn test_activation_condition_negation() {
        let condition = ActivationCondition::new("!false");
        let context = HashMap::new();
        assert!(condition.evaluate(&context).unwrap());

        let condition = ActivationCondition::new("!true");
        assert!(!condition.evaluate(&context).unwrap());
    }

    #[test]
    fn test_activation_condition_and_operator() {
        let condition = ActivationCondition::new("true && true");
        let context = HashMap::new();
        assert!(condition.evaluate(&context).unwrap());

        let condition = ActivationCondition::new("true && false");
        assert!(!condition.evaluate(&context).unwrap());
    }

    #[test]
    fn test_activation_condition_or_operator() {
        let condition = ActivationCondition::new("false || true");
        let context = HashMap::new();
        assert!(condition.evaluate(&context).unwrap());

        let condition = ActivationCondition::new("false || false");
        assert!(!condition.evaluate(&context).unwrap());
    }

    #[test]
    fn test_activation_condition_variable_lookup() {
        let mut variables = HashMap::new();
        variables.insert("approved".to_string(), serde_json::Value::Bool(true));

        let condition = ActivationCondition::new("approved");
        assert!(condition.evaluate(&variables).unwrap());

        variables.insert("approved".to_string(), serde_json::Value::Bool(false));
        assert!(!condition.evaluate(&variables).unwrap());
    }

    #[test]
    fn test_activation_condition_equality() {
        let mut variables = HashMap::new();
        variables.insert(
            "status".to_string(),
            serde_json::Value::String("ready".to_string()),
        );

        let condition = ActivationCondition::new("status == 'ready'");
        assert!(condition.evaluate(&variables).unwrap());

        let condition = ActivationCondition::new("status == 'pending'");
        assert!(!condition.evaluate(&variables).unwrap());
    }

    #[test]
    fn test_activation_condition_inequality() {
        let mut variables = HashMap::new();
        variables.insert(
            "status".to_string(),
            serde_json::Value::String("ready".to_string()),
        );

        let condition = ActivationCondition::new("status != 'pending'");
        assert!(condition.evaluate(&variables).unwrap());

        let condition = ActivationCondition::new("status != 'ready'");
        assert!(!condition.evaluate(&variables).unwrap());
    }

    #[test]
    fn test_activation_condition_complex_expression() {
        let mut variables = HashMap::new();
        variables.insert("approved".to_string(), serde_json::Value::Bool(true));
        variables.insert(
            "status".to_string(),
            serde_json::Value::String("ready".to_string()),
        );

        let condition = ActivationCondition::new("approved && status == 'ready'");
        assert!(condition.evaluate(&variables).unwrap());

        let condition = ActivationCondition::new("approved && status == 'pending'");
        assert!(!condition.evaluate(&variables).unwrap());
    }

    #[test]
    fn test_activation_condition_with_inline_variables() {
        let condition = ActivationCondition::new("approved").with_variables({
            let mut vars = HashMap::new();
            vars.insert("approved".to_string(), serde_json::Value::Bool(true));
            vars
        });

        let context = HashMap::new();
        assert!(condition.evaluate(&context).unwrap());
    }

    #[test]
    fn test_activation_condition_missing_variable() {
        let condition = ActivationCondition::new("missing_var");
        let context = HashMap::new();
        assert!(condition.evaluate(&context).is_err());
    }

    #[test]
    fn test_activation_condition_parenthesized() {
        let condition = ActivationCondition::new("(true)");
        let context = HashMap::new();
        assert!(condition.evaluate(&context).unwrap());

        let condition = ActivationCondition::new("(false)");
        assert!(!condition.evaluate(&context).unwrap());
    }

    #[test]
    fn test_activation_condition_number_comparison() {
        let mut variables = HashMap::new();
        variables.insert("count".to_string(), serde_json::json!(5));

        let condition = ActivationCondition::new("count == 5");
        assert!(condition.evaluate(&variables).unwrap());

        let condition = ActivationCondition::new("count == 3");
        assert!(!condition.evaluate(&variables).unwrap());
    }
}
