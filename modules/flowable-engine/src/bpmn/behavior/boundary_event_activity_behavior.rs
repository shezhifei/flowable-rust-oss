use crate::bpmn::behavior::event_registry_event_support::resolve_event_type_extension;
use crate::bpmn::event_registry_correlation::{
    correlation_key_from_base_element, extension_element_text, ELEMENT_EVENT_TYPE,
};
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventSubscription, EventSubscriptionKind, RuntimeBoundaryEventState,
};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{BoundaryEvent, BpmnModel, EventDefinitionEnum};
use flowable_engine_common::el::variable_container::VariableContainer;

/// Insert boundary subscription and dispatch the matching `ACTIVITY_*_WAITING`
/// event (Java Boundary{Signal,Message,Conditional,Escalation}EventActivityBehavior).
pub(crate) fn insert_boundary_event_state_with_waiting(
    command_context: &mut CommandContext,
    state: RuntimeBoundaryEventState,
    process_definition_id: Option<&str>,
) {
    let activity_id = state.boundary_event_id.clone();
    let kind = state.event_subscription.kind.clone();
    let event_ref = state.event_subscription.event_ref.clone();
    let process_instance_id = state.process_instance_id.clone();
    let host_execution_id = state.host_execution_id.clone();
    command_context
        .runtime_store
        .insert_boundary_event_state(state, &mut command_context.session);
    // P125: ACTIVITY_*_WAITING on boundary subscription create.
    crate::engine::event_dispatcher::dispatch_activity_waiting_for_subscription(
        command_context,
        &activity_id,
        kind,
        &event_ref,
        Some(&process_instance_id),
        Some(&host_execution_id),
        process_definition_id,
    );
}

pub struct BoundaryEventActivityBehavior;

impl Default for BoundaryEventActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundaryEventActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

fn resolve_message_event_ref(model: Option<&BpmnModel>, message_ref: &str) -> String {
    model
        .and_then(|model| {
            model
                .messages
                .iter()
                .find(|message| message.base_element.id.as_deref() == Some(message_ref))
                .and_then(|message| message.name.clone())
        })
        .unwrap_or_else(|| message_ref.to_string())
}

/// Runtime-side `cancelActivity` for a boundary event registration.
///
/// Java splits model and runtime semantics for error boundary events: the
/// model converter forces `cancelActivity=false` when the boundary event has
/// exactly one `ErrorEventDefinition` (`BoundaryEventXMLConverter.java:86-93`),
/// while the runtime parse handler hardcodes `interrupting=true` when creating
/// the behavior and never reads the model flag
/// (`ErrorEventDefinitionParseHandler.java:34`). All other kinds use the model
/// value.
pub(crate) fn runtime_cancel_activity(
    boundary_event: &BoundaryEvent,
    event_subscription: &EventSubscription,
) -> bool {
    if event_subscription.kind == EventSubscriptionKind::Error {
        true
    } else {
        boundary_event.cancel_activity
    }
}

/// Resolves the runtime event subscription for a supported boundary event.
///
/// Message refs are normalized to global definition names when the model
/// declares them, matching intermediate catch and receive task behavior.
///
/// Empty event-definitions + `flowable:eventType` maps to
/// `EventSubscriptionKind::EventRegistry`
/// (Java `BoundaryEventParseHandler.java:76` /
/// `BoundaryEventRegistryEventActivityBehavior.java:59-72`).
pub(crate) fn resolve_boundary_event_subscription(
    boundary_event: &BoundaryEvent,
    model: Option<&BpmnModel>,
) -> Option<EventSubscription> {
    if boundary_event.event.event_definitions.is_empty() {
        return resolve_event_type_extension(
            &boundary_event.event.flow_node.flow_element.base_element,
        )
        .map(|event_ref| EventSubscription {
            kind: EventSubscriptionKind::EventRegistry,
            event_ref,
        });
    }
    match boundary_event.event.event_definitions.as_slice() {
        [EventDefinitionEnum::MessageEventDefinition(message_definition)] => message_definition
            .message_ref
            .as_ref()
            .map(|message_ref| EventSubscription {
                kind: EventSubscriptionKind::Message,
                event_ref: resolve_message_event_ref(model, message_ref),
            }),
        [EventDefinitionEnum::SignalEventDefinition(signal_definition)] => signal_definition
            .signal_ref
            .as_ref()
            .map(|signal_ref| EventSubscription {
                kind: EventSubscriptionKind::Signal,
                event_ref: signal_ref.clone(),
            }),
        [EventDefinitionEnum::ConditionalEventDefinition(conditional_definition)] => {
            conditional_definition
                .condition_expression
                .as_ref()
                .map(|expression| EventSubscription {
                    kind: EventSubscriptionKind::Conditional,
                    event_ref: expression.clone(),
                })
        }
        [EventDefinitionEnum::ErrorEventDefinition(error_definition)] => Some(EventSubscription {
            kind: EventSubscriptionKind::Error,
            event_ref: crate::bpmn::behavior::error_event_support::resolve_error_event_ref(
                error_definition,
                model,
            ),
        }),
        [EventDefinitionEnum::CancelEventDefinition(_)] => Some(EventSubscription {
            kind: EventSubscriptionKind::Cancel,
            event_ref: String::new(),
        }),
        [EventDefinitionEnum::CompensateEventDefinition(compensate_definition)] => {
            Some(EventSubscription {
                kind: EventSubscriptionKind::Compensate,
                event_ref: compensate_definition
                    .activity_ref
                    .clone()
                    .unwrap_or_default(),
            })
        }
        [EventDefinitionEnum::EscalationEventDefinition(escalation_definition)] => {
            Some(EventSubscription {
                kind: EventSubscriptionKind::Escalation,
                event_ref:
                    crate::bpmn::behavior::escalation_event_support::resolve_escalation_event_ref(
                        escalation_definition,
                        model,
                    ),
            })
        }
        // Event-registry boundary: flowable:eventType
        // (Java BoundaryEventParseHandler.java:76-81 /
        // BoundaryEventRegistryEventActivityBehavior.java:58-69).
        // Reuses Message kind; event_ref = eventType text.
        _ => extension_element_text(
            &boundary_event
                .event
                .flow_node
                .flow_element
                .base_element
                .extension_elements,
            ELEMENT_EVENT_TYPE,
        )
        .map(|event_type| EventSubscription {
            kind: EventSubscriptionKind::Message,
            event_ref: event_type,
        }),
    }
}

/// Runtime correlation configuration for a boundary event.
/// Java `BoundaryEventRegistryEventActivityBehavior.java:68`.
pub(crate) fn resolve_boundary_configuration(
    boundary_event: &BoundaryEvent,
    variable_scope: Option<&dyn VariableContainer>,
) -> Option<String> {
    correlation_key_from_base_element(
        &boundary_event.event.flow_node.flow_element.base_element,
        variable_scope,
    )
}

impl ActivityBehavior for BoundaryEventActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        _command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // Mark execution as no longer active; the trigger_boundary_event_cmd
        // already plans both ContinueProcessOperation and
        // TakeOutgoingSequenceFlowsOperation, so we do NOT re-plan here.
        execution.is_active = false;
        Ok(())
    }
}
