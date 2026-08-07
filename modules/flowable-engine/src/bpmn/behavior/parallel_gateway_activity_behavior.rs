use crate::agenda::FlowableEngineAgenda;
use crate::agenda::continue_process_operation::find_flow_element;
use crate::bpmn::execution_graph_util::{has_loop_characteristics, has_multi_instance_parent};
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{FlowElementEnum, Process, SequenceFlow};
use std::collections::HashMap;
use uuid::Uuid;

pub struct ParallelGatewayActivityBehavior;

impl Default for ParallelGatewayActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl ParallelGatewayActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

fn gateway_flows(
    flow_element: &FlowElementEnum,
) -> Option<(&Vec<SequenceFlow>, &Vec<SequenceFlow>)> {
    match flow_element {
        FlowElementEnum::ParallelGateway(gateway) => Some((
            &gateway.gateway.flow_node.incoming_flows,
            &gateway.gateway.flow_node.outgoing_flows,
        )),
        _ => None,
    }
}

fn is_end_event(flow_element: Option<&FlowElementEnum>) -> bool {
    matches!(flow_element, Some(FlowElementEnum::EndEvent(_)))
}

fn is_waiting_token_at_parallel_gateway(
    execution: &Execution,
    gateway_id: &str,
    scope_id: &str,
) -> bool {
    execution.activity_id.as_deref() == Some(gateway_id)
        && execution.parallel_scope_id() == scope_id
}

/// Java parity: the process instance itself is an execution
/// (`ExecutionEntityImpl`), so a fork never deletes its scope row — the row is
/// kept inactive as the scope parent of the forked branches
/// (`ParallelGatewayActivityBehavior#execute` inactivates the incoming
/// execution instead of destroying it). Any other token is deleted.
fn delete_or_preserve_scope_execution(command_context: &mut CommandContext, execution: &Execution) {
    if execution.is_process_instance_scope_execution() {
        let mut preserved = execution.clone();
        preserved.is_active = false;
        preserved.is_scope = true;
        preserved.activity_id = None;
        command_context
            .execution_entity_manager
            .update(&preserved, &mut command_context.session);
    } else {
        command_context
            .execution_entity_manager
            .delete(&execution.id, &mut command_context.session);
    }
}

/// Java `ParallelGatewayActivityBehavior#findMultiInstanceParentExecution`
/// (lines 168–188): walk parents until an execution whose current flow
/// element has multi-instance loop characteristics (or is the MI root).
fn find_multi_instance_parent_execution(
    command_context: &mut CommandContext,
    process: &Process,
    execution: &Execution,
) -> Option<Execution> {
    let mut current_parent_id = execution.parent_id.clone();
    while let Some(parent_id) = current_parent_id {
        let parent = command_context
            .execution_entity_manager
            .find_by_id(&parent_id, &mut command_context.session)?;

        if parent.is_multi_instance_root {
            return Some(parent);
        }

        if let Some(activity_id) = parent.activity_id.as_deref()
            && let Some(flow_element) = find_flow_element(process, activity_id)
            && has_loop_characteristics(flow_element)
        {
            return Some(parent);
        }

        current_parent_id = parent.parent_id.clone();
    }
    None
}

/// Java `ParallelGatewayActivityBehavior#isChildOfMultiInstanceExecution`
/// (lines 135–150).
fn is_child_of_multi_instance_execution(
    execution: &Execution,
    multi_instance_execution: &Execution,
    by_id: &HashMap<String, Execution>,
) -> bool {
    let mut current_parent_id = execution.parent_id.clone();
    while let Some(parent_id) = current_parent_id {
        if parent_id == multi_instance_execution.id {
            return true;
        }
        current_parent_id = by_id
            .get(&parent_id)
            .and_then(|parent| parent.parent_id.clone());
    }
    false
}

/// Java `ParallelGatewayActivityBehavior#cleanJoinedExecutions` (lines 125–133):
/// when the parallel join lives under a multi-instance parent, only count
/// joined tokens that belong to the same MI instance tree.
fn clean_joined_executions(
    joined: Vec<Execution>,
    multi_instance_execution: &Execution,
    by_id: &HashMap<String, Execution>,
) -> Vec<Execution> {
    joined
        .into_iter()
        .filter(|execution| {
            is_child_of_multi_instance_execution(execution, multi_instance_execution, by_id)
        })
        .collect()
}

impl ActivityBehavior for ParallelGatewayActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let activity_id = match execution.activity_id.clone() {
            Some(activity_id) => activity_id,
            None => {
                return Ok(());
            }
        };

        let process_definition_id = match execution.process_definition_id.clone() {
            Some(process_definition_id) => process_definition_id,
            None => {
                return Ok(());
            }
        };

        let (incoming_count, outgoing_flows, main_process) = {
            let bpmn_model = match command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id)
            {
                Some(model) => model,
                None => {
                    return Ok(());
                }
            };

            let main_process = match bpmn_model.main_process.as_ref() {
                Some(process) => process,
                None => {
                    return Ok(());
                }
            };

            let flow_element = match find_flow_element(main_process, &activity_id) {
                Some(flow_element) => flow_element,
                None => {
                    return Ok(());
                }
            };

            let (incoming_flows, outgoing_flows) = match gateway_flows(flow_element) {
                Some(flows) => flows,
                None => {
                    return Ok(());
                }
            };

            (
                incoming_flows.len(),
                outgoing_flows.clone(),
                main_process.clone(),
            )
        };

        let scope_id = execution.parallel_scope_id();

        if incoming_count > 1 {
            let mut waiting_execution = execution.clone();
            waiting_execution.is_active = false;
            waiting_execution.is_concurrent = true;
            command_context
                .execution_entity_manager
                .update(&waiting_execution, &mut command_context.session);

            let snapshot = command_context
                .runtime_store
                .snapshot_executions(&mut command_context.session);

            let mut waiting_tokens: Vec<Execution> = snapshot
                .values()
                .filter(|candidate| {
                    candidate.process_instance_id == execution.process_instance_id
                        && is_waiting_token_at_parallel_gateway(candidate, &activity_id, &scope_id)
                })
                .cloned()
                .collect();

            // G1-4: Java cleanJoinedExecutions — under multi-instance parents,
            // drop tokens from sibling MI instances (defense in depth; scope_id
            // already isolates most cases).
            if has_multi_instance_parent(&main_process, &activity_id)
                && let Some(mi_parent) =
                    find_multi_instance_parent_execution(command_context, &main_process, execution)
            {
                waiting_tokens = clean_joined_executions(waiting_tokens, &mi_parent, &snapshot);
            }

            let arrived_count = waiting_tokens.len();
            if arrived_count < incoming_count {
                return Ok(());
            }

            for waiting_token in waiting_tokens
                .into_iter()
                .filter(|candidate| candidate.id != execution.id)
            {
                command_context
                    .execution_entity_manager
                    .delete(&waiting_token.id, &mut command_context.session);
            }

            delete_or_preserve_scope_execution(command_context, execution);
        } else {
            delete_or_preserve_scope_execution(command_context, execution);
        }

        for flow in outgoing_flows {
            let Some(target_ref) = flow.target_ref.clone() else {
                continue;
            };

            let mut child = execution.clone();
            child.id = Uuid::new_v4().to_string();
            child.parent_id = Some(scope_id.clone());
            child.activity_id = Some(target_ref.clone());
            child.is_active = !is_end_event(main_process.flow_element_map.get(&target_ref));
            child.is_concurrent = true;
            child.is_ended = false;
            child.is_scope = false;
            child.is_multi_instance_root = false;
            // Concurrent children start with empty variable maps. Process-level
            // names resolve through the parent VariableScope chain during EL
            // evaluation (see variable_service::evaluation_execution); they
            // must not be snapshot-copied from the fork token.
            child.variables.clear();
            child.local_variables.clear();
            child.transient_variables.clear();

            command_context
                .execution_entity_manager
                .insert(&child, &mut command_context.session);
            command_context
                .agenda
                .plan_continue_process_operation(child);
        }
        Ok(())
    }
}
