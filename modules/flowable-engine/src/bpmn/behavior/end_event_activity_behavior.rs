use crate::agenda::FlowableEngineAgenda;
use crate::bpmn::behavior::call_activity_behavior::apply_call_activity_out_parameters;
use crate::bpmn::behavior::error_event_support::resolve_error_event_ref;
use crate::bpmn::behavior::escalation_event_support::resolve_escalation_event_ref;
use crate::bpmn::behavior::intermediate_throw_event_activity_behavior::IntermediateThrowEventActivityBehavior;
use crate::bpmn::fault::{
    propagate_bpmn_error_across_call_activities, propagate_escalation_across_call_activities,
    try_catch_bpmn_error_in_process_instance, try_catch_escalation_in_process_instance,
};
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::engine::cmmn_process_task_callback::{
    CmmnProcessTaskCallbackOutcome, notify_cmmn_process_task_callback_for_instance,
};
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use crate::runtime::process_instance::ProcessInstance;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};
use uuid::Uuid;

use crate::cmd::trigger_start_event_subscription_cmd::NON_INTERRUPTING_EVENT_SUBPROCESS_PATH_VARIABLE;
pub struct EndEventActivityBehavior;

impl Default for EndEventActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl EndEventActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for EndEventActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let process_definition_id = execution.process_definition_id.as_deref().unwrap_or("");
        let activity_id = execution.activity_id.as_deref().unwrap_or("");

        let mut error_ref = None;
        let mut escalation_ref = None;
        let mut compensation_activity_ref = None;
        let mut terminate: Option<(bool, bool)> = None;

        {
            if let Some(model) = command_context
                .deployment_manager
                .get_bpmn_model(process_definition_id)
                && let Some(process) = model.main_process.as_ref()
                && let Some(FlowElementEnum::EndEvent(event)) =
                    process.flow_element_map.get(activity_id)
            {
                // Cancel end events never reach this behavior: factory routes
                // `CancelEventDefinition` to `CancelEndEventActivityBehavior`
                // (`activity_behavior_factory.rs`). Do not re-handle cancel here.
                if let [EventDefinitionEnum::ErrorEventDefinition(err_def)] =
                    event.event.event_definitions.as_slice()
                {
                    error_ref = Some(resolve_error_event_ref(err_def, Some(model.as_ref())));
                } else if let [EventDefinitionEnum::EscalationEventDefinition(escalation_def)] =
                    event.event.event_definitions.as_slice()
                {
                    escalation_ref = Some(resolve_escalation_event_ref(
                        escalation_def,
                        Some(model.as_ref()),
                    ));
                } else if let [EventDefinitionEnum::CompensateEventDefinition(compensate_def)] =
                    event.event.event_definitions.as_slice()
                {
                    compensation_activity_ref = Some(compensate_def.activity_ref.clone());
                } else if let [EventDefinitionEnum::TerminateEventDefinition(terminate_def)] =
                    event.event.event_definitions.as_slice()
                {
                    terminate = Some((
                        terminate_def.terminate_all,
                        terminate_def.terminate_multi_instance,
                    ));
                }
            }
        }

        // Java `TerminateEndEventActivityBehavior#execute` (60-207): terminate
        // end events never fall through to the regular end event leave.
        if let Some((terminate_all, terminate_multi_instance)) = terminate {
            return execute_terminate_end_event(
                execution,
                command_context,
                terminate_all,
                terminate_multi_instance,
            );
        }

        if let Some(err_ref) = error_ref
            && let Some(pi_id) = &execution.process_instance_id
        {
            let source_execution_id = execution.id.clone();
            let pi_id = pi_id.clone();

            // Local catch (same process instance): event subprocess then boundary.
            if try_catch_bpmn_error_in_process_instance(
                command_context,
                &pi_id,
                &err_ref,
                &source_execution_id,
            )? {
                // Local boundary may have destroyed the host tree; drop the
                // error-end token if it still exists.
                command_context
                    .runtime_store
                    .delete_execution(&source_execution_id, &mut command_context.session);
                return Ok(());
            }

            // Cross call-activity: Java ErrorPropagation walks superExecution
            // and lets a parent call activity / wrapping scope boundary catch.
            if propagate_bpmn_error_across_call_activities(command_context, &pi_id, &err_ref)? {
                // Child PI(s) were ended with ERROR_EVENT by the propagator;
                // the error-end token may already be gone with them.
                command_context
                    .runtime_store
                    .delete_execution(&source_execution_id, &mut command_context.session);
                return Ok(());
            }

            command_context
                .runtime_store
                .delete_execution(&source_execution_id, &mut command_context.session);

            // Uncaught error: end the process instance with a failure outcome
            // and propagate it to the CMMN processTask parent (if any).
            // Existing call-activity leave path (out-params + take outgoing)
            // is preserved via `end_process_instance_with_callback_outcome`.
            let failure_message = if err_ref.is_empty() {
                format!(
                    "BPMN child process instance '{}' ended with an uncaught error",
                    pi_id
                )
            } else {
                format!(
                    "BPMN child process instance '{}' ended with uncaught error '{}'",
                    pi_id, err_ref
                )
            };
            end_process_instance_with_callback_outcome(
                command_context,
                &pi_id,
                CmmnProcessTaskCallbackOutcome::Failed {
                    failure_message: failure_message.clone(),
                },
                Some(&failure_message),
                None,
                // Java: only the regular `EndExecutionOperation` defers on
                // completeAsync; error propagation ends synchronously.
                true,
            )?;
            return Ok(());
        }

        if let Some(escalation_ref) = escalation_ref
            && let Some(pi_id) = &execution.process_instance_id
        {
            let source_execution_id = execution.id.clone();
            let caught_locally = try_catch_escalation_in_process_instance(
                command_context,
                pi_id,
                &escalation_ref,
                &source_execution_id,
            )?
            .is_some();

            if !caught_locally {
                propagate_escalation_across_call_activities(
                    command_context,
                    pi_id,
                    &escalation_ref,
                )?;
            }

            // Interrupting handlers delete the throwing execution. Do not run
            // normal end-event completion on a detached in-memory snapshot.
            if command_context
                .runtime_store
                .find_execution(&source_execution_id, &mut command_context.session)
                .is_none()
            {
                return Ok(());
            }
        }

        if let Some(activity_ref) = compensation_activity_ref {
            IntermediateThrowEventActivityBehavior::trigger_registered_compensation(
                execution,
                command_context,
                activity_ref.as_deref(),
            );
        }

        execution.is_ended = true;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);

        // Structural flag set when the non-interrupting event-subprocess path
        // was injected (see trigger_start_event_subscription_cmd). Not a
        // process variable — survives commit and is invisible to REST.
        let is_non_interrupting_event_subprocess_path =
            execution.non_interrupting_event_subprocess_path
                || execution
                    .process_variable(NON_INTERRUPTING_EVENT_SUBPROCESS_PATH_VARIABLE)
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
        if is_non_interrupting_event_subprocess_path {
            // For non-interrupting event subprocess paths, we should not end the process
            // This allows the host process to continue running
            return Ok(());
        }

        if let Some(pi_id) = &execution.process_instance_id {
            let all_executions = command_context
                .runtime_store
                .snapshot_executions(&mut command_context.session);
            let active_siblings: Vec<_> = all_executions
                .values()
                .filter(|e| {
                    e.process_instance_id.as_deref() == Some(pi_id)
                        && e.id != execution.id
                        && e.parent_id == execution.parent_id
                        && !e.is_ended
                        // Exclude non-interrupting event subprocess path executions
                        && !e.non_interrupting_event_subprocess_path
                        && !e
                            .process_variable(NON_INTERRUPTING_EVENT_SUBPROCESS_PATH_VARIABLE)
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false)
                })
                .collect();

            if active_siblings.is_empty() {
                // Determine if we are in a SubProcess or other scope
                let mut is_subprocess_scope = false;
                if let Some(parent_id) = &execution.parent_id {
                    let parent = command_context
                        .runtime_store
                        .find_execution(parent_id, &mut command_context.session);
                    if let Some(mut p) = parent
                        && p.is_scope
                        && p.activity_id.is_some()
                    {
                        // Java `EndExecutionOperation#handleMultiInstanceSubProcess`:
                        // when the SubProcess scope's parent is the dedicated MI
                        // root, leave goes through sequential/parallel MI leave
                        // (DestroyScope + continue or cleanupMiRoot), not the
                        // SubProcess's own outgoing sequence flows.
                        let parent_of_scope_is_mi_root = p
                            .parent_id
                            .as_deref()
                            .and_then(|gp_id| {
                                command_context
                                    .runtime_store
                                    .find_execution(gp_id, &mut command_context.session)
                            })
                            .map(|gp| gp.is_multi_instance_root)
                            .unwrap_or(false);

                        if parent_of_scope_is_mi_root {
                            // Mark the ending child done; MI leave destroys the
                            // scope tree (including this execution if still present).
                            execution.is_ended = true;
                            command_context
                                .execution_entity_manager
                                .update(execution, &mut command_context.session);
                            if crate::bpmn::behavior::multi_instance_support::leave_sequential_subprocess_mi_instance(
                                &p,
                                command_context,
                            )? {
                                return Ok(());
                            }
                            // Fall through to normal SubProcess leave if MI leave
                            // could not resolve characteristics.
                        }

                        // Non-MI SubProcess scope leave: take the SubProcess
                        // activity's outgoing flows (embedded subprocess continue).
                        p.is_ended = true;
                        command_context
                            .execution_entity_manager
                            .update(&p, &mut command_context.session);
                        command_context
                            .runtime_store
                            .delete_event_subprocess_event_subscriptions_by_scope_execution_id(
                                &p.id,
                                &mut command_context.session,
                            );
                        command_context
                            .agenda
                            .plan_take_outgoing_sequence_flows_operation(p);
                        is_subprocess_scope = true;
                    }
                }

                if !is_subprocess_scope
                    && command_context
                        .runtime_store
                        .find_process_instance(pi_id, &mut command_context.session)
                        .is_some()
                {
                    end_process_instance_with_callback_outcome(
                        command_context,
                        pi_id,
                        CmmnProcessTaskCallbackOutcome::Completed,
                        None,
                        None,
                        // Regular completion path — Java `EndExecutionOperation`
                        // with forceSynchronous=false (completeAsync may defer).
                        false,
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Java `EndExecutionOperation.java:126-131`: execute the process-level
/// `end` execution listeners on the process instance execution (the root row,
/// id == process instance id). The row is marked ended but still present and
/// carries the final process variables; listener writes are persisted so they
/// survive the end-of-command flush.
fn fire_process_end_listeners(
    command_context: &mut CommandContext,
    process_instance_id: &str,
) -> Result<(), crate::error::FlowableError> {
    let Some(mut root_execution) = command_context
        .runtime_store
        .find_execution(process_instance_id, &mut command_context.session)
    else {
        return Ok(());
    };
    let Some(pd_id) = root_execution.process_definition_id.clone() else {
        return Ok(());
    };
    let Some(model) = command_context.deployment_manager.get_bpmn_model(&pd_id) else {
        return Ok(());
    };
    let Some(main_process) = model.main_process.as_ref() else {
        return Ok(());
    };
    if main_process.execution_listeners.is_empty() {
        return Ok(());
    }
    let listeners: Vec<_> = main_process.execution_listeners.clone();
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, &root_execution);
    crate::bpmn::listener::execute_execution_listeners(
        &mut root_execution,
        command_context,
        &listeners,
        "end",
        &evaluation_execution,
    )?;
    // Persist any process variables written by the process end listener.
    command_context
        .execution_entity_manager
        .update(&root_execution, &mut command_context.session);
    Ok(())
}

/// Which process-completed typed event to emit when the PI ends successfully.
/// Java terminate end fires `PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT`
/// (`TerminateEndEventActivityBehavior.java:247-248`) instead of plain
/// `PROCESS_COMPLETED` (`ExecutionEntityManagerImpl.java:642`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessCompletedEventKind {
    /// Plain `PROCESS_COMPLETED`.
    Completed,
    /// `PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT`.
    WithTerminateEnd,
}

fn end_process_instance_with_callback_outcome(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    outcome: CmmnProcessTaskCallbackOutcome,
    failure_message: Option<&str>,
    delete_reason: Option<&str>,
    force_synchronous: bool,
) -> Result<(), crate::error::FlowableError> {
    end_process_instance_with_callback_outcome_and_event(
        command_context,
        process_instance_id,
        outcome,
        failure_message,
        delete_reason,
        force_synchronous,
        ProcessCompletedEventKind::Completed,
    )
}

fn end_process_instance_with_callback_outcome_and_event(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    outcome: CmmnProcessTaskCallbackOutcome,
    failure_message: Option<&str>,
    delete_reason: Option<&str>,
    force_synchronous: bool,
    completed_event: ProcessCompletedEventKind,
) -> Result<(), crate::error::FlowableError> {
    let Some(mut pi) = command_context
        .runtime_store
        .find_process_instance(process_instance_id, &mut command_context.session)
    else {
        return Ok(());
    };
    if pi.is_ended {
        return Ok(());
    }
    // Java `EndExecutionOperation.handleProcessInstanceExecution` (:94-96):
    // `!forceSynchronous && isAsyncCompleteCallActivity(superExecution)` — the
    // whole end operation is deferred to an exclusive async job on the *parent*
    // execution; the child PI stays alive until the job runs.
    if !force_synchronous
        && let Some(super_exec_id) = pi.super_execution_id.as_deref()
        && let Some(super_exec) = command_context
            .runtime_store
            .find_execution(super_exec_id, &mut command_context.session)
        && is_async_complete_call_activity(command_context, &super_exec)
    {
        schedule_async_complete_call_activity(command_context, &super_exec, &pi);
        return Ok(());
    }
    pi.is_ended = true;
    command_context
        .runtime_store
        .update_process_instance(&pi, &mut command_context.session);
    // P53 layer 1: dispatch `PROCESS_COMPLETED` once the PI row is marked
    // ended. Java emits `PROCESS_COMPLETED` (or the
    // `PROCESS_COMPLETED_WITH_ERROR_END_EVENT` variant for error end) at the
    // same logical moment in `ProcessInstanceHelper.endProcessInstance`.
    // The escalation variant is already routed by the existing
    // `ProcessCompletedWithEscalationEndEvent` call site — we add the plain
    // success path here.
    // P125: terminate end uses `PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT`
    // instead (Java `TerminateEndEventActivityBehavior.java:247-248`).
    if matches!(outcome, CmmnProcessTaskCallbackOutcome::Completed) {
        match completed_event {
            ProcessCompletedEventKind::Completed => {
                crate::engine::event_dispatcher::dispatch_process_instance_completed(
                    command_context,
                    &pi.id,
                    &pi.process_definition_id,
                );
            }
            ProcessCompletedEventKind::WithTerminateEnd => {
                crate::engine::event_dispatcher::dispatch_process_completed_with_terminate_end_event(
                    command_context,
                    &pi.id,
                    &pi.process_definition_id,
                );
            }
        }
    }
    command_context
        .runtime_store
        .delete_event_subprocess_event_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );
    command_context
        .runtime_store
        .delete_boundary_event_states_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );
    command_context
        .runtime_store
        .delete_timer_job_states_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );
    command_context
        .runtime_store
        .delete_event_subprocess_timer_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );

    let audit_outcome = match &outcome {
        CmmnProcessTaskCallbackOutcome::Completed => "complete",
        CmmnProcessTaskCallbackOutcome::Failed { .. } => "failed",
    };
    let audit_message = failure_message
        .map(|message| message.to_string())
        .unwrap_or_else(|| format!("process instance ended with outcome '{audit_outcome}'"));

    let recorded_reason = if matches!(outcome, CmmnProcessTaskCallbackOutcome::Failed { .. }) {
        failure_message
    } else {
        // Terminate end events record their delete reason on the completed
        // historic PI (Java `recordProcessInstanceEnd(..., deleteReason, ...)`).
        delete_reason
    };
    command_context.history_manager.record_process_instance_end(
        process_instance_id,
        recorded_reason,
        &mut command_context.session,
    );

    command_context.history_manager.record_audit_event(
        "process-instance-end",
        Some(process_instance_id),
        Some(&pi.process_definition_id),
        Some(&audit_message),
        &mut command_context.session,
    );

    // Java `EndExecutionOperation.java:126-131`: execute the process-level
    // execution listeners for `end` when the process instance ends. Fired on
    // the process instance execution (root row, id == pi id) so the listener
    // sees the final process variables.
    fire_process_end_listeners(command_context, process_instance_id)?;

    let callback_outcome = if let CmmnProcessTaskCallbackOutcome::Failed { .. } = &outcome {
        match failure_message {
            Some(message) => CmmnProcessTaskCallbackOutcome::Failed {
                failure_message: message.to_string(),
            },
            None => CmmnProcessTaskCallbackOutcome::Failed {
                failure_message: audit_message.clone(),
            },
        }
    } else {
        outcome
    };

    notify_cmmn_process_task_callback_for_instance(command_context, &pi, callback_outcome)?;

    if let Some(super_exec_id) = pi.super_execution_id.as_deref()
        && let Some(super_exec) = command_context
            .runtime_store
            .find_execution(super_exec_id, &mut command_context.session)
    {
        let mut super_exec = super_exec;
        // Java CallActivityBehavior#completed (:279-285): refuse when parent
        // execution / definition is suspended. completeAsync deferral happened
        // above (P47); by the time we get here the end is synchronous.
        crate::bpmn::behavior::call_activity_behavior::ensure_call_activity_parent_not_suspended(
            command_context,
            &super_exec,
        )?;
        apply_call_activity_out_parameters(command_context, &pi, &mut super_exec)?;
        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(super_exec);
    }
    Ok(())
}

/// Java `EndExecutionOperation.isAsyncCompleteCallActivity` (:149-157): the
/// super execution's current flow element is a `CallActivity` with
/// `completeAsync="true"`.
fn is_async_complete_call_activity(
    command_context: &mut CommandContext,
    super_exec: &Execution,
) -> bool {
    let Some(activity_id) = super_exec.activity_id.as_deref() else {
        return false;
    };
    let Some(pd_id) = super_exec.process_definition_id.as_deref() else {
        return false;
    };
    let Some(model) = command_context.deployment_manager.get_bpmn_model(pd_id) else {
        return false;
    };
    let Some(process) = model.main_process.as_ref() else {
        return false;
    };
    matches!(
        process.flow_element_map.get(activity_id),
        Some(FlowElementEnum::CallActivity(call_activity)) if call_activity.complete_async
    )
}

/// Java `EndExecutionOperation.scheduleAsyncCompleteCallActivity` (:159-180):
/// an async job on the *parent* execution (locks the parent PI so multiple
/// call activities cannot complete concurrently), carrying the child process
/// instance execution id as configuration. Java creates it always-exclusive
/// (`createAsyncJob(job, true)`).
fn schedule_async_complete_call_activity(
    command_context: &mut CommandContext,
    super_exec: &Execution,
    child_pi: &ProcessInstance,
) {
    use crate::persistence::runtime_store::{
        RuntimeTimerJobState, job_handler_types, stamp_new_job_metadata,
    };
    // Java :170: parent process instance id (super execution's PI).
    let parent_process_instance_id = super_exec
        .process_instance_id
        .clone()
        .unwrap_or_else(|| super_exec.id.clone());
    let store = command_context.runtime_store.clone();
    let now = store.time_source().now().timestamp_millis();
    let mut job = RuntimeTimerJobState {
        timer_job_id: Uuid::new_v4().to_string(),
        process_instance_id: parent_process_instance_id,
        // Java :164-165: the parent execution, used for (exclusive) locking.
        execution_id: super_exec.id.clone(),
        activity_id: super_exec.activity_id.clone().unwrap_or_default(),
        // Same async job family as continuations: acquired by
        // `acquire_async_jobs`, executed via `ExecuteTimerWorkCmd`.
        job_state: Some("async".to_string()),
        due_time: Some(now),
        retries: Some(
            command_context
                .config
                .async_executor
                .number_of_retries
                .max(0),
        ),
        // Java :167-168: child process instance execution id as configuration.
        job_handler_configuration: Some(child_pi.id.clone()),
        // Java EndExecutionOperation.java:178: createAsyncJob(job, true)
        // "Always exclusive to avoid concurrency problems".
        exclusive: true,
        ..Default::default()
    };
    stamp_new_job_metadata(
        &mut job,
        now,
        job_handler_types::ASYNC_COMPLETE_CALL_ACTIVITY,
        // Java :175: tenant of the child process instance.
        child_pi.tenant_id.clone(),
        // Java :172: process definition of the *child* process instance.
        Some(child_pi.process_definition_id.clone()),
        super_exec.activity_name.clone(),
    );
    store.insert_timer_job_state(&job, &mut command_context.session);
}

/// Java `AsyncCompleteCallActivityJobHandler#execute` (:44-47): resolve the
/// child process instance from the job configuration and replay the end
/// operation synchronously (`planEndExecutionOperationSynchronous` →
/// `EndExecutionOperation` with forceSynchronous=true → out-parameter copy +
/// parent continuation). Dispatched from `ExecuteTimerWorkCmd`.
pub(crate) fn execute_async_complete_call_activity_job(
    command_context: &mut CommandContext,
    job: &crate::persistence::runtime_store::RuntimeTimerJobState,
) -> Result<(), crate::error::FlowableError> {
    let child_pi_id = job
        .job_handler_configuration
        .as_deref()
        .unwrap_or_default()
        .to_string();
    if command_context
        .runtime_store
        .find_process_instance(&child_pi_id, &mut command_context.session)
        .is_none()
    {
        // ExecutionError (not NotFound) so REST job execute maps to 500 like
        // Java when the referenced entity is gone.
        return Err(crate::error::FlowableError::ExecutionError(format!(
            "Child process instance '{}' for async-complete-call-activity job '{}' not found",
            child_pi_id, job.timer_job_id
        )));
    }

    let store = command_context.runtime_store.clone();
    store.delete_timer_job_state(&job.timer_job_id, &mut command_context.session);

    end_process_instance_with_callback_outcome(
        command_context,
        &child_pi_id,
        CmmnProcessTaskCallbackOutcome::Completed,
        None,
        None,
        // Java `planEndExecutionOperationSynchronous`: forceSynchronous=true.
        true,
    )
}

// ─── Terminate end event (Java `TerminateEndEventActivityBehavior`) ───

/// Java `DeleteReason.TERMINATE_END_EVENT` ("terminate end event") combined
/// with `DeleteReason.createDeleteReason` → `"terminate end event (<id>)"`.
fn terminate_delete_reason(activity_id: &str) -> String {
    format!("terminate end event ({activity_id})")
}

/// Java `TerminateEndEventActivityBehavior#execute` (60-207): the current
/// execution is always deleted first (`deleteExecutionAndRelatedData`), then
/// the behavior branches on `terminateAll` / `terminateMultiInstance` /
/// default scope termination.
fn execute_terminate_end_event(
    execution: &mut Execution,
    command_context: &mut CommandContext,
    terminate_all: bool,
    terminate_multi_instance: bool,
) -> Result<(), crate::error::FlowableError> {
    let snapshot = execution.clone();
    let activity_id = snapshot.activity_id.clone().unwrap_or_default();
    let delete_reason = terminate_delete_reason(&activity_id);

    command_context.history_manager.record_activity_end(
        &snapshot.id,
        &activity_id,
        Some(&delete_reason),
        &mut command_context.session,
    );
    crate::bpmn::behavior::multi_instance_support::delete_execution_tree(
        command_context,
        &snapshot.id,
    );

    if terminate_all {
        terminate_all_behaviour(&snapshot, command_context, &delete_reason)
    } else if terminate_multi_instance {
        terminate_multi_instance_root(&snapshot, command_context, &delete_reason)
    } else {
        default_terminate_end_event_behaviour(&snapshot, command_context, &delete_reason)
    }
}

/// Java `terminateAllBehaviour` (95-108): walk up to the root process
/// instance (across the call activity chain) and delete the whole tree.
fn terminate_all_behaviour(
    execution: &Execution,
    command_context: &mut CommandContext,
    delete_reason: &str,
) -> Result<(), crate::error::FlowableError> {
    let Some(pi_id) = execution.process_instance_id.as_deref() else {
        return Ok(());
    };
    let root_id = find_root_process_instance_id(command_context, pi_id);

    let all_instances: Vec<ProcessInstance> = command_context
        .runtime_store
        .snapshot_process_instances(&mut command_context.session)
        .into_values()
        .collect();
    let member_ids: Vec<String> = all_instances
        .iter()
        .filter(|pi| find_root_process_instance_id(command_context, &pi.id) == root_id)
        .map(|pi| pi.id.clone())
        .collect();

    // Java `deleteExecutionEntities(..., root, ...)`: child instances end with
    // the terminate delete reason; only the root records the callback outcome.
    for member_id in &member_ids {
        if member_id == &root_id {
            continue;
        }
        if let Some(mut pi) = command_context
            .runtime_store
            .find_process_instance(member_id, &mut command_context.session)
            && !pi.is_ended
        {
            pi.is_ended = true;
            command_context
                .runtime_store
                .update_process_instance(&pi, &mut command_context.session);
            command_context.history_manager.record_process_instance_end(
                member_id,
                Some(delete_reason),
                &mut command_context.session,
            );
        }
    }
    end_process_instance_with_callback_outcome_and_event(
        command_context,
        &root_id,
        CmmnProcessTaskCallbackOutcome::Completed,
        None,
        Some(delete_reason),
        // Java terminate deletes bypass `EndExecutionOperation` — never deferred.
        true,
        // P125: Java TerminateEndEventActivityBehavior.sendProcessInstanceCompletedEvent
        // (226-248) → PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT.
        ProcessCompletedEventKind::WithTerminateEnd,
    )?;
    for member_id in &member_ids {
        delete_process_instance_runtime(command_context, member_id, delete_reason);
    }
    Ok(())
}

/// Java `terminateMultiInstanceRoot` (188-207): create a sibling execution
/// under the MI root's parent to take the MI activity's outgoing flows, then
/// delete the whole MI root subtree. Without an MI root the default behaviour
/// applies.
fn terminate_multi_instance_root(
    execution: &Execution,
    command_context: &mut CommandContext,
    delete_reason: &str,
) -> Result<(), crate::error::FlowableError> {
    let Some(mi_root) = crate::bpmn::behavior::multi_instance_support::resolve_multi_instance_root(
        command_context,
        execution,
    ) else {
        return default_terminate_end_event_behaviour(execution, command_context, delete_reason);
    };

    let mut sibling = mi_root.clone();
    sibling.id = Uuid::new_v4().to_string();
    sibling.parent_id = mi_root.parent_id.clone();
    sibling.is_active = true;
    sibling.is_ended = false;
    sibling.is_scope = false;
    sibling.is_multi_instance_root = false;
    sibling.variables.clear();
    sibling.local_variables.clear();
    sibling.transient_variables.clear();

    crate::bpmn::behavior::multi_instance_support::delete_execution_tree(
        command_context,
        &mi_root.id,
    );
    command_context
        .execution_entity_manager
        .insert(&sibling, &mut command_context.session);
    command_context
        .agenda
        .plan_take_outgoing_sequence_flows_operation(sibling);
    Ok(())
}

/// Java `defaultTerminateEndEventBehaviour` (110-142): `findFirstScope`, then
/// terminate either the whole process instance, an embedded SubProcess scope
/// (continuing along its outgoing flows), or a call activity child instance
/// (continuing the parent process).
fn default_terminate_end_event_behaviour(
    execution: &Execution,
    command_context: &mut CommandContext,
    delete_reason: &str,
) -> Result<(), crate::error::FlowableError> {
    let scope = find_first_scope(command_context, execution);

    if let Some(scope) = scope
        && scope_is_sub_process_element(command_context, &scope)
    {
        // Embedded SubProcess scope (Java `scopeElement instanceof SubProcess`).
        // MI SubProcess instances go through the MI leave (Java
        // `MultiInstanceActivityBehavior.leave`, only one instance
        // terminates); otherwise destroy the scope contents and take the
        // SubProcess's outgoing flows.
        if crate::bpmn::behavior::multi_instance_support::leave_sequential_subprocess_mi_instance(
            &scope,
            command_context,
        )? {
            return Ok(());
        }

        // Java `destroyScope`: all child executions (inner tokens, tasks,
        // wait states) of the scope are deleted. The scope row itself is
        // kept: this engine reuses it as the token that takes the
        // SubProcess's outgoing flows (same idiom as the normal SubProcess
        // leave in `EndEventActivityBehavior::execute`), which also covers
        // the case where the scope row doubles as the PI scope execution.
        let child_ids: Vec<String> = command_context
            .execution_entity_manager
            .find_child_executions_by_parent_execution_id(&scope.id, &mut command_context.session)
            .into_iter()
            .map(|child| child.id)
            .collect();
        for child_id in child_ids {
            crate::bpmn::behavior::multi_instance_support::delete_execution_tree(
                command_context,
                &child_id,
            );
        }
        command_context
            .runtime_store
            .delete_event_subprocess_event_subscriptions_by_scope_execution_id(
                &scope.id,
                &mut command_context.session,
            );
        command_context.history_manager.record_activity_end(
            &scope.id,
            scope.activity_id.as_deref().unwrap_or(""),
            Some(delete_reason),
            &mut command_context.session,
        );

        let mut scope_exec = scope;
        scope_exec.is_ended = true;
        command_context
            .execution_entity_manager
            .update(&scope_exec, &mut command_context.session);
        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(scope_exec);
        return Ok(());
    }

    // Scope is the process instance itself: a top-level PI terminates as
    // COMPLETED with the delete reason; a call activity child PI additionally
    // continues the parent via the super execution (both are handled by
    // `end_process_instance_with_callback_outcome`).
    let Some(pi_id) = execution.process_instance_id.as_deref() else {
        return Ok(());
    };
    let Some(pi) = command_context
        .runtime_store
        .find_process_instance(pi_id, &mut command_context.session)
    else {
        return Ok(());
    };
    // Java `deleteProcessInstanceExecutionEntity` for the call activity child
    // uses the bare `DeleteReason.TERMINATE_END_EVENT` without activity id.
    let reason = if pi.super_execution_id.is_some() {
        "terminate end event".to_string()
    } else {
        delete_reason.to_string()
    };
    end_process_instance_with_callback_outcome_and_event(
        command_context,
        pi_id,
        CmmnProcessTaskCallbackOutcome::Completed,
        None,
        Some(&reason),
        // Java terminate deletes bypass `EndExecutionOperation` — never deferred.
        true,
        // P125: Java TerminateEndEventActivityBehavior.sendProcessInstanceCompletedEvent
        // (226-248) → PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT.
        ProcessCompletedEventKind::WithTerminateEnd,
    )?;
    delete_process_instance_runtime(command_context, pi_id, &reason);
    Ok(())
}

/// Java `findFirstScope` (`TerminateEndEventActivityBehavior#execute` 66-78):
/// walk the parent chain to the first scope execution.
fn find_first_scope(
    command_context: &mut CommandContext,
    execution: &Execution,
) -> Option<Execution> {
    let mut current_parent = execution.parent_id.clone();
    let mut guard = 0;
    while let Some(parent_id) = current_parent {
        guard += 1;
        if guard > 64 {
            return None;
        }
        let parent = command_context
            .runtime_store
            .find_execution(&parent_id, &mut command_context.session)?;
        if parent.is_scope {
            return Some(parent);
        }
        current_parent = parent.parent_id.clone();
    }
    None
}

/// Java `defaultTerminateEndEventBehaviour` (117): `scopeElement instanceof
/// SubProcess` — resolve the scope's activity against the model. Note the
/// scope row may simultaneously be the PI scope execution in this engine
/// (single-token walk reuses the PI row), so the model element is the only
/// reliable discriminator.
fn scope_is_sub_process_element(command_context: &mut CommandContext, scope: &Execution) -> bool {
    let Some(activity_id) = scope.activity_id.as_deref() else {
        return false;
    };
    let Some(pd_id) = scope.process_definition_id.as_deref() else {
        return false;
    };
    let Some(model) = command_context.deployment_manager.get_bpmn_model(pd_id) else {
        return false;
    };
    let Some(process) = model.main_process.as_ref() else {
        return false;
    };
    matches!(
        process.flow_element_map.get(activity_id),
        Some(FlowElementEnum::SubProcess(_))
            | Some(FlowElementEnum::Transaction(_))
            | Some(FlowElementEnum::EventSubProcess(_))
            | Some(FlowElementEnum::AdhocSubProcess(_))
    )
}

fn parent_process_instance_id(
    command_context: &mut CommandContext,
    pi: &ProcessInstance,
) -> Option<String> {
    let super_exec_id = pi.super_execution_id.as_deref()?;
    let super_exec = command_context
        .runtime_store
        .find_execution(super_exec_id, &mut command_context.session)?;
    super_exec.process_instance_id
}

/// Java `ExecutionEntityImpl#getRootProcessInstanceId`: follow the super
/// execution chain across call activities up to the top-level instance.
fn find_root_process_instance_id(command_context: &mut CommandContext, pi_id: &str) -> String {
    let mut current = pi_id.to_string();
    let mut guard = 0;
    while guard < 64 {
        guard += 1;
        let Some(pi) = command_context
            .runtime_store
            .find_process_instance(&current, &mut command_context.session)
        else {
            break;
        };
        match parent_process_instance_id(command_context, &pi) {
            Some(parent_id) => current = parent_id,
            None => break,
        }
    }
    current
}

/// Runtime cleanup for a terminated process instance, mirroring the entity
/// deletes of Java `deleteProcessInstanceExecutionEntity` (tasks end with the
/// terminate delete reason, all executions and runtime states are removed;
/// the PI row itself stays with `is_ended=true` like a normal completion).
fn delete_process_instance_runtime(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    delete_reason: &str,
) {
    let tasks = command_context
        .task_entity_manager
        .find_by_process_instance_id(process_instance_id, &mut command_context.session);
    for task in tasks {
        command_context.history_manager.record_task_end(
            &task.id,
            Some(delete_reason),
            &mut command_context.session,
        );
        command_context
            .task_entity_manager
            .delete(&task.id, &mut command_context.session);
    }

    // P125: ACTIVITY_MESSAGE_CANCELLED for message wait states removed when a
    // process instance is torn down without walking delete_execution_and_related_data
    // (terminate end / bulk PI cleanup). Java fires from
    // ExecutionEntityManagerImpl.deleteEventSubScriptions (1063-1066) per
    // execution; this bulk path is the Rust equivalent for whole-PI teardown.
    dispatch_message_cancelled_for_process_instance(command_context, process_instance_id);

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
}

/// Fire `ACTIVITY_MESSAGE_CANCELLED` for every message subscription still
/// present on a process instance (wait states + boundary states).
fn dispatch_message_cancelled_for_process_instance(
    command_context: &mut CommandContext,
    process_instance_id: &str,
) {
    use crate::persistence::runtime_store::EventSubscriptionKind;

    let waits = command_context
        .runtime_store
        .find_event_wait_states_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );
    for wait in waits {
        let Some(sub) = wait.event_subscription.as_ref() else {
            continue;
        };
        if sub.kind != EventSubscriptionKind::Message {
            continue;
        }
        let activity_id = wait.activity_id.as_deref().unwrap_or("");
        let pd = command_context
            .runtime_store
            .find_execution(&wait.execution_id, &mut command_context.session)
            .and_then(|e| e.process_definition_id);
        crate::engine::event_dispatcher::dispatch_activity_message_cancelled(
            command_context,
            activity_id,
            &sub.event_ref,
            Some(process_instance_id),
            Some(&wait.execution_id),
            pd.as_deref(),
        );
    }

    let boundaries = command_context
        .runtime_store
        .find_boundary_event_states_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );
    for boundary in boundaries {
        if boundary.event_subscription.kind != EventSubscriptionKind::Message {
            continue;
        }
        let pd = command_context
            .runtime_store
            .find_execution(&boundary.host_execution_id, &mut command_context.session)
            .and_then(|e| e.process_definition_id);
        crate::engine::event_dispatcher::dispatch_activity_message_cancelled(
            command_context,
            &boundary.boundary_event_id,
            &boundary.event_subscription.event_ref,
            Some(process_instance_id),
            Some(&boundary.host_execution_id),
            pd.as_deref(),
        );
    }
}
