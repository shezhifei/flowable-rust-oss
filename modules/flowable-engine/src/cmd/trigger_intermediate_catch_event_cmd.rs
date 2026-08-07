use crate::agenda::FlowableEngineAgenda;
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventSubscriptionKind, RuntimeEventWaitKind, RuntimeEventWaitState,
};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::BpmnModel;
use flowable_bpmn_model::model::FlowElementEnum;
use serde_json::Value;
use std::collections::HashMap;

/// Returns an error if the execution is suspended. Mirrors Java `NeedsActiveExecutionCmd`.
/// Java: FlowableException → Rust: ExecutionError → HTTP 500.
pub(crate) fn require_active_execution(execution: &Execution) -> Result<(), FlowableError> {
    if execution.is_suspended {
        return Err(FlowableError::ExecutionError(format!(
            "Cannot trigger a suspended execution '{}'",
            execution.id
        )));
    }
    Ok(())
}

fn is_none_intermediate_catch_event(
    _store: &crate::persistence::runtime_store::RuntimeStore,
    dm: &crate::engine::deployment_manager::DeploymentManager,
    execution: &Execution,
    session: &mut crate::persistence::db_session::DbSession,
) -> bool {
    let process_definition_id = match execution.process_definition_id.as_deref() {
        Some(process_definition_id) => process_definition_id,
        None => return false,
    };

    let activity_id = match execution.activity_id.as_deref() {
        Some(activity_id) => activity_id,
        None => return false,
    };

    let _ = session;
    dm.get_bpmn_model(process_definition_id)
        .as_ref()
        .and_then(|model| model.main_process.as_ref())
        .and_then(|process| process.flow_element_map.get(activity_id))
        .map(|flow_element| {
            matches!(
                flow_element,
                FlowElementEnum::IntermediateCatchEvent(event)
                    if event.event.event_definitions.is_empty()
            )
        })
        .unwrap_or(false)
}

fn find_waiting_none_intermediate_catch_execution(
    store: &crate::persistence::runtime_store::RuntimeStore,
    dm: &crate::engine::deployment_manager::DeploymentManager,
    session: &mut crate::persistence::db_session::DbSession,
    process_instance_id: &str,
) -> Option<Execution> {
    let mut executions: Vec<Execution> = store
        .snapshot_executions(session)
        .into_values()
        .filter(|execution| execution.process_instance_id.as_deref() == Some(process_instance_id))
        .filter(|execution| !execution.is_active)
        .filter(|execution| is_none_intermediate_catch_event(store, dm, execution, session))
        .collect();

    executions.sort_by(|left, right| left.id.cmp(&right.id));
    executions.into_iter().next()
}

fn find_waiting_event_intermediate_catch_execution(
    store: &crate::persistence::runtime_store::RuntimeStore,
    dm: &crate::engine::deployment_manager::DeploymentManager,
    em: &mut dyn crate::persistence::entity_manager::EntityManager<Execution>,
    session: &mut crate::persistence::db_session::DbSession,
    execution_id: &str,
    expected_kind: &EventSubscriptionKind,
    event_ref: &str,
) -> Option<Execution> {
    let wait_state = store.find_event_wait_state_by_execution_id(execution_id, session)?;

    let kind_matches = matches!(
        (&wait_state.wait_kind, expected_kind),
        (
            RuntimeEventWaitKind::MessageIntermediateCatchEvent,
            EventSubscriptionKind::Message
        ) | (
            RuntimeEventWaitKind::SignalIntermediateCatchEvent,
            EventSubscriptionKind::Signal
        ) | (
            RuntimeEventWaitKind::ConditionalIntermediateCatchEvent,
            EventSubscriptionKind::Conditional
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
            // Intermediate catch on flowable:eventType
            // (IntermediateCatchEventRegistryEventActivityBehavior.java).
            RuntimeEventWaitKind::EventRegistryIntermediateCatchEvent,
            EventSubscriptionKind::EventRegistry
        ) | (
            // Event-registry receive task without a user Task row
            // (ReceiveEventTaskActivityBehavior.java) — leave via trigger.
            RuntimeEventWaitKind::ReceiveTask,
            EventSubscriptionKind::EventRegistry
        )
    );
    if !kind_matches {
        return None;
    }

    let ref_matches = wait_state.event_subscription.as_ref().is_some_and(|sub| {
        sub.kind == *expected_kind
            && event_ref_matches(
                store,
                dm,
                sub,
                expected_kind,
                event_ref,
                &wait_state,
                session,
            )
    });
    if !ref_matches {
        return None;
    }

    let execution = em.find_by_id(execution_id, session)?.clone();

    if execution.is_active {
        return None;
    }

    Some(execution)
}

fn event_ref_matches(
    store: &crate::persistence::runtime_store::RuntimeStore,
    dm: &crate::engine::deployment_manager::DeploymentManager,
    subscription: &crate::persistence::runtime_store::EventSubscription,
    expected_kind: &EventSubscriptionKind,
    trigger_ref: &str,
    wait_state: &RuntimeEventWaitState,
    session: &mut crate::persistence::db_session::DbSession,
) -> bool {
    if subscription.event_ref == trigger_ref {
        return true;
    }

    if *expected_kind == EventSubscriptionKind::Signal {
        let model = signal_model_for_wait_state(store, dm, wait_state, session);
        signal_refs_match_in_model(model.as_deref(), &subscription.event_ref, trigger_ref)
    } else {
        false
    }
}

fn signal_model_for_wait_state(
    store: &crate::persistence::runtime_store::RuntimeStore,
    dm: &crate::engine::deployment_manager::DeploymentManager,
    wait_state: &RuntimeEventWaitState,
    session: &mut crate::persistence::db_session::DbSession,
) -> Option<std::sync::Arc<BpmnModel>> {
    let process_definition_id = store
        .find_execution(&wait_state.execution_id, session)
        .and_then(|execution| execution.process_definition_id)
        .or_else(|| {
            store
                .find_process_instance(&wait_state.process_instance_id, session)
                .map(|instance| instance.process_definition_id)
        })?;

    dm.get_bpmn_model(&process_definition_id)
}

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

pub struct TriggerIntermediateCatchEventCmd {
    process_instance_id: String,
}

impl TriggerIntermediateCatchEventCmd {
    pub fn new(process_instance_id: String) -> Self {
        Self {
            process_instance_id,
        }
    }
}

impl Command<()> for TriggerIntermediateCatchEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();
        let session = &mut command_context.session;
        let em = &mut *command_context.execution_entity_manager;
        let agenda = &mut command_context.agenda;

        let mut execution = match find_waiting_none_intermediate_catch_execution(
            &store,
            &dm,
            session,
            &self.process_instance_id,
        ) {
            Some(execution) => execution,
            None => {
                tracing::warn!(
                    "No waiting none intermediate catch event found for process instance id {}",
                    self.process_instance_id
                );
                return Ok(());
            }
        };

        // Java parity: NeedsActiveExecutionCmd checks execution.isSuspended()
        require_active_execution(&execution)?;

        execution.is_active = true;
        em.update(&execution, session);
        agenda.plan_take_outgoing_sequence_flows_operation(execution);
        Ok(())
    }
}

pub struct TriggerEventIntermediateCatchCmd {
    subscription_kind: EventSubscriptionKind,
    event_ref: String,
    execution_id: String,
    variables: HashMap<String, Value>,
    /// Java parity: `SignalEventReceivedCmd` only checks suspension for targeted
    /// signals (executionId != null). Global signals skip the check. Message
    /// triggers (`MessageEventReceivedCmd extends NeedsActiveExecutionCmd`) always
    /// check. Default is `true`.
    check_suspension: bool,
}

impl TriggerEventIntermediateCatchCmd {
    pub fn new(
        subscription_kind: EventSubscriptionKind,
        event_ref: String,
        execution_id: String,
    ) -> Self {
        Self {
            subscription_kind,
            event_ref,
            execution_id,
            variables: HashMap::new(),
            check_suspension: true,
        }
    }

    pub fn with_variables(
        subscription_kind: EventSubscriptionKind,
        event_ref: String,
        execution_id: String,
        variables: HashMap<String, Value>,
    ) -> Self {
        Self {
            subscription_kind,
            event_ref,
            execution_id,
            variables,
            check_suspension: true,
        }
    }

    /// Disables the suspension guard. Used by global signal broadcast where Java
    /// `SignalEventReceivedCmd` does NOT check suspension (executionId == null path).
    pub fn without_suspension_check(mut self) -> Self {
        self.check_suspension = false;
        self
    }
}

impl Command<()> for TriggerEventIntermediateCatchCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let dm = command_context.deployment_manager_handle();

        let mut execution = {
            let session = &mut command_context.session;
            let em = &mut *command_context.execution_entity_manager;
            let hm = &mut command_context.history_manager;

            let mut execution = match find_waiting_event_intermediate_catch_execution(
                &store,
                &dm,
                em,
                session,
                &self.execution_id,
                &self.subscription_kind,
                &self.event_ref,
            ) {
                Some(execution) => execution,
                None => {
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
                    tracing::warn!(
                        "No waiting {} intermediate catch event found for execution id {} and event ref {}",
                        kind_label,
                        self.execution_id,
                        self.event_ref
                    );
                    return Ok(());
                }
            };

            // Java parity: NeedsActiveExecutionCmd / SignalEventReceivedCmd inline check.
            // Global signal broadcast skips this (check_suspension == false).
            if self.check_suspension {
                require_active_execution(&execution)?;
            }

            store.delete_event_wait_state_by_execution_id(&execution.id, session);

            if !self.variables.is_empty() {
                let process_instance_id = execution
                    .process_instance_id
                    .clone()
                    .unwrap_or_else(|| execution.id.clone());
                for (name, value) in &self.variables {
                    execution.set_process_variable(name.clone(), value.clone());
                    let variable_id = format!("{}:{}", execution.id, name);
                    if store
                        .get_historic_variable_instance(&variable_id, session)
                        .is_some()
                    {
                        hm.record_variable_updated(&variable_id, value.clone(), session);
                    } else {
                        hm.record_variable_created(
                            &variable_id,
                            name,
                            crate::engine::variable_service::variable_type_name(value),
                            value.clone(),
                            &process_instance_id,
                            Some(&execution.id),
                            None,
                            session,
                        );
                    }
                }
            }

            execution
        };

        // P125: ACTIVITY_CONDITIONAL_RECEIVED when leaving a conditional wait.
        // Java IntermediateCatchConditionalEventActivityBehavior.java:63-65.
        if self.subscription_kind == EventSubscriptionKind::Conditional {
            let activity_id = execution.activity_id.clone().unwrap_or_default();
            let pi = execution
                .process_instance_id
                .clone()
                .unwrap_or_else(|| execution.id.clone());
            crate::engine::event_dispatcher::dispatch_activity_conditional_received(
                command_context,
                &activity_id,
                &self.event_ref,
                Some(&pi),
                Some(&execution.id),
                execution.process_definition_id.as_deref(),
            );
        }

        if let Some(parent_id) = &execution.parent_id {
            let dm = command_context.deployment_manager_handle();
            let session = &mut command_context.session;
            let em = &mut *command_context.execution_entity_manager;

            if let Some(parent) = store.find_execution(parent_id, session) {
                let mut is_event_gateway = false;
                if let Some(pd_id) = &parent.process_definition_id
                    && let Some(act_id) = &parent.activity_id
                    && let Some(model) = dm.get_bpmn_model(pd_id)
                    && let Some(process) = &model.main_process
                    && let Some(flow_element) = process.flow_element_map.get(act_id)
                {
                    is_event_gateway =
                        matches!(flow_element, FlowElementEnum::EventBasedGateway(_));
                }

                if is_event_gateway {
                    // Collect id + activity_id so we can end historic activities
                    // with Java DeleteReason.EVENT_BASED_GATEWAY_CANCEL before
                    // deleting the sibling execution rows.
                    let siblings: Vec<(String, Option<String>)> = store
                        .snapshot_executions(session)
                        .into_values()
                        .filter(|e| {
                            e.parent_id.as_deref() == Some(parent_id)
                                && e.id != execution.id
                                && !e.is_ended
                        })
                        .map(|e| (e.id, e.activity_id))
                        .collect();

                    for (sibling_id, activity_id) in siblings {
                        if let Some(activity_id) = activity_id.as_deref() {
                            command_context.history_manager.record_activity_end(
                                &sibling_id,
                                activity_id,
                                Some(crate::history::delete_reason::EVENT_BASED_GATEWAY_CANCEL),
                                session,
                            );
                        }
                        store.delete_event_wait_state_by_execution_id(&sibling_id, session);
                        store.delete_boundary_event_states_by_host_execution_id(
                            &sibling_id,
                            session,
                        );
                        store.delete_timer_job_states_by_execution_id(&sibling_id, session);
                        em.delete(&sibling_id, session);
                    }
                }
            }
        }

        {
            let session = &mut command_context.session;
            let em = &mut *command_context.execution_entity_manager;
            let agenda = &mut command_context.agenda;

            execution.is_active = true;
            em.update(&execution, session);
            agenda.plan_take_outgoing_sequence_flows_operation(execution);
        }
        Ok(())
    }
}

pub type TriggerMessageIntermediateCatchEventCmd = TriggerEventIntermediateCatchCmd;
pub type TriggerSignalIntermediateCatchEventCmd = TriggerEventIntermediateCatchCmd;

pub struct TriggerTimerIntermediateCatchEventCmd {
    execution_id: String,
}

impl TriggerTimerIntermediateCatchEventCmd {
    pub fn new(execution_id: String) -> Self {
        Self { execution_id }
    }
}

impl Command<()> for TriggerTimerIntermediateCatchEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();

        let timer_state = {
            let session = command_context.session();
            let timer_states =
                store.find_timer_job_states_by_execution_id(&self.execution_id, session);
            match timer_states.into_iter().find(|state| !state.is_boundary) {
                Some(state) => state,
                None => {
                    tracing::warn!(
                        "No waiting timer intermediate catch event found for execution id {}",
                        self.execution_id
                    );
                    return Ok(());
                }
            }
        };

        let mut execution = {
            let session = &mut command_context.session;
            let em = &mut *command_context.execution_entity_manager;
            match em.find_by_id(&self.execution_id, session) {
                Some(exec) => exec.clone(),
                None => {
                    tracing::error!("Execution {} not found", self.execution_id);
                    return Ok(());
                }
            }
        };

        // Java parity: NeedsActiveExecutionCmd checks execution.isSuspended()
        require_active_execution(&execution)?;

        if execution.is_active {
            tracing::warn!(
                "Execution {} is active, cannot trigger timer",
                self.execution_id
            );
            return Ok(());
        }

        {
            let session = command_context.session();
            store.delete_timer_job_state(&timer_state.timer_job_id, session);
        }

        if let Some(parent_id) = &execution.parent_id {
            let dm = command_context.deployment_manager_handle();
            let session = &mut command_context.session;
            let em = &mut *command_context.execution_entity_manager;

            if let Some(parent) = store.find_execution(parent_id, session) {
                let mut is_event_gateway = false;
                if let Some(pd_id) = &parent.process_definition_id
                    && let Some(act_id) = &parent.activity_id
                    && let Some(model) = dm.get_bpmn_model(pd_id)
                    && let Some(process) = &model.main_process
                    && let Some(flow_element) = process.flow_element_map.get(act_id)
                {
                    is_event_gateway =
                        matches!(flow_element, FlowElementEnum::EventBasedGateway(_));
                }

                if is_event_gateway {
                    let siblings: Vec<(String, Option<String>)> = store
                        .snapshot_executions(session)
                        .into_values()
                        .filter(|e| {
                            e.parent_id.as_deref() == Some(parent_id)
                                && e.id != execution.id
                                && !e.is_ended
                        })
                        .map(|e| (e.id, e.activity_id))
                        .collect();

                    for (sibling_id, activity_id) in siblings {
                        if let Some(activity_id) = activity_id.as_deref() {
                            command_context.history_manager.record_activity_end(
                                &sibling_id,
                                activity_id,
                                Some(crate::history::delete_reason::EVENT_BASED_GATEWAY_CANCEL),
                                session,
                            );
                        }
                        store.delete_event_wait_state_by_execution_id(&sibling_id, session);
                        store.delete_boundary_event_states_by_host_execution_id(
                            &sibling_id,
                            session,
                        );
                        store.delete_timer_job_states_by_execution_id(&sibling_id, session);
                        em.delete(&sibling_id, session);
                    }
                }
            }
        }

        {
            let session = &mut command_context.session;
            let em = &mut *command_context.execution_entity_manager;
            let agenda = &mut command_context.agenda;

            execution.is_active = true;
            em.update(&execution, session);
            agenda.plan_take_outgoing_sequence_flows_operation(execution);
        }
        Ok(())
    }
}
