use crate::agenda::{AgendaOperation, FlowableEngineAgenda};
use crate::el::condition::Condition;
use crate::el::expression::SimpleExpression;
use crate::el::uel_expression_condition::UelExpressionCondition;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{FlowElementEnum, Process, SequenceFlow};
use uuid::Uuid;

pub struct TakeOutgoingSequenceFlowsOperation {
    execution: Execution,
}

impl TakeOutgoingSequenceFlowsOperation {
    pub fn new(execution: Execution) -> Self {
        Self { execution }
    }
}

fn get_outgoing_flows(element: &FlowElementEnum) -> Option<&Vec<SequenceFlow>> {
    match element {
        FlowElementEnum::Task(t) => Some(&t.activity.flow_node.outgoing_flows),
        FlowElementEnum::UserTask(t) => Some(&t.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::ServiceTask(t) => Some(&t.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::CaseServiceTask(t) => Some(&t.service_task.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::SendTask(t) => Some(&t.service_task.task.activity.flow_node.outgoing_flows),
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
        FlowElementEnum::ComplexGateway(g) => Some(&g.gateway.flow_node.outgoing_flows),
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

fn get_gateway_default_flow_id(element: &FlowElementEnum) -> Option<&str> {
    match element {
        FlowElementEnum::ExclusiveGateway(g) => g.gateway.default_flow.as_deref(),
        FlowElementEnum::ParallelGateway(g) => g.gateway.default_flow.as_deref(),
        FlowElementEnum::InclusiveGateway(g) => g.gateway.default_flow.as_deref(),
        FlowElementEnum::EventBasedGateway(g) => g.gateway.default_flow.as_deref(),
        FlowElementEnum::ComplexGateway(g) => g.gateway.default_flow.as_deref(),
        _ => None,
    }
}

fn get_element_id(element: &FlowElementEnum) -> Option<&String> {
    match element {
        FlowElementEnum::Task(t) => t.activity.flow_node.flow_element.base_element.id.as_ref(),
        FlowElementEnum::UserTask(t) => t
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::ServiceTask(t) => t
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::CaseServiceTask(t) => t
            .service_task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::SendTask(t) => t
            .service_task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::ScriptTask(t) => t
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::ManualTask(t) => t
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::ReceiveTask(t) => t
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::BusinessRuleTask(t) => t
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::StartEvent(e) => e.event.flow_node.flow_element.base_element.id.as_ref(),
        FlowElementEnum::EndEvent(e) => e.event.flow_node.flow_element.base_element.id.as_ref(),
        FlowElementEnum::ExclusiveGateway(g) => {
            g.gateway.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::ParallelGateway(g) => {
            g.gateway.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::InclusiveGateway(g) => {
            g.gateway.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::EventBasedGateway(g) => {
            g.gateway.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::ComplexGateway(g) => {
            g.gateway.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::IntermediateCatchEvent(e) => {
            e.event.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::IntermediateThrowEvent(e) => {
            e.event.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::SubProcess(s) => {
            s.activity.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::Transaction(s) => s
            .sub_process
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::EventSubProcess(s) => s
            .sub_process
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::AdhocSubProcess(s) => s
            .sub_process
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_ref(),
        FlowElementEnum::CallActivity(a) => {
            a.activity.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::BoundaryEvent(e) => {
            e.event.flow_node.flow_element.base_element.id.as_ref()
        }
        FlowElementEnum::SequenceFlow(s) => s.flow_element.base_element.id.as_ref(),
        FlowElementEnum::ValuedDataObject(v) => v.base_element.id.as_ref(),
    }
}

fn is_end_event(flow_element: Option<&FlowElementEnum>) -> bool {
    matches!(flow_element, Some(FlowElementEnum::EndEvent(_)))
}

enum InclusiveGatewayAction {
    Continue(Vec<(SequenceFlow, bool)>),
    Split {
        flows: Vec<(SequenceFlow, bool)>,
    },
}

fn collect_matching_outgoing_flows(
    main_process: &Process,
    default_flow_id: Option<&str>,
    outgoing_flows: &[SequenceFlow],
    execution: &Execution,
) -> Result<Vec<(SequenceFlow, bool)>, crate::error::FlowableError> {
    let mut selected_flows = Vec::new();
    let mut default_flow = None;

    for flow in outgoing_flows {
        let flow_id = flow.flow_element.base_element.id.as_deref();

        // Java `TakeOutgoingSequenceFlowsOperation.java:215-228`: a flow whose
        // skipExpression is enabled is selected directly, bypassing condition
        // evaluation (and the default-flow deferral). The single-outgoing-flow
        // shortcut lives in `should_skip_sequence_flow`.
        if crate::bpmn::skip_expression::should_skip_sequence_flow(
            outgoing_flows.len(),
            flow.skip_expression.as_deref(),
            flow_id,
            execution,
        )? {
            let target_is_end_event = flow
                .target_ref
                .as_ref()
                .map(|target_ref| is_end_event(main_process.flow_element_map.get(target_ref)))
                .unwrap_or(false);
            selected_flows.push((flow.clone(), target_is_end_event));
            continue;
        }

        if default_flow_id.is_some() && flow_id == default_flow_id {
            default_flow = Some(flow.clone());
            continue;
        }

        let mut condition_met = true;
        if let Some(ref expr_str) = flow.condition_expression {
            let expression = Box::new(SimpleExpression::new(expr_str.clone()));
            let condition = UelExpressionCondition::new(expression);
            condition_met = condition.evaluate(
                Some(flow.flow_element.base_element.id.as_deref().unwrap_or("")),
                execution,
            )?;
        }

        if condition_met {
            let target_is_end_event = flow
                .target_ref
                .as_ref()
                .map(|target_ref| is_end_event(main_process.flow_element_map.get(target_ref)))
                .unwrap_or(false);
            selected_flows.push((flow.clone(), target_is_end_event));
        }
    }

    if selected_flows.is_empty()
        && let Some(default_flow) = default_flow
    {
        let target_is_end_event = default_flow
            .target_ref
            .as_ref()
            .map(|target_ref| is_end_event(main_process.flow_element_map.get(target_ref)))
            .unwrap_or(false);
        selected_flows.push((default_flow, target_is_end_event));
    }

    Ok(selected_flows)
}

/// Java `ContinueProcessOperation.java:308-319`: a sequence flow carries
/// execution listeners fired for `start`, `take`, `end` (all three, in that
/// order) while the execution is on the flow, before the `SEQUENCEFLOW_TAKEN`
/// event. The listener context sees the flow's id as activity id (Java
/// `execution.setCurrentFlowElement(sequenceFlow)` in `leaveFlowNode`).
fn fire_sequence_flow_listeners(
    execution: &mut Execution,
    command_context: &mut CommandContext,
    flow: &SequenceFlow,
    flow_id: &str,
) -> Result<(), crate::error::FlowableError> {
    if flow.flow_element.execution_listeners.is_empty() {
        return Ok(());
    }
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);
    let listeners: Vec<_> = flow.flow_element.execution_listeners.clone();
    let saved_activity_id = execution.activity_id.clone();
    execution.activity_id = Some(flow_id.to_string());
    for event in ["start", "take", "end"] {
        crate::bpmn::listener::execute_execution_listeners(
            execution,
            command_context,
            &listeners,
            event,
            &evaluation_execution,
        )?;
    }
    execution.activity_id = saved_activity_id;
    Ok(())
}

fn schedule_sequence_flow(
    command_context: &mut CommandContext,
    execution: &Execution,
    flow: &SequenceFlow,
    _target_is_end_event: bool,
) -> Result<(), crate::error::FlowableError> {
    if let Some(ref target_ref) = flow.target_ref {
        let mut new_execution = execution.clone();
        let flow_id = flow
            .flow_element
            .base_element
            .id
            .as_deref()
            .unwrap_or("<unnamed>");
        // Java `ContinueProcessOperation.java:308-319`: sequence-flow execution
        // listeners fire for start/take/end while the execution is on the flow,
        // before the SEQUENCEFLOW_TAKEN event below.
        fire_sequence_flow_listeners(&mut new_execution, command_context, flow, flow_id)?;
        new_execution.activity_id = Some(target_ref.clone());
        // P53 layer 2: dispatch `SEQUENCEFLOW_TAKEN` for the outgoing flow
        // (Java `ContinueProcessOperation.java:308-345`). We emit it here
        // because Rust fans the outgoing flows out in this helper, equivalent
        // to the Java "takeOutgoingSequenceFlows" loop body.
        crate::engine::event_dispatcher::dispatch_sequenceflow_taken(
            command_context,
            flow_id,
            execution.process_instance_id.as_deref(),
            Some(&execution.id),
            execution.process_definition_id.as_deref(),
        );
        command_context
            .execution_entity_manager
            .update(&new_execution, &mut command_context.session);
        command_context
            .agenda
            .plan_continue_process_operation(new_execution);
    }
    Ok(())
}

fn spawn_child_execution(
    command_context: &mut CommandContext,
    execution: &Execution,
    flow: &SequenceFlow,
    target_is_end_event: bool,
) -> Result<(), crate::error::FlowableError> {
    if let Some(ref target_ref) = flow.target_ref {
        let mut child = execution.clone();
        child.id = Uuid::new_v4().to_string();
        child.parent_id = Some(execution.id.clone());
        child.is_active = !target_is_end_event;
        child.is_concurrent = true;
        child.is_ended = false;
        child.is_scope = false;
        child.is_multi_instance_root = false;
        // Concurrent children start empty; parent-chain EL evaluation supplies
        // process-level variables (P4-7a evaluation_execution).
        child.variables.clear();
        child.local_variables.clear();
        child.transient_variables.clear();
        child.non_interrupting_event_subprocess_path = false;

        let flow_id = flow
            .flow_element
            .base_element
            .id
            .as_deref()
            .unwrap_or("<unnamed>");
        // Java `ContinueProcessOperation.java:308-319`: each outgoing execution
        // (including split children) fires the flow's start/take/end listeners.
        fire_sequence_flow_listeners(&mut child, command_context, flow, flow_id)?;
        child.activity_id = Some(target_ref.clone());

        command_context
            .execution_entity_manager
            .insert(&child, &mut command_context.session);
        command_context
            .agenda
            .plan_continue_process_operation(child);
    }
    Ok(())
}

fn schedule_inclusive_gateway_child(
    command_context: &mut CommandContext,
    execution: &Execution,
    flow: &SequenceFlow,
    target_is_end_event: bool,
) -> Result<(), crate::error::FlowableError> {
    if let Some(ref target_ref) = flow.target_ref {
        let mut child = execution.clone();

        child.id = Uuid::new_v4().to_string();
        child.parent_id = Some(execution.parallel_scope_id());
        child.is_active = !target_is_end_event;
        child.is_concurrent = true;
        child.is_ended = false;
        child.is_scope = false;
        child.is_multi_instance_root = false;
        // Empty maps first — the inclusive join count is the child's own
        // bookkeeping variable, not a snapshot of the parent process vars.
        child.variables.clear();
        child.local_variables.clear();
        child.transient_variables.clear();
        child.non_interrupting_event_subprocess_path = false;

        let flow_id = flow
            .flow_element
            .base_element
            .id
            .as_deref()
            .unwrap_or("<unnamed>");
        fire_sequence_flow_listeners(&mut child, command_context, flow, flow_id)?;
        child.activity_id = Some(target_ref.clone());

        command_context
            .execution_entity_manager
            .insert(&child, &mut command_context.session);
        command_context
            .agenda
            .plan_continue_process_operation(child);
    }
    Ok(())
}

fn select_exclusive_gateway_flow<'a>(
    flow_element: &'a FlowElementEnum,
    outgoing_flows: &'a [SequenceFlow],
    execution: &Execution,
) -> Result<Option<&'a SequenceFlow>, crate::error::FlowableError> {
    let default_flow_id = get_gateway_default_flow_id(flow_element);
    let mut default_flow = None;

    for flow in outgoing_flows {
        let flow_id = flow.flow_element.base_element.id.as_deref();
        let is_default = default_flow_id.is_some() && flow_id == default_flow_id;
        if is_default {
            default_flow = Some(flow);
        }

        // Java `ExclusiveGatewayActivityBehavior.java:83-95`: when the flow's
        // skipExpression is enabled, condition evaluation is bypassed entirely
        // (no single-outgoing-flow shortcut here); the flow is selected iff the
        // skip expression evaluates to true, otherwise the gateway moves on.
        if flow.skip_expression.as_deref().is_some()
            && crate::bpmn::skip_expression::is_skip_expression_enabled(flow_id, execution)?
        {
            if crate::bpmn::skip_expression::should_skip_flow_element(
                flow.skip_expression.as_deref(),
                "SequenceFlow",
                flow_id,
                execution,
            )? {
                return Ok(Some(flow));
            }
            continue;
        }

        if is_default {
            continue;
        }

        let mut condition_met = true;
        if let Some(ref expr_str) = flow.condition_expression {
            let expression = Box::new(SimpleExpression::new(expr_str.clone()));
            let condition = UelExpressionCondition::new(expression);
            condition_met = condition.evaluate(
                Some(flow.flow_element.base_element.id.as_deref().unwrap_or("")),
                execution,
            )?;
        }

        if condition_met {
            return Ok(Some(flow));
        }
    }

    Ok(default_flow)
}

impl AgendaOperation for TakeOutgoingSequenceFlowsOperation {
    fn run(&self, command_context: &mut CommandContext) -> Result<(), crate::error::FlowableError> {
        let mut execution = self.execution.clone();
        // Snapshot for ad-hoc completion check after leave (Java
        // TakeOutgoingSequenceFlowsOperation.handleAdhocSubProcess :293-326).
        // Captured before the leave path may delete or re-parent this row.
        let leaving_snapshot = execution.clone();
        // Set true only when the leaving node lives under an AdhocSubProcess
        // (see assignment after flow_element resolve). Default false is the
        // non-adhoc path; the initial binding is intentionally overwritten.
        #[allow(unused_assignments)]
        let mut adhoc_child_leave = false;

        // Fire execution end listeners before activity-end history / leave.
        if let Some(activity_id_for_listeners) = execution.activity_id.clone() {
            let dm = command_context.deployment_manager_handle();
            if let Some(process_def_id) = execution.process_definition_id.as_ref()
                && let Some(bpmn_model) = dm.get_bpmn_model(process_def_id)
                && let Some(main_process) = bpmn_model.main_process.as_ref()
                && let Some(flow_element) =
                    crate::agenda::continue_process_operation::find_flow_element(
                        main_process,
                        &activity_id_for_listeners,
                    )
            {
                let end_listeners: Vec<_> =
                    crate::bpmn::listener::flow_element_execution_listeners(flow_element).to_vec();
                let end_evaluation_execution =
                    crate::engine::variable_service::evaluation_execution(
                        command_context,
                        &execution,
                    );
                crate::bpmn::listener::execute_execution_listeners(
                    &mut execution,
                    command_context,
                    &end_listeners,
                    "end",
                    &end_evaluation_execution,
                )?;
                // Persist any process variables written by end listeners.
                command_context
                    .execution_entity_manager
                    .update(&execution, &mut command_context.session);
            }
        }

        if let Some(ref activity_id) = execution.activity_id {
            let store = command_context.runtime_store_handle();
            let dm = command_context.deployment_manager_handle();

            command_context.history_manager.record_activity_end(
                &execution.id,
                activity_id,
                None,
                &mut command_context.session,
            );
            // P53 layer 2: dispatch `ACTIVITY_COMPLETED` once the execution
            // has left the current flow node (Java
            // `TakeOutgoingSequenceFlowsOperation.java:159-196`). Use the
            // activity type derived from the BPMN model so listeners see the
            // same kind string as in Java.
            {
                let activity_type_for_event =
                    if let Some(process_def_id) = execution.process_definition_id.as_ref()
                        && let Some(bpmn_model) = dm.get_bpmn_model(process_def_id)
                        && let Some(main_process) = bpmn_model.main_process.as_ref()
                        && let Some(flow_element) = crate::agenda::continue_process_operation::find_flow_element(
                            main_process,
                            activity_id,
                        )
                    {
                        crate::agenda::continue_process_operation::flow_element_type(flow_element)
                    } else {
                        "unknown"
                    };
                crate::engine::event_dispatcher::dispatch_activity_completed(
                    command_context,
                    activity_id,
                    activity_type_for_event,
                    execution.process_instance_id.as_deref(),
                    Some(&execution.id),
                    execution.process_definition_id.as_deref(),
                );
            }

            let (action, is_start_event) = {
                let process_def_id = match execution.process_definition_id.as_ref() {
                    Some(id) => id,
                    None => {
                        return Ok(());
                    }
                };

                let bpmn_model = match dm.get_bpmn_model(process_def_id) {
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

                fn collect_boundary_events(
                    elements: &[FlowElementEnum],
                    activity_id: &str,
                    collected: &mut Vec<flowable_bpmn_model::model::BoundaryEvent>,
                ) {
                    for el in elements {
                        match el {
                            FlowElementEnum::BoundaryEvent(b)
                                if b.attached_to_ref_id.as_deref() == Some(activity_id) =>
                            {
                                collected.push(b.clone());
                            }
                            FlowElementEnum::SubProcess(s) => {
                                collect_boundary_events(&s.flow_elements, activity_id, collected);
                            }
                            FlowElementEnum::Transaction(t) => {
                                collect_boundary_events(
                                    &t.sub_process.flow_elements,
                                    activity_id,
                                    collected,
                                );
                            }
                            FlowElementEnum::EventSubProcess(e) => {
                                collect_boundary_events(
                                    &e.sub_process.flow_elements,
                                    activity_id,
                                    collected,
                                );
                            }
                            FlowElementEnum::AdhocSubProcess(a) => {
                                collect_boundary_events(
                                    &a.sub_process.flow_elements,
                                    activity_id,
                                    collected,
                                );
                            }
                            _ => {}
                        }
                    }
                }

                fn find_compensation_handler(
                    artifacts: &[flowable_bpmn_model::model::ArtifactEnum],
                    flow_elements: &[FlowElementEnum],
                    boundary_id: &str,
                ) -> Option<String> {
                    fn compensation_activity_id(flow_element: &FlowElementEnum) -> Option<String> {
                        match flow_element {
                            FlowElementEnum::Task(task) if task.activity.is_for_compensation => {
                                task.activity.flow_node.flow_element.base_element.id.clone()
                            }
                            FlowElementEnum::UserTask(task)
                                if task.task.activity.is_for_compensation =>
                            {
                                task.task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .clone()
                            }
                            FlowElementEnum::ServiceTask(task)
                                if task.task.activity.is_for_compensation =>
                            {
                                task.task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .clone()
                            }
                            FlowElementEnum::CaseServiceTask(task)
                                if task.service_task.task.activity.is_for_compensation =>
                            {
                                task.service_task.task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .clone()
                            }
                            FlowElementEnum::ScriptTask(task)
                                if task.task.activity.is_for_compensation =>
                            {
                                task.task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .clone()
                            }
                            FlowElementEnum::ManualTask(task)
                                if task.task.activity.is_for_compensation =>
                            {
                                task.task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .clone()
                            }
                            FlowElementEnum::ReceiveTask(task)
                                if task.task.activity.is_for_compensation =>
                            {
                                task.task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .clone()
                            }
                            FlowElementEnum::BusinessRuleTask(task)
                                if task.task.activity.is_for_compensation =>
                            {
                                task.task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .clone()
                            }
                            _ => None,
                        }
                    }

                    fn find_compensation_handler_in_container(
                        flow_elements: &[FlowElementEnum],
                        boundary_id: &str,
                    ) -> Option<String> {
                        let contains_boundary = flow_elements.iter().any(|element| {
                            get_element_id(element).is_some_and(|id| id == boundary_id)
                        });

                        if contains_boundary {
                            let mut candidates = flow_elements
                                .iter()
                                .filter_map(compensation_activity_id)
                                .collect::<Vec<_>>();
                            candidates.sort();
                            candidates.dedup();

                            if candidates.len() == 1 {
                                return candidates.into_iter().next();
                            }
                        }

                        for element in flow_elements {
                            let nested_flow_elements = match element {
                                FlowElementEnum::SubProcess(sub_process) => {
                                    Some(&sub_process.flow_elements)
                                }
                                FlowElementEnum::Transaction(transaction) => {
                                    Some(&transaction.sub_process.flow_elements)
                                }
                                FlowElementEnum::EventSubProcess(event_sub_process) => {
                                    Some(&event_sub_process.sub_process.flow_elements)
                                }
                                FlowElementEnum::AdhocSubProcess(adhoc_sub_process) => {
                                    Some(&adhoc_sub_process.sub_process.flow_elements)
                                }
                                _ => None,
                            };

                            if let Some(nested_flow_elements) = nested_flow_elements
                                && let Some(target) = find_compensation_handler_in_container(
                                    nested_flow_elements,
                                    boundary_id,
                                )
                            {
                                return Some(target);
                            }
                        }

                        None
                    }

                    for artifact in artifacts {
                        if let flowable_bpmn_model::model::ArtifactEnum::Association(assoc) =
                            artifact
                        {
                            if assoc.source_ref.as_deref() == Some(boundary_id) {
                                return assoc.target_ref.clone();
                            }
                        }
                    }
                    for elem in flow_elements {
                        match elem {
                            FlowElementEnum::SubProcess(s) => {
                                if let Some(target) = find_compensation_handler(
                                    &s.artifacts,
                                    &s.flow_elements,
                                    boundary_id,
                                ) {
                                    return Some(target);
                                }
                            }
                            FlowElementEnum::Transaction(t) => {
                                if let Some(target) = find_compensation_handler(
                                    &t.sub_process.artifacts,
                                    &t.sub_process.flow_elements,
                                    boundary_id,
                                ) {
                                    return Some(target);
                                }
                            }
                            FlowElementEnum::EventSubProcess(e) => {
                                if let Some(target) = find_compensation_handler(
                                    &e.sub_process.artifacts,
                                    &e.sub_process.flow_elements,
                                    boundary_id,
                                ) {
                                    return Some(target);
                                }
                            }
                            FlowElementEnum::AdhocSubProcess(a) => {
                                if let Some(target) = find_compensation_handler(
                                    &a.sub_process.artifacts,
                                    &a.sub_process.flow_elements,
                                    boundary_id,
                                ) {
                                    return Some(target);
                                }
                            }
                            _ => {}
                        }
                    }

                    find_compensation_handler_in_container(flow_elements, boundary_id)
                }

                let mut attached_boundaries = Vec::new();
                collect_boundary_events(
                    &main_process.flow_elements,
                    activity_id,
                    &mut attached_boundaries,
                );

                for b in attached_boundaries {
                    if let [
                        flowable_bpmn_model::model::EventDefinitionEnum::CompensateEventDefinition(
                            _,
                        ),
                    ] = b.event.event_definitions.as_slice()
                    {
                        let b_id = b
                            .event
                            .flow_node
                            .flow_element
                            .base_element
                            .id
                            .clone()
                            .unwrap_or_default();

                        let compensation_activity_id = find_compensation_handler(
                            &main_process.artifacts,
                            &main_process.flow_elements,
                            &b_id,
                        )
                        .unwrap_or_else(|| b_id.clone());

                        // P18-C: snapshot the scope variables visible right
                        // now — the compensation handler must see this state,
                        // not later mutations (Java `ScopeUtil` copy).
                        let variables_snapshot =
                            crate::runtime::compensation::snapshot_scope_variables(
                                command_context,
                                &execution,
                            );

                        let sub = crate::runtime::compensation::CompensationSubscription {
                            id: uuid::Uuid::new_v4().to_string(),
                            process_instance_id: execution
                                .process_instance_id
                                .clone()
                                .unwrap_or_else(|| execution.id.clone()),
                            execution_id: execution.id.clone(),
                            activity_id: activity_id.to_string(),
                            compensation_activity_id,
                            subscription_order: 0,
                            variables_snapshot,
                        };
                        store.insert_compensation_subscription(sub, &mut command_context.session);
                    }
                }

                let mut flow_element_opt = main_process.flow_element_map.get(activity_id);
                if flow_element_opt.is_none() {
                    flow_element_opt = main_process.flow_elements.iter().find(|e| {
                        if let Some(id) = get_element_id(e) {
                            id == activity_id
                        } else {
                            false
                        }
                    });
                }

                let flow_element = match flow_element_opt {
                    Some(element) => element,
                    None => {
                        return Ok(());
                    }
                };

                // Java handleFlowNode :151-152 — when the leaving flow node's
                // parent container is an AdhocSubProcess, evaluate completion
                // condition after leave (see end of this method).
                adhoc_child_leave = crate::bpmn::execution_graph_util::find_parent_element_for_child(
                    main_process,
                    activity_id,
                )
                .is_some_and(|parent| matches!(parent, FlowElementEnum::AdhocSubProcess(_)));

                let outgoing_flows = match get_outgoing_flows(flow_element) {
                    Some(flows) => flows,
                    None => {
                        return Ok(());
                    }
                };

                outgoing_flows.is_empty();

                let is_start_event = matches!(flow_element, FlowElementEnum::StartEvent(_));

                // Sequence-flow conditions evaluate through the parent VariableScope
                // chain (Java ExecutionEntity / VariableScopeImpl#getVariable). The
                // real execution row is left untouched; only the temporary view used
                // for condition selection inherits ancestor variables.
                let evaluation_execution = crate::engine::variable_service::evaluation_execution(
                    command_context,
                    &execution,
                );

                let action = if matches!(flow_element, FlowElementEnum::ExclusiveGateway(_)) {
                    match select_exclusive_gateway_flow(
                        flow_element,
                        outgoing_flows,
                        &evaluation_execution,
                    )? {
                        Some(flow) => {
                            let target_is_end_event = flow
                                .target_ref
                                .as_ref()
                                .map(|target_ref| {
                                    is_end_event(main_process.flow_element_map.get(target_ref))
                                })
                                .unwrap_or(false);
                            InclusiveGatewayAction::Continue(vec![(
                                flow.clone(),
                                target_is_end_event,
                            )])
                        }
                        // Java parity (ExclusiveGatewayActivityBehavior.java:104-115):
                        // when no outgoing flow matches and no default flow is
                        // configured, raise a FlowableException instead of
                        // silently deleting the execution.
                        None => {
                            return Err(crate::error::FlowableError::ExecutionError(
                                format!(
                                    "No outgoing sequence flow of the exclusive gateway '{}' could be selected for continuing execution {}",
                                    activity_id, execution.id
                                ),
                            ));
                        }
                    }
                } else if let FlowElementEnum::InclusiveGateway(_gateway) = flow_element {
                    let flows = collect_matching_outgoing_flows(
                        main_process,
                        get_gateway_default_flow_id(flow_element),
                        outgoing_flows,
                        &evaluation_execution,
                    )?;

                    if flows.len() > 1 {
                        InclusiveGatewayAction::Split { flows }
                    } else {
                        InclusiveGatewayAction::Split { flows }
                    }
                } else {
                    InclusiveGatewayAction::Continue(collect_matching_outgoing_flows(
                        main_process,
                        get_gateway_default_flow_id(flow_element),
                        outgoing_flows,
                        &evaluation_execution,
                    )?)
                };

                (action, is_start_event)
            };

            let selected_flows = match action {
                InclusiveGatewayAction::Continue(flows) => flows,
                InclusiveGatewayAction::Split { flows } => {
                    // Java parity: the process instance itself is an execution
                    // (`ExecutionEntityImpl`); a split preserves its scope row
                    // (inactive) as the parent of the split branches instead of
                    // deleting it.
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

                    for (flow, target_is_end_event) in &flows {
                        schedule_inclusive_gateway_child(
                            command_context,
                            &execution,
                            flow,
                            *target_is_end_event,
                        )?;
                    }

                    // The split is fully handled here; returning avoids falling
                    // through to the empty-selection cleanup below, which would
                    // delete the (possibly preserved) scope row a second time.
                    // Java handleAdhocSubProcess still runs after leave.
                    if adhoc_child_leave {
                        crate::bpmn::behavior::adhoc_subprocess_activity_behavior::try_auto_complete_adhoc_after_child_leave(
                            &leaving_snapshot,
                            command_context,
                        )?;
                    }
                    return Ok(());
                }
            };

            if selected_flows.len() > 1 {
                if !execution.is_scope {
                    command_context
                        .execution_entity_manager
                        .delete(&execution.id, &mut command_context.session);
                } else {
                    let mut scope_exec = execution.clone();
                    scope_exec.is_active = false;
                    command_context
                        .execution_entity_manager
                        .update(&scope_exec, &mut command_context.session);
                }

                for (flow, target_is_end_event) in selected_flows {
                    spawn_child_execution(command_context, &execution, &flow, target_is_end_event)?;
                }
            } else if !selected_flows.is_empty() {
                for (flow, target_is_end_event) in selected_flows {
                    schedule_sequence_flow(command_context, &execution, &flow, target_is_end_event)?;
                }
            } else if selected_flows.is_empty() {
                if is_start_event {
                    return Ok(());
                }

                let store = command_context.runtime_store_handle();
                let dm = command_context.deployment_manager_handle();
                command_context
                    .execution_entity_manager
                    .delete(&execution.id, &mut command_context.session);

                let is_for_compensation = if let Some(process_def_id) =
                    execution.process_definition_id.as_ref()
                {
                    if let Some(bpmn_model) = dm.get_bpmn_model(process_def_id) {
                        if let Some(main_process) = bpmn_model.main_process.as_ref() {
                            let mut fe = main_process.flow_element_map.get(activity_id.as_str());
                            if fe.is_none() {
                                fe = main_process
                                    .flow_elements
                                    .iter()
                                    .find(|e| get_element_id(e) == Some(activity_id));
                            }
                            match fe {
                                Some(FlowElementEnum::Task(t)) => t.activity.is_for_compensation,
                                Some(FlowElementEnum::UserTask(t)) => {
                                    t.task.activity.is_for_compensation
                                }
                                Some(FlowElementEnum::ServiceTask(t)) => {
                                    t.task.activity.is_for_compensation
                                }
                                _ => false,
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_for_compensation && let Some(parent_id) = &execution.parent_id {
                    let all_executions = store.snapshot_executions(&mut command_context.session);
                    let siblings: Vec<_> = all_executions
                        .values()
                        .filter(|e| e.parent_id.as_deref() == Some(parent_id))
                        .collect();
                    let has_active_compensation = siblings.iter().any(|s| s.id != execution.id);
                    if !has_active_compensation
                        && let Some(parent_exec) = all_executions.get(parent_id)
                        && let Some(grandparent_id) = &parent_exec.parent_id
                        && let Some(grandparent_exec) = all_executions.get(grandparent_id)
                        && let Some(tx_id) = grandparent_exec.activity_id.as_deref()
                        && let Some(process_def_id) = execution.process_definition_id.as_ref()
                    {
                        let dm = command_context.deployment_manager_handle();
                        if let Some(bpmn_model) = dm.get_bpmn_model(process_def_id)
                            && let Some(main_process) = bpmn_model.main_process.as_ref()
                        {
                            // P20: nested transactions keep their cancel
                            // boundary inside the enclosing container, so the
                            // lookup must recurse (shared with the cancel end
                            // event behavior).
                            let cancel_boundary_id =
                                crate::bpmn::behavior::cancel_end_event_activity_behavior::find_cancel_boundary_id(
                                    &main_process.flow_elements,
                                    tx_id,
                                );

                            if let Some(b_id) = cancel_boundary_id {
                                // Compensation-complete cancel path also bypasses
                                // `execute_boundary_trigger`; drop one-shot state (P13).
                                if let Some(pi_id) = execution
                                    .process_instance_id
                                    .as_deref()
                                    .or(grandparent_exec.process_instance_id.as_deref())
                                {
                                    store.delete_boundary_event_state(
                                        &b_id,
                                        pi_id,
                                        &mut command_context.session,
                                    );
                                }

                                let mut boundary_exec = grandparent_exec.clone();
                                boundary_exec.activity_id = Some(b_id);
                                command_context
                                    .execution_entity_manager
                                    .update(&boundary_exec, &mut command_context.session);
                                command_context
                                    .agenda
                                    .plan_continue_process_operation(boundary_exec.clone());
                                command_context
                                    .agenda
                                    .plan_take_outgoing_sequence_flows_operation(boundary_exec);
                            }
                        }
                    }
                }
            }

            // Java TakeOutgoingSequenceFlowsOperation.handleAdhocSubProcess
            // (:293-326): after any leave of a node whose parent container is
            // an AdhocSubProcess, evaluate completionCondition and honour
            // cancelRemainingInstances.
            if adhoc_child_leave {
                crate::bpmn::behavior::adhoc_subprocess_activity_behavior::try_auto_complete_adhoc_after_child_leave(
                    &leaving_snapshot,
                    command_context,
                )?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowable_bpmn_model::model::{BaseElement, ExclusiveGateway, FlowElement, FlowNode, Gateway};
    use serde_json::json;

    fn test_sequence_flow(
        id: &str,
        condition: Option<&str>,
        skip_expression: Option<&str>,
    ) -> SequenceFlow {
        SequenceFlow {
            flow_element: FlowElement {
                base_element: BaseElement {
                    id: Some(id.to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            condition_expression: condition.map(str::to_string),
            skip_expression: skip_expression.map(str::to_string),
            target_ref: Some(format!("target-{id}")),
            ..Default::default()
        }
    }

    fn test_process(flows: &[SequenceFlow]) -> Process {
        Process {
            flow_elements: flows
                .iter()
                .map(|flow| FlowElementEnum::SequenceFlow(flow.clone()))
                .collect(),
            ..Default::default()
        }
    }

    fn test_execution(vars: &[(&str, serde_json::Value)]) -> Execution {
        Execution {
            variables: vars
                .iter()
                .map(|(name, value)| (name.to_string(), value.clone()))
                .collect(),
            ..Default::default()
        }
    }

    /// Java `TakeOutgoingSequenceFlowsOperation.java:215-228`: with the skip
    /// switch on and the skip expression evaluating to true, the flow is
    /// selected even though its condition is false (the condition is skipped).
    #[test]
    fn skip_expression_selects_flow_with_false_condition_in_take_path() {
        let flow_a = test_sequence_flow("flowA", Some("${a}"), None);
        let flow_b = test_sequence_flow("flowB", Some("${b}"), Some("${skipFlow}"));
        let process = test_process(&[flow_a.clone(), flow_b.clone()]);
        let execution = test_execution(&[
            ("a", json!(false)),
            ("b", json!(false)),
            ("skipFlow", json!(true)),
            ("_FLOWABLE_SKIP_EXPRESSION_ENABLED", json!(true)),
        ]);

        let selected =
            collect_matching_outgoing_flows(&process, None, &[flow_a, flow_b], &execution).unwrap();
        assert_eq!(
            selected.len(),
            1,
            "only the skip-enabled flow must be selected"
        );
        assert_eq!(
            selected[0].0.flow_element.base_element.id.as_deref(),
            Some("flowB"),
            "flowB has a false condition but skipExpression=true must select it"
        );
    }

    /// Java `ExclusiveGatewayActivityBehavior.java:83-95`: the exclusive
    /// gateway has no single-outgoing-flow shortcut; the skip expression must
    /// itself evaluate to true to select the flow.
    #[test]
    fn skip_expression_selects_flow_with_false_condition_on_exclusive_gateway() {
        let flow_a = test_sequence_flow("flowA", Some("${a}"), None);
        let flow_b = test_sequence_flow("flowB", Some("${b}"), Some("${skipFlow}"));
        let gateway = FlowElementEnum::ExclusiveGateway(ExclusiveGateway {
            gateway: Gateway {
                flow_node: FlowNode::default(),
                default_flow: None,
            },
        });
        let execution = test_execution(&[
            ("a", json!(false)),
            ("b", json!(false)),
            ("skipFlow", json!(true)),
            ("_FLOWABLE_SKIP_EXPRESSION_ENABLED", json!(true)),
        ]);

        let flows = [flow_a, flow_b];
        let selected = select_exclusive_gateway_flow(&gateway, &flows, &execution)
            .unwrap()
            .expect("skip-enabled flow must be selected");
        assert_eq!(
            selected.flow_element.base_element.id.as_deref(),
            Some("flowB"),
            "exclusive gateway must select the skip-enabled flow despite false conditions"
        );
    }

    /// Java `TakeOutgoingSequenceFlowsOperation.java:223`: with exactly one
    /// outgoing flow and the skip switch enabled, the flow is selected
    /// regardless of the skip expression's value.
    #[test]
    fn single_outgoing_flow_with_skip_enabled_is_selected_even_when_skip_false() {
        let flow = test_sequence_flow("flowA", Some("${a}"), Some("${skipFlow}"));
        let process = test_process(&[flow.clone()]);
        let execution = test_execution(&[
            ("a", json!(false)),
            ("skipFlow", json!(false)),
            ("_FLOWABLE_SKIP_EXPRESSION_ENABLED", json!(true)),
        ]);

        let selected = collect_matching_outgoing_flows(&process, None, &[flow], &execution).unwrap();
        assert_eq!(
            selected.len(),
            1,
            "single outgoing flow with skip switch on must still be selected"
        );
    }

    /// Java `SkipExpressionUtil.isSkipExpressionEnabled` (:30-46): without the
    /// enable switch, the skip expression is inert and the normal condition
    /// evaluation applies.
    #[test]
    fn skip_expression_without_enable_switch_falls_back_to_condition() {
        let flow_a = test_sequence_flow("flowA", Some("${a}"), None);
        let flow_b = test_sequence_flow("flowB", Some("${b}"), Some("${skipFlow}"));
        let process = test_process(&[flow_a.clone(), flow_b.clone()]);
        let execution = test_execution(&[
            ("a", json!(false)),
            ("b", json!(false)),
            ("skipFlow", json!(true)),
        ]);

        let selected =
            collect_matching_outgoing_flows(&process, None, &[flow_a, flow_b], &execution).unwrap();
        assert!(
            selected.is_empty(),
            "without the enable switch, skipExpression is inert and both conditions are false"
        );
    }
}
