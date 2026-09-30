use crate::bpmn::behavior::boundary_event_activity_behavior::{
    resolve_boundary_event_subscription, runtime_cancel_activity,
};
use crate::cmd::trigger_boundary_event_cmd::TriggerBoundaryEventByEventRefCmd;
use crate::cmd::trigger_start_event_subscription_cmd::TriggerEventSubprocessByEventCmd;
use crate::engine::event_dispatcher::{EngineEvent, EngineEventType, EntityEventData, EntityKind};
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventSubprocessEventSubscription, EventSubscription, EventSubscriptionKind,
    RuntimeBoundaryEventState,
};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{BoundaryEvent, EventDefinitionEnum, FlowElementEnum};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EngineFault {
    BpmnError {
        code: String,
        message: Option<String>,
    },
    Execution {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EscalationCatchResult {
    pub(crate) interrupting: bool,
}

impl EngineFault {
    pub(crate) fn into_flowable_error(self) -> FlowableError {
        match self {
            Self::BpmnError { code, .. } => FlowableError::ExecutionError(code),
            Self::Execution { message } => FlowableError::ExecutionError(message),
        }
    }
}

/// Register error boundary subscriptions for the current activity execution.
///
/// **MI hosting note (P9-1 probe, 不可观测):** this is only called from
/// `ServiceTaskActivityBehavior` for `flowable:type="http"` tasks. Each MI
/// instance child re-enters here and `insert_boundary_event_state` keys by
/// `(process_instance_id, boundary_event_id)`, so siblings collapse to a
/// single row whose `host_execution_id` is the last writer (an instance child,
/// not the MI root). Java
/// `ContinueProcessOperation#executeMultiInstanceSynchronous` would host the
/// error boundary once on the MI root.
///
/// Attempts to construct a stable red contract (parallel MI HTTP + interrupting
/// error boundary via `handleStatusCodes`) did not yield a reliable observable
/// difference at process rest: registration is ephemeral inside the same
/// execute (register → HTTP → clear/propagate), and there is no wait-state
/// window to assert host/count the way timer/message boundaries do. Left
/// unchanged — do not "fix for parity" without an observable hang/trigger
/// difference.
pub(crate) fn register_error_boundaries_for_execution(
    execution: &Execution,
    command_context: &mut CommandContext,
) -> Result<(), FlowableError> {
    let Some(process_definition_id) = execution.process_definition_id.as_deref() else {
        return Ok(());
    };
    let Some(process_instance_id) = execution.process_instance_id.as_deref() else {
        return Ok(());
    };
    let Some(activity_id) = execution.activity_id.as_deref() else {
        return Ok(());
    };
    let Some(model) = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)
    else {
        return Ok(());
    };
    let Some(process) = model.main_process.as_ref() else {
        return Ok(());
    };

    let mut boundaries = Vec::new();
    collect_attached_boundaries(&process.flow_elements, activity_id, &mut boundaries);
    for boundary in boundaries {
        if !matches!(
            boundary.event.event_definitions.as_slice(),
            [EventDefinitionEnum::ErrorEventDefinition(_)]
        ) {
            continue;
        }
        let Some(boundary_event_id) = boundary
            .event
            .flow_node
            .flow_element
            .base_element
            .id
            .clone()
        else {
            continue;
        };
        let Some(event_subscription) =
            resolve_boundary_event_subscription(&boundary, Some(model.as_ref()))
        else {
            continue;
        };
        // Boundary correlation key (BoundaryEventRegistryEventActivityBehavior.java:68).
        let configuration =
            crate::bpmn::behavior::boundary_event_activity_behavior::resolve_boundary_configuration(
                &boundary,
                Some(execution),
            )?;
        crate::bpmn::behavior::boundary_event_activity_behavior::insert_boundary_event_state_with_waiting(
            command_context,
            RuntimeBoundaryEventState {
                boundary_event_id,
                attached_activity_id: activity_id.to_string(),
                process_instance_id: process_instance_id.to_string(),
                host_execution_id: execution.id.clone(),
                cancel_activity: runtime_cancel_activity(&boundary, &event_subscription),
                event_subscription,
                configuration,
            },
            execution.process_definition_id.as_deref(),
        );
    }
    register_error_event_subprocesses(execution, &model, &process.flow_elements, command_context);
    Ok(())
}

pub(crate) fn register_process_event_subprocesses(
    execution: &Execution,
    command_context: &mut CommandContext,
) -> Result<(), FlowableError> {
    let Some(process_definition_id) = execution.process_definition_id.as_deref() else {
        return Ok(());
    };
    let Some(model) = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)
    else {
        return Ok(());
    };
    let Some(process) = model.main_process.as_ref() else {
        return Ok(());
    };
    register_error_event_subprocesses(execution, &model, &process.flow_elements, command_context);
    Ok(())
}

pub(crate) fn clear_boundaries_for_execution(
    execution_id: &str,
    command_context: &mut CommandContext,
) {
    command_context
        .runtime_store
        .delete_boundary_event_states_by_host_execution_id(
            execution_id,
            &mut command_context.session,
        );
}

pub(crate) fn propagate_bpmn_error(
    execution: &mut Execution,
    code: &str,
    command_context: &mut CommandContext,
) -> Result<bool, FlowableError> {
    let Some(process_instance_id) = execution.process_instance_id.clone() else {
        return Ok(false);
    };

    execution.is_active = false;
    command_context
        .execution_entity_manager
        .update(execution, &mut command_context.session)?;

    if try_catch_bpmn_error_in_process_instance(
        command_context,
        &process_instance_id,
        code,
        &execution.id,
    )? {
        return Ok(true);
    }

    // Java `ErrorPropagation#propagateError` / `executeCatch`: when the current
    // process instance has no catch, walk the call-activity superExecution chain
    // and try parent process definitions.
    if propagate_bpmn_error_across_call_activities(command_context, &process_instance_id, code)? {
        return Ok(true);
    }

    execution.is_active = true;
    command_context
        .execution_entity_manager
        .update(execution, &mut command_context.session)?;
    Ok(false)
}

/// Synthetic Java-style class name for a Rust [`FlowableError`].
///
/// `flowable:mapException` entries carry Java `class=` FQCNs. Rust errors have
/// no JVM Class object, so we expose a stable type-name mapping that BPMN
/// authors can target (`java.lang.Exception` matches every variant via
/// `andChildren`). Errors that embed a Java-style class in their message are
/// also matched by [`find_matching_exception_mapping`] via substring.
fn java_class_name(error: &FlowableError) -> &'static str {
    match error.primary_error() {
        FlowableError::InvalidBpmnXml { .. } => {
            "org.flowable.common.engine.api.FlowableIllegalStateException"
        }
        FlowableError::UnsupportedElement { .. } => {
            "org.flowable.common.engine.api.FlowableIllegalStateException"
        }
        FlowableError::DeploymentValidationError(_) => {
            "org.flowable.common.engine.api.FlowableIllegalStateException"
        }
        FlowableError::BadRequest(_) => {
            "org.flowable.common.engine.api.FlowableIllegalArgumentException"
        }
        FlowableError::Forbidden(_) => "org.flowable.common.engine.api.FlowableSecurityException",
        FlowableError::Conflict(_) => "org.flowable.common.engine.api.FlowableConflictException",
        FlowableError::ExecutionError(_) => "org.flowable.engine.FlowableException",
        FlowableError::UnrecoverableJobError(_) => {
            "org.flowable.job.api.FlowableUnrecoverableJobException"
        }
        FlowableError::NotFound(_) => {
            "org.flowable.common.engine.api.FlowableObjectNotFoundException"
        }
        FlowableError::Internal(_) => "java.lang.RuntimeException",
        FlowableError::Generic(_) => "java.lang.Exception",
        FlowableError::Caused(_) => "java.lang.Exception",
    }
}

/// True when `child` is `parent` or a subclass of `parent` in the synthetic
/// hierarchy (Java `Class.isAssignableFrom` for `andChildren="true"`).
fn is_assignable_from(parent: &str, child: &str) -> bool {
    if parent == child {
        return true;
    }
    const ROOT: &str = "java.lang.Exception";
    const RUNTIME: &str = "java.lang.RuntimeException";
    const FLOWABLE_EX: &str = "org.flowable.engine.FlowableException";
    const ILLEGAL_STATE: &str = "org.flowable.common.engine.api.FlowableIllegalStateException";
    const ILLEGAL_ARG: &str = "org.flowable.common.engine.api.FlowableIllegalArgumentException";
    const OBJECT_NOT_FOUND: &str = "org.flowable.common.engine.api.FlowableObjectNotFoundException";
    const CONFLICT: &str = "org.flowable.common.engine.api.FlowableConflictException";
    const SECURITY: &str = "org.flowable.common.engine.api.FlowableSecurityException";

    match parent {
        ROOT => true,
        RUNTIME => matches!(child, RUNTIME | ILLEGAL_STATE | ILLEGAL_ARG),
        FLOWABLE_EX => matches!(child, FLOWABLE_EX | OBJECT_NOT_FOUND | CONFLICT | SECURITY),
        _ => false,
    }
}

/// Java `ErrorPropagation.findMatchingExceptionMapping:394-446`.
///
/// Matching rules (in order):
/// 1. First entry with empty `class_name` and non-empty `error_code` is the
///    default mapping (honours optional `root_cause`).
/// 2. Exact `class_name` match against the synthetic Java class name, or
///    `class_name` appearing in the error message (Java-style FQCNs embedded
///    by delegates). Honours optional `root_cause`.
/// 3. `and_children`: `is_assignable_from` on the synthetic hierarchy.
/// 4. Fall back to the default mapping.
pub(crate) fn find_matching_exception_mapping(
    error: &FlowableError,
    exception_map: &[flowable_bpmn_model::model::MapExceptionEntry],
) -> Option<String> {
    use flowable_bpmn_model::model::MapExceptionEntry;

    let class_name = java_class_name(error);
    let message = error.to_string();
    let root_cause_name = root_cause_class_name(error);
    let mut default_exception_mapping: Option<String> = None;

    for MapExceptionEntry {
        class_name: entry_class,
        error_code,
        and_children,
        root_cause,
    } in exception_map
    {
        let Some(error_code) = error_code.as_deref().filter(|code| !code.is_empty()) else {
            continue;
        };
        let entry_class = entry_class.as_deref().unwrap_or_default();
        let root_cause = root_cause.as_deref().unwrap_or_default();

        // Default mapping: no class specified.
        if entry_class.is_empty() && default_exception_mapping.is_none() {
            if !root_cause.is_empty() {
                if root_cause_name == Some(root_cause) {
                    default_exception_mapping = Some(error_code.to_string());
                }
                continue;
            }
            default_exception_mapping = Some(error_code.to_string());
            continue;
        }

        if entry_class.is_empty() {
            continue;
        }

        let class_matches = entry_class == class_name
            || message.contains(entry_class)
            || (*and_children && is_assignable_from(entry_class, class_name));

        if !class_matches {
            continue;
        }

        if !root_cause.is_empty() {
            if root_cause_name == Some(root_cause) {
                return Some(error_code.to_string());
            }
            continue;
        }
        return Some(error_code.to_string());
    }

    default_exception_mapping
}

/// Best-effort root-cause class name from a [`FlowableError`] cause chain.
fn root_cause_class_name(error: &FlowableError) -> Option<&'static str> {
    let mut current = error;
    loop {
        match current {
            FlowableError::Caused(chain) => current = chain.cause(),
            other => return Some(java_class_name(other)),
        }
    }
}

/// Java `ErrorPropagation.mapException:359-391` (activity-local half).
///
/// If `exception_map` maps `error` to an error code, propagate it as a BPMN
/// error and return `true`. Returns `false` when nothing matched so the caller
/// can surface the original error.
pub(crate) fn try_map_exception(
    error: &FlowableError,
    execution: &mut Execution,
    exception_map: &[flowable_bpmn_model::model::MapExceptionEntry],
    command_context: &mut CommandContext,
) -> Result<bool, FlowableError> {
    let Some(error_code) = find_matching_exception_mapping(error, exception_map) else {
        return Ok(false);
    };
    propagate_bpmn_error(execution, &error_code, command_context)
}

/// Java `ErrorPropagation.handleException:488-508` without the BpmnError type
/// (Rust has no `BpmnError` throwable; HTTP path already handles EngineFault).
///
/// Walks the activity `map_exceptions` first, then parent call-activity
/// `map_exceptions` (Java `mapException:365-388`). Returns the original error
/// when nothing maps.
pub(crate) fn handle_exception_with_map_exceptions(
    error: FlowableError,
    execution: &mut Execution,
    activity_map_exceptions: &[flowable_bpmn_model::model::MapExceptionEntry],
    command_context: &mut CommandContext,
) -> Result<(), FlowableError> {
    if try_map_exception(&error, execution, activity_map_exceptions, command_context)? {
        return Ok(());
    }

    // Java mapException: walk parent process / call-activity mapExceptions.
    if let Some(call_maps) = find_call_activity_map_exceptions(execution, command_context) {
        if try_map_exception(&error, execution, &call_maps, command_context)? {
            return Ok(());
        }
    }

    Err(error)
}

/// Collect `map_exceptions` from the nearest call-activity that started this
/// process instance (Java `mapException:365-388`).
fn find_call_activity_map_exceptions(
    execution: &Execution,
    command_context: &mut CommandContext,
) -> Option<Vec<flowable_bpmn_model::model::MapExceptionEntry>> {
    let process_instance_id = execution.process_instance_id.as_deref()?;
    let root = command_context
        .runtime_store
        .find_execution(process_instance_id, &mut command_context.session)?;
    let super_execution_id = root.super_execution_id.as_ref()?;
    let super_execution = command_context.runtime_store.find_execution(
        super_execution_id,
        &mut command_context.session,
    )?;
    let activity_id = super_execution.activity_id.as_deref()?;
    let super_model = command_context
        .deployment_manager
        .get_bpmn_model(super_execution.process_definition_id.as_deref()?)?;
    let process = super_model.main_process.as_ref()?;
    for flow_element in &process.flow_elements {
        if let FlowElementEnum::CallActivity(call) = flow_element {
            let call_id = call
                .activity
                .flow_node
                .flow_element
                .base_element
                .id
                .as_deref()
                .unwrap_or_default();
            if call_id == activity_id && !call.activity.map_exceptions.is_empty() {
                return Some(call.activity.map_exceptions.clone());
            }
        }
    }
    None
}

/// Try event-subprocess + error-boundary catches inside a single process instance.
pub(crate) fn try_catch_bpmn_error_in_process_instance(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    code: &str,
    source_execution_id: &str,
) -> Result<bool, FlowableError> {
    let boundary_cmd = TriggerBoundaryEventByEventRefCmd::with_source_execution(
        EventSubscriptionKind::Error,
        code.to_string(),
        process_instance_id.to_string(),
        source_execution_id.to_string(),
    );
    if boundary_cmd.execute_with_trigger_result(command_context)? {
        return Ok(true);
    }

    let event_subprocess_cmd = TriggerEventSubprocessByEventCmd::with_source_execution(
        EventSubscriptionKind::Error,
        code.to_string(),
        process_instance_id.to_string(),
        source_execution_id.to_string(),
    );
    Ok(!event_subprocess_cmd.execute(command_context)?.is_empty())
}

/// Try event-subprocess + escalation-boundary catches inside one process
/// instance and retain whether the selected handler interrupts its host.
///
/// The richer result is required for cross-call propagation: Java's
/// `EscalationPropagation#executeCatch` lets a non-interrupting parent handler
/// run alongside the called process, while an interrupting handler destroys
/// the call-activity subtree (including the child process instance).
pub(crate) fn try_catch_escalation_in_process_instance(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    escalation_ref: &str,
    source_execution_id: &str,
) -> Result<Option<EscalationCatchResult>, FlowableError> {
    let boundary_cmd = TriggerBoundaryEventByEventRefCmd::with_source_execution(
        EventSubscriptionKind::Escalation,
        escalation_ref.to_string(),
        process_instance_id.to_string(),
        source_execution_id.to_string(),
    );
    let boundary_result = boundary_cmd.execute_with_catch_result(command_context)?;
    if boundary_result.triggered {
        return Ok(Some(EscalationCatchResult {
            interrupting: boundary_result.interrupting,
        }));
    }

    let event_subprocess_cmd = TriggerEventSubprocessByEventCmd::with_source_execution(
        EventSubscriptionKind::Escalation,
        escalation_ref.to_string(),
        process_instance_id.to_string(),
        source_execution_id.to_string(),
    );
    let event_subprocess_result =
        event_subprocess_cmd.execute_with_trigger_result(command_context)?;
    Ok(
        (!event_subprocess_result.triggered_ids.is_empty()).then_some(EscalationCatchResult {
            interrupting: event_subprocess_result.interrupting,
        }),
    )
}

/// Walk the call-activity `super_execution` chain to find an escalation
/// catcher in a parent process instance.
///
/// Java `EscalationPropagation#executeCatch` (lines 146-178) records every
/// crossed called process instance in `toDeleteProcessInstanceIds`, emits
/// `PROCESS_COMPLETED_WITH_ESCALATION_END_EVENT` for it, and executes the
/// handler against the parent execution. In this engine, call-activity child
/// instances are not execution-tree descendants of the parent host, so an
/// interrupting parent handler additionally needs explicit runtime cleanup.
pub(crate) fn propagate_escalation_across_call_activities(
    command_context: &mut CommandContext,
    child_process_instance_id: &str,
    escalation_ref: &str,
) -> Result<Option<EscalationCatchResult>, FlowableError> {
    let mut current_pi_id = child_process_instance_id.to_string();
    let mut crossed_process_instance_ids = Vec::new();

    for _ in 0..64 {
        let Some(pi) = command_context
            .runtime_store
            .find_process_instance(&current_pi_id, &mut command_context.session)?
        else {
            return Ok(None);
        };
        let Some(super_execution_id) = pi.super_execution_id.clone() else {
            return Ok(None);
        };
        let Some(super_execution) = command_context
            .runtime_store
            .find_execution(&super_execution_id, &mut command_context.session)
        else {
            return Ok(None);
        };
        let Some(parent_process_instance_id) = super_execution.process_instance_id.clone() else {
            return Ok(None);
        };

        crossed_process_instance_ids.push(current_pi_id.clone());

        if let Some(catch_result) = try_catch_escalation_in_process_instance(
            command_context,
            &parent_process_instance_id,
            escalation_ref,
            &super_execution_id,
        )? {
            for process_instance_id in &crossed_process_instance_ids {
                dispatch_process_completed_with_escalation_end_event(
                    command_context,
                    process_instance_id,
                )?;
            }

            if catch_result.interrupting {
                for process_instance_id in &crossed_process_instance_ids {
                    end_process_instance_for_escalation_propagation(
                        command_context,
                        process_instance_id,
                        escalation_ref,
                    )?;
                }
            }

            return Ok(Some(catch_result));
        }

        current_pi_id = parent_process_instance_id;
    }

    Ok(None)
}

/// When a BPMN error is uncaught in `child_process_instance_id`, walk the
/// call-activity `super_execution` chain (Java `ErrorPropagation#executeCatch`
/// lines 187–195) and try parent catches. Intermediate child process instances
/// left via super are deleted with an ERROR_EVENT reason and do **not** take
/// the call activity normal outgoing (Java `:204–211`).
pub(crate) fn propagate_bpmn_error_across_call_activities(
    command_context: &mut CommandContext,
    child_process_instance_id: &str,
    code: &str,
) -> Result<bool, FlowableError> {
    let mut current_pi_id = child_process_instance_id.to_string();
    let mut to_delete: Vec<String> = Vec::new();
    let mut guard = 0;

    while guard < 64 {
        guard += 1;

        let Some(pi) = command_context
            .runtime_store
            .find_process_instance(&current_pi_id, &mut command_context.session)?
        else {
            return Ok(false);
        };
        let Some(super_exec_id) = pi.super_execution_id.clone() else {
            return Ok(false);
        };
        let Some(super_exec) = command_context
            .runtime_store
            .find_execution(&super_exec_id, &mut command_context.session)
        else {
            return Ok(false);
        };
        let Some(parent_pi_id) = super_exec.process_instance_id.clone() else {
            return Ok(false);
        };

        // Leaving this PI via super_execution — candidate for ERROR_EVENT delete
        // if a higher parent eventually catches (Java toDeleteProcessInstanceIds).
        to_delete.push(current_pi_id.clone());

        if try_catch_bpmn_error_in_process_instance(
            command_context,
            &parent_pi_id,
            code,
            &super_exec_id,
        )? {
            for pi_id in &to_delete {
                end_process_instance_for_error_propagation(command_context, pi_id, code)?;
            }
            return Ok(true);
        }

        // No catch at this parent level — continue upward from the parent PI.
        current_pi_id = parent_pi_id;
    }

    Ok(false)
}

/// End a call-activity child PI that is being discarded because a parent catch
/// handled the error. Mirrors Java
/// `deleteProcessInstanceExecutionEntity(..., "ERROR_EVENT " + errorId, ...)`.
/// Does **not** apply call-activity out-parameters or take the super execution's
/// normal outgoing sequence flows.
fn end_process_instance_for_error_propagation(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    error_code: &str,
) -> Result<(), FlowableError> {
    let delete_reason = format!("ERROR_EVENT {error_code}");
    end_process_instance_for_cross_call_propagation(
        command_context,
        process_instance_id,
        &delete_reason,
    )
}

fn end_process_instance_for_escalation_propagation(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    escalation_ref: &str,
) -> Result<(), FlowableError> {
    let delete_reason = if escalation_ref.is_empty() {
        "escalation end event".to_string()
    } else {
        format!("escalation end event ({escalation_ref})")
    };
    end_process_instance_for_cross_call_propagation(
        command_context,
        process_instance_id,
        &delete_reason,
    )
}

fn dispatch_process_completed_with_escalation_end_event(
    command_context: &mut CommandContext,
    process_instance_id: &str,
) -> Result<(), crate::error::FlowableError> {
    let Some(process_instance) = command_context
        .runtime_store
        .find_process_instance(process_instance_id, &mut command_context.session)?
    else {
        return Ok(());
    };

    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type: EngineEventType::ProcessCompletedWithEscalationEndEvent,
        data: EntityEventData {
            entity_kind: EntityKind::ProcessInstance,
            entity_id: process_instance.id.clone(),
            process_instance_id: Some(process_instance.id.clone()),
            execution_id: Some(process_instance.id.clone()),
            process_definition_id: Some(process_instance.process_definition_id.clone()),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });
    Ok(())
}

/// Removes a called process instance without applying call-activity output
/// parameters or taking the super execution's normal outgoing sequence flows.
fn end_process_instance_for_cross_call_propagation(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    delete_reason: &str,
) -> Result<(), FlowableError> {
    if let Some(mut pi) = command_context
        .runtime_store
        .find_process_instance(process_instance_id, &mut command_context.session)?
        && !pi.is_ended
    {
        pi.is_ended = true;
        command_context
            .runtime_store
            .update_process_instance(&pi, &mut command_context.session);
        command_context
            .history_manager
            .record_process_instance_end(
                process_instance_id,
                Some(delete_reason),
                &mut command_context.session,
            )?;
        command_context.history_manager.record_audit_event(
            "process-instance-end",
            Some(process_instance_id),
            Some(&pi.process_definition_id),
            Some(delete_reason),
            &mut command_context.session,
        );
    }

    let tasks = command_context
        .task_entity_manager
        .find_by_process_instance_id(process_instance_id, &mut command_context.session);
    for task in tasks {
        command_context.history_manager.record_task_end(
            &task.id,
            Some(delete_reason),
            &mut command_context.session,
        )?;
        command_context
            .task_entity_manager
            .delete(&task.id, &mut command_context.session);
    }

    let (store, session) = command_context.store_and_session();
    let executions: Vec<_> = store
        .snapshot_executions(session)
        .into_values()
        .filter(|execution| execution.process_instance_id.as_deref() == Some(process_instance_id))
        .collect();
    for execution in executions {
        store.delete_execution(&execution.id, session);
    }
    store.delete_event_wait_states_by_process_instance_id(process_instance_id, session);
    store.delete_boundary_event_states_by_process_instance_id(process_instance_id, session);
    store.delete_timer_job_states_by_process_instance_id(process_instance_id, session);
    store.delete_event_subprocess_timer_subscriptions_by_process_instance_id(
        process_instance_id,
        session,
    );
    store.delete_event_subprocess_event_subscriptions_by_process_instance_id(
        process_instance_id,
        session,
    );
    store.delete_compensation_subscriptions_by_process_instance_id(process_instance_id, session);

    Ok(())
}

pub(crate) fn uncaught_bpmn_error(code: &str) -> FlowableError {
    // Message matches Java `ErrorPropagation` when the full parent-chain walk
    // finds no catch (this engine now searches the call-activity super chain).
    FlowableError::ExecutionError(format!(
        "No catching boundary event found for error with errorCode '{code}', neither in same process nor in parent process"
    ))
}

fn collect_attached_boundaries(
    elements: &[FlowElementEnum],
    activity_id: &str,
    collected: &mut Vec<BoundaryEvent>,
) {
    for element in elements {
        match element {
            FlowElementEnum::BoundaryEvent(boundary)
                if boundary.attached_to_ref_id.as_deref() == Some(activity_id) =>
            {
                collected.push(boundary.clone());
            }
            FlowElementEnum::SubProcess(sub_process) => {
                collect_attached_boundaries(&sub_process.flow_elements, activity_id, collected);
            }
            FlowElementEnum::Transaction(transaction) => collect_attached_boundaries(
                &transaction.sub_process.flow_elements,
                activity_id,
                collected,
            ),
            FlowElementEnum::EventSubProcess(event_subprocess) => collect_attached_boundaries(
                &event_subprocess.sub_process.flow_elements,
                activity_id,
                collected,
            ),
            FlowElementEnum::AdhocSubProcess(adhoc) => collect_attached_boundaries(
                &adhoc.sub_process.flow_elements,
                activity_id,
                collected,
            ),
            _ => {}
        }
    }
}

fn register_error_event_subprocesses(
    execution: &Execution,
    model: &flowable_bpmn_model::model::BpmnModel,
    elements: &[FlowElementEnum],
    command_context: &mut CommandContext,
) {
    let Some(process_instance_id) = execution.process_instance_id.as_deref() else {
        return;
    };
    let existing = command_context
        .runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );
    for element in elements {
        let Some(event_subprocess) = (match element {
            FlowElementEnum::EventSubProcess(event_subprocess) => {
                Some(&event_subprocess.sub_process)
            }
            FlowElementEnum::SubProcess(sub_process) if sub_process.triggered_by_event => {
                Some(sub_process)
            }
            _ => None,
        }) else {
            continue;
        };
        let event_subprocess_id = flow_element_id(element).unwrap_or_default();
        for inner in &event_subprocess.flow_elements {
            let FlowElementEnum::StartEvent(start_event) = inner else {
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
            for definition in &start_event.event.event_definitions {
                let Some(subscription) = event_subprocess_subscription(definition, Some(model))
                else {
                    continue;
                };
                if !matches!(
                    subscription.kind,
                    EventSubscriptionKind::Error | EventSubscriptionKind::Escalation
                ) {
                    continue;
                }
                if existing.iter().any(|current| {
                    current.event_subprocess_id == event_subprocess_id
                        && current.start_event_id == start_event_id
                        && current.event_kind == subscription.kind
                        && current.event_ref == subscription.event_ref
                        && current.scope_execution_id.as_deref() == Some(&execution.id)
                }) {
                    continue;
                }
                // P134/P125: Java ProcessInstanceHelper.java:343-358 — error/
                // escalation kinds do not emit *_WAITING (helper no-ops).
                crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                    command_context,
                    EventSubprocessEventSubscription {
                        subscription_id: Uuid::new_v4().to_string(),
                        process_instance_id: process_instance_id.to_string(),
                        scope_execution_id: Some(execution.id.clone()),
                        scope_activity_id: execution.activity_id.clone(),
                        event_subprocess_id: event_subprocess_id.clone(),
                        start_event_id: start_event_id.clone(),
                        // P44: Java `StartEventParseHandler` (66-67) forces
                        // error start events to interrupting=true at parse
                        // time; `ErrorPropagation#executeCatch` (263-275)
                        // always deletes the source scope. Ignore the
                        // model's isInterrupting flag for error events.
                        interrupting: if matches!(subscription.kind, EventSubscriptionKind::Error) {
                            true
                        } else {
                            start_event.interrupting
                        },
                        event_kind: subscription.kind,
                        event_ref: subscription.event_ref,
                        configuration: None,
                    },
                    execution.process_definition_id.as_deref(),
                );
            }
        }
    }
}

fn event_subprocess_subscription(
    definition: &EventDefinitionEnum,
    model: Option<&flowable_bpmn_model::model::BpmnModel>,
) -> Option<EventSubscription> {
    match definition {
        EventDefinitionEnum::ErrorEventDefinition(error) => Some(EventSubscription {
            kind: EventSubscriptionKind::Error,
            event_ref: crate::bpmn::behavior::error_event_support::resolve_error_event_ref(
                error, model,
            ),
        }),
        EventDefinitionEnum::EscalationEventDefinition(escalation) => Some(EventSubscription {
            kind: EventSubscriptionKind::Escalation,
            event_ref:
                crate::bpmn::behavior::escalation_event_support::resolve_escalation_event_ref(
                    escalation, model,
                ),
        }),
        _ => None,
    }
}

fn flow_element_id(element: &FlowElementEnum) -> Option<String> {
    match element {
        FlowElementEnum::EventSubProcess(event_subprocess) => event_subprocess
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

#[cfg(test)]
mod tests {
    use super::*;
    use flowable_bpmn_model::model::MapExceptionEntry;

    fn entry(class: &str, code: &str, and_children: bool) -> MapExceptionEntry {
        MapExceptionEntry {
            class_name: Some(class.to_string()),
            error_code: Some(code.to_string()),
            and_children,
            root_cause: None,
        }
    }

    #[test]
    fn map_exception_exact_class_match() {
        let error = FlowableError::ExecutionError("boom".to_string());
        let map = vec![entry(
            "org.flowable.engine.FlowableException",
            "mappedCode",
            false,
        )];
        assert_eq!(
            find_matching_exception_mapping(&error, &map),
            Some("mappedCode".to_string())
        );
    }

    #[test]
    fn map_exception_default_when_no_class() {
        let error = FlowableError::ExecutionError("boom".to_string());
        let map = vec![MapExceptionEntry {
            class_name: None,
            error_code: Some("defaultCode".to_string()),
            and_children: false,
            root_cause: None,
        }];
        assert_eq!(
            find_matching_exception_mapping(&error, &map),
            Some("defaultCode".to_string())
        );
    }

    #[test]
    fn map_exception_and_children_matches_superclass() {
        let error = FlowableError::BadRequest("bad".to_string());
        // java.lang.Exception + andChildren matches every synthetic class.
        let map = vec![entry("java.lang.Exception", "anyError", true)];
        assert_eq!(
            find_matching_exception_mapping(&error, &map),
            Some("anyError".to_string())
        );
    }

    #[test]
    fn map_exception_no_match_returns_none() {
        let error = FlowableError::NotFound("missing".to_string());
        let map = vec![entry(
            "com.acme.InsufficientFundsException",
            "fundsError",
            false,
        )];
        assert_eq!(find_matching_exception_mapping(&error, &map), None);
    }

    #[test]
    fn map_exception_message_substring_match() {
        // Delegates may embed a Java-style FQCN in the message.
        let error = FlowableError::ExecutionError(
            "com.acme.InsufficientFundsException: balance too low".to_string(),
        );
        let map = vec![entry(
            "com.acme.InsufficientFundsException",
            "fundsError",
            false,
        )];
        assert_eq!(
            find_matching_exception_mapping(&error, &map),
            Some("fundsError".to_string())
        );
    }
}
