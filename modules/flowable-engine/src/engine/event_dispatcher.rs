use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::RuntimeTimerJobState;
use std::collections::BTreeMap;
use std::sync::Arc;

// =====================================================================
// P53 layer 1 — typed-event dispatch helpers (Java parity).
// Each helper is a thin builder that funnels a typed event into the
// existing `command_context.add_post_agenda_event` channel. No new
// dispatcher architecture is introduced — the existing double-layer
// (global + typed) dispatch is reused unchanged.
// =====================================================================

/// Dispatch `ENTITY_INITIALIZED` for a freshly inserted process instance.
/// Java reference: `ProcessInstanceHelper.java:227-275` — emitted after the
/// process instance row is inserted but before `PROCESS_STARTED`.
pub(crate) fn dispatch_process_instance_initialized(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    process_definition_id: &str,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::EntityInitialized,
        data: EntityEventData {
            entity_kind: EntityKind::ProcessInstance,
            entity_id: process_instance_id.to_string(),
            process_instance_id: Some(process_instance_id.to_string()),
            execution_id: Some(process_instance_id.to_string()),
            process_definition_id: Some(process_definition_id.to_string()),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `PROCESS_CREATED` for a freshly inserted process instance.
/// Java reference: `ProcessInstanceHelper.java:227-275`.
pub(crate) fn dispatch_process_instance_created(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    process_definition_id: &str,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::ProcessCreated,
        data: EntityEventData {
            entity_kind: EntityKind::ProcessInstance,
            entity_id: process_instance_id.to_string(),
            process_instance_id: Some(process_instance_id.to_string()),
            execution_id: Some(process_instance_id.to_string()),
            process_definition_id: Some(process_definition_id.to_string()),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `PROCESS_STARTED` when the start event fires.
/// Java reference: `ProcessInstanceHelper.java:302-317`.
pub(crate) fn dispatch_process_instance_started(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    process_definition_id: &str,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::ProcessStarted,
        data: EntityEventData {
            entity_kind: EntityKind::ProcessInstance,
            entity_id: process_instance_id.to_string(),
            process_instance_id: Some(process_instance_id.to_string()),
            execution_id: Some(process_instance_id.to_string()),
            process_definition_id: Some(process_definition_id.to_string()),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `PROCESS_COMPLETED` when the root execution ends (non-error,
/// non-escalation).
pub(crate) fn dispatch_process_instance_completed(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    process_definition_id: &str,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::ProcessCompleted,
        data: EntityEventData {
            entity_kind: EntityKind::ProcessInstance,
            entity_id: process_instance_id.to_string(),
            process_instance_id: Some(process_instance_id.to_string()),
            execution_id: Some(process_instance_id.to_string()),
            process_definition_id: Some(process_definition_id.to_string()),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `TASK_CREATED` after a task row is inserted in the create-task
/// path (Java `TaskHelper` flow).
pub(crate) fn dispatch_task_created(
    command_context: &mut CommandContext,
    task_id: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::TaskCreated,
        data: EntityEventData {
            entity_kind: EntityKind::Task,
            entity_id: task_id.to_string(),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: process_definition_id.map(str::to_string),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `TASK_COMPLETED` after the complete-task path finishes.
pub(crate) fn dispatch_task_completed(
    command_context: &mut CommandContext,
    task_id: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::TaskCompleted,
        data: EntityEventData {
            entity_kind: EntityKind::Task,
            entity_id: task_id.to_string(),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: None,
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `TASK_ASSIGNED` when the assignee or candidates change.
pub(crate) fn dispatch_task_assigned(
    command_context: &mut CommandContext,
    task_id: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::TaskAssigned,
        data: EntityEventData {
            entity_kind: EntityKind::Task,
            entity_id: task_id.to_string(),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: None,
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

// =====================================================================
// P119 — task field-change events (Java TaskEntityManagerImpl.logTaskUpdateEvents).
// =====================================================================

fn dispatch_task_field_changed(
    command_context: &mut CommandContext,
    event_type: EngineEventType,
    task_id: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type,
        data: EntityEventData {
            entity_kind: EntityKind::Task,
            entity_id: task_id.to_string(),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: None,
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `TASK_OWNER_CHANGED`.
/// Java: `TaskEntityManagerImpl.java:276-279` (`logTaskUpdateEvents`).
pub(crate) fn dispatch_task_owner_changed(
    command_context: &mut CommandContext,
    task_id: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
) {
    dispatch_task_field_changed(
        command_context,
        EngineEventType::TaskOwnerChanged,
        task_id,
        process_instance_id,
        execution_id,
    );
}

/// Dispatch `TASK_PRIORITY_CHANGED`.
/// Java: `TaskEntityManagerImpl.java:284-288`.
pub(crate) fn dispatch_task_priority_changed(
    command_context: &mut CommandContext,
    task_id: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
) {
    dispatch_task_field_changed(
        command_context,
        EngineEventType::TaskPriorityChanged,
        task_id,
        process_instance_id,
        execution_id,
    );
}

/// Dispatch `TASK_DUEDATE_CHANGED`.
/// Java: `TaskEntityManagerImpl.java:291-295`.
pub(crate) fn dispatch_task_duedate_changed(
    command_context: &mut CommandContext,
    task_id: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
) {
    dispatch_task_field_changed(
        command_context,
        EngineEventType::TaskDuedateChanged,
        task_id,
        process_instance_id,
        execution_id,
    );
}

/// Dispatch `TASK_NAME_CHANGED`.
/// Java: `TaskEntityManagerImpl.java:298-302`.
pub(crate) fn dispatch_task_name_changed(
    command_context: &mut CommandContext,
    task_id: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
) {
    dispatch_task_field_changed(
        command_context,
        EngineEventType::TaskNameChanged,
        task_id,
        process_instance_id,
        execution_id,
    );
}

// =====================================================================
// P119 — multi-instance activity events.
// =====================================================================

/// Dispatch `MULTI_INSTANCE_ACTIVITY_STARTED`.
/// Java: `ContinueProcessOperation.java:276-279`.
pub(crate) fn dispatch_multi_instance_activity_started(
    command_context: &mut CommandContext,
    activity_id: &str,
    activity_type: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::MultiInstanceActivityStarted,
        data: EntityEventData {
            entity_kind: EntityKind::Activity,
            entity_id: format!("{activity_id}:{activity_type}"),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: process_definition_id.map(str::to_string),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `MULTI_INSTANCE_ACTIVITY_COMPLETED` or
/// `MULTI_INSTANCE_ACTIVITY_COMPLETED_WITH_CONDITION`.
/// Java: `MultiInstanceActivityBehavior.java:424-436` /
/// `SequentialMultiInstanceBehavior.java:90-97` /
/// `ParallelMultiInstanceBehavior.java:302-319`.
pub(crate) fn dispatch_multi_instance_activity_completed(
    command_context: &mut CommandContext,
    activity_id: &str,
    activity_type: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
    with_condition: bool,
) {
    let event_type = if with_condition {
        EngineEventType::MultiInstanceActivityCompletedWithCondition
    } else {
        EngineEventType::MultiInstanceActivityCompleted
    };
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type,
        data: EntityEventData {
            entity_kind: EntityKind::Activity,
            entity_id: format!("{activity_id}:{activity_type}"),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: process_definition_id.map(str::to_string),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `MULTI_INSTANCE_ACTIVITY_CANCELLED`.
/// Java: `ExecutionEntityManagerImpl.java:777-785`.
pub(crate) fn dispatch_multi_instance_activity_cancelled(
    command_context: &mut CommandContext,
    activity_id: &str,
    activity_type: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::MultiInstanceActivityCancelled,
        data: EntityEventData {
            entity_kind: EntityKind::Activity,
            entity_id: format!("{activity_id}:{activity_type}"),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: process_definition_id.map(str::to_string),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

// =====================================================================
// P119 — timer job lifecycle events.
// =====================================================================

/// Dispatch `TIMER_SCHEDULED` after a timer job is inserted.
/// Java: `TimerJobSchedulerImpl.java:69-73`.
pub(crate) fn dispatch_timer_scheduled(
    command_context: &mut CommandContext,
    job: &RuntimeTimerJobState,
) {
    command_context.add_post_agenda_event(EngineEvent::Job {
        event_type: EngineEventType::TimerScheduled,
        job: job.clone(),
    });
}

/// Dispatch `TIMER_FIRED` when a timer job is about to execute.
/// Java: `TriggerTimerEventJobHandler.java:44-46` /
/// `TimerStartEventJobHandler.java:58`.
pub(crate) fn dispatch_timer_fired(
    command_context: &mut CommandContext,
    job: &RuntimeTimerJobState,
) {
    command_context.add_post_agenda_event(EngineEvent::Job {
        event_type: EngineEventType::TimerFired,
        job: job.clone(),
    });
}

/// Dispatch `JOB_RESCHEDULED` after a management timer reschedule.
/// Java: `TimerUtil.java:277-278` (`rescheduleTimerJob`) — fires before the
/// subsequent `TIMER_SCHEDULED` for the new job.
pub(crate) fn dispatch_job_rescheduled(
    command_context: &mut CommandContext,
    job: &RuntimeTimerJobState,
) {
    command_context.add_post_agenda_event(EngineEvent::Job {
        event_type: EngineEventType::JobRescheduled,
        job: job.clone(),
    });
}

// =====================================================================
// P125 — activity waiting / cancelled / conditional-received events.
// =====================================================================

fn dispatch_activity_named_event(
    command_context: &mut CommandContext,
    event_type: EngineEventType,
    activity_id: &str,
    event_name: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type,
        data: EntityEventData {
            entity_kind: EntityKind::Activity,
            // activityId + payload name (signal/message/condition/escalation).
            entity_id: format!("{activity_id}:{event_name}"),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: process_definition_id.map(str::to_string),
            scope_type: None,
            scope_id: Some(event_name.to_string()),
            sub_scope_id: None,
        },
    });
}

/// Dispatch `ACTIVITY_SIGNAL_WAITING`.
/// Java: `IntermediateCatchSignalEventActivityBehavior.java:74-76`,
/// `BoundarySignalEventActivityBehavior.java:79-81`,
/// `ProcessInstanceHelper.java:353-356` (event-subprocess start).
pub(crate) fn dispatch_activity_signal_waiting(
    command_context: &mut CommandContext,
    activity_id: &str,
    signal_name: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    dispatch_activity_named_event(
        command_context,
        EngineEventType::ActivitySignalWaiting,
        activity_id,
        signal_name,
        process_instance_id,
        execution_id,
        process_definition_id,
    );
}

/// Dispatch `ACTIVITY_MESSAGE_WAITING`.
/// Java: `IntermediateCatchMessageEventActivityBehavior.java:67-73`,
/// `BoundaryMessageEventActivityBehavior.java:71-73`,
/// `ProcessInstanceHelper.java:346-349`.
pub(crate) fn dispatch_activity_message_waiting(
    command_context: &mut CommandContext,
    activity_id: &str,
    message_name: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    dispatch_activity_named_event(
        command_context,
        EngineEventType::ActivityMessageWaiting,
        activity_id,
        message_name,
        process_instance_id,
        execution_id,
        process_definition_id,
    );
}

/// Dispatch `ACTIVITY_CONDITIONAL_WAITING`.
/// Java: `IntermediateCatchConditionalEventActivityBehavior.java:46-49`,
/// `BoundaryConditionalEventActivityBehavior.java:52-54`.
pub(crate) fn dispatch_activity_conditional_waiting(
    command_context: &mut CommandContext,
    activity_id: &str,
    condition_expression: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    dispatch_activity_named_event(
        command_context,
        EngineEventType::ActivityConditionalWaiting,
        activity_id,
        condition_expression,
        process_instance_id,
        execution_id,
        process_definition_id,
    );
}

/// Dispatch `ACTIVITY_ESCALATION_WAITING`.
/// Java: `BoundaryEscalationEventActivityBehavior.java:60-62`.
pub(crate) fn dispatch_activity_escalation_waiting(
    command_context: &mut CommandContext,
    activity_id: &str,
    escalation_code: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    dispatch_activity_named_event(
        command_context,
        EngineEventType::ActivityEscalationWaiting,
        activity_id,
        escalation_code,
        process_instance_id,
        execution_id,
        process_definition_id,
    );
}

/// Dispatch `ACTIVITY_MESSAGE_CANCELLED`.
/// Java: `ExecutionEntityManagerImpl.java:1063-1066`
/// (`deleteEventSubScriptions` — only for message event subscriptions).
pub(crate) fn dispatch_activity_message_cancelled(
    command_context: &mut CommandContext,
    activity_id: &str,
    message_name: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    dispatch_activity_named_event(
        command_context,
        EngineEventType::ActivityMessageCancelled,
        activity_id,
        message_name,
        process_instance_id,
        execution_id,
        process_definition_id,
    );
}

/// Dispatch `ACTIVITY_CONDITIONAL_RECEIVED`.
/// Java: `IntermediateCatchConditionalEventActivityBehavior.java:63-65`,
/// `BoundaryConditionalEventActivityBehavior.java:70-72`.
pub(crate) fn dispatch_activity_conditional_received(
    command_context: &mut CommandContext,
    activity_id: &str,
    condition_expression: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    dispatch_activity_named_event(
        command_context,
        EngineEventType::ActivityConditionalReceived,
        activity_id,
        condition_expression,
        process_instance_id,
        execution_id,
        process_definition_id,
    );
}

/// Dispatch `PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT`.
/// Java: `TerminateEndEventActivityBehavior.java:247-248`
/// (`sendProcessInstanceCompletedEvent` → `FlowableEventBuilder.createTerminateEvent`).
pub(crate) fn dispatch_process_completed_with_terminate_end_event(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    process_definition_id: &str,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::ProcessCompletedWithTerminateEndEvent,
        data: EntityEventData {
            entity_kind: EntityKind::ProcessInstance,
            entity_id: process_instance_id.to_string(),
            process_instance_id: Some(process_instance_id.to_string()),
            execution_id: Some(process_instance_id.to_string()),
            process_definition_id: Some(process_definition_id.to_string()),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Route a subscription insert to the matching `ACTIVITY_*_WAITING` event.
/// Covers intermediate catch, boundary, and event-subprocess start
/// subscription creation sites (Java throw points listed on each helper).
pub(crate) fn dispatch_activity_waiting_for_subscription(
    command_context: &mut CommandContext,
    activity_id: &str,
    kind: crate::persistence::runtime_store::EventSubscriptionKind,
    event_ref: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    use crate::persistence::runtime_store::EventSubscriptionKind;
    match kind {
        EventSubscriptionKind::Signal => dispatch_activity_signal_waiting(
            command_context,
            activity_id,
            event_ref,
            process_instance_id,
            execution_id,
            process_definition_id,
        ),
        EventSubscriptionKind::Message => dispatch_activity_message_waiting(
            command_context,
            activity_id,
            event_ref,
            process_instance_id,
            execution_id,
            process_definition_id,
        ),
        EventSubscriptionKind::Conditional => dispatch_activity_conditional_waiting(
            command_context,
            activity_id,
            event_ref,
            process_instance_id,
            execution_id,
            process_definition_id,
        ),
        EventSubscriptionKind::Escalation => dispatch_activity_escalation_waiting(
            command_context,
            activity_id,
            event_ref,
            process_instance_id,
            execution_id,
            process_definition_id,
        ),
        _ => {}
    }
}

/// Insert an event-subprocess start subscription and, for message/signal kinds,
/// dispatch the matching `ACTIVITY_*_WAITING` event.
///
/// Java: `ProcessInstanceHelper.java:343-358` — after
/// `processEventSubProcessStartEvent` registers message/signal subscriptions,
/// the dispatcher emits ACTIVITY_MESSAGE_WAITING / ACTIVITY_SIGNAL_WAITING.
/// Error / escalation / event-registry registrations do not emit WAITING here
/// (Java only loops the message and signal subscription lists).
///
/// Recovery snapshot import must keep calling the raw store insert so it does
/// not re-fire WAITING for restored rows.
pub(crate) fn insert_event_subprocess_subscription_with_waiting(
    command_context: &mut CommandContext,
    sub: crate::persistence::runtime_store::EventSubprocessEventSubscription,
    process_definition_id: Option<&str>,
) {
    use crate::persistence::runtime_store::EventSubscriptionKind;
    let activity_id = sub.start_event_id.clone();
    let kind = sub.event_kind.clone();
    let event_ref = sub.event_ref.clone();
    let process_instance_id = sub.process_instance_id.clone();
    let execution_id = sub.scope_execution_id.clone();
    command_context
        .runtime_store
        .insert_event_subprocess_event_subscription(sub, &mut command_context.session);
    if matches!(
        kind,
        EventSubscriptionKind::Message | EventSubscriptionKind::Signal
    ) {
        dispatch_activity_waiting_for_subscription(
            command_context,
            &activity_id,
            kind,
            &event_ref,
            Some(&process_instance_id),
            execution_id.as_deref(),
            process_definition_id,
        );
    }
}

// =====================================================================
// P119 — historic entity events (sync + async-replay).
// =====================================================================

/// Build `HISTORIC_PROCESS_INSTANCE_CREATED` entity event.
/// Java: `DefaultHistoryManager.java:120-126`.
pub(crate) fn historic_process_instance_created_event(
    process_instance_id: &str,
    process_definition_id: &str,
) -> EngineEvent {
    EngineEvent::Entity {
        event_type: EngineEventType::HistoricProcessInstanceCreated,
        data: EntityEventData {
            entity_kind: EntityKind::HistoricProcessInstance,
            entity_id: process_instance_id.to_string(),
            process_instance_id: Some(process_instance_id.to_string()),
            execution_id: Some(process_instance_id.to_string()),
            process_definition_id: Some(process_definition_id.to_string()),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    }
}

/// Build `HISTORIC_PROCESS_INSTANCE_ENDED` entity event.
/// Java: `DefaultHistoryManager.java:90-95`.
pub(crate) fn historic_process_instance_ended_event(
    process_instance_id: &str,
    process_definition_id: Option<&str>,
) -> EngineEvent {
    EngineEvent::Entity {
        event_type: EngineEventType::HistoricProcessInstanceEnded,
        data: EntityEventData {
            entity_kind: EntityKind::HistoricProcessInstance,
            entity_id: process_instance_id.to_string(),
            process_instance_id: Some(process_instance_id.to_string()),
            execution_id: Some(process_instance_id.to_string()),
            process_definition_id: process_definition_id.map(str::to_string),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    }
}

/// Build `HISTORIC_ACTIVITY_INSTANCE_CREATED` entity event.
/// Java: `DefaultHistoryManager.java:215-218`.
pub(crate) fn historic_activity_instance_created_event(
    historic_activity_instance_id: &str,
    activity_id: &str,
    process_instance_id: &str,
    execution_id: &str,
) -> EngineEvent {
    EngineEvent::Entity {
        event_type: EngineEventType::HistoricActivityInstanceCreated,
        data: EntityEventData {
            entity_kind: EntityKind::HistoricActivityInstance,
            entity_id: historic_activity_instance_id.to_string(),
            process_instance_id: Some(process_instance_id.to_string()),
            execution_id: Some(execution_id.to_string()),
            process_definition_id: None,
            scope_type: None,
            scope_id: Some(activity_id.to_string()),
            sub_scope_id: None,
        },
    }
}

/// Build `HISTORIC_ACTIVITY_INSTANCE_ENDED` entity event.
/// Java: `DefaultHistoryManager.java:234-237`.
pub(crate) fn historic_activity_instance_ended_event(
    historic_activity_instance_id: &str,
    activity_id: &str,
    process_instance_id: &str,
    execution_id: &str,
) -> EngineEvent {
    EngineEvent::Entity {
        event_type: EngineEventType::HistoricActivityInstanceEnded,
        data: EntityEventData {
            entity_kind: EntityKind::HistoricActivityInstance,
            entity_id: historic_activity_instance_id.to_string(),
            process_instance_id: Some(process_instance_id.to_string()),
            execution_id: Some(execution_id.to_string()),
            process_definition_id: None,
            scope_type: None,
            scope_id: Some(activity_id.to_string()),
            sub_scope_id: None,
        },
    }
}

// =====================================================================
// P53 layer 2 — activity / sequenceflow dispatch helpers (Java parity).
// =====================================================================

/// Dispatch `ACTIVITY_STARTED` (Java `FlowableActivityStartedEvent`).
/// Emitted from `ContinueProcessOperation.java:266-306` when execution
/// enters a flow node.
pub(crate) fn dispatch_activity_started(
    command_context: &mut CommandContext,
    activity_id: &str,
    activity_type: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::ActivityStarted,
        data: EntityEventData {
            entity_kind: EntityKind::Activity,
            entity_id: format!("{activity_id}:{activity_type}"),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: process_definition_id.map(str::to_string),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `ACTIVITY_COMPLETED` (Java `FlowableActivityCompletedEvent`).
/// Emitted from `TakeOutgoingSequenceFlowsOperation.java:159-196` when
/// execution leaves a flow node.
pub(crate) fn dispatch_activity_completed(
    command_context: &mut CommandContext,
    activity_id: &str,
    activity_type: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::ActivityCompleted,
        data: EntityEventData {
            entity_kind: EntityKind::Activity,
            entity_id: format!("{activity_id}:{activity_type}"),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: process_definition_id.map(str::to_string),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

/// Dispatch `SEQUENCEFLOW_TAKEN` (Java `FlowableSequenceFlowTakenEvent`).
/// Emitted from `ContinueProcessOperation.java:308-345`.
pub(crate) fn dispatch_sequenceflow_taken(
    command_context: &mut CommandContext,
    sequence_flow_id: &str,
    process_instance_id: Option<&str>,
    execution_id: Option<&str>,
    process_definition_id: Option<&str>,
) {
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::SequenceflowTaken,
        data: EntityEventData {
            entity_kind: EntityKind::SequenceFlow,
            entity_id: sequence_flow_id.to_string(),
            process_instance_id: process_instance_id.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_definition_id: process_definition_id.map(str::to_string),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EngineEventType {
    // === Generic entity lifecycle (existing) ===
    EntityUpdated,
    EntitySuspended,
    EntityActivated,
    /// Java `ENTITY_INITIALIZED` — emitted when an entity (process instance,
    /// task, etc.) is freshly created in the store, BEFORE any lifecycle
    /// transition (Java `ProcessInstanceHelper.java:227-275`).
    EntityInitialized,
    /// Java `PROCESS_CREATED` — emitted when a process instance row has been
    /// inserted (Java `ProcessInstanceHelper.java:227-275`).
    ProcessCreated,
    /// Java `PROCESS_STARTED` — emitted when execution enters the start event
    /// (Java `ProcessInstanceHelper.java:302-317`).
    ProcessStarted,
    /// Java `PROCESS_COMPLETED` — emitted when the root execution ends without
    /// escalation.
    ProcessCompleted,
    /// Java `PROCESS_COMPLETED_WITH_ESCALATION_END_EVENT` (pre-existing).
    ProcessCompletedWithEscalationEndEvent,
    /// Java `PROCESS_COMPLETED_WITH_ERROR_END_EVENT`.
    ProcessCompletedWithErrorEndEvent,
    /// Java `PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT` —
    /// `TerminateEndEventActivityBehavior.java:247-248`.
    ProcessCompletedWithTerminateEndEvent,
    /// Java `PROCESS_CANCELLED`.
    ProcessCancelled,
    /// Java `TASK_CREATED` — emitted in the create-task path before the create
    /// task listener runs (matches `TaskHelper` Java).
    TaskCreated,
    /// Java `TASK_COMPLETED` — emitted in the complete-task path after the
    /// complete listener runs.
    TaskCompleted,
    /// Java `TASK_ASSIGNED` — emitted whenever task assignment is set or
    /// cleared (set assignee, add candidate, claim, delegate).
    TaskAssigned,
    /// Java `TASK_OWNER_CHANGED` — `TaskEntityManagerImpl.java:276-279`.
    TaskOwnerChanged,
    /// Java `TASK_PRIORITY_CHANGED` — `TaskEntityManagerImpl.java:284-288`.
    TaskPriorityChanged,
    /// Java `TASK_DUEDATE_CHANGED` — `TaskEntityManagerImpl.java:291-295`.
    TaskDuedateChanged,
    /// Java `TASK_NAME_CHANGED` — `TaskEntityManagerImpl.java:298-302`.
    TaskNameChanged,
    /// Java `ACTIVITY_STARTED` — emitted when execution enters a flow node
    /// (Java `ContinueProcessOperation.java:266-306`).
    ActivityStarted,
    /// Java `ACTIVITY_COMPLETED` — emitted when execution leaves a flow node
    /// (Java `TakeOutgoingSequenceFlowsOperation.java:159-196`).
    ActivityCompleted,
    /// Java `ACTIVITY_CANCELLED` — emitted when an activity's execution is
    /// cancelled (e.g. boundary error, terminate, MI child removal).
    ActivityCancelled,
    /// Java `ACTIVITY_SIGNALED` — emitted on receive-task / signal-catch.
    ActivitySignaled,
    /// Java `ACTIVITY_SIGNAL_WAITING` —
    /// `IntermediateCatchSignalEventActivityBehavior.java:74-76` /
    /// `BoundarySignalEventActivityBehavior.java:79-81`.
    ActivitySignalWaiting,
    /// Java `ACTIVITY_MESSAGE_RECEIVED` — emitted on message-catch.
    ActivityMessageReceived,
    /// Java `ACTIVITY_MESSAGE_WAITING` —
    /// `IntermediateCatchMessageEventActivityBehavior.java:67-73` /
    /// `BoundaryMessageEventActivityBehavior.java:71-73`.
    ActivityMessageWaiting,
    /// Java `ACTIVITY_MESSAGE_CANCELLED` —
    /// `ExecutionEntityManagerImpl.java:1063-1066`.
    ActivityMessageCancelled,
    /// Java `ACTIVITY_ERROR_RECEIVED` — emitted on error catch / boundary.
    ActivityErrorReceived,
    /// Java `ACTIVITY_ESCALATION_RECEIVED`.
    ActivityEscalationReceived,
    /// Java `ACTIVITY_ESCALATION_WAITING` —
    /// `BoundaryEscalationEventActivityBehavior.java:60-62`.
    ActivityEscalationWaiting,
    /// Java `ACTIVITY_CONDITIONAL_WAITING` —
    /// `IntermediateCatchConditionalEventActivityBehavior.java:46-49` /
    /// `BoundaryConditionalEventActivityBehavior.java:52-54`.
    ActivityConditionalWaiting,
    /// Java `ACTIVITY_CONDITIONAL_RECEIVED` —
    /// `IntermediateCatchConditionalEventActivityBehavior.java:63-65` /
    /// `BoundaryConditionalEventActivityBehavior.java:70-72`.
    ActivityConditionalReceived,
    /// Java `ACTIVITY_COMPENSATE`.
    ActivityCompensate,
    /// Java `MULTI_INSTANCE_ACTIVITY_STARTED` —
    /// `ContinueProcessOperation.java:276-279`.
    MultiInstanceActivityStarted,
    /// Java `MULTI_INSTANCE_ACTIVITY_COMPLETED` —
    /// `MultiInstanceActivityBehavior.java:431-435`.
    MultiInstanceActivityCompleted,
    /// Java `MULTI_INSTANCE_ACTIVITY_COMPLETED_WITH_CONDITION` —
    /// `MultiInstanceActivityBehavior.java:424-428`.
    MultiInstanceActivityCompletedWithCondition,
    /// Java `MULTI_INSTANCE_ACTIVITY_CANCELLED` —
    /// `ExecutionEntityManagerImpl.java:777-785`.
    MultiInstanceActivityCancelled,
    /// Java `SEQUENCEFLOW_TAKEN` — emitted when a sequence flow is traversed
    /// (Java `ContinueProcessOperation.java:308-345`).
    SequenceflowTaken,
    /// Java `VARIABLE_CREATED`.
    VariableCreated,
    /// Java `VARIABLE_UPDATED`.
    VariableUpdated,
    /// Java `VARIABLE_DELETED`.
    VariableDeleted,
    /// Java `VARIABLE_PERSISTED` (post-commit).
    VariablePersisted,
    // === Job / async lifecycle (existing) ===
    JobCanceled,
    JobExecutionFailure,
    JobExecutionSuccess,
    JobMovedToDeadLetter,
    JobRejected,
    JobRetriesDecremented,
    /// Java `TIMER_SCHEDULED` — `TimerJobSchedulerImpl.java:69-73`.
    TimerScheduled,
    /// Java `TIMER_FIRED` — `TriggerTimerEventJobHandler.java:44-46`.
    TimerFired,
    /// Java `JOB_RESCHEDULED` — `TimerUtil.java:277-278`.
    JobRescheduled,
    /// Java `HISTORIC_PROCESS_INSTANCE_CREATED` —
    /// `DefaultHistoryManager.java:120-126`.
    HistoricProcessInstanceCreated,
    /// Java `HISTORIC_PROCESS_INSTANCE_ENDED` —
    /// `DefaultHistoryManager.java:90-95`.
    HistoricProcessInstanceEnded,
    /// Java `HISTORIC_ACTIVITY_INSTANCE_CREATED` —
    /// `DefaultHistoryManager.java:215-218`.
    HistoricActivityInstanceCreated,
    /// Java `HISTORIC_ACTIVITY_INSTANCE_ENDED` —
    /// `DefaultHistoryManager.java:234-237`.
    HistoricActivityInstanceEnded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransactionState {
    Committing,
    Committed,
    RollingBack,
    RolledBack,
}

/// Typed metadata for entity suspension/activation events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    ProcessInstance,
    Execution,
    Task,
    ProcessDefinition,
    /// Java `FlowableActivityEvent` — emitted together with ACTIVITY_STARTED
    /// / ACTIVITY_COMPLETED / ACTIVITY_CANCELLED / ACTIVITY_SIGNALED etc.
    Activity,
    /// Java `FlowableSequenceFlowTakenEvent`.
    SequenceFlow,
    /// Java `FlowableVariableEvent` — emitted together with VARIABLE_CREATED /
    /// VARIABLE_UPDATED / VARIABLE_DELETED.
    Variable,
    /// Java historic process instance entity (HISTORIC_PROCESS_INSTANCE_*).
    HistoricProcessInstance,
    /// Java historic activity instance entity (HISTORIC_ACTIVITY_INSTANCE_*).
    HistoricActivityInstance,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProcessInstance => "processInstance",
            Self::Execution => "execution",
            Self::Task => "task",
            Self::ProcessDefinition => "processDefinition",
            Self::Activity => "activity",
            Self::SequenceFlow => "sequenceFlow",
            Self::Variable => "variable",
            Self::HistoricProcessInstance => "historicProcessInstance",
            Self::HistoricActivityInstance => "historicActivityInstance",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EntityEventData {
    pub entity_kind: EntityKind,
    pub entity_id: String,
    pub process_instance_id: Option<String>,
    pub execution_id: Option<String>,
    pub process_definition_id: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub sub_scope_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum EngineEvent {
    Job {
        event_type: EngineEventType,
        job: RuntimeTimerJobState,
    },
    JobExecutionFailure {
        job: RuntimeTimerJobState,
        error: FlowableError,
    },
    Entity {
        event_type: EngineEventType,
        data: EntityEventData,
    },
}

impl EngineEvent {
    pub fn event_type(&self) -> EngineEventType {
        match self {
            Self::Job { event_type, .. } => *event_type,
            Self::JobExecutionFailure { .. } => EngineEventType::JobExecutionFailure,
            Self::Entity { event_type, .. } => *event_type,
        }
    }

    pub fn job(&self) -> &RuntimeTimerJobState {
        match self {
            Self::Job { job, .. } => job,
            Self::JobExecutionFailure { job, .. } => job,
            Self::Entity { .. } => panic!("EngineEvent::Entity has no job"),
        }
    }

    pub fn error(&self) -> Option<&FlowableError> {
        match self {
            Self::Job { .. } => None,
            Self::JobExecutionFailure { error, .. } => Some(error),
            Self::Entity { .. } => None,
        }
    }

    pub fn entity_data(&self) -> Option<&EntityEventData> {
        match self {
            Self::Entity { data, .. } => Some(data),
            _ => None,
        }
    }
}

pub trait EngineEventListener: Send + Sync {
    fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError>;

    fn is_fail_on_exception(&self) -> bool {
        false
    }

    fn is_fire_on_transaction_lifecycle_event(&self) -> bool {
        false
    }

    fn on_transaction(&self) -> TransactionState {
        TransactionState::Committed
    }
}

#[derive(Clone)]
pub(crate) struct TransactionEventInvocation {
    listener: Arc<dyn EngineEventListener>,
    event: EngineEvent,
}

impl TransactionEventInvocation {
    fn new(listener: Arc<dyn EngineEventListener>, event: EngineEvent) -> Self {
        Self { listener, event }
    }

    pub(crate) fn invoke(&self) -> Result<(), FlowableError> {
        dispatch_to_listener(&self.listener, &self.event)
    }
}

#[derive(Clone)]
pub struct EngineEventDispatcher {
    enabled: bool,
    global_listeners: Vec<Arc<dyn EngineEventListener>>,
    typed_listeners: BTreeMap<EngineEventType, Vec<Arc<dyn EngineEventListener>>>,
}

impl Default for EngineEventDispatcher {
    fn default() -> Self {
        Self {
            enabled: true,
            global_listeners: Vec::new(),
            typed_listeners: BTreeMap::new(),
        }
    }
}

impl std::fmt::Debug for EngineEventDispatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EngineEventDispatcher")
            .field("enabled", &self.enabled)
            .field("global_listener_count", &self.global_listeners.len())
            .field(
                "typed_listener_counts",
                &self
                    .typed_listeners
                    .iter()
                    .map(|(event_type, listeners)| (*event_type, listeners.len()))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl EngineEventDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn add_event_listener(&mut self, listener: Arc<dyn EngineEventListener>) {
        self.global_listeners.push(listener);
    }

    pub fn add_typed_event_listener(
        &mut self,
        event_type: EngineEventType,
        listener: Arc<dyn EngineEventListener>,
    ) {
        self.typed_listeners
            .entry(event_type)
            .or_default()
            .push(listener);
    }

    pub fn dispatch(&self, event: &EngineEvent) -> Result<(), FlowableError> {
        if !self.enabled {
            return Ok(());
        }
        self.visit_listeners(event, |listener| {
            if listener.is_fire_on_transaction_lifecycle_event() {
                return Ok(());
            }
            dispatch_to_listener(listener, event)
        })?;
        Ok(())
    }

    pub fn dispatch_in_context(
        &self,
        event: &EngineEvent,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        if !self.enabled {
            return Ok(());
        }
        self.visit_listeners(event, |listener| {
            if listener.is_fire_on_transaction_lifecycle_event() {
                command_context.add_transaction_event(
                    listener.on_transaction(),
                    TransactionEventInvocation::new(Arc::clone(listener), event.clone()),
                );
                Ok(())
            } else {
                dispatch_to_listener(listener, event)
            }
        })
    }

    fn visit_listeners(
        &self,
        event: &EngineEvent,
        mut visitor: impl FnMut(&Arc<dyn EngineEventListener>) -> Result<(), FlowableError>,
    ) -> Result<(), FlowableError> {
        for listener in &self.global_listeners {
            visitor(listener)?;
        }
        if let Some(listeners) = self.typed_listeners.get(&event.event_type()) {
            for listener in listeners {
                visitor(listener)?;
            }
        }
        Ok(())
    }
}

fn dispatch_to_listener(
    listener: &Arc<dyn EngineEventListener>,
    event: &EngineEvent,
) -> Result<(), FlowableError> {
    match listener.on_event(event) {
        Ok(()) => Ok(()),
        Err(error) if listener.is_fail_on_exception() => Err(error),
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingListener {
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
        error: Option<&'static str>,
        fail_on_exception: bool,
    }

    impl EngineEventListener for RecordingListener {
        fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
            self.events
                .lock()
                .unwrap()
                .push(format!("{}:{:?}", self.name, event.event_type()));
            match self.error {
                Some(error) => Err(FlowableError::ExecutionError(error.to_string())),
                None => Ok(()),
            }
        }

        fn is_fail_on_exception(&self) -> bool {
            self.fail_on_exception
        }
    }

    fn event() -> EngineEvent {
        EngineEvent::Job {
            event_type: EngineEventType::EntityUpdated,
            job: RuntimeTimerJobState {
                timer_job_id: "job-1".to_string(),
                process_instance_id: "process-1".to_string(),
                execution_id: "execution-1".to_string(),
                activity_id: "activity-1".to_string(),
                job_state: Some("async".to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                due_time: None,
                lock_owner: None,
                lock_time: None,
                lock_expiration_time: None,
                retries: Some(1),
                error_message: None,
                error_details: None,
                category: None,
                ..Default::default()
            },
        }
    }

    #[test]
    fn dispatches_global_before_typed_listeners() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = EngineEventDispatcher::new();
        dispatcher.add_event_listener(Arc::new(RecordingListener {
            name: "global",
            events: Arc::clone(&events),
            error: None,
            fail_on_exception: false,
        }));
        dispatcher.add_typed_event_listener(
            EngineEventType::EntityUpdated,
            Arc::new(RecordingListener {
                name: "typed",
                events: Arc::clone(&events),
                error: None,
                fail_on_exception: false,
            }),
        );

        dispatcher.dispatch(&event()).unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec!["global:EntityUpdated", "typed:EntityUpdated"]
        );
    }

    #[test]
    fn listener_error_only_fails_dispatch_when_requested() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = EngineEventDispatcher::new();
        dispatcher.add_event_listener(Arc::new(RecordingListener {
            name: "ignored",
            events: Arc::clone(&events),
            error: Some("ignored listener error"),
            fail_on_exception: false,
        }));
        dispatcher.add_event_listener(Arc::new(RecordingListener {
            name: "fatal",
            events,
            error: Some("fatal listener error"),
            fail_on_exception: true,
        }));

        let error = dispatcher.dispatch(&event()).unwrap_err();
        assert!(error.to_string().contains("fatal listener error"));
    }
}
