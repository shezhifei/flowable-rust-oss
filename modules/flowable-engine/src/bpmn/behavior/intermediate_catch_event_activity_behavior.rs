use crate::bpmn::behavior::event_registry_event_support::resolve_event_type_extension;
use crate::bpmn::event_registry_correlation::{
    correlation_key_from_base_element, extension_element_text, ELEMENT_EVENT_TYPE,
};
use crate::bpmn::job_category::resolve_job_category;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventSubscription, EventSubscriptionKind, RuntimeEventWaitKind, RuntimeEventWaitState,
    RuntimeTimerJobState,
};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};
use uuid::Uuid;

pub struct IntermediateCatchEventActivityBehavior;

impl Default for IntermediateCatchEventActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl IntermediateCatchEventActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

/// Resolves the unified event subscription (message or signal) from the BPMN model
/// for an intermediate catch event.
fn resolve_event_subscription(
    command_context: &CommandContext,
    execution: &Execution,
) -> Option<EventSubscription> {
    let process_definition_id = execution.process_definition_id.as_deref()?;
    let activity_id = execution.activity_id.as_deref()?;

    let model = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)?;
    let process = model.main_process.as_ref()?;
    let flow_element = process.flow_element_map.get(activity_id)?;

    match flow_element {
        FlowElementEnum::IntermediateCatchEvent(event) => {
            // Java IntermediateCatchEventParseHandler.java:57 — empty event
            // definitions + flowable:eventType → Event Registry catch wait.
            if event.event.event_definitions.is_empty() {
                return resolve_event_type_extension(
                    &event.event.flow_node.flow_element.base_element,
                )
                .map(|event_ref| EventSubscription {
                    kind: EventSubscriptionKind::EventRegistry,
                    event_ref,
                });
            }
            match event.event.event_definitions.as_slice() {
                [EventDefinitionEnum::MessageEventDefinition(msg_def)] => {
                    msg_def.message_ref.as_ref().map(|r| {
                        // Resolve message name from message definition if exists
                        let event_name = model
                            .messages
                            .iter()
                            .find(|m| m.base_element.id.as_deref() == Some(r))
                            .and_then(|m| m.name.clone())
                            .unwrap_or_else(|| r.clone());

                        EventSubscription {
                            kind: EventSubscriptionKind::Message,
                            event_ref: event_name,
                        }
                    })
                }
                [EventDefinitionEnum::SignalEventDefinition(sig_def)] => {
                    sig_def.signal_ref.as_ref().map(|r| {
                        // Resolve signal name from signal definition if exists
                        let event_name = model
                            .signals
                            .iter()
                            .find(|s| s.base_element.id.as_deref() == Some(r))
                            .and_then(|s| s.name.clone())
                            .unwrap_or_else(|| r.clone());

                        EventSubscription {
                            kind: EventSubscriptionKind::Signal,
                            event_ref: event_name,
                        }
                    })
                }
                [EventDefinitionEnum::ConditionalEventDefinition(cond_def)] => cond_def
                    .condition_expression
                    .as_ref()
                    .map(|r| EventSubscription {
                        kind: EventSubscriptionKind::Conditional,
                        event_ref: r.clone(),
                    }),
                // Event-registry intermediate catch: flowable:eventType extension
                // (Java IntermediateCatchEventParseHandler.java:57-61 /
                // IntermediateCatchEventRegistryEventActivityBehavior.java:56-68).
                // Reuses Message kind; event_ref = eventType text.
                _ => extension_element_text(
                    &event
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
        _ => None,
    }
}

/// Runtime correlation key for the current intermediate catch (or None).
/// Java `CorrelationUtil.getCorrelationKey` with execution present (:48-51).
fn resolve_catch_configuration(
    command_context: &CommandContext,
    execution: &Execution,
) -> Option<String> {
    let process_definition_id = execution.process_definition_id.as_deref()?;
    let activity_id = execution.activity_id.as_deref()?;
    let model = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)?;
    let process = model.main_process.as_ref()?;
    let flow_element = process.flow_element_map.get(activity_id)?;
    match flow_element {
        FlowElementEnum::IntermediateCatchEvent(event) => correlation_key_from_base_element(
            &event.event.flow_node.flow_element.base_element,
            Some(execution),
        ),
        _ => None,
    }
}

fn resolve_display_name(command_context: &CommandContext, execution: &Execution) -> Option<String> {
    let process_definition_id = execution.process_definition_id.as_deref()?;
    let activity_id = execution.activity_id.as_deref()?;

    command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)
        .as_ref()
        .and_then(|model| model.main_process.as_ref())
        .and_then(|process| process.flow_element_map.get(activity_id))
        .and_then(|flow_element| match flow_element {
            FlowElementEnum::IntermediateCatchEvent(event) => {
                event.event.flow_node.flow_element.name.clone()
            }
            _ => None,
        })
}

impl ActivityBehavior for IntermediateCatchEventActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        execution.is_active = false;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        let process_instance_id = execution
            .process_instance_id
            .clone()
            .unwrap_or_else(|| execution.id.clone());

        // We fetch the flow element to see its definitions
        let process_definition_id = execution.process_definition_id.as_deref();
        let activity_id = execution.activity_id.as_deref();

        let event_element =
            if let (Some(pd_id), Some(act_id)) = (process_definition_id, activity_id) {
                command_context
                    .deployment_manager
                    .get_bpmn_model(pd_id)
                    .as_ref()
                    .and_then(|model| model.main_process.as_ref())
                    .and_then(|process| process.flow_element_map.get(act_id))
                    .and_then(|flow_element| match flow_element {
                        FlowElementEnum::IntermediateCatchEvent(event) => Some(event.clone()),
                        _ => None,
                    })
            } else {
                None
            };

        let mut handled = false;

        if let Some(event) = event_element
            && let [EventDefinitionEnum::TimerEventDefinition(timer_def)] =
                event.event.event_definitions.as_slice()
        {
            // P6-B: job_category expression must walk the parent scope chain
            // (forked child maps may be empty after P4-7b).
            let evaluation_execution =
                crate::engine::variable_service::evaluation_execution(command_context, execution);
            // P17: EL-evaluate timer fields before P16 prepare_repeat.
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
            let timer_job = RuntimeTimerJobState {
                timer_job_id: Uuid::new_v4().to_string(),
                process_instance_id: process_instance_id.clone(),
                execution_id: execution.id.clone(),
                activity_id: execution.activity_id.clone().unwrap_or_default(),
                job_state: Some("timer".to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: true,
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
                    &event.event.flow_node.flow_element.base_element,
                    &evaluation_execution,
                ),
                ..Default::default()
            };
            command_context
                .runtime_store
                .insert_timer_job_state(&timer_job, &mut command_context.session);
            // P119: TIMER_SCHEDULED — Java TimerJobSchedulerImpl.java:69-73.
            crate::engine::event_dispatcher::dispatch_timer_scheduled(command_context, &timer_job);
            handled = true;
        }

        if !handled {
            let event_subscription = resolve_event_subscription(command_context, execution);

            if let Some(event_subscription) = event_subscription {
                let wait_kind = match event_subscription.kind {
                    EventSubscriptionKind::Message => {
                        RuntimeEventWaitKind::MessageIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Signal => {
                        RuntimeEventWaitKind::SignalIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Conditional => {
                        RuntimeEventWaitKind::ConditionalIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Error => {
                        RuntimeEventWaitKind::ErrorIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Cancel => {
                        RuntimeEventWaitKind::CancelIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Compensate => {
                        RuntimeEventWaitKind::CompensateIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Escalation => {
                        RuntimeEventWaitKind::EscalationIntermediateCatchEvent
                    }
                    EventSubscriptionKind::EventRegistry => {
                        RuntimeEventWaitKind::EventRegistryIntermediateCatchEvent
                    }
                };

                let configuration = resolve_catch_configuration(command_context, execution);
                // P125: ACTIVITY_*_WAITING when subscription is created.
                // Java IntermediateCatch{Signal,Message,Conditional}EventActivityBehavior.execute.
                let waiting_kind = event_subscription.kind.clone();
                let waiting_ref = event_subscription.event_ref.clone();
                let waiting_activity = execution.activity_id.clone().unwrap_or_default();
                let waiting_pd = execution.process_definition_id.clone();
                let waiting_exec = execution.id.clone();
                command_context.runtime_store.insert_event_wait_state(
                    &RuntimeEventWaitState {
                        wait_kind,
                        process_instance_id: process_instance_id.clone(),
                        execution_id: execution.id.clone(),
                        task_id: None,
                        activity_id: execution.activity_id.clone(),
                        display_name: resolve_display_name(command_context, execution),
                        event_subscription: Some(event_subscription),
                        configuration,
                    },
                    &mut command_context.session,
                );
                crate::engine::event_dispatcher::dispatch_activity_waiting_for_subscription(
                    command_context,
                    &waiting_activity,
                    waiting_kind,
                    &waiting_ref,
                    Some(&process_instance_id),
                    Some(&waiting_exec),
                    waiting_pd.as_deref(),
                );
            }
        }

        Ok(())
    }
}
