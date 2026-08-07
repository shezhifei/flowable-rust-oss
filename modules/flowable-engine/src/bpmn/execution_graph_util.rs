//! Execution graph helpers for gateway join reachability.
//!
//! Java: `org.flowable.engine.impl.util.ExecutionGraphUtil` (isReachable
//! lines 35–133). Used by inclusive join to decide whether any active
//! execution can still arrive at a waiting inclusive gateway.

use crate::agenda::continue_process_operation::{find_flow_element, flow_element_id};
use flowable_bpmn_model::model::{FlowElementEnum, Process, SequenceFlow};
use std::collections::HashSet;

/// Java `ExecutionGraphUtil.isReachable(processDefinitionId, source, target)`.
///
/// Verifies that `source_element_id` can reach `target_element_id` by following
/// sequence flows (and climbing out of embedded subprocesses with no outgoing
/// flows). Event-subprocess start events are treated as non-reachable sources.
pub fn is_reachable(process: &Process, source_element_id: &str, target_element_id: &str) -> bool {
    let Some(source_element) = resolve_flow_node(process, source_element_id) else {
        return false;
    };
    let Some(target_element) = resolve_flow_node(process, target_element_id) else {
        return false;
    };

    let mut visited = HashSet::new();
    is_reachable_from(process, source_element, target_element, &mut visited)
}

/// Resolve a flow-node id (or sequence-flow id → its target flow node).
fn resolve_flow_node<'a>(process: &'a Process, element_id: &str) -> Option<&'a FlowElementEnum> {
    let element = find_flow_element(process, element_id)?;
    match element {
        FlowElementEnum::SequenceFlow(flow) => {
            let target_ref = flow.target_ref.as_deref()?;
            let target = find_flow_element(process, target_ref)?;
            if is_flow_node(target) {
                Some(target)
            } else {
                None
            }
        }
        other if is_flow_node(other) => Some(other),
        _ => None,
    }
}

fn is_flow_node(element: &FlowElementEnum) -> bool {
    !matches!(
        element,
        FlowElementEnum::SequenceFlow(_) | FlowElementEnum::ValuedDataObject(_)
    )
}

fn is_reachable_from<'a>(
    process: &'a Process,
    mut source_element: &'a FlowElementEnum,
    target_element: &'a FlowElementEnum,
    visited: &mut HashSet<String>,
) -> bool {
    // Java: start events in an event subprocess are not real runtime tokens
    // for reachability purposes.
    if matches!(source_element, FlowElementEnum::StartEvent(_))
        && is_in_event_subprocess(process, source_element)
    {
        return false;
    }

    let Some(source_id) = flow_element_id(source_element).map(str::to_string) else {
        return false;
    };
    let Some(target_id) = flow_element_id(target_element) else {
        return false;
    };

    // No outgoing sequence flow: end of process or embedded subprocess —
    // climb to the enclosing SubProcess and continue from there.
    let outgoing = get_outgoing_flows(source_element);
    if outgoing.map(|flows| flows.is_empty()).unwrap_or(true) {
        visited.insert(source_id);
        let Some(source_lookup_id) = flow_element_id(source_element) else {
            return false;
        };
        match find_parent_element_for_child(process, source_lookup_id) {
            Some(parent)
                if matches!(
                    parent,
                    FlowElementEnum::SubProcess(_)
                        | FlowElementEnum::Transaction(_)
                        | FlowElementEnum::EventSubProcess(_)
                        | FlowElementEnum::AdhocSubProcess(_)
                ) =>
            {
                source_element = parent;
            }
            _ => return false,
        }
    }

    let Some(source_id) = flow_element_id(source_element).map(str::to_string) else {
        return false;
    };

    if source_id == target_id {
        return true;
    }

    if !visited.insert(source_id) {
        return false;
    }

    let Some(sequence_flows) = get_outgoing_flows(source_element) else {
        return false;
    };

    for sequence_flow in sequence_flows {
        let Some(target_ref) = sequence_flow.target_ref.as_deref() else {
            continue;
        };
        let Some(sequence_flow_target) = find_flow_element(process, target_ref) else {
            continue;
        };
        if !is_flow_node(sequence_flow_target) {
            continue;
        }
        let Some(seq_target_id) = flow_element_id(sequence_flow_target) else {
            continue;
        };
        if visited.contains(seq_target_id) {
            continue;
        }
        if is_reachable_from(process, sequence_flow_target, target_element, visited) {
            return true;
        }
    }

    false
}

fn get_outgoing_flows(element: &FlowElementEnum) -> Option<&Vec<SequenceFlow>> {
    match element {
        FlowElementEnum::Task(t) => Some(&t.activity.flow_node.outgoing_flows),
        FlowElementEnum::UserTask(t) => Some(&t.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::ServiceTask(t) => Some(&t.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::CaseServiceTask(t) => Some(&t.service_task.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::ScriptTask(t) => Some(&t.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::ManualTask(t) => Some(&t.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::ReceiveTask(t) => Some(&t.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::BusinessRuleTask(t) => Some(&t.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::StartEvent(e) => Some(&e.event.flow_node.outgoing_flows),
        FlowElementEnum::EndEvent(e) => Some(&e.event.flow_node.outgoing_flows),
        FlowElementEnum::ExclusiveGateway(g) => Some(&g.gateway.flow_node.outgoing_flows),
        FlowElementEnum::ParallelGateway(g) => Some(&g.gateway.flow_node.outgoing_flows),
        FlowElementEnum::InclusiveGateway(g) => Some(&g.gateway.flow_node.outgoing_flows),
        FlowElementEnum::EventBasedGateway(g) => Some(&g.gateway.flow_node.outgoing_flows),
        FlowElementEnum::IntermediateCatchEvent(e) => Some(&e.event.flow_node.outgoing_flows),
        FlowElementEnum::IntermediateThrowEvent(e) => Some(&e.event.flow_node.outgoing_flows),
        FlowElementEnum::SubProcess(s) => Some(&s.activity.flow_node.outgoing_flows),
        FlowElementEnum::Transaction(s) => Some(&s.sub_process.activity.flow_node.outgoing_flows),
        FlowElementEnum::EventSubProcess(s) => {
            Some(&s.sub_process.activity.flow_node.outgoing_flows)
        }
        FlowElementEnum::AdhocSubProcess(s) => {
            Some(&s.sub_process.activity.flow_node.outgoing_flows)
        }
        FlowElementEnum::CallActivity(a) => Some(&a.activity.flow_node.outgoing_flows),
        FlowElementEnum::BoundaryEvent(e) => Some(&e.event.flow_node.outgoing_flows),
        _ => None,
    }
}

/// Whether a flow node sits inside an `EventSubProcess` container.
fn is_in_event_subprocess(process: &Process, flow_node: &FlowElementEnum) -> bool {
    let Some(mut current_id) = flow_element_id(flow_node).map(str::to_string) else {
        return false;
    };
    loop {
        let Some(parent) = find_parent_element_for_child(process, &current_id) else {
            return false;
        };
        if matches!(parent, FlowElementEnum::EventSubProcess(_)) {
            return true;
        }
        match parent {
            FlowElementEnum::SubProcess(_)
            | FlowElementEnum::Transaction(_)
            | FlowElementEnum::AdhocSubProcess(_) => {
                if let Some(id) = flow_element_id(parent) {
                    current_id = id.to_string();
                    continue;
                }
                return false;
            }
            _ => return false,
        }
    }
}

/// Find the immediate container flow element (SubProcess / Transaction /
/// EventSubProcess / AdhocSubProcess) that holds `element_id`.
pub fn find_parent_element_for_child<'a>(
    process: &'a Process,
    element_id: &str,
) -> Option<&'a FlowElementEnum> {
    for element in &process.flow_elements {
        if let Some(found) = find_parent_in_element(element, element_id) {
            return Some(found);
        }
    }
    None
}

fn find_parent_in_element<'a>(
    container: &'a FlowElementEnum,
    element_id: &str,
) -> Option<&'a FlowElementEnum> {
    let children = nested_flow_elements(container)?;

    for child in children {
        if flow_element_id(child) == Some(element_id) {
            return Some(container);
        }
        if let Some(found) = find_parent_in_element(child, element_id) {
            return Some(found);
        }
    }
    None
}

fn nested_flow_elements(element: &FlowElementEnum) -> Option<&Vec<FlowElementEnum>> {
    match element {
        FlowElementEnum::SubProcess(s) => Some(&s.flow_elements),
        FlowElementEnum::Transaction(t) => Some(&t.sub_process.flow_elements),
        FlowElementEnum::EventSubProcess(e) => Some(&e.sub_process.flow_elements),
        FlowElementEnum::AdhocSubProcess(a) => Some(&a.sub_process.flow_elements),
        _ => None,
    }
}

/// Whether a flow node is asynchronous (inclusive-join "already arrived but
/// not yet executed" special case). Java `isAsynchronousActivity`.
pub fn is_asynchronous_activity(element: &FlowElementEnum) -> bool {
    match element {
        FlowElementEnum::Task(t) => t.activity.flow_node.asynchronous,
        FlowElementEnum::UserTask(t) => t.task.activity.flow_node.asynchronous,
        FlowElementEnum::ServiceTask(t) => t.task.activity.flow_node.asynchronous,
        FlowElementEnum::CaseServiceTask(t) => t.service_task.task.activity.flow_node.asynchronous,
        FlowElementEnum::ScriptTask(t) => t.task.activity.flow_node.asynchronous,
        FlowElementEnum::ManualTask(t) => t.task.activity.flow_node.asynchronous,
        FlowElementEnum::ReceiveTask(t) => t.task.activity.flow_node.asynchronous,
        FlowElementEnum::BusinessRuleTask(t) => t.task.activity.flow_node.asynchronous,
        FlowElementEnum::CallActivity(a) => a.activity.flow_node.asynchronous,
        FlowElementEnum::SubProcess(s) => s.activity.flow_node.asynchronous,
        FlowElementEnum::Transaction(t) => t.sub_process.activity.flow_node.asynchronous,
        FlowElementEnum::EventSubProcess(e) => e.sub_process.activity.flow_node.asynchronous,
        FlowElementEnum::AdhocSubProcess(a) => a.sub_process.activity.flow_node.asynchronous,
        FlowElementEnum::StartEvent(e) => e.event.flow_node.asynchronous,
        FlowElementEnum::EndEvent(e) => e.event.flow_node.asynchronous,
        FlowElementEnum::ExclusiveGateway(g) => g.gateway.flow_node.asynchronous,
        FlowElementEnum::ParallelGateway(g) => g.gateway.flow_node.asynchronous,
        FlowElementEnum::InclusiveGateway(g) => g.gateway.flow_node.asynchronous,
        FlowElementEnum::EventBasedGateway(g) => g.gateway.flow_node.asynchronous,
        FlowElementEnum::IntermediateCatchEvent(e) => e.event.flow_node.asynchronous,
        FlowElementEnum::IntermediateThrowEvent(e) => e.event.flow_node.asynchronous,
        FlowElementEnum::BoundaryEvent(e) => e.event.flow_node.asynchronous,
        _ => false,
    }
}

/// Activity loop-characteristics present (multi-instance).
pub fn has_loop_characteristics(element: &FlowElementEnum) -> bool {
    match element {
        FlowElementEnum::Task(t) => t.activity.loop_characteristics.is_some(),
        FlowElementEnum::UserTask(t) => t.task.activity.loop_characteristics.is_some(),
        FlowElementEnum::ServiceTask(t) => t.task.activity.loop_characteristics.is_some(),
        FlowElementEnum::CaseServiceTask(t) => t.service_task.task.activity.loop_characteristics.is_some(),
        FlowElementEnum::ScriptTask(t) => t.task.activity.loop_characteristics.is_some(),
        FlowElementEnum::ManualTask(t) => t.task.activity.loop_characteristics.is_some(),
        FlowElementEnum::ReceiveTask(t) => t.task.activity.loop_characteristics.is_some(),
        FlowElementEnum::BusinessRuleTask(t) => t.task.activity.loop_characteristics.is_some(),
        FlowElementEnum::CallActivity(a) => a.activity.loop_characteristics.is_some(),
        FlowElementEnum::SubProcess(s) => s.activity.loop_characteristics.is_some(),
        FlowElementEnum::Transaction(t) => t.sub_process.activity.loop_characteristics.is_some(),
        FlowElementEnum::EventSubProcess(e) => e.sub_process.activity.loop_characteristics.is_some(),
        FlowElementEnum::AdhocSubProcess(a) => a.sub_process.activity.loop_characteristics.is_some(),
        _ => false,
    }
}

/// Java `ParallelGatewayActivityBehavior#hasMultiInstanceParent`.
pub fn has_multi_instance_parent(process: &Process, flow_node_id: &str) -> bool {
    let mut current_id = flow_node_id.to_string();
    loop {
        let Some(parent) = find_parent_element_for_child(process, &current_id) else {
            return false;
        };
        if has_loop_characteristics(parent) {
            return true;
        }
        match parent {
            FlowElementEnum::SubProcess(_)
            | FlowElementEnum::Transaction(_)
            | FlowElementEnum::EventSubProcess(_)
            | FlowElementEnum::AdhocSubProcess(_) => {
                if let Some(id) = flow_element_id(parent) {
                    current_id = id.to_string();
                    continue;
                }
                return false;
            }
            _ => return false,
        }
    }
}
