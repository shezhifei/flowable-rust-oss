use crate::agenda::FlowableEngineAgenda;
use crate::bpmn::behavior::boundary_event_activity_behavior::{
    resolve_boundary_configuration, resolve_boundary_event_subscription, runtime_cancel_activity,
};
use crate::bpmn::behavior::error_event_support::resolve_error_event_ref;
use crate::bpmn::behavior::escalation_event_support::resolve_escalation_event_ref;
use crate::bpmn::behavior::event_registry_event_support::resolve_event_type_extension;
use crate::bpmn::event_registry_correlation::{
    correlation_key_from_base_element, extension_element_text, ELEMENT_EVENT_TYPE,
};
use crate::bpmn::job_category::resolve_job_category;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventSubprocessTimerSubscription, EventSubscription, EventSubscriptionKind,
    RuntimeBoundaryEventState, RuntimeEventWaitKind, RuntimeEventWaitState, RuntimeTimerJobState,
};
use crate::runtime::execution::Execution;
use crate::task::Task;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};
use uuid::Uuid;

fn resolve_message_ref_for_receive_task(
    command_context: &CommandContext,
    execution: &Execution,
) -> Option<EventSubscription> {
    let process_definition_id = execution.process_definition_id.as_deref()?;
    let activity_id = execution.activity_id.as_deref()?;

    let model = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)?;
    model
        .main_process
        .as_ref()
        .and_then(|process| process.flow_element_map.get(activity_id))
        .and_then(|flow_element| match flow_element {
            FlowElementEnum::ReceiveTask(receive_task) => {
                if let Some(r) = receive_task.message_ref.as_ref() {
                    // Resolve message name from model definition (consistent with catch events)
                    let event_ref = model
                        .messages
                        .iter()
                        .find(|m| m.base_element.id.as_deref() == Some(r))
                        .and_then(|m| m.name.clone())
                        .unwrap_or_else(|| r.clone());

                    return Some(EventSubscription {
                        kind: EventSubscriptionKind::Message,
                        event_ref,
                    });
                }
                // Event-registry receive task: flowable:eventType
                // (Java ReceiveTaskParseHandler.java:41-45).
                extension_element_text(
                    &receive_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .extension_elements,
                    ELEMENT_EVENT_TYPE,
                )
                .map(|event_type| EventSubscription {
                    kind: EventSubscriptionKind::Message,
                    event_ref: event_type,
                })
            }
            _ => None,
        })
}

fn resolve_receive_task_configuration(
    command_context: &CommandContext,
    execution: &Execution,
) -> Option<String> {
    let process_definition_id = execution.process_definition_id.as_deref()?;
    let activity_id = execution.activity_id.as_deref()?;
    let model = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)?;
    model
        .main_process
        .as_ref()
        .and_then(|process| process.flow_element_map.get(activity_id))
        .and_then(|flow_element| match flow_element {
            FlowElementEnum::ReceiveTask(receive_task) => correlation_key_from_base_element(
                &receive_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element,
                Some(execution),
            ),
            _ => None,
        })
}

pub struct ReceiveTaskActivityBehavior;

impl Default for ReceiveTaskActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl ReceiveTaskActivityBehavior {
    pub fn new() -> Self {
        Self
    }

    /// Registers a receive-task wait for an Event Registry inbound event.
    ///
    /// Java: `ReceiveEventTaskActivityBehavior.execute` (ReceiveTaskParseHandler.java:41)
    /// — stores an event-type subscription without creating a user Task.
    /// Kind is `EventRegistry` (not Message) so event definition keys do not collide
    /// with BPMN message names.
    pub fn execute_event_registry_receive(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
        event_definition_key: &str,
    ) -> Result<(), FlowableError> {
        // No definition/channel validation at registration: Java
        // `ReceiveEventTaskActivityBehavior.execute` creates the event-type
        // subscription without resolving the event definition (definition
        // resolution happens at event delivery). The pre-P92 base validated
        // here; that was the deviation, and P92 dropped it deliberately.
        let process_instance_id = execution.process_instance_id.clone().ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Receive task execution '{}' has no process_instance_id",
                execution.id
            ))
        })?;

        let activity_id = execution.activity_id.clone().ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Receive task execution '{}' has no activity_id",
                execution.id
            ))
        })?;

        execution.is_active = false;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        // Correlation from flowable:eventCorrelationParameter
        // (ReceiveEventTaskActivityBehavior.java:77 / CorrelationUtil.java:30-67).
        let configuration = resolve_receive_task_configuration(command_context, execution);
        command_context.runtime_store.insert_event_wait_state(
            &RuntimeEventWaitState {
                wait_kind: RuntimeEventWaitKind::ReceiveTask,
                process_instance_id: process_instance_id.clone(),
                execution_id: execution.id.clone(),
                task_id: None,
                activity_id: Some(activity_id),
                display_name: None,
                event_subscription: Some(EventSubscription {
                    kind: EventSubscriptionKind::EventRegistry,
                    event_ref: event_definition_key.to_string(),
                }),
                configuration,
            },
            &mut command_context.session,
        );

        command_context.history_manager.record_audit_event(
            "event-registry-receive-registered",
            execution.process_instance_id.as_deref(),
            execution.process_definition_id.as_deref(),
            Some(&format!(
                "Event registry receive task registered for event definition '{}'",
                event_definition_key
            )),
            &mut command_context.session,
        );

        Ok(())
    }
}

impl ActivityBehavior for ReceiveTaskActivityBehavior {
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

        let (task_name, boundary_events, skip_expression) = {
            let bpmn_model = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id);
            let maybe_flow_element = bpmn_model
                .as_ref()
                .and_then(|model| model.main_process.as_ref())
                .and_then(|process| process.flow_element_map.get(&activity_id));

            match maybe_flow_element {
                Some(FlowElementEnum::ReceiveTask(receive_task)) => {
                    let name = receive_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .name
                        .clone()
                        .unwrap_or_else(|| activity_id.clone());
                    let boundary_events = receive_task.task.activity.boundary_events.clone();
                    let skip_expression = receive_task.skip_expression.clone();
                    (name, boundary_events, skip_expression)
                }
                _ => (activity_id.clone(), Vec::new(), None),
            }
        };

        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);
        if crate::bpmn::skip_expression::should_skip_flow_element(
            skip_expression.as_deref(),
            "ReceiveTask",
            Some(&activity_id),
            &evaluation_execution,
        )? {
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
            return Ok(());
        }

        // Java ReceiveTaskParseHandler.java:41 — flowable:eventType selects the
        // Event Registry receive behavior (no user Task row).
        if let Some(event_type) = {
            let bpmn_model = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id);
            bpmn_model
                .as_ref()
                .and_then(|model| model.main_process.as_ref())
                .and_then(|process| process.flow_element_map.get(&activity_id))
                .and_then(|flow_element| match flow_element {
                    FlowElementEnum::ReceiveTask(receive_task) => resolve_event_type_extension(
                        &receive_task.task.activity.flow_node.flow_element.base_element,
                    ),
                    _ => None,
                })
        } {
            return self.execute_event_registry_receive(
                execution,
                command_context,
                &event_type,
            );
        }

        let process_instance_id = execution
            .process_instance_id
            .clone()
            .unwrap_or_else(|| execution.id.clone());
        let task_id = Uuid::new_v4().to_string();
        let task_activity_id = activity_id.clone();

        let mut task = Task::new(
            task_id.clone(),
            process_instance_id.clone(),
            execution.id.clone(),
            task_activity_id,
            task_name,
        );
        task.tenant_id = execution.tenant_id.clone();

        command_context
            .task_entity_manager
            .insert(&task, &mut command_context.session);

        let bpmn_model = command_context
            .deployment_manager
            .get_bpmn_model(&process_definition_id);

        // Register boundary events for this receive task.
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
                            retries: crate::bpmn::timer_util::default_timer_retries(
                                command_context,
                            ),
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
                    resolve_boundary_configuration(&boundary_event, Some(execution));
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

        execution.is_active = false;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);
        let configuration = resolve_receive_task_configuration(command_context, execution);
        command_context.runtime_store.insert_event_wait_state(
            &RuntimeEventWaitState {
                wait_kind: RuntimeEventWaitKind::ReceiveTask,
                process_instance_id: process_instance_id.clone(),
                execution_id: execution.id.clone(),
                task_id: Some(task_id),
                activity_id: Some(activity_id.clone()),
                display_name: None,
                event_subscription: resolve_message_ref_for_receive_task(
                    command_context,
                    execution,
                ),
                configuration,
            },
            &mut command_context.session,
        );

        register_event_subprocess_timer_subscriptions(
            command_context,
            &process_definition_id,
            &process_instance_id,
            execution,
        )?;

        Ok(())
    }
}

fn register_event_subprocess_timer_subscriptions(
    command_context: &mut CommandContext,
    process_definition_id: &str,
    process_instance_id: &str,
    execution: &Execution,
) -> Result<(), crate::error::FlowableError> {
    register_event_subprocess_subscriptions(
        command_context,
        process_definition_id,
        process_instance_id,
        execution,
    )
}

/// Unified event subprocess subscription registration for receive tasks.
fn register_event_subprocess_subscriptions(
    command_context: &mut CommandContext,
    process_definition_id: &str,
    process_instance_id: &str,
    execution: &Execution,
) -> Result<(), crate::error::FlowableError> {
    let bpmn_model = match command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)
    {
        Some(m) => m,
        None => return Ok(()),
    };
    let main_process = match bpmn_model.main_process.as_ref() {
        Some(p) => p,
        None => return Ok(()),
    };

    let now = command_context.runtime_store.time_source().now();

    // P6-B: job_category expression must walk the parent scope chain (forked
    // child maps may be empty after P4-7b).
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);

    let existing_timer_subs = command_context
        .runtime_store
        .find_event_subprocess_timer_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );

    let existing_event_subs = command_context
        .runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );

    let mut event_subprocesses: Vec<(&flowable_bpmn_model::model::SubProcess, String)> = Vec::new();
    for flow_element in &main_process.flow_elements {
        match flow_element {
            FlowElementEnum::EventSubProcess(esp) => {
                let esp_id = esp
                    .sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .clone()
                    .unwrap_or_default();
                event_subprocesses.push((&esp.sub_process, esp_id));
            }
            FlowElementEnum::SubProcess(sub) if sub.triggered_by_event => {
                let sub_id = sub
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .clone()
                    .unwrap_or_default();
                event_subprocesses.push((sub, sub_id));
            }
            _ => continue,
        }
    }

    for (sub_process, event_sub_id) in event_subprocesses {
        for inner_element in &sub_process.flow_elements {
            let start_event = match inner_element {
                FlowElementEnum::StartEvent(se) => se,
                _ => continue,
            };

            let start_event_id = start_event
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .clone()
                .unwrap_or_default();

            // Java ProcessInstanceHelper.java:371-398 — empty defs + eventType
            // registers an Event Registry event-subprocess subscription.
            if start_event.event.event_definitions.is_empty() {
                if let Some(event_type) = resolve_event_type_extension(
                    &start_event.event.flow_node.flow_element.base_element,
                ) {
                    if existing_event_subs.iter().any(|s| {
                        s.event_subprocess_id == event_sub_id
                            && s.event_kind == EventSubscriptionKind::EventRegistry
                            && s.event_ref == event_type
                    }) {
                        continue;
                    }
                    let sub_id = Uuid::new_v4().to_string();
                    // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                    // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                    crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                        command_context,
                            crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                subscription_id: sub_id,
                                process_instance_id: process_instance_id.to_string(),
                                scope_execution_id: Some(process_instance_id.to_string()),
                                scope_activity_id: None,
                                event_subprocess_id: event_sub_id.clone(),
                                start_event_id: start_event_id.clone(),
                                interrupting: start_event.interrupting,
                                event_kind: EventSubscriptionKind::EventRegistry,
                                event_ref: event_type,
                                // Event-subprocess event-registry correlation is
                                // not computed at runtime yet (P93 scope note).
                                configuration: None,
                            },
                        Some(process_definition_id),
                    );
                }
                continue;
            }

            for event_def in &start_event.event.event_definitions {
                match event_def {
                    EventDefinitionEnum::TimerEventDefinition(timer_def) => {
                        if existing_timer_subs
                            .iter()
                            .any(|s| s.event_subprocess_id == event_sub_id)
                        {
                            continue;
                        }

                        let sub_id = Uuid::new_v4().to_string();
                        let category = resolve_job_category(
                            &start_event.event.flow_node.flow_element.base_element,
                            &evaluation_execution,
                        );
                        // P17: EL-evaluate event-subprocess timer fields.
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
                        command_context
                            .runtime_store
                            .insert_event_subprocess_timer_subscription(
                                EventSubprocessTimerSubscription {
                                    subscription_id: sub_id,
                                    process_instance_id: process_instance_id.to_string(),
                                    event_subprocess_id: event_sub_id.clone(),
                                    start_event_id: start_event_id.clone(),
                                    interrupting: start_event.interrupting,
                                    time_duration: schedule.time_duration,
                                    time_date: schedule.time_date,
                                    time_cycle: schedule.time_cycle,
                                    end_date: schedule.end_date,
                                    calendar_name: schedule.calendar_name,
                                    due_time: schedule.due_time,
                                    lock_owner: None,
                                    lock_time: None,
                                    category,
                                },
                                &mut command_context.session,
                            );
                    }
                    EventDefinitionEnum::MessageEventDefinition(msg_def) => {
                        if let Some(ref msg_ref) = msg_def.message_ref {
                            if existing_event_subs.iter().any(|s| {
                                s.event_subprocess_id == event_sub_id
                                    && s.event_kind == EventSubscriptionKind::Message
                                    && s.event_ref == *msg_ref
                            }) {
                                continue;
                            }

                            let sub_id = Uuid::new_v4().to_string();
                            // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                            // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                            crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                                command_context,
                                    crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                        subscription_id: sub_id,
                                        process_instance_id: process_instance_id.to_string(),
                                        scope_execution_id: Some(process_instance_id.to_string()),
                                        scope_activity_id: None,
                                        event_subprocess_id: event_sub_id.clone(),
                                        start_event_id: start_event_id.clone(),
                                        interrupting: start_event.interrupting,
                                        event_kind: EventSubscriptionKind::Message,
                                        event_ref: msg_ref.clone(),
                                        configuration: None,
                                    },
                                Some(process_definition_id),
                            );
                        }
                    }
                    EventDefinitionEnum::SignalEventDefinition(sig_def) => {
                        if let Some(ref sig_ref) = sig_def.signal_ref {
                            if existing_event_subs.iter().any(|s| {
                                s.event_subprocess_id == event_sub_id
                                    && s.event_kind == EventSubscriptionKind::Signal
                                    && s.event_ref == *sig_ref
                            }) {
                                continue;
                            }

                            let sub_id = Uuid::new_v4().to_string();
                            // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                            // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                            crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                                command_context,
                                    crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                        subscription_id: sub_id,
                                        process_instance_id: process_instance_id.to_string(),
                                        scope_execution_id: Some(process_instance_id.to_string()),
                                        scope_activity_id: None,
                                        event_subprocess_id: event_sub_id.clone(),
                                        start_event_id: start_event_id.clone(),
                                        interrupting: start_event.interrupting,
                                        event_kind: EventSubscriptionKind::Signal,
                                        event_ref: sig_ref.clone(),
                                        configuration: None,
                                    },
                                Some(process_definition_id),
                            );
                        }
                    }
                    EventDefinitionEnum::EscalationEventDefinition(escalation_def) => {
                        let escalation_ref =
                            resolve_escalation_event_ref(escalation_def, Some(bpmn_model.as_ref()));

                        if existing_event_subs.iter().any(|s| {
                            s.event_subprocess_id == event_sub_id
                                && s.event_kind == EventSubscriptionKind::Escalation
                                && s.event_ref == escalation_ref
                        }) {
                            continue;
                        }

                        let sub_id = Uuid::new_v4().to_string();
                        // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                        // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                        crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                            command_context,
                            crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                subscription_id: sub_id,
                                process_instance_id: process_instance_id.to_string(),
                                scope_execution_id: Some(process_instance_id.to_string()),
                                scope_activity_id: None,
                                event_subprocess_id: event_sub_id.clone(),
                                start_event_id: start_event_id.clone(),
                                interrupting: start_event.interrupting,
                                event_kind: EventSubscriptionKind::Escalation,
                                event_ref: escalation_ref.clone(),
                                configuration: None,
                            },
                            Some(process_definition_id),
                        );
                    }
                    EventDefinitionEnum::ErrorEventDefinition(error_def) => {
                        let error_ref =
                            resolve_error_event_ref(error_def, Some(bpmn_model.as_ref()));

                        if existing_event_subs.iter().any(|s| {
                            s.event_subprocess_id == event_sub_id
                                && s.event_kind == EventSubscriptionKind::Error
                                && s.event_ref == error_ref
                        }) {
                            continue;
                        }

                        let sub_id = Uuid::new_v4().to_string();
                        let subscription =
                            crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                subscription_id: sub_id,
                                process_instance_id: process_instance_id.to_string(),
                                scope_execution_id: Some(process_instance_id.to_string()),
                                scope_activity_id: None,
                                event_subprocess_id: event_sub_id.clone(),
                                start_event_id: start_event_id.clone(),
                                interrupting: start_event.interrupting,
                                event_kind: EventSubscriptionKind::Error,
                                event_ref: error_ref.clone(),
                                configuration: None,
                            };
                        // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                        // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                        crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                            command_context,
                                subscription,
                            Some(process_definition_id),
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
