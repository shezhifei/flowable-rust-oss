use crate::agenda::FlowableEngineAgenda;
use crate::agenda::continue_process_operation::find_flow_element;
use crate::bpmn::execution_graph_util::{is_asynchronous_activity, is_reachable};
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{FlowElementEnum, Process};
use uuid::Uuid;

pub struct InclusiveGatewayActivityBehavior;

impl Default for InclusiveGatewayActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl InclusiveGatewayActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

fn gateway_incoming_count(flow_element: &FlowElementEnum) -> usize {
    match flow_element {
        FlowElementEnum::InclusiveGateway(gateway) => {
            gateway.gateway.flow_node.incoming_flows.len()
        }
        _ => 0,
    }
}

/// Java `InclusiveGatewayActivityBehavior#executeInclusiveGatewayLogic`
/// (lines 63–115): activate the join only when **no** other process-instance
/// execution can still reach this gateway (via `ExecutionGraphUtil.isReachable`)
/// on the same parent path.
///
/// This replaces the previous `__inclusive_gateway_expected_token_count`
/// counter, which permanently stuck the join when a forked branch was
/// destroyed by boundary/terminate/dead-path end without arriving.
fn one_execution_can_reach_gateway(
    process: &Process,
    process_definition_id: &str,
    gateway_activity_id: &str,
    join_execution: &Execution,
    all_executions: &[Execution],
) -> bool {
    let _ = process_definition_id;
    let join_parent_id = join_execution.parent_id.as_deref();

    for candidate in all_executions {
        if candidate.process_instance_id != join_execution.process_instance_id {
            continue;
        }

        let Some(candidate_activity_id) = candidate.activity_id.as_deref() else {
            continue;
        };

        if candidate_activity_id != gateway_activity_id {
            if is_reachable(process, candidate_activity_id, gateway_activity_id)
                && candidate.parent_id.as_deref() == join_parent_id
            {
                return true;
            }
        } else if candidate.is_active
            && (candidate.id == join_execution.id
                || find_flow_element(process, candidate_activity_id)
                    .is_some_and(is_asynchronous_activity))
        {
            // Special case: already at the gateway but not yet inactivated /
            // async activity still pending.
            return true;
        }
    }

    false
}

/// P58: Java `CommandInvoker#execute` (CommandInvoker.java:82-88) plans an
/// `ExecuteInactiveBehaviorsOperation` once the agenda has drained; that
/// operation (ExecuteInactiveBehaviorsOperation.java:49-101) re-runs the join
/// logic of every inactive execution whose current activity implements
/// `InactiveActivityBehavior` — for BPMN that is the inclusive gateway
/// (InclusiveGatewayActivityBehavior.java:58-61 `executeInactive` →
/// `executeInclusiveGatewayLogic(execution, true)`).
///
/// Without this re-evaluation, the ordering «token A parks at the join →
/// sibling branch B is destroyed later in the same command (interrupting
/// boundary / terminate end / dead path)» leaves the join stuck forever:
/// nothing re-runs the reachability check for the parked token.
///
/// Returns `true` when at least one waiting join was activated (new agenda
/// operations were planned), so the caller can drain the agenda again and
/// call this once more until a fixpoint is reached — activating one join may
/// unblock another (Java re-plans the operation through the agenda loop).
/// Java `ExecuteInactiveBehaviorsOperation.java:69-74` collects the flow-node
/// ids whose behavior implements `InactiveActivityBehavior` before touching
/// any execution — for this engine that is the multi-incoming inclusive
/// gateway. Returns the join ids (recursing into subprocesses); empty means
/// the definition can be skipped entirely.
fn collect_inclusive_join_ids(flow_elements: &[FlowElementEnum], join_ids: &mut Vec<String>) {
    for flow_element in flow_elements {
        match flow_element {
            FlowElementEnum::InclusiveGateway(gateway) => {
                if gateway.gateway.flow_node.incoming_flows.len() > 1
                    && let Some(id) = gateway.gateway.flow_node.flow_element.base_element.id.clone()
                {
                    join_ids.push(id);
                }
            }
            FlowElementEnum::SubProcess(sub) => {
                collect_inclusive_join_ids(&sub.flow_elements, join_ids);
            }
            FlowElementEnum::EventSubProcess(sub) => {
                collect_inclusive_join_ids(&sub.sub_process.flow_elements, join_ids);
            }
            FlowElementEnum::AdhocSubProcess(sub) => {
                collect_inclusive_join_ids(&sub.sub_process.flow_elements, join_ids);
            }
            _ => {}
        }
    }
}

pub fn execute_inactive_inclusive_joins(command_context: &mut CommandContext) -> bool {
    // Java scopes the scan to the executions involved in this command
    // (CommandInvoker.java:83-84); a command that never wrote an execution
    // costs nothing here.
    let involved = crate::persistence::runtime_store::take_involved_process_instances();
    if involved.is_empty() {
        return false;
    }

    // Model gate (ExecuteInactiveBehaviorsOperation.java:69-76): only fetch
    // executions when the definition actually contains a multi-incoming
    // inclusive gateway.
    let mut gated: Vec<(String, std::sync::Arc<Process>, Vec<String>)> = Vec::new();
    for (pi_id, def_id) in involved {
        let Some(bpmn_model) = command_context.deployment_manager.get_bpmn_model(&def_id) else {
            continue;
        };
        let Some(main_process) = bpmn_model.main_process.as_ref() else {
            continue;
        };
        let mut join_ids = Vec::new();
        collect_inclusive_join_ids(&main_process.flow_elements, &mut join_ids);
        if !join_ids.is_empty() {
            gated.push((pi_id, std::sync::Arc::new(main_process.clone()), join_ids));
        }
    }
    if gated.is_empty() {
        return false;
    }

    let all_executions: Vec<Execution> = command_context
        .runtime_store
        .snapshot_executions(&mut command_context.session)
        .into_values()
        .collect();

    let mut activated = false;
    let mut handled_groups: Vec<(Option<String>, String, String)> = Vec::new();

    for (pi_id, main_process, join_ids) in &gated {
        for waiting in &all_executions {
            if waiting.process_instance_id.as_deref() != Some(pi_id.as_str()) {
                continue;
            }
            // Waiting join tokens carry the signature set by `execute` below:
            // parked at the gateway, inactivated, concurrent (Java
            // ExecuteInactiveBehaviorsOperation.java:79-88 filters on
            // `!inactiveExecution.isActive()` at a flow node whose behavior
            // implements InactiveActivityBehavior).
            if waiting.is_active || !waiting.is_concurrent || waiting.is_ended {
                continue;
            }
            let Some(activity_id) = waiting.activity_id.clone() else {
                continue;
            };
            if !join_ids.contains(&activity_id) {
                continue;
            }
            let Some(process_definition_id) = waiting.process_definition_id.clone() else {
                continue;
            };
            let scope_id = waiting.parallel_scope_id();
            let group_key = (
                waiting.process_instance_id.clone(),
                activity_id.clone(),
                scope_id.clone(),
            );
            if handled_groups.contains(&group_key) {
                continue;
            }
            handled_groups.push(group_key);

            if one_execution_can_reach_gateway(
                main_process,
                &process_definition_id,
                &activity_id,
                waiting,
                &all_executions,
            ) {
                // Another token can still arrive — keep waiting. (If a join we
                // activated earlier in this scan changes that, the caller's
                // fixpoint loop re-evaluates with a fresh snapshot.)
                continue;
            }

            // Java InclusiveGatewayActivityBehavior.java:95-115: no execution
            // can reach the join any more — remove the waiting tokens and leave.
            let waiting_tokens: Vec<&Execution> = all_executions
                .iter()
                .filter(|candidate| {
                    candidate.process_instance_id == waiting.process_instance_id
                        && candidate.activity_id.as_deref() == Some(activity_id.as_str())
                        && candidate.parallel_scope_id() == scope_id
                })
                .collect();

            for waiting_token in &waiting_tokens {
                command_context
                    .execution_entity_manager
                    .delete(&waiting_token.id, &mut command_context.session);
            }

            let mut merged = waiting.clone();
            merged.id = Uuid::new_v4().to_string();
            merged.parent_id = Some(scope_id.clone());
            merged.activity_id = Some(activity_id.clone());
            merged.is_active = true;
            merged.is_concurrent = false;
            merged.is_ended = false;

            command_context
                .execution_entity_manager
                .insert(&merged, &mut command_context.session);

            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(merged);

            activated = true;
        }
    }

    activated
}

impl ActivityBehavior for InclusiveGatewayActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let activity_id = match execution.activity_id.clone() {
            Some(id) => id,
            None => {
                return Ok(());
            }
        };

        let process_definition_id = match execution.process_definition_id.clone() {
            Some(id) => id,
            None => {
                return Ok(());
            }
        };

        let (incoming_count, main_process) = {
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
                Some(fe) => fe,
                None => {
                    return Ok(());
                }
            };

            (gateway_incoming_count(flow_element), main_process.clone())
        };

        let scope_id = execution.parallel_scope_id();

        if incoming_count > 1 {
            // ── Join behavior (Java InclusiveGatewayActivityBehavior) ──
            // Inactivate this token and keep it at the gateway until no other
            // same-path execution can still reach the join.
            let mut waiting_execution = execution.clone();
            waiting_execution.is_active = false;
            waiting_execution.is_concurrent = true;
            command_context
                .execution_entity_manager
                .update(&waiting_execution, &mut command_context.session);

            let all_executions: Vec<Execution> = command_context
                .runtime_store
                .snapshot_executions(&mut command_context.session)
                .into_values()
                .collect();

            if one_execution_can_reach_gateway(
                &main_process,
                &process_definition_id,
                &activity_id,
                &waiting_execution,
                &all_executions,
            ) {
                // At least one other token can still arrive — stay inactive.
                return Ok(());
            }

            // No remaining path can reach the join — activate.
            let waiting_tokens: Vec<Execution> = all_executions
                .into_iter()
                .filter(|candidate| {
                    candidate.process_instance_id == execution.process_instance_id
                        && candidate.activity_id.as_deref() == Some(activity_id.as_str())
                        && candidate.parallel_scope_id() == scope_id
                })
                .collect();

            for waiting_token in &waiting_tokens {
                command_context
                    .execution_entity_manager
                    .delete(&waiting_token.id, &mut command_context.session);
            }

            // Create a single merged execution to continue through outgoing flows
            let mut merged = execution.clone();
            merged.id = Uuid::new_v4().to_string();
            merged.parent_id = Some(scope_id.clone());
            merged.activity_id = Some(activity_id.clone());
            merged.is_active = true;
            merged.is_concurrent = false;
            merged.is_ended = false;

            command_context
                .execution_entity_manager
                .insert(&merged, &mut command_context.session);

            // Let TakeOutgoingSequenceFlowsOperation handle the outgoing routing
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(merged);
        } else {
            // Single incoming flow — pass through, let TakeOutgoing handle routing
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
        }

        Ok(())
    }
}
