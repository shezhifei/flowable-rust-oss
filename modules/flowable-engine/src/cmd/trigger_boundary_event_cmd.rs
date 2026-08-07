use crate::agenda::FlowableEngineAgenda;

use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventSubscriptionKind, RuntimeBoundaryEventState, RuntimeTimerJobState,
};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::BpmnModel;
use std::collections::HashSet;
use uuid::Uuid;

fn descendant_execution_ids(
    command_context: &mut CommandContext,
    root_execution_id: &str,
) -> Vec<String> {
    let executions = command_context
        .runtime_store
        .snapshot_executions(&mut command_context.session);
    let mut descendants = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root_execution_id.to_string()];

    while let Some(parent_id) = stack.pop() {
        for execution in executions.values() {
            if execution.parent_id.as_deref() == Some(parent_id.as_str())
                && seen.insert(execution.id.clone())
            {
                descendants.push(execution.id.clone());
                stack.push(execution.id.clone());
            }
        }
    }

    descendants
}

fn delete_execution_runtime_state(command_context: &mut CommandContext, execution_id: &str) {
    strip_execution_runtime_state(command_context, execution_id);

    command_context
        .execution_entity_manager
        .delete(execution_id, &mut command_context.session);
}

/// Removes the runtime state attached to an execution (task, wait state,
/// boundary registrations, jobs, event-subprocess subscriptions) without
/// deleting the execution row itself. Used when the interrupted host row is
/// the process-instance scope row: Java `deleteChildExecutions` never deletes
/// the process instance execution, and in this engine's collapsed single-token
/// topology that row is the process-level variable store.
fn strip_execution_runtime_state(command_context: &mut CommandContext, execution_id: &str) {
    if let Some(task) = command_context
        .task_entity_manager
        .find_by_execution_id(execution_id, &mut command_context.session)
    {
        command_context
            .task_entity_manager
            .delete(&task.id, &mut command_context.session);
    }

    command_context
        .runtime_store
        .delete_event_wait_state_by_execution_id(execution_id, &mut command_context.session);

    command_context
        .runtime_store
        .delete_boundary_event_states_by_host_execution_id(
            execution_id,
            &mut command_context.session,
        );

    command_context
        .runtime_store
        .delete_timer_job_states_by_execution_id(execution_id, &mut command_context.session);

    command_context
        .runtime_store
        .delete_event_subprocess_event_subscriptions_by_scope_execution_id(
            execution_id,
            &mut command_context.session,
        );
}

/// Interrupting-boundary variant of `delete_execution_runtime_state` for the
/// case where the host row doubles as the process-instance scope row
/// (single-token topology). Java `deleteChildExecutions` never deletes the
/// process instance execution, and this row is the process-level variable
/// store — so strip its runtime state and detach it from the interrupted
/// activity instead of deleting it (same retained-scope-row idiom as
/// `EndEventActivityBehavior`).
fn retire_process_scope_row_after_interrupt(
    command_context: &mut CommandContext,
    execution_id: &str,
) {
    strip_execution_runtime_state(command_context, execution_id);
    if let Some(mut row) = command_context
        .runtime_store
        .find_execution(execution_id, &mut command_context.session)
    {
        row.activity_id = None;
        row.is_active = false;
        row.is_ended = true;
        command_context
            .execution_entity_manager
            .update(&row, &mut command_context.session);
    }
}

/// Core boundary event trigger logic shared by all trigger entry points.
/// This eliminates the massive code duplication that previously existed across
/// `TriggerBoundaryEventCmd`, `TriggerBoundaryEventByMessageRefCmd`, and
/// `TriggerBoundaryEventBySignalRefCmd`.
fn execute_boundary_trigger(
    command_context: &mut CommandContext,
    boundary_state: RuntimeBoundaryEventState,
    process_instance_id: &str,
) -> Result<(), crate::error::FlowableError> {
    let boundary_event_id = boundary_state.boundary_event_id.clone();

    // Find the host execution (the execution of the activity the boundary event is attached to)
    let host_execution = match command_context.execution_entity_manager.find_by_id(
        &boundary_state.host_execution_id,
        &mut command_context.session,
    ) {
        Some(exec) => exec.clone(),
        None => {
            tracing::error!(
                "Host execution {} not found for boundary event {}",
                boundary_state.host_execution_id,
                boundary_event_id
            );
            return Ok(());
        }
    };

    // Check if the host execution is in a wait state (not active) or is a scope execution (e.g. SubProcess)
    if host_execution.is_active && !host_execution.is_scope {
        tracing::warn!(
            "Host execution {} is still active, cannot trigger boundary event {}",
            boundary_state.host_execution_id,
            boundary_event_id
        );
        return Ok(());
    }

    // For interrupting boundaries: clean up host execution, task, wait-state, and boundary registrations.
    if boundary_state.cancel_activity {
        // Java `BoundaryEventActivityBehavior#deleteChildExecutions` 157-164:
        // `DeleteReason.BOUNDARY_EVENT_INTERRUPTING + " (" + boundaryActivityId + ")"`
        // applied via `deleteExecutionAndRelatedData` to the host and its children.
        let delete_reason =
            crate::history::delete_reason::boundary_event_interrupting(&boundary_event_id);

        let mut execution_ids_to_delete =
            descendant_execution_ids(command_context, &boundary_state.host_execution_id);
        execution_ids_to_delete.push(boundary_state.host_execution_id.clone());

        for execution_id in execution_ids_to_delete {
            crate::bpmn::behavior::multi_instance_support::record_activity_end_for_execution(
                command_context,
                &execution_id,
                Some(&delete_reason),
            );
            if execution_id == process_instance_id {
                // Java parity (`BoundaryEventActivityBehavior#executeInterruptingBehavior`
                // → `deleteChildExecutions`): the process instance execution is
                // never deleted when an interrupting boundary cancels its host.
                retire_process_scope_row_after_interrupt(command_context, &execution_id);
            } else {
                delete_execution_runtime_state(command_context, &execution_id);
            }
        }

        // Java parity (`BoundaryEventActivityBehavior.java:63-112,157-164`): an
        // interrupting boundary only deletes the host's child execution subtree.
        // Process-instance-level event-subprocess timer subscriptions stay alive
        // until the process instance itself ends (fault/end-event/delete paths).
    } else {
        // Non-interrupting (cancelActivity=false):
        // Message/signal/conditional/escalation: keep subscription (Java repeat —
        // BoundaryEventActivityBehavior#executeNonInterruptingBehavior never
        // deletes the waiting execution; BoundaryConditionalEventTest
        // testCatchNonInterruptingConditionalOnEmbeddedSubprocess fires twice;
        // EscalationPropagation re-finds the boundary child by activityId and
        // re-triggers — BoundaryEscalationEventActivityBehavior inherits the
        // same non-interrupting path).
        // Removed only when the host ends via delete_execution_related_runtime.
        // Other kinds (error/cancel/compensate): still consume (error is always
        // interrupting in Java parse; cancel/compensate are one-shot by model).
        match boundary_state.event_subscription.kind {
            EventSubscriptionKind::Message
            | EventSubscriptionKind::Signal
            | EventSubscriptionKind::Conditional
            | EventSubscriptionKind::Escalation
            // Java BoundaryEventRegistryEventActivityBehavior non-interrupting path
            // keeps the subscription for repeat fires.
            | EventSubscriptionKind::EventRegistry => {}
            _ => {
                command_context.runtime_store.delete_boundary_event_state(
                    &boundary_event_id,
                    process_instance_id,
                    &mut command_context.session,
                );
            }
        }
    }

    // Seed from the process-instance scope execution row: it is the single
    // process-level variable store.
    let process_variables = command_context
        .runtime_store
        .find_execution(process_instance_id, &mut command_context.session)
        .map(|root_execution| root_execution.variables)
        .unwrap_or_default();

    let boundary_execution = Execution {
        id: Uuid::new_v4().to_string(),
        parent_id: host_execution.parent_id.clone(),
        process_instance_id: Some(process_instance_id.to_string()),
        process_definition_id: host_execution.process_definition_id.clone(),
        activity_id: Some(boundary_event_id.clone()),
        is_active: true,
        is_concurrent: host_execution.is_concurrent,
        is_ended: false,
        is_scope: host_execution.is_scope,
        is_multi_instance_root: false,
        variables: process_variables,
        ..Default::default()
    };

    command_context
        .execution_entity_manager
        .insert(&boundary_execution, &mut command_context.session);

    // Plan the boundary event execution
    command_context
        .agenda
        .plan_continue_process_operation(boundary_execution.clone());

    // Plan taking outgoing sequence flows from the boundary event
    command_context
        .agenda
        .plan_take_outgoing_sequence_flows_operation(boundary_execution);

    Ok(())
}

fn normalize_escalation_ref(model: Option<&BpmnModel>, event_ref: &str) -> String {
    if let Some(model) = model
        && let Some(escalation) = model
            .escalations
            .iter()
            .find(|escalation| escalation.base_element.id.as_deref() == Some(event_ref))
    {
        return escalation
            .escalation_code
            .clone()
            .unwrap_or_else(|| event_ref.to_string());
    }

    event_ref.to_string()
}

/// Returns true if the trigger's signal ref refers to the same global signal as
/// the subscription's stored event ref, comparing the raw signalRef id, the
/// resolved global name, and any cross-id/name pair defined in the model.
fn signal_refs_match_for_boundary(
    command_context: &mut CommandContext,
    boundary_state: &RuntimeBoundaryEventState,
    trigger_ref: &str,
) -> bool {
    let subscription_ref = &boundary_state.event_subscription.event_ref;
    if subscription_ref == trigger_ref {
        return true;
    }

    let process_definition_id = command_context
        .runtime_store
        .find_execution(
            &boundary_state.host_execution_id,
            &mut command_context.session,
        )
        .and_then(|execution| execution.process_definition_id);

    let model = process_definition_id.as_deref().and_then(|definition_id| {
        command_context
            .deployment_manager
            .get_bpmn_model(definition_id)
    });

    signal_refs_match_in_model(model.as_deref(), subscription_ref, trigger_ref)
}

/// Returns true if both refs correspond to the same global signal definition
/// (id or name) within the supplied model. Direct equality is handled by the
/// caller, so this helper only evaluates the cross-id/name combinations.
fn signal_refs_match_in_model(
    model: Option<&BpmnModel>,
    subscription_ref: &str,
    trigger_ref: &str,
) -> bool {
    let Some(model) = model else {
        return false;
    };

    for signal in &model.signals {
        let id = signal.base_element.id.as_deref().unwrap_or("");
        let name = signal.name.as_deref().unwrap_or("");

        let has_id = !id.is_empty();
        let has_name = !name.is_empty();
        if !has_id && !has_name {
            continue;
        }

        let subscription_matches =
            (has_id && subscription_ref == id) || (has_name && subscription_ref == name);
        let trigger_matches = (has_id && trigger_ref == id) || (has_name && trigger_ref == name);

        if subscription_matches && trigger_matches {
            return true;
        }
    }

    false
}

fn escalation_refs_match(
    command_context: &mut CommandContext,
    boundary_state: &RuntimeBoundaryEventState,
    thrown_ref: &str,
) -> bool {
    let boundary_ref = &boundary_state.event_subscription.event_ref;
    if boundary_ref.is_empty() {
        return true;
    }

    if boundary_ref == thrown_ref {
        return true;
    }

    let process_definition_id = command_context
        .runtime_store
        .find_execution(
            &boundary_state.host_execution_id,
            &mut command_context.session,
        )
        .and_then(|execution| execution.process_definition_id);

    let model = process_definition_id.as_deref().and_then(|definition_id| {
        command_context
            .deployment_manager
            .get_bpmn_model(definition_id)
    });

    normalize_escalation_ref(model.as_deref(), boundary_ref)
        == normalize_escalation_ref(model.as_deref(), thrown_ref)
}

fn escalation_refs_match_exact(
    command_context: &mut CommandContext,
    boundary_state: &RuntimeBoundaryEventState,
    thrown_ref: &str,
) -> bool {
    let boundary_ref = &boundary_state.event_subscription.event_ref;
    if boundary_ref.is_empty() {
        return false;
    }

    if boundary_ref == thrown_ref {
        return true;
    }

    let process_definition_id = command_context
        .runtime_store
        .find_execution(
            &boundary_state.host_execution_id,
            &mut command_context.session,
        )
        .and_then(|execution| execution.process_definition_id);

    let model = process_definition_id.as_deref().and_then(|definition_id| {
        command_context
            .deployment_manager
            .get_bpmn_model(definition_id)
    });

    normalize_escalation_ref(model.as_deref(), boundary_ref)
        == normalize_escalation_ref(model.as_deref(), thrown_ref)
}

fn execution_ancestry(
    command_context: &mut CommandContext,
    source_execution_id: &str,
) -> Vec<String> {
    let mut ancestry = Vec::new();
    let mut current_id = Some(source_execution_id.to_string());

    for _ in 0..256 {
        let Some(execution_id) = current_id else {
            break;
        };
        let Some(execution) = command_context
            .runtime_store
            .find_execution(&execution_id, &mut command_context.session)
        else {
            break;
        };

        ancestry.push(execution.id.clone());
        current_id = execution.parent_id.clone();
    }

    ancestry
}

fn select_nearest_escalation_boundary(
    command_context: &mut CommandContext,
    mut matching_states: Vec<RuntimeBoundaryEventState>,
    source_execution_id: Option<&str>,
    thrown_ref: &str,
) -> Option<RuntimeBoundaryEventState> {
    if let Some(source_execution_id) = source_execution_id {
        let ancestry = execution_ancestry(command_context, source_execution_id);
        for execution_id in ancestry {
            if let Some(position) = matching_states.iter().position(|state| {
                state.host_execution_id == execution_id
                    && escalation_refs_match_exact(command_context, state, thrown_ref)
            }) {
                return Some(matching_states.remove(position));
            }

            if let Some(position) = matching_states
                .iter()
                .position(|state| state.host_execution_id == execution_id)
            {
                return Some(matching_states.remove(position));
            }
        }
    }

    if let Some(position) = matching_states
        .iter()
        .position(|state| escalation_refs_match_exact(command_context, state, thrown_ref))
    {
        return Some(matching_states.remove(position));
    }

    matching_states.into_iter().next()
}

fn normalize_error_ref(model: Option<&BpmnModel>, event_ref: &str) -> String {
    if let Some(model) = model
        && let Some(error_code) = model.errors.get(event_ref)
    {
        return error_code.clone();
    }

    event_ref.to_string()
}

fn error_model_for_boundary_state(
    command_context: &mut CommandContext,
    boundary_state: &RuntimeBoundaryEventState,
) -> Option<std::sync::Arc<BpmnModel>> {
    let process_definition_id = command_context
        .runtime_store
        .find_execution(
            &boundary_state.host_execution_id,
            &mut command_context.session,
        )
        .and_then(|execution| execution.process_definition_id)?;

    command_context
        .deployment_manager
        .get_bpmn_model(&process_definition_id)
}

fn error_refs_match(
    command_context: &mut CommandContext,
    boundary_state: &RuntimeBoundaryEventState,
    thrown_ref: &str,
) -> bool {
    let boundary_ref = &boundary_state.event_subscription.event_ref;
    if boundary_ref.is_empty() {
        return true;
    }

    if boundary_ref == thrown_ref {
        return true;
    }

    let model = error_model_for_boundary_state(command_context, boundary_state);
    let model = model.as_deref();

    normalize_error_ref(model, boundary_ref) == normalize_error_ref(model, thrown_ref)
}

fn error_refs_match_exact(
    command_context: &mut CommandContext,
    boundary_state: &RuntimeBoundaryEventState,
    thrown_ref: &str,
) -> bool {
    let boundary_ref = &boundary_state.event_subscription.event_ref;
    if boundary_ref.is_empty() {
        return false;
    }

    if boundary_ref == thrown_ref {
        return true;
    }

    let model = error_model_for_boundary_state(command_context, boundary_state);
    let model = model.as_deref();

    normalize_error_ref(model, boundary_ref) == normalize_error_ref(model, thrown_ref)
}

fn select_nearest_error_boundary(
    command_context: &mut CommandContext,
    mut matching_states: Vec<RuntimeBoundaryEventState>,
    source_execution_id: Option<&str>,
    thrown_ref: &str,
) -> Option<RuntimeBoundaryEventState> {
    if let Some(source_execution_id) = source_execution_id {
        let ancestry = execution_ancestry(command_context, source_execution_id);
        for execution_id in ancestry {
            if let Some(position) = matching_states.iter().position(|state| {
                state.host_execution_id == execution_id
                    && error_refs_match_exact(command_context, state, thrown_ref)
            }) {
                return Some(matching_states.remove(position));
            }

            if let Some(position) = matching_states.iter().position(|state| {
                state.host_execution_id == execution_id
                    && state.event_subscription.event_ref.is_empty()
            }) {
                return Some(matching_states.remove(position));
            }
        }
    }

    if let Some(position) = matching_states
        .iter()
        .position(|state| error_refs_match_exact(command_context, state, thrown_ref))
    {
        return Some(matching_states.remove(position));
    }

    matching_states.into_iter().next()
}

/// Triggers a boundary event by its exact boundary event ID.
pub struct TriggerBoundaryEventCmd {
    boundary_event_id: String,
    process_instance_id: String,
}

impl TriggerBoundaryEventCmd {
    pub fn new(boundary_event_id: String, process_instance_id: String) -> Self {
        Self {
            boundary_event_id,
            process_instance_id,
        }
    }
}

impl Command<()> for TriggerBoundaryEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let boundary_state = match command_context.runtime_store.find_boundary_event_state(
            &self.boundary_event_id,
            &self.process_instance_id,
            &mut command_context.session,
        ) {
            Some(state) => state,
            None => {
                tracing::warn!(
                    "No boundary event state found for boundary event id {}",
                    self.boundary_event_id
                );
                return Ok(());
            }
        };

        // Verify this boundary event belongs to the specified process instance
        if boundary_state.process_instance_id != self.process_instance_id {
            tracing::warn!(
                "Boundary event {} does not belong to process instance {} (belongs to {})",
                self.boundary_event_id,
                self.process_instance_id,
                boundary_state.process_instance_id
            );
            return Ok(());
        }

        // Java parity: BoundaryConditionalEventActivityBehavior.trigger
        // (flowable-engine .../BoundaryConditionalEventActivityBehavior.java:59-81)
        // re-evaluates ConditionUtil.hasTrueCondition before firing. When false:
        // silent no-op — no child execution, no outgoing, waiting state retained.
        // Other subscription kinds do not re-evaluate conditions.
        if boundary_state.event_subscription.kind == EventSubscriptionKind::Conditional {
            let host_execution = match command_context.execution_entity_manager.find_by_id(
                &boundary_state.host_execution_id,
                &mut command_context.session,
            ) {
                Some(exec) => exec.clone(),
                None => {
                    tracing::error!(
                        "Host execution {} not found for conditional boundary event {}",
                        boundary_state.host_execution_id,
                        self.boundary_event_id
                    );
                    return Ok(());
                }
            };

            // Same variable context as EvaluateConditionalEventsCmd boundary branch:
            // host execution + process-instance scope merge via evaluation_execution.
            let evaluation_execution = crate::engine::variable_service::evaluation_execution(
                command_context,
                &host_execution,
            );
            if !crate::engine::runtime_service::condition_is_true(
                &boundary_state.event_subscription.event_ref,
                &evaluation_execution,
            )? {
                tracing::debug!(
                    "Conditional boundary {} condition not satisfied; trigger is a no-op",
                    self.boundary_event_id
                );
                return Ok(());
            }
            // P125: ACTIVITY_CONDITIONAL_RECEIVED when condition evaluates true.
            // Java BoundaryConditionalEventActivityBehavior.java:70-72.
            crate::engine::event_dispatcher::dispatch_activity_conditional_received(
                command_context,
                &boundary_state.boundary_event_id,
                &boundary_state.event_subscription.event_ref,
                Some(&boundary_state.process_instance_id),
                Some(&boundary_state.host_execution_id),
                host_execution.process_definition_id.as_deref(),
            );
        }

        execute_boundary_trigger(command_context, boundary_state, &self.process_instance_id)
    }
}

/// Unified command that triggers a boundary event by event subscription
/// (kind + event_ref). Replaces the old separate `TriggerBoundaryEventByMessageRefCmd`
/// and `TriggerBoundaryEventBySignalRefCmd`.
pub struct TriggerBoundaryEventByEventRefCmd {
    subscription_kind: EventSubscriptionKind,
    event_ref: String,
    process_instance_id: String,
    source_execution_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BoundaryEventTriggerResult {
    pub(crate) triggered: bool,
    pub(crate) interrupting: bool,
}

impl TriggerBoundaryEventByEventRefCmd {
    pub fn new(
        subscription_kind: EventSubscriptionKind,
        event_ref: String,
        process_instance_id: String,
    ) -> Self {
        Self {
            subscription_kind,
            event_ref,
            process_instance_id,
            source_execution_id: None,
        }
    }

    pub fn with_source_execution(
        subscription_kind: EventSubscriptionKind,
        event_ref: String,
        process_instance_id: String,
        source_execution_id: String,
    ) -> Self {
        Self {
            subscription_kind,
            event_ref,
            process_instance_id,
            source_execution_id: Some(source_execution_id),
        }
    }

    pub(crate) fn execute_with_catch_result(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<BoundaryEventTriggerResult, crate::error::FlowableError> {
        let kind_label = match self.subscription_kind {
            EventSubscriptionKind::Message => "message",
            EventSubscriptionKind::Signal => "signal",
            EventSubscriptionKind::Conditional => "conditional",
            EventSubscriptionKind::Error => "error",
            EventSubscriptionKind::Cancel => "cancel",
            EventSubscriptionKind::Compensate => "compensate",
            EventSubscriptionKind::Escalation => "escalation",
            EventSubscriptionKind::EventRegistry => "event-registry",
        };

        // Find the boundary event state by event subscription kind + ref
        let boundary_states = command_context
            .runtime_store
            .find_boundary_event_states_by_process_instance_id(
                &self.process_instance_id,
                &mut command_context.session,
            );

        let matching_states: Vec<_> = boundary_states
            .into_iter()
            .filter(|state| {
                if state.event_subscription.kind != self.subscription_kind {
                    return false;
                }

                if self.subscription_kind == EventSubscriptionKind::Escalation {
                    escalation_refs_match(command_context, state, &self.event_ref)
                } else if self.subscription_kind == EventSubscriptionKind::Error {
                    error_refs_match(command_context, state, &self.event_ref)
                } else if self.subscription_kind == EventSubscriptionKind::Signal {
                    signal_refs_match_for_boundary(command_context, state, &self.event_ref)
                } else {
                    state.event_subscription.event_ref == self.event_ref
                }
            })
            .collect();

        let boundary_state = match if self.subscription_kind == EventSubscriptionKind::Escalation {
            select_nearest_escalation_boundary(
                command_context,
                matching_states,
                self.source_execution_id.as_deref(),
                &self.event_ref,
            )
        } else if self.subscription_kind == EventSubscriptionKind::Error {
            select_nearest_error_boundary(
                command_context,
                matching_states,
                self.source_execution_id.as_deref(),
                &self.event_ref,
            )
        } else {
            matching_states.into_iter().next()
        } {
            Some(state) => state,
            None => {
                tracing::warn!(
                    "No boundary event state found for {} ref {} in process instance {}",
                    kind_label,
                    self.event_ref,
                    self.process_instance_id
                );
                return Ok(BoundaryEventTriggerResult::default());
            }
        };

        let interrupting = boundary_state.cancel_activity;
        execute_boundary_trigger(command_context, boundary_state, &self.process_instance_id)?;
        Ok(BoundaryEventTriggerResult {
            triggered: true,
            interrupting,
        })
    }

    pub(crate) fn execute_with_trigger_result(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<bool, crate::error::FlowableError> {
        Ok(self.execute_with_catch_result(command_context)?.triggered)
    }
}

impl Command<()> for TriggerBoundaryEventByEventRefCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        self.execute_with_trigger_result(command_context)?;
        Ok(())
    }
}

// ── Stable entry-point shims ──
// These structs wrap the unified command to preserve the old constructor
// signatures used in runtime_service.rs.

pub struct TriggerBoundaryEventByMessageRefCmd {
    inner: TriggerBoundaryEventByEventRefCmd,
}

impl TriggerBoundaryEventByMessageRefCmd {
    pub fn new(message_ref: String, process_instance_id: String) -> Self {
        Self {
            inner: TriggerBoundaryEventByEventRefCmd::new(
                EventSubscriptionKind::Message,
                message_ref,
                process_instance_id,
            ),
        }
    }
}

impl Command<()> for TriggerBoundaryEventByMessageRefCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        self.inner.execute(command_context)
    }
}

pub struct TriggerBoundaryEventBySignalRefCmd {
    inner: TriggerBoundaryEventByEventRefCmd,
}

impl TriggerBoundaryEventBySignalRefCmd {
    pub fn new(signal_ref: String, process_instance_id: String) -> Self {
        Self {
            inner: TriggerBoundaryEventByEventRefCmd::new(
                EventSubscriptionKind::Signal,
                signal_ref,
                process_instance_id,
            ),
        }
    }
}

impl Command<()> for TriggerBoundaryEventBySignalRefCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        self.inner.execute(command_context)
    }
}

// ── Timer Boundary Triggers ──

fn execute_timer_boundary_trigger(
    command_context: &mut CommandContext,
    timer_state: RuntimeTimerJobState,
    process_instance_id: &str,
) -> Result<(), crate::error::FlowableError> {
    let boundary_event_id = timer_state.activity_id.clone();

    // Find the host execution (the execution of the activity the boundary event is attached to)
    let host_execution = match command_context
        .execution_entity_manager
        .find_by_id(&timer_state.execution_id, &mut command_context.session)
    {
        Some(exec) => exec.clone(),
        None => {
            tracing::error!(
                "Host execution {} not found for timer boundary event {}",
                timer_state.execution_id,
                boundary_event_id
            );
            return Ok(());
        }
    };

    if host_execution.is_active {
        tracing::warn!(
            "Host execution {} is still active, cannot trigger timer boundary event {}",
            timer_state.execution_id,
            boundary_event_id
        );
        return Ok(());
    }

    if timer_state.cancel_activity {
        // Java parity (`BoundaryEventActivityBehavior#executeInterruptingBehavior`
        // → `#deleteChildExecutions` 157–164): an interrupting boundary deletes
        // the host's child executions and the host itself. For an MI host (the
        // MI root) this cancels EVERY instance, not just one. Mirrors the
        // message/signal path in `execute_boundary_trigger` above.
        let delete_reason =
            crate::history::delete_reason::boundary_event_interrupting(&boundary_event_id);

        let mut execution_ids_to_delete =
            descendant_execution_ids(command_context, &timer_state.execution_id);
        execution_ids_to_delete.push(timer_state.execution_id.clone());

        for execution_id in execution_ids_to_delete {
            crate::bpmn::behavior::multi_instance_support::record_activity_end_for_execution(
                command_context,
                &execution_id,
                Some(&delete_reason),
            );
            if execution_id == process_instance_id {
                // Keep the PI scope row (process-level variable store); Java
                // `deleteChildExecutions` never deletes the process instance
                // execution. See `execute_boundary_trigger` above.
                retire_process_scope_row_after_interrupt(command_context, &execution_id);
            } else {
                delete_execution_runtime_state(command_context, &execution_id);
            }
        }

        // Java parity (`BoundaryEventActivityBehavior.java:63-112,157-164`): an
        // interrupting boundary only deletes the host's child execution subtree.
        // Process-instance-level event-subprocess timer subscriptions stay alive
        // until the process instance itself ends (fault/end-event/delete paths).
    } else {
        match reschedule_non_interrupting_timer_cycle(command_context, timer_state.clone())? {
            Some(next_timer_state) => {
                command_context
                    .runtime_store
                    .insert_timer_job_state(&next_timer_state, &mut command_context.session);
            }
            None => {
                command_context.runtime_store.delete_timer_job_state(
                    &timer_state.timer_job_id,
                    &mut command_context.session,
                );
            }
        }
    }

    // Seed from the process-instance scope execution row: it is the single
    // process-level variable store.
    let process_variables = command_context
        .runtime_store
        .find_execution(process_instance_id, &mut command_context.session)
        .map(|root_execution| root_execution.variables)
        .unwrap_or_default();

    let boundary_execution = Execution {
        id: Uuid::new_v4().to_string(),
        parent_id: host_execution.parent_id.clone(),
        process_instance_id: Some(process_instance_id.to_string()),
        process_definition_id: host_execution.process_definition_id.clone(),
        activity_id: Some(boundary_event_id.clone()),
        is_active: true,
        is_concurrent: host_execution.is_concurrent,
        is_ended: false,
        is_scope: host_execution.is_scope,
        is_multi_instance_root: false,
        variables: process_variables,
        ..Default::default()
    };

    command_context
        .execution_entity_manager
        .insert(&boundary_execution, &mut command_context.session);

    command_context
        .agenda
        .plan_continue_process_operation(boundary_execution.clone());

    command_context
        .agenda
        .plan_take_outgoing_sequence_flows_operation(boundary_execution);

    Ok(())
}

fn reschedule_non_interrupting_timer_cycle(
    command_context: &mut CommandContext,
    mut timer_state: RuntimeTimerJobState,
) -> Result<Option<RuntimeTimerJobState>, crate::error::FlowableError> {
    let Some(current_cycle) = timer_state.time_cycle.clone() else {
        return Ok(None);
    };
    let now = command_context.runtime_store.time_source().now();

    // Variable scope for calendarName EL re-evaluation (ADR-2). Prefer the host
    // execution, then the process instance, then an empty scope.
    let execution = command_context
        .runtime_store
        .find_execution(&timer_state.execution_id, &mut command_context.session)
        .or_else(|| {
            command_context
                .runtime_store
                .find_execution(&timer_state.process_instance_id, &mut command_context.session)
        })
        .unwrap_or_default();

    let calendars = command_context.config.business_calendar_registry.clone();
    let Some(schedule) = crate::bpmn::timer_util::resolve_next_timer_schedule(
        &current_cycle,
        timer_state.end_date.as_deref(),
        timer_state.calendar_name.as_ref(),
        &execution,
        &calendars,
        now,
    )?
    else {
        return Ok(None);
    };

    timer_state.time_cycle = Some(schedule.cycle);
    timer_state.due_time = Some(schedule.due_time_millis);
    timer_state.lock_owner = None;
    timer_state.lock_time = None;
    timer_state.lock_expiration_time = None;
    timer_state.error_message = None;
    timer_state.error_details = None;
    Ok(Some(timer_state))
}

pub struct TriggerTimerBoundaryEventCmd {
    boundary_event_id: String,
    process_instance_id: String,
}

impl TriggerTimerBoundaryEventCmd {
    pub fn new(boundary_event_id: String, process_instance_id: String) -> Self {
        Self {
            boundary_event_id,
            process_instance_id,
        }
    }
}

impl Command<()> for TriggerTimerBoundaryEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let timer_states = command_context
            .runtime_store
            .find_timer_job_states_by_process_instance_id(
                &self.process_instance_id,
                &mut command_context.session,
            );

        let timer_state = match timer_states
            .into_iter()
            .find(|state| state.is_boundary && state.activity_id == self.boundary_event_id)
        {
            Some(state) => state,
            None => {
                tracing::warn!(
                    "No timer boundary event state found for boundary event id {}",
                    self.boundary_event_id
                );
                return Ok(());
            }
        };

        execute_timer_boundary_trigger(command_context, timer_state, &self.process_instance_id)
    }
}
