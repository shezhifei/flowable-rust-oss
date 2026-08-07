use crate::agenda::FlowableEngineAgenda;
use crate::bpmn::behavior::boundary_event_activity_behavior::{
    resolve_boundary_event_subscription, runtime_cancel_activity,
};
use crate::bpmn::behavior::error_event_support::resolve_error_event_ref;
use crate::bpmn::behavior::escalation_event_support::resolve_escalation_event_ref;
use crate::bpmn::job_category::resolve_job_category;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventSubprocessEventSubscription, EventSubscription, EventSubscriptionKind,
    RuntimeBoundaryEventState, RuntimeTimerJobState,
};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{BpmnModel, EventDefinitionEnum, FlowElementEnum, SubProcess};
use uuid::Uuid;

fn sub_process_id(sub_process: &SubProcess) -> Option<&str> {
    sub_process
        .activity
        .flow_node
        .flow_element
        .base_element
        .id
        .as_deref()
}

fn find_sub_process<'a>(
    flow_elements: &'a [FlowElementEnum],
    activity_id: &str,
) -> Option<&'a SubProcess> {
    for flow_element in flow_elements {
        let nested: Option<&[FlowElementEnum]> = match flow_element {
            FlowElementEnum::SubProcess(sub_process) => {
                if sub_process_id(sub_process) == Some(activity_id) {
                    return Some(sub_process);
                }
                Some(&sub_process.flow_elements)
            }
            FlowElementEnum::Transaction(transaction) => {
                let sub_process = &transaction.sub_process;
                if sub_process_id(sub_process) == Some(activity_id) {
                    return Some(sub_process);
                }
                Some(&sub_process.flow_elements)
            }
            FlowElementEnum::EventSubProcess(event_sub_process) => {
                let sub_process = &event_sub_process.sub_process;
                if sub_process_id(sub_process) == Some(activity_id) {
                    return Some(sub_process);
                }
                Some(&sub_process.flow_elements)
            }
            FlowElementEnum::AdhocSubProcess(adhoc_sub_process) => {
                let sub_process = &adhoc_sub_process.sub_process;
                if sub_process_id(sub_process) == Some(activity_id) {
                    return Some(sub_process);
                }
                Some(&sub_process.flow_elements)
            }
            _ => None,
        };

        if let Some(nested_flow_elements) = nested
            && let Some(found) = find_sub_process(nested_flow_elements, activity_id)
        {
            return Some(found);
        }
    }

    None
}

fn flow_element_base_id(flow_element: &FlowElementEnum) -> Option<String> {
    match flow_element {
        FlowElementEnum::EventSubProcess(event_sub_process) => event_sub_process
            .sub_process
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

fn event_subprocess_subscription(
    event_definition: &EventDefinitionEnum,
    model: Option<&BpmnModel>,
) -> Option<EventSubscription> {
    match event_definition {
        EventDefinitionEnum::MessageEventDefinition(message) => {
            message
                .message_ref
                .as_ref()
                .map(|event_ref| EventSubscription {
                    kind: EventSubscriptionKind::Message,
                    event_ref: event_ref.clone(),
                })
        }
        EventDefinitionEnum::SignalEventDefinition(signal) => {
            signal
                .signal_ref
                .as_ref()
                .map(|event_ref| EventSubscription {
                    kind: EventSubscriptionKind::Signal,
                    event_ref: event_ref.clone(),
                })
        }
        EventDefinitionEnum::EscalationEventDefinition(escalation) => Some(EventSubscription {
            kind: EventSubscriptionKind::Escalation,
            event_ref: resolve_escalation_event_ref(escalation, model),
        }),
        EventDefinitionEnum::ErrorEventDefinition(error) => Some(EventSubscription {
            kind: EventSubscriptionKind::Error,
            event_ref: resolve_error_event_ref(error, model),
        }),
        _ => None,
    }
}

/// Event-registry start for an event subprocess (empty event defs + eventType).
/// Java: `ProcessInstanceHelper.processEventSubProcessStartEvent:371-398`.
fn event_subprocess_event_registry_subscription(
    start_event: &flowable_bpmn_model::model::StartEvent,
) -> Option<EventSubscription> {
    if !start_event.event.event_definitions.is_empty() {
        return None;
    }
    crate::bpmn::behavior::event_registry_event_support::resolve_event_type_extension(
        &start_event.event.flow_node.flow_element.base_element,
    )
    .map(|event_ref| EventSubscription {
        kind: EventSubscriptionKind::EventRegistry,
        event_ref,
    })
}

fn register_event_subprocess_event_subscriptions(
    command_context: &mut CommandContext,
    sub_process: &SubProcess,
    process_instance_id: &str,
    scope_execution_id: &str,
    scope_activity_id: &str,
) {
    let process_definition_id = command_context
        .runtime_store
        .find_execution(scope_execution_id, &mut command_context.session)
        .and_then(|execution| execution.process_definition_id);
    let bpmn_model = process_definition_id.as_deref().and_then(|definition_id| {
        command_context
            .deployment_manager
            .get_bpmn_model(definition_id)
    });

    let existing_event_subs = command_context
        .runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );

    for flow_element in &sub_process.flow_elements {
        let event_subprocess = match flow_element {
            FlowElementEnum::EventSubProcess(event_subprocess) => {
                Some(&event_subprocess.sub_process)
            }
            FlowElementEnum::SubProcess(event_subprocess)
                if event_subprocess.triggered_by_event =>
            {
                Some(event_subprocess)
            }
            _ => None,
        };

        let Some(event_subprocess) = event_subprocess else {
            continue;
        };

        let event_subprocess_id = flow_element_base_id(flow_element).unwrap_or_default();

        for inner_element in &event_subprocess.flow_elements {
            let FlowElementEnum::StartEvent(start_event) = inner_element else {
                continue;
            };

            let start_event_id = start_event
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .clone()
                .unwrap_or_default();

            let mut subscriptions: Vec<EventSubscription> = Vec::new();
            if let Some(event_registry_sub) =
                event_subprocess_event_registry_subscription(start_event)
            {
                subscriptions.push(event_registry_sub);
            }
            for event_definition in &start_event.event.event_definitions {
                if let Some(event_subscription) = event_subprocess_subscription(
                    event_definition,
                    bpmn_model.as_ref().map(|arc| arc.as_ref()),
                ) {
                    subscriptions.push(event_subscription);
                }
            }

            for event_subscription in subscriptions {
                if existing_event_subs.iter().any(|subscription| {
                    subscription.event_subprocess_id == event_subprocess_id
                        && subscription.start_event_id == start_event_id
                        && subscription.event_kind == event_subscription.kind
                        && subscription.event_ref == event_subscription.event_ref
                        && subscription.scope_execution_id.as_deref() == Some(scope_execution_id)
                }) {
                    continue;
                }

                // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                    command_context,
                    EventSubprocessEventSubscription {
                        subscription_id: Uuid::new_v4().to_string(),
                        process_instance_id: process_instance_id.to_string(),
                        scope_execution_id: Some(scope_execution_id.to_string()),
                        scope_activity_id: Some(scope_activity_id.to_string()),
                        event_subprocess_id: event_subprocess_id.clone(),
                        start_event_id: start_event_id.clone(),
                        // P44: Java `StartEventParseHandler` (66-67) forces
                        // error start events to interrupting=true at parse
                        // time; `ErrorPropagation#executeCatch` (263-275)
                        // always deletes the source scope. Other event
                        // kinds honor the model's isInterrupting flag.
                        interrupting: event_subscription.kind == EventSubscriptionKind::Error
                            || start_event.interrupting,
                        event_kind: event_subscription.kind,
                        event_ref: event_subscription.event_ref,
                        configuration: None,
                    },
                    process_definition_id.as_deref(),
                );
            }
        }
    }
}

pub struct SubProcessActivityBehavior;

impl Default for SubProcessActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl SubProcessActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for SubProcessActivityBehavior {
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
        let mut active_sub_process_clone = None;
        {
            if let Some(bpmn_model) = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id)
                && let Some(process) = bpmn_model.main_process.as_ref()
                && let Some(sub_process) = find_sub_process(&process.flow_elements, &activity_id)
            {
                boundary_events = sub_process.activity.boundary_events.clone();
                for inner_element in &sub_process.flow_elements {
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
                active_sub_process_clone = Some(sub_process.clone()); // Arc<BpmnModel>已应用，clone仍然需要
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

        // The current execution becomes the scope execution for the subprocess.
        execution.is_scope = true;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        let process_instance_id = execution
            .process_instance_id
            .clone()
            .unwrap_or_else(|| execution.id.clone());

        register_event_subprocess_event_subscriptions(
            command_context,
            active_sub_process_clone.as_ref().unwrap(),
            &process_instance_id,
            &execution.id,
            &activity_id,
        );

        let bpmn_model = command_context
            .deployment_manager
            .get_bpmn_model(&process_definition_id);

        // P6-B: job_category expression must walk the parent scope chain
        // (forked child maps may be empty after P4-7b).
        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);

        // Register boundary events for this SubProcess.
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
                    // for the whole MI activity. Also covers sequential
                    // SubProcess rounds that recreate the scope child — without
                    // this, DestroyScope of the old child deletes the timer and
                    // the new round re-schedules with a reset due_time.
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

        // Initialize subprocess data objects as local variables.
        // Java SubProcessActivityBehavior#processDataObjects: converter-typed
        // values only — no EL evaluation of `${...}`.
        if let Some(sub_process) = active_sub_process_clone.as_ref() {
            for data_object in &sub_process.data_objects {
                if let Some(name) = &data_object.name {
                    if execution.local_variables.contains_key(name) {
                        continue;
                    }
                    let value = data_object.value.clone().unwrap_or(serde_json::Value::Null);
                    execution.local_variables.insert(name.to_string(), value);
                }
            }
            command_context
                .execution_entity_manager
                .update(execution, &mut command_context.session);
        }

        // Create a child execution for the inner start event
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
