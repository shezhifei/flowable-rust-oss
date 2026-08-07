use crate::agenda::FlowableEngineAgenda;
use crate::bpmn::behavior::escalation_event_support::resolve_escalation_event_ref;
use crate::bpmn::fault::{
    propagate_escalation_across_call_activities, try_catch_escalation_in_process_instance,
};
use crate::cmd::start_process_instance_cmd::StartProcessInstanceCmd;
use crate::cmd::trigger_boundary_event_cmd::TriggerBoundaryEventByEventRefCmd;
use crate::cmd::trigger_intermediate_catch_event_cmd::TriggerEventIntermediateCatchCmd;
use crate::cmd::trigger_start_event_subscription_cmd::TriggerEventSubprocessByEventCmd;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{EventSubscriptionKind, RuntimeEventWaitKind};
use crate::runtime::execution::Execution;
use crate::runtime::process_instance_builder::ProcessInstanceBuilder;
use flowable_bpmn_model::model::{BpmnModel, EventDefinitionEnum, FlowElementEnum};
use std::collections::{BTreeMap, HashSet};

pub struct IntermediateThrowEventActivityBehavior;

impl Default for IntermediateThrowEventActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl IntermediateThrowEventActivityBehavior {
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn trigger_registered_compensation(
        execution: &Execution,
        command_context: &mut CommandContext,
        activity_ref: Option<&str>,
    ) {
        let Some(process_instance_id) = execution.process_instance_id.as_ref() else {
            return;
        };

        // Java `IntermediateThrowCompensationEventActivityBehavior#execute`
        // (112-131): without an activityRef only the throwing event's own
        // flow-elements container (sub-process or process) is compensated.
        // `None` means the container is the process itself: no filtering.
        let scope_activity_ids = if activity_ref.is_none() {
            compensation_scope_activity_ids(execution, command_context)
        } else {
            None
        };

        let subscriptions = command_context
            .runtime_store
            .find_compensation_subscriptions_by_process_instance_id_newest_first(
                process_instance_id,
                &mut command_context.session,
            )
            .into_iter()
            .filter(|subscription| {
                if let Some(activity_ref) = activity_ref {
                    // P44: pre-6.4.0 compat — Java
                    // `IntermediateThrowCompensationEventActivityBehavior`
                    // (78-108) falls back to scanning the model when an exact
                    // activityRef match fails, resolving an
                    // isForCompensation handler back to the compensated
                    // activity. The subscription already stores the handler id
                    // in `compensation_activity_id`, so matching activityRef
                    // against both fields yields the same result without a
                    // model traversal.
                    subscription.activity_id == activity_ref
                        || subscription.compensation_activity_id == activity_ref
                } else if let Some(scope_activity_ids) = &scope_activity_ids {
                    scope_activity_ids.contains(&subscription.activity_id)
                } else {
                    true
                }
            })
            .collect::<Vec<_>>();

        for subscription in subscriptions {
            let compensation_execution = Execution {
                id: uuid::Uuid::new_v4().to_string(),
                process_instance_id: Some(process_instance_id.clone()),
                process_definition_id: execution.process_definition_id.clone(),
                activity_id: Some(subscription.compensation_activity_id.clone()),
                parent_id: Some(execution.id.clone()),
                is_active: true,
                // P18-C: handlers observe the scope-variable snapshot taken
                // when the compensated activity completed (Java `ScopeUtil.
                // createCopyOfSubProcessExecutionForCompensation`).
                variables: subscription.variables_snapshot.clone(),
                ..Default::default()
            };

            command_context
                .execution_entity_manager
                .insert(&compensation_execution, &mut command_context.session);
            command_context
                .agenda
                .plan_continue_process_operation(compensation_execution);
            command_context
                .runtime_store
                .delete_compensation_subscription(&subscription.id, &mut command_context.session);
        }
    }
}

fn unique_event_refs(mut refs: Vec<String>) -> Vec<String> {
    refs.retain(|event_ref| !event_ref.is_empty());
    refs.sort();
    refs.dedup();
    refs
}

/// Nested flow elements of a scope-container element, if it is one.
/// P20: shared with the transaction/cancel behaviors (scope filtering).
pub(crate) fn container_flow_elements(element: &FlowElementEnum) -> Option<&[FlowElementEnum]> {
    match element {
        FlowElementEnum::SubProcess(sub_process) => Some(&sub_process.flow_elements),
        FlowElementEnum::Transaction(transaction) => Some(&transaction.sub_process.flow_elements),
        FlowElementEnum::EventSubProcess(event_sub_process) => {
            Some(&event_sub_process.sub_process.flow_elements)
        }
        FlowElementEnum::AdhocSubProcess(adhoc_sub_process) => {
            Some(&adhoc_sub_process.sub_process.flow_elements)
        }
        _ => None,
    }
}

/// Finds the flow-elements list that DIRECTLY contains `activity_id`.
fn find_direct_container<'a>(
    flow_elements: &'a [FlowElementEnum],
    activity_id: &str,
) -> Option<&'a [FlowElementEnum]> {
    let directly_contained = flow_elements.iter().any(|element| {
        crate::agenda::continue_process_operation::flow_element_id(element) == Some(activity_id)
    });
    if directly_contained {
        return Some(flow_elements);
    }
    for element in flow_elements {
        if let Some(nested) = container_flow_elements(element)
            && let Some(found) = find_direct_container(nested, activity_id)
        {
            return Some(found);
        }
    }
    None
}

pub(crate) fn collect_activity_ids_transitively(
    flow_elements: &[FlowElementEnum],
    collected: &mut HashSet<String>,
) {
    for element in flow_elements {
        if let Some(id) = crate::agenda::continue_process_operation::flow_element_id(element) {
            collected.insert(id.to_string());
        }
        if let Some(nested) = container_flow_elements(element) {
            collect_activity_ids_transitively(nested, collected);
        }
    }
}

/// Java compensation-scope semantics: the activity ids eligible for an
/// unscoped compensation throw are those inside the throwing event's own
/// container. Nested elements stay eligible because Java cascades sub-process
/// compensation into completed children (`CompensationEventHandler`); Rust's
/// flat subscription store models that cascade as transitive membership.
/// Returns `None` when the container is the process itself (no filtering).
fn compensation_scope_activity_ids(
    execution: &Execution,
    command_context: &mut CommandContext,
) -> Option<HashSet<String>> {
    let process_definition_id = execution.process_definition_id.as_deref()?;
    let activity_id = execution.activity_id.as_deref()?;
    let model = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)?;
    let main_process = model.main_process.as_ref()?;

    let container = find_direct_container(&main_process.flow_elements, activity_id)?;
    if std::ptr::eq(container.as_ptr(), main_process.flow_elements.as_ptr()) {
        // The throw sits at process level: every subscription of the process
        // instance is in scope.
        return None;
    }

    let mut scope_activity_ids = HashSet::new();
    collect_activity_ids_transitively(container, &mut scope_activity_ids);
    Some(scope_activity_ids)
}

fn signal_event_refs(model: &BpmnModel, signal_ref: &str) -> Vec<String> {
    let mut refs = vec![signal_ref.to_string()];
    if let Some(name) = model
        .signals
        .iter()
        .find(|signal| signal.base_element.id.as_deref() == Some(signal_ref))
        .and_then(|signal| signal.name.clone())
    {
        refs.push(name);
    }
    unique_event_refs(refs)
}

fn wait_kind_matches_event_kind(
    wait_kind: &RuntimeEventWaitKind,
    event_kind: &EventSubscriptionKind,
) -> bool {
    matches!(
        (wait_kind, event_kind),
        (
            RuntimeEventWaitKind::MessageIntermediateCatchEvent,
            EventSubscriptionKind::Message
        ) | (
            RuntimeEventWaitKind::SignalIntermediateCatchEvent,
            EventSubscriptionKind::Signal
        ) | (
            RuntimeEventWaitKind::ErrorIntermediateCatchEvent,
            EventSubscriptionKind::Error
        ) | (
            RuntimeEventWaitKind::CancelIntermediateCatchEvent,
            EventSubscriptionKind::Cancel
        ) | (
            RuntimeEventWaitKind::CompensateIntermediateCatchEvent,
            EventSubscriptionKind::Compensate
        ) | (
            RuntimeEventWaitKind::EscalationIntermediateCatchEvent,
            EventSubscriptionKind::Escalation
        ) | (
            RuntimeEventWaitKind::EventRegistryIntermediateCatchEvent,
            EventSubscriptionKind::EventRegistry
        )
    )
}

fn event_kind_label(event_kind: &EventSubscriptionKind) -> &'static str {
    match event_kind {
        EventSubscriptionKind::Message => "message",
        EventSubscriptionKind::Signal => "signal",
        EventSubscriptionKind::Conditional => "conditional",
        EventSubscriptionKind::Error => "error",
        EventSubscriptionKind::Cancel => "cancel",
        EventSubscriptionKind::Compensate => "compensate",
        EventSubscriptionKind::Escalation => "escalation",
        EventSubscriptionKind::EventRegistry => "event-registry",
    }
}

fn record_throw_audit(
    execution: &Execution,
    command_context: &mut CommandContext,
    event_kind: &EventSubscriptionKind,
    event_refs: &[String],
) {
    command_context.history_manager.record_audit_event(
        &format!("bpmn-{}-throw", event_kind_label(event_kind)),
        execution.process_instance_id.as_deref(),
        execution.process_definition_id.as_deref(),
        Some(&format!(
            "Intermediate throw event {} threw {} event(s): {}",
            execution.activity_id.as_deref().unwrap_or_default(),
            event_kind_label(event_kind),
            event_refs.join(",")
        )),
        &mut command_context.session,
    );
}

fn promote_throw_variables_to_process_scope(
    execution: &Execution,
    command_context: &mut CommandContext,
) {
    let Some(process_instance_id) = execution.process_instance_id.as_deref() else {
        return;
    };

    // P6-B audit (not observable): this function promotes the throwing
    // execution's OWN variable maps to the PI scope. On a forked child those
    // maps are empty (P4-7b), so nothing is promoted — but the parent
    // variables are already on the PI scope row and thus visible to signal
    // catches. There is no EL evaluation here that needs `evaluation_execution`;
    // promoting inherited variables would duplicate them. The
    // `signal_expression` field on SignalEventDefinition is parsed but not yet
    // evaluated by this behavior (missing feature, not a parent-chain gap).

    // Promote the throwing execution's own variables onto the process-instance
    // scope execution row — the single process-level variable store.
    let mut variables = execution.variables.clone();
    variables.extend(execution.local_variables.clone());
    variables.extend(execution.transient_variables.clone());
    if variables.is_empty() {
        return;
    }

    let Some(mut root_execution) = command_context
        .runtime_store
        .find_execution(process_instance_id, &mut command_context.session)
    else {
        return;
    };

    let root_execution_id = root_execution.id.clone();
    for (name, value) in variables {
        root_execution.set_process_variable(name.clone(), value.clone());
        let variable_id = format!("{}:{}", root_execution_id, name);
        if command_context
            .runtime_store
            .get_historic_variable_instance(&variable_id, &mut command_context.session)
            .is_some()
        {
            command_context.history_manager.record_variable_updated(
                &variable_id,
                value,
                &mut command_context.session,
            );
        } else {
            command_context.history_manager.record_variable_created(
                &variable_id,
                &name,
                crate::engine::variable_service::variable_type_name(&value),
                value,
                process_instance_id,
                Some(&root_execution_id),
                None,
                &mut command_context.session,
            );
        }
    }

    command_context
        .execution_entity_manager
        .update(&root_execution, &mut command_context.session);
}

fn trigger_matching_intermediate_catches(
    execution: &Execution,
    command_context: &mut CommandContext,
    event_kind: EventSubscriptionKind,
    event_refs: &[String],
) -> Result<usize, crate::error::FlowableError> {
    let Some(process_instance_id) = execution.process_instance_id.as_deref() else {
        return Ok(0);
    };

    let mut wait_states = command_context
        .runtime_store
        .find_event_wait_states_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        )
        .into_iter()
        .filter(|wait_state| wait_state.execution_id != execution.id)
        .filter(|wait_state| wait_kind_matches_event_kind(&wait_state.wait_kind, &event_kind))
        .filter(|wait_state| {
            wait_state
                .event_subscription
                .as_ref()
                .is_some_and(|subscription| {
                    subscription.kind == event_kind && event_refs.contains(&subscription.event_ref)
                })
        })
        .collect::<Vec<_>>();

    wait_states.sort_by(|left, right| left.execution_id.cmp(&right.execution_id));

    let mut triggered = 0;
    for wait_state in wait_states {
        let Some(subscription) = wait_state.event_subscription else {
            continue;
        };
        let cmd = TriggerEventIntermediateCatchCmd::new(
            event_kind.clone(),
            subscription.event_ref,
            wait_state.execution_id,
        );
        cmd.execute(command_context)?;
        triggered += 1;
        if event_kind == EventSubscriptionKind::Message {
            break;
        }
    }

    Ok(triggered)
}

fn trigger_message_or_signal_runtime(
    execution: &Execution,
    command_context: &mut CommandContext,
    event_kind: EventSubscriptionKind,
    event_refs: Vec<String>,
) -> Result<(), crate::error::FlowableError> {
    let Some(process_instance_id) = execution.process_instance_id.clone() else {
        return Ok(());
    };
    let event_refs = unique_event_refs(event_refs);
    if event_refs.is_empty() {
        return Ok(());
    }

    promote_throw_variables_to_process_scope(execution, command_context);
    record_throw_audit(execution, command_context, &event_kind, &event_refs);

    // Deliver once per throw. `event_refs` may hold both signal/message id and
    // global name as matching aliases; iterating them all would re-fire the same
    // non-interrupting boundary/event-subprocess subscription (P9-4 repeat
    // semantics). Matching already resolves id↔name inside the trigger cmds.
    // Intermediate catches below keep the full alias list for wait-state lookup.
    if let Some(event_ref) = event_refs.first() {
        let event_subprocess_cmd = TriggerEventSubprocessByEventCmd::with_source_execution(
            event_kind.clone(),
            event_ref.clone(),
            process_instance_id.clone(),
            execution.id.clone(),
        );
        event_subprocess_cmd.execute(command_context)?;

        let boundary_cmd = TriggerBoundaryEventByEventRefCmd::with_source_execution(
            event_kind.clone(),
            event_ref.clone(),
            process_instance_id.clone(),
            execution.id.clone(),
        );
        boundary_cmd.execute_with_trigger_result(command_context)?;
    }

    trigger_matching_intermediate_catches(execution, command_context, event_kind, &event_refs)?;
    Ok(())
}

/// Java `IntermediateThrowSignalEventActivityBehavior#execute` (79-103):
/// unless the signal is declared with `scope="processInstance"`, a signal
/// throw is an ENGINE-WIDE broadcast — it wakes every subscription matching
/// the signal across all process instances (event subprocesses, boundary
/// events, intermediate catches) and fires matching signal START events,
/// spawning new process instances. Delivery is synchronous (Rust has no
/// `flowable:async` signal delivery yet).
fn broadcast_signal_engine_wide(
    execution: &Execution,
    command_context: &mut CommandContext,
    event_refs: Vec<String>,
) -> Result<(), crate::error::FlowableError> {
    let event_refs = unique_event_refs(event_refs);
    if event_refs.is_empty() {
        return Ok(());
    }

    promote_throw_variables_to_process_scope(execution, command_context);
    record_throw_audit(
        execution,
        command_context,
        &EventSubscriptionKind::Signal,
        &event_refs,
    );

    let own_process_instance_id = execution.process_instance_id.clone();

    // 1. Event subprocess + boundary subscriptions of every live process
    // instance (sorted for determinism). The throwing instance keeps the
    // source-execution form to preserve P9-4 repeat semantics.
    let mut process_instance_ids = command_context
        .runtime_store
        .snapshot_process_instances(&mut command_context.session)
        .into_values()
        .filter(|process_instance| !process_instance.is_ended)
        .map(|process_instance| process_instance.id)
        .collect::<Vec<_>>();
    process_instance_ids.sort();

    if let Some(event_ref) = event_refs.first() {
        for process_instance_id in &process_instance_ids {
            let is_own = own_process_instance_id.as_deref() == Some(process_instance_id.as_str());
            let event_subprocess_cmd = if is_own {
                TriggerEventSubprocessByEventCmd::with_source_execution(
                    EventSubscriptionKind::Signal,
                    event_ref.clone(),
                    process_instance_id.clone(),
                    execution.id.clone(),
                )
            } else {
                TriggerEventSubprocessByEventCmd::new(
                    EventSubscriptionKind::Signal,
                    event_ref.clone(),
                    process_instance_id.clone(),
                )
            };
            event_subprocess_cmd.execute(command_context)?;

            let boundary_cmd = if is_own {
                TriggerBoundaryEventByEventRefCmd::with_source_execution(
                    EventSubscriptionKind::Signal,
                    event_ref.clone(),
                    process_instance_id.clone(),
                    execution.id.clone(),
                )
            } else {
                TriggerBoundaryEventByEventRefCmd::new(
                    EventSubscriptionKind::Signal,
                    event_ref.clone(),
                    process_instance_id.clone(),
                )
            };
            boundary_cmd.execute_with_trigger_result(command_context)?;
        }
    }

    // 2. Signal intermediate catches waiting anywhere in the engine.
    let mut wait_states = command_context
        .runtime_store
        .snapshot_event_wait_states(&mut command_context.session)
        .into_values()
        .filter(|wait_state| wait_state.execution_id != execution.id)
        .filter(|wait_state| {
            matches!(
                wait_state.wait_kind,
                RuntimeEventWaitKind::SignalIntermediateCatchEvent
            )
        })
        .filter(|wait_state| {
            wait_state
                .event_subscription
                .as_ref()
                .is_some_and(|subscription| {
                    subscription.kind == EventSubscriptionKind::Signal
                        && event_refs.contains(&subscription.event_ref)
                })
        })
        .collect::<Vec<_>>();
    wait_states.sort_by(|left, right| left.execution_id.cmp(&right.execution_id));

    for wait_state in wait_states {
        let Some(subscription) = wait_state.event_subscription else {
            continue;
        };
        let cmd = TriggerEventIntermediateCatchCmd::new(
            EventSubscriptionKind::Signal,
            subscription.event_ref,
            wait_state.execution_id,
        );
        cmd.execute(command_context)?;
    }

    // 3. Signal start event subscriptions: fire the latest deployed version
    // per process definition key (Java matches subscriptions by event name;
    // redeploys replace the start subscription with the newest version).
    let mut start_subscriptions = BTreeMap::new();
    for event_ref in &event_refs {
        for subscription in command_context
            .deployment_manager
            .find_event_start_subscriptions_by_event_ref(
                &EventSubscriptionKind::Signal,
                event_ref,
                &mut command_context.session,
            )
        {
            // Query order is deploy order → later entries win per key.
            start_subscriptions.insert(subscription.process_definition_key.clone(), subscription);
        }
    }

    for subscription in start_subscriptions.into_values() {
        let builder = ProcessInstanceBuilder::new()
            .process_definition_id(subscription.process_definition_id.clone());
        let cmd =
            StartProcessInstanceCmd::with_start_event_id(builder, subscription.start_event_id);
        cmd.execute(command_context)?;
    }

    Ok(())
}

impl ActivityBehavior for IntermediateThrowEventActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let process_definition_id = execution.process_definition_id.as_deref().unwrap_or("");
        let activity_id = execution.activity_id.as_deref().unwrap_or("");

        let mut link_target = None;
        let mut escalation_ref = None;
        let mut compensation_activity_ref = None;
        let mut signal_refs = Vec::new();
        let mut signal_process_instance_scope = false;

        if let Some(model) = command_context
            .deployment_manager
            .get_bpmn_model(process_definition_id)
            && let Some(process) = model.main_process.as_ref()
            && let Some(FlowElementEnum::IntermediateThrowEvent(event)) =
                process.flow_element_map.get(activity_id)
        {
            if let [EventDefinitionEnum::LinkEventDefinition(link)] =
                event.event.event_definitions.as_slice()
                && let Some(link_name) = &link.name
            {
                // Find corresponding catch event
                for el in &process.flow_elements {
                    if let FlowElementEnum::IntermediateCatchEvent(catch_ev) = el
                        && let [EventDefinitionEnum::LinkEventDefinition(catch_link)] =
                            catch_ev.event.event_definitions.as_slice()
                        && catch_link.name.as_deref() == Some(link_name)
                    {
                        link_target = catch_ev
                            .event
                            .flow_node
                            .flow_element
                            .base_element
                            .id
                            .clone();
                        break;
                    }
                }
            } else if let [EventDefinitionEnum::EscalationEventDefinition(escalation)] =
                event.event.event_definitions.as_slice()
            {
                escalation_ref = Some(resolve_escalation_event_ref(
                    escalation,
                    Some(model.as_ref()),
                ));
            } else if let [EventDefinitionEnum::CompensateEventDefinition(compensate)] =
                event.event.event_definitions.as_slice()
            {
                compensation_activity_ref = Some(compensate.activity_ref.clone());
            } else if let [EventDefinitionEnum::SignalEventDefinition(signal)] =
                event.event.event_definitions.as_slice()
                && let Some(signal_ref) = signal.signal_ref.as_deref()
            {
                signal_refs = signal_event_refs(model.as_ref(), signal_ref);
                // Java `Signal.SCOPE_PROCESS_INSTANCE`: only an explicit
                // scope="processInstance" narrows delivery to the throwing
                // process instance; the default is an engine-wide broadcast.
                signal_process_instance_scope = model
                    .signals
                    .iter()
                    .find(|signal| signal.base_element.id.as_deref() == Some(signal_ref))
                    .and_then(|signal| signal.scope.as_deref())
                    == Some("processInstance");
            }
            // MessageEventDefinition is intentionally ignored: Java
            // IntermediateThrowEventParseHandler.java:51-56 falls into the
            // unsupported else (LOGGER.warn only, no behavior set) so a message
            // intermediate throw is a pure no-op that takes outgoing flows.
            // Do not call trigger_message_or_signal_runtime for Message.
        }

        if let Some(escalation_ref) = escalation_ref {
            if let Some(process_instance_id) = &execution.process_instance_id {
                let caught_locally = try_catch_escalation_in_process_instance(
                    command_context,
                    process_instance_id,
                    &escalation_ref,
                    &execution.id,
                )?
                .is_some();

                if !caught_locally {
                    propagate_escalation_across_call_activities(
                        command_context,
                        process_instance_id,
                        &escalation_ref,
                    )?;
                }

                // Java always plans the throw event's outgoing path. When an
                // interrupting catcher destroyed that token, the Rust agenda
                // must not enqueue a stale execution snapshot.
                if command_context
                    .runtime_store
                    .find_execution(&execution.id, &mut command_context.session)
                    .is_none()
                {
                    return Ok(());
                }
            }
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
        } else if let Some(activity_ref) = compensation_activity_ref {
            Self::trigger_registered_compensation(
                execution,
                command_context,
                activity_ref.as_deref(),
            );
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
        } else if !signal_refs.is_empty() {
            if signal_process_instance_scope {
                trigger_message_or_signal_runtime(
                    execution,
                    command_context,
                    EventSubscriptionKind::Signal,
                    signal_refs,
                )?;
            } else {
                broadcast_signal_engine_wide(execution, command_context, signal_refs)?;
            }
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
        } else if let Some(target_id) = link_target {
            execution.activity_id = Some(target_id);
            command_context
                .execution_entity_manager
                .update(execution, &mut command_context.session);
            command_context
                .agenda
                .plan_continue_process_operation(execution.clone());
        } else {
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
        }

        Ok(())
    }
}
