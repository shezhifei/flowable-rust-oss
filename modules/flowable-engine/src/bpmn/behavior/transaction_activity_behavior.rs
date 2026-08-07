use crate::agenda::FlowableEngineAgenda;
use crate::bpmn::behavior::boundary_event_activity_behavior::{
    resolve_boundary_event_subscription, runtime_cancel_activity,
};
use crate::bpmn::behavior::intermediate_throw_event_activity_behavior::container_flow_elements;
use crate::bpmn::job_category::resolve_job_category;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{RuntimeBoundaryEventState, RuntimeTimerJobState};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum, Transaction};
use uuid::Uuid;

/// Finds the `<transaction>` element with the given activity id anywhere in
/// the process, nested containers included. Java resolves the element via
/// `execution.getCurrentFlowElement()`, which is container-agnostic — the
/// previous top-level-only scan silently skipped NESTED transactions
/// (`TransactionSubProcessTest.testNestedCancelInner/Outer`).
pub(crate) fn find_transaction<'a>(
    flow_elements: &'a [FlowElementEnum],
    activity_id: &str,
) -> Option<&'a Transaction> {
    for flow_element in flow_elements {
        if let FlowElementEnum::Transaction(transaction) = flow_element
            && transaction
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref()
                == Some(activity_id)
        {
            return Some(transaction);
        }
        if let Some(nested) = container_flow_elements(flow_element)
            && let Some(found) = find_transaction(nested, activity_id)
        {
            return Some(found);
        }
    }
    None
}

pub struct TransactionActivityBehavior;

impl Default for TransactionActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl TransactionActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for TransactionActivityBehavior {
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

        let mut start_event_id = None;
        let mut boundary_events = Vec::new();
        {
            if let Some(bpmn_model) = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id)
                && let Some(process) = bpmn_model.main_process.as_ref()
                && let Some(transaction) = find_transaction(&process.flow_elements, &activity_id)
            {
                boundary_events = transaction.sub_process.activity.boundary_events.clone();
                for inner_element in &transaction.sub_process.flow_elements {
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

        let start_event_id = match start_event_id {
            Some(id) => id,
            None => {
                command_context
                    .agenda
                    .plan_take_outgoing_sequence_flows_operation(execution.clone());
                return Ok(());
            }
        };

        execution.is_scope = true;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        let process_instance_id = execution
            .process_instance_id
            .clone()
            .unwrap_or_else(|| execution.id.clone());

        let bpmn_model = command_context
            .deployment_manager
            .get_bpmn_model(&process_definition_id);

        // P6-B: job_category expression must walk the parent scope chain
        // (forked child maps may be empty after P4-7b).
        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);

        // Register boundary events for this Transaction.
        //
        // Java parity (`ContinueProcessOperation#executeMultiInstanceSynchronous`
        // 221–233): for a multi-instance activity the boundary events attach
        // once, on the MI root execution — never per instance child
        // (`ContinueMultiInstanceOperation` creates no boundary events).
        // Resolve the host before the loop; re-registration by sibling
        // instances is deduplicated below.
        let boundary_host_id =
            crate::bpmn::behavior::multi_instance_support::boundary_host_execution_id(
                command_context,
                execution,
            );
        for boundary_event in boundary_events {
            if let Some(ref boundary_event_id) =
                boundary_event.event.flow_node.flow_element.base_element.id
            {
                if let [EventDefinitionEnum::TimerEventDefinition(timer_def)] =
                    boundary_event.event.event_definitions.as_slice()
                {
                    // Dedup: every MI instance child reaches this loop, but
                    // Java schedules exactly one timer job per boundary event
                    // for the whole MI activity.
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
                    &boundary_event,
                    bpmn_model.as_deref(),
                ) {
                    Some(sub) => sub,
                    None => {
                        return Err(crate::error::FlowableError::UnsupportedElement {
                            element_type: "BoundaryEvent".to_string(),
                            activity_id: boundary_event_id.clone(),
                        });
                    }
                };

                let configuration =
                    crate::bpmn::behavior::boundary_event_activity_behavior::resolve_boundary_configuration(
                        &boundary_event,
                        Some(execution),
                    );
                let state = RuntimeBoundaryEventState {
                    boundary_event_id: boundary_event_id.clone(),
                    attached_activity_id: activity_id.clone(),
                    process_instance_id: process_instance_id.clone(),
                    // MI parity: keyed by (process_instance_id, boundary_event_id),
                    // so sibling instances idempotently re-write the same row
                    // hosted on the MI root.
                    host_execution_id: boundary_host_id.clone(),
                    cancel_activity: runtime_cancel_activity(&boundary_event, &event_sub),
                    event_subscription: event_sub,
                    configuration,
                };
                crate::bpmn::behavior::boundary_event_activity_behavior::insert_boundary_event_state_with_waiting(
                    command_context,
                    state,
                    execution.process_definition_id.as_deref(),
                );
            }
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
            activity_id: Some(start_event_id),
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
}
