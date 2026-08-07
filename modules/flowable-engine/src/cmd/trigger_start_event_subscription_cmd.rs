use crate::agenda::FlowableEngineAgenda;
use crate::cmd::start_process_instance_cmd::StartProcessInstanceCmd;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{EventSubprocessEventSubscription, EventSubscriptionKind};
use crate::runtime::execution::Execution;
use crate::runtime::process_instance::ProcessInstance;
use crate::runtime::process_instance_builder::ProcessInstanceBuilder;
use flowable_bpmn_model::model::BpmnModel;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

pub(crate) const NON_INTERRUPTING_EVENT_SUBPROCESS_PATH_VARIABLE: &str =
    "__flowable_non_interrupting_event_subprocess_path";

// ── Process-level message/signal start event trigger ──

/// Triggers a process-level message or signal start event, creating a new process instance.
/// Semantically equivalent to starting a process instance by message correlation.
pub struct TriggerProcessStartByEventCmd {
    event_kind: EventSubscriptionKind,
    event_ref: String,
    tenant_id: Option<String>,
    /// When set, pin the start to this process definition id (P136: one-sub-one-start
    /// after tenant key dedup, instead of last-deployed-wins for the whole event).
    process_definition_id: Option<String>,
    business_key: Option<String>,
    start_user_id: Option<String>,
    variables: HashMap<String, Value>,
    transient_variables: HashMap<String, Value>,
}

impl TriggerProcessStartByEventCmd {
    pub fn new(event_kind: EventSubscriptionKind, event_ref: String) -> Self {
        Self {
            event_kind,
            event_ref,
            tenant_id: None,
            process_definition_id: None,
            business_key: None,
            start_user_id: None,
            variables: HashMap::new(),
            transient_variables: HashMap::new(),
        }
    }

    pub fn with_tenant_id(mut self, tenant_id: String) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    /// Pin to a specific process-definition start subscription
    /// (Java starts one process per surviving EventSubscription).
    pub fn with_process_definition_id(mut self, process_definition_id: String) -> Self {
        self.process_definition_id = Some(process_definition_id);
        self
    }

    pub fn with_business_key(mut self, business_key: String) -> Self {
        self.business_key = Some(business_key);
        self
    }

    /// Carries the authenticated caller for externally initiated event starts.
    /// Engine-internal message, signal and timer paths leave this unset.
    pub fn with_start_user_id(mut self, start_user_id: String) -> Self {
        self.start_user_id = Some(start_user_id);
        self
    }

    pub fn with_variables(mut self, variables: HashMap<String, Value>) -> Self {
        self.variables = variables;
        self
    }

    /// Java parity: `ProcessInstanceBuilder.transientVariables` is honored by the
    /// message-start path (`ProcessInstanceCollectionResource.java:402-403`).
    pub fn with_transient_variables(mut self, transient_variables: HashMap<String, Value>) -> Self {
        self.transient_variables = transient_variables;
        self
    }
}

impl Command<ProcessInstance> for TriggerProcessStartByEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        // Find the matching process-level event start subscription (last-deployed-wins)
        let matching_subs = command_context
            .deployment_manager
            .find_event_start_subscriptions_by_event_ref(
                &self.event_kind,
                &self.event_ref,
                &mut command_context.session,
            );

        let tenant_id = self.tenant_id.as_deref();
        let selected_sub = if let Some(pd_id) = self.process_definition_id.as_deref() {
            // P136: exact process-definition pin (one subscription one start).
            matching_subs
                .iter()
                .find(|sub| sub.process_definition_id == pd_id)
        } else {
            matching_subs
                .iter()
                .rev()
                .find(|sub| sub.tenant_id.as_deref() == tenant_id)
                .or_else(|| {
                    if tenant_id.is_some() {
                        None
                    } else {
                        matching_subs.last()
                    }
                })
        };

        let sub = match selected_sub {
            Some(s) => s,
            None => {
                tracing::warn!(
                    "No {:?} start event subscription found for event_ref '{}' and tenant {:?}",
                    self.event_kind,
                    self.event_ref,
                    self.tenant_id
                );
                return Err(crate::error::FlowableError::NotFound(format!(
                    "No {:?} start event subscription found for event_ref '{}' and tenant {:?}",
                    self.event_kind, self.event_ref, self.tenant_id
                )));
            }
        };

        let mut builder =
            ProcessInstanceBuilder::new().process_definition_id(sub.process_definition_id.clone());

        if let Some(tenant_id) = &self.tenant_id {
            builder = builder.tenant_id(tenant_id.clone());
        }
        if let Some(business_key) = &self.business_key {
            builder = builder.business_key(business_key.clone());
        }
        if let Some(start_user_id) = &self.start_user_id {
            builder = builder.start_user_id(start_user_id.clone());
        }
        for (name, value) in &self.variables {
            builder = builder.variable(name.clone(), value.clone());
        }
        for (name, value) in &self.transient_variables {
            builder = builder.transient_variable(name.clone(), value.clone());
        }

        let cmd = StartProcessInstanceCmd::with_start_event_id(builder, sub.start_event_id.clone());
        cmd.execute(command_context)
    }
}

// ── Event subprocess activation by message/signal ──

/// Triggers event subprocess activation for all matching message/signal subscriptions.
/// For interrupting subprocesses: cancels host activities. For non-interrupting: runs in parallel.
pub struct TriggerEventSubprocessByEventCmd {
    event_kind: EventSubscriptionKind,
    event_ref: String,
    process_instance_id: String,
    source_execution_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EventSubprocessTriggerResult {
    pub(crate) triggered_ids: Vec<String>,
    pub(crate) interrupting: bool,
}

impl TriggerEventSubprocessByEventCmd {
    pub fn new(
        event_kind: EventSubscriptionKind,
        event_ref: String,
        process_instance_id: String,
    ) -> Self {
        Self {
            event_kind,
            event_ref,
            process_instance_id,
            source_execution_id: None,
        }
    }

    pub fn with_source_execution(
        event_kind: EventSubscriptionKind,
        event_ref: String,
        process_instance_id: String,
        source_execution_id: String,
    ) -> Self {
        Self {
            event_kind,
            event_ref,
            process_instance_id,
            source_execution_id: Some(source_execution_id),
        }
    }

    pub(crate) fn execute_with_trigger_result(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<EventSubprocessTriggerResult, crate::error::FlowableError> {
        let matching_subs: Vec<EventSubprocessEventSubscription> = command_context
            .runtime_store
            .find_event_subprocess_event_subscriptions_by_process_instance_id(
                &self.process_instance_id,
                &mut command_context.session,
            )
            .into_iter()
            .filter(|sub| {
                if sub.event_kind != self.event_kind {
                    return false;
                }

                if self.event_kind == EventSubscriptionKind::Escalation {
                    escalation_refs_match(command_context, sub, &self.event_ref)
                } else if self.event_kind == EventSubscriptionKind::Error {
                    error_refs_match(command_context, sub, &self.event_ref)
                } else {
                    sub.event_ref == self.event_ref
                }
            })
            .collect();

        let mut triggered_ids = Vec::new();

        let selected_subs = if self.event_kind == EventSubscriptionKind::Escalation {
            select_nearest_event_subprocess_subscription(
                command_context,
                matching_subs,
                self.source_execution_id.as_deref(),
                &self.event_ref,
                event_subprocess_ref_matches_exact_escalation,
                true,
            )
            .into_iter()
            .collect::<Vec<_>>()
        } else if self.event_kind == EventSubscriptionKind::Error {
            select_nearest_event_subprocess_subscription(
                command_context,
                matching_subs,
                self.source_execution_id.as_deref(),
                &self.event_ref,
                error_refs_match_exact,
                false,
            )
            .into_iter()
            .collect::<Vec<_>>()
        } else {
            matching_subs
        };

        let interrupting = selected_subs.iter().any(|sub| sub.interrupting);
        for sub in &selected_subs {
            if sub.interrupting {
                activate_interrupting_event_subprocess(command_context, sub);
            } else {
                activate_non_interrupting_event_subprocess(command_context, sub);
            }

            if sub.interrupting
                && !matches!(
                    sub.event_kind,
                    EventSubscriptionKind::Error | EventSubscriptionKind::Escalation
                )
            {
                command_context
                    .runtime_store
                    .delete_event_subprocess_event_subscription(
                        &sub.subscription_id,
                        &mut command_context.session,
                    );
            }

            triggered_ids.push(format!(
                "event_subprocess_{}:{}:{}",
                match self.event_kind {
                    EventSubscriptionKind::Message => "message",
                    EventSubscriptionKind::Signal => "signal",
                    EventSubscriptionKind::Conditional => "conditional",
                    EventSubscriptionKind::Error => "error",
                    EventSubscriptionKind::Cancel => "cancel",
                    EventSubscriptionKind::Compensate => "compensate",
                    EventSubscriptionKind::Escalation => "escalation",
                    EventSubscriptionKind::EventRegistry => "event-registry",
                },
                sub.process_instance_id,
                sub.start_event_id
            ));
        }

        Ok(EventSubprocessTriggerResult {
            triggered_ids,
            interrupting,
        })
    }
}

impl Command<Vec<String>> for TriggerEventSubprocessByEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<String>, crate::error::FlowableError> {
        Ok(self
            .execute_with_trigger_result(command_context)?
            .triggered_ids)
    }
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

fn escalation_refs_match(
    command_context: &mut CommandContext,
    subscription: &EventSubprocessEventSubscription,
    thrown_ref: &str,
) -> bool {
    if subscription.event_ref.is_empty() {
        return true;
    }

    if subscription.event_ref == thrown_ref {
        return true;
    }

    let process_definition_id = subscription
        .scope_execution_id
        .as_deref()
        .and_then(|execution_id| {
            command_context
                .runtime_store
                .find_execution(execution_id, &mut command_context.session)
        })
        .and_then(|execution| execution.process_definition_id)
        .or_else(|| {
            command_context
                .runtime_store
                .find_process_instance(
                    &subscription.process_instance_id,
                    &mut command_context.session,
                )
                .map(|process_instance| process_instance.process_definition_id)
        });

    let model_arc = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id.as_deref().unwrap_or(""));
    let model = model_arc.as_deref();

    normalize_escalation_ref(model, &subscription.event_ref)
        == normalize_escalation_ref(model, thrown_ref)
}

fn escalation_refs_match_exact(
    command_context: &mut CommandContext,
    subscription: &EventSubprocessEventSubscription,
    thrown_ref: &str,
) -> bool {
    if subscription.event_ref.is_empty() {
        return false;
    }

    if subscription.event_ref == thrown_ref {
        return true;
    }

    let process_definition_id = subscription
        .scope_execution_id
        .as_deref()
        .and_then(|execution_id| {
            command_context
                .runtime_store
                .find_execution(execution_id, &mut command_context.session)
        })
        .and_then(|execution| execution.process_definition_id)
        .or_else(|| {
            command_context
                .runtime_store
                .find_process_instance(
                    &subscription.process_instance_id,
                    &mut command_context.session,
                )
                .map(|process_instance| process_instance.process_definition_id)
        });

    let model_arc = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id.as_deref().unwrap_or(""));
    let model = model_arc.as_deref();

    normalize_escalation_ref(model, &subscription.event_ref)
        == normalize_escalation_ref(model, thrown_ref)
}

fn event_subprocess_ref_matches_exact_escalation(
    command_context: &mut CommandContext,
    subscription: &EventSubprocessEventSubscription,
    thrown_ref: &str,
) -> bool {
    escalation_refs_match_exact(command_context, subscription, thrown_ref)
}

fn normalize_error_ref(model: Option<&BpmnModel>, event_ref: &str) -> String {
    if let Some(model) = model
        && let Some(error_code) = model.errors.get(event_ref)
    {
        return error_code.clone();
    }

    event_ref.to_string()
}

fn error_model_for_subscription(
    command_context: &mut CommandContext,
    subscription: &EventSubprocessEventSubscription,
) -> Option<std::sync::Arc<BpmnModel>> {
    let process_definition_id = subscription
        .scope_execution_id
        .as_deref()
        .and_then(|execution_id| {
            command_context
                .runtime_store
                .find_execution(execution_id, &mut command_context.session)
        })
        .and_then(|execution| execution.process_definition_id)
        .or_else(|| {
            command_context
                .runtime_store
                .find_process_instance(
                    &subscription.process_instance_id,
                    &mut command_context.session,
                )
                .map(|process_instance| process_instance.process_definition_id)
        })?;

    command_context
        .deployment_manager
        .get_bpmn_model(&process_definition_id)
}

fn error_refs_match(
    command_context: &mut CommandContext,
    subscription: &EventSubprocessEventSubscription,
    thrown_ref: &str,
) -> bool {
    if subscription.event_ref.is_empty() {
        return true;
    }

    if subscription.event_ref == thrown_ref {
        return true;
    }

    let model = error_model_for_subscription(command_context, subscription);
    let model = model.as_deref();

    normalize_error_ref(model, &subscription.event_ref) == normalize_error_ref(model, thrown_ref)
}

fn error_refs_match_exact(
    command_context: &mut CommandContext,
    subscription: &EventSubprocessEventSubscription,
    thrown_ref: &str,
) -> bool {
    if subscription.event_ref.is_empty() {
        return false;
    }

    if subscription.event_ref == thrown_ref {
        return true;
    }

    let model = error_model_for_subscription(command_context, subscription);
    let model = model.as_deref();

    normalize_error_ref(model, &subscription.event_ref) == normalize_error_ref(model, thrown_ref)
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

fn select_nearest_event_subprocess_subscription(
    command_context: &mut CommandContext,
    mut subscriptions: Vec<EventSubprocessEventSubscription>,
    source_execution_id: Option<&str>,
    thrown_ref: &str,
    exact_match: fn(&mut CommandContext, &EventSubprocessEventSubscription, &str) -> bool,
    accept_nearest_non_exact: bool,
) -> Option<EventSubprocessEventSubscription> {
    if let Some(source_execution_id) = source_execution_id {
        let ancestry = execution_ancestry(command_context, source_execution_id);
        for execution_id in ancestry {
            if let Some(position) = subscriptions.iter().position(|subscription| {
                subscription
                    .scope_execution_id
                    .as_deref()
                    .unwrap_or(&subscription.process_instance_id)
                    == execution_id
                    && exact_match(command_context, subscription, thrown_ref)
            }) {
                return Some(subscriptions.remove(position));
            }

            if let Some(position) = subscriptions.iter().position(|subscription| {
                subscription
                    .scope_execution_id
                    .as_deref()
                    .unwrap_or(&subscription.process_instance_id)
                    == execution_id
                    && subscription.event_ref.is_empty()
            }) {
                return Some(subscriptions.remove(position));
            }

            if accept_nearest_non_exact
                && let Some(position) = subscriptions.iter().position(|subscription| {
                    subscription
                        .scope_execution_id
                        .as_deref()
                        .unwrap_or(&subscription.process_instance_id)
                        == execution_id
                })
            {
                return Some(subscriptions.remove(position));
            }
        }
    }

    if let Some(position) = subscriptions
        .iter()
        .position(|subscription| exact_match(command_context, subscription, thrown_ref))
    {
        return Some(subscriptions.remove(position));
    }

    subscriptions.into_iter().next()
}

fn execution_is_descendant_of_scope(
    executions: &std::collections::HashMap<String, Execution>,
    execution: &Execution,
    scope_execution_id: &str,
) -> bool {
    if execution.id == scope_execution_id {
        return true;
    }

    let mut parent_id = execution.parent_id.as_deref();
    for _ in 0..256 {
        let Some(current_id) = parent_id else {
            return false;
        };
        if current_id == scope_execution_id {
            return true;
        }
        parent_id = executions
            .get(current_id)
            .and_then(|parent| parent.parent_id.as_deref());
    }

    false
}

fn activate_interrupting_event_subprocess(
    command_context: &mut CommandContext,
    sub: &EventSubprocessEventSubscription,
) {
    let scope_execution_id = sub
        .scope_execution_id
        .as_deref()
        .unwrap_or(&sub.process_instance_id);

    // Java `EventSubProcess*StartEventActivityBehavior#trigger`:
    // `DeleteReason.EVENT_SUBPROCESS_INTERRUPTING + "(" + startEvent.getId() + ")"`
    let delete_reason =
        crate::history::delete_reason::event_subprocess_interrupting(&sub.start_event_id);

    let all_executions = command_context
        .runtime_store
        .snapshot_executions(&mut command_context.session);

    // Cancel host executions in the subscribed scope. Sibling scopes stay alive.
    let host_executions: Vec<_> = command_context
        .runtime_store
        .snapshot_executions(&mut command_context.session)
        .into_values()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(&sub.process_instance_id)
                && execution_is_descendant_of_scope(&all_executions, execution, scope_execution_id)
        })
        .collect();

    for exec in &host_executions {
        // Java `deleteExecutionAndRelatedData(child, EVENT_SUBPROCESS_INTERRUPTING+(id), …)`
        // records activity end with reason before deleting. Scope row may double
        // as the host (flat tree / userTask reuse) so end it even when retained.
        crate::bpmn::behavior::multi_instance_support::record_activity_end_for_execution(
            command_context,
            &exec.id,
            Some(&delete_reason),
        );

        if let Some(task) = command_context
            .task_entity_manager
            .find_by_execution_id(&exec.id, &mut command_context.session)
        {
            command_context
                .task_entity_manager
                .delete(&task.id, &mut command_context.session);
        }

        command_context
            .runtime_store
            .delete_event_wait_state_by_execution_id(&exec.id, &mut command_context.session);
        command_context
            .runtime_store
            .delete_timer_job_states_by_execution_id(&exec.id, &mut command_context.session);
        command_context
            .runtime_store
            .delete_boundary_event_states_by_host_execution_id(
                &exec.id,
                &mut command_context.session,
            );

        if exec.id != scope_execution_id {
            command_context
                .execution_entity_manager
                .delete(&exec.id, &mut command_context.session);
        } else {
            let mut scope_execution = exec.clone();
            scope_execution.is_active = false;
            if scope_execution.id == sub.process_instance_id {
                scope_execution.activity_id = None;
            }
            command_context
                .execution_entity_manager
                .update(&scope_execution, &mut command_context.session);
        }
    }

    command_context
        .runtime_store
        .delete_event_subprocess_event_subscriptions_by_scope_execution_id(
            scope_execution_id,
            &mut command_context.session,
        );

    inject_event_subprocess_execution(command_context, sub, false);
}

fn activate_non_interrupting_event_subprocess(
    command_context: &mut CommandContext,
    sub: &EventSubprocessEventSubscription,
) {
    inject_event_subprocess_execution(command_context, sub, true);
}

fn inject_event_subprocess_execution(
    command_context: &mut CommandContext,
    sub: &EventSubprocessEventSubscription,
    non_interrupting_path: bool,
) {
    let process_instance = match command_context
        .runtime_store
        .find_process_instance(&sub.process_instance_id, &mut command_context.session)
    {
        Some(pi) => pi,
        None => {
            tracing::error!(
                "Process instance {} not found for event subprocess activation",
                sub.process_instance_id
            );
            return;
        }
    };

    let process_definition_id = process_instance.process_definition_id.clone();
    // Seed from the process-instance scope execution row: it is the single
    // process-level variable store.
    let process_variables = command_context
        .runtime_store
        .find_execution(&process_instance.id, &mut command_context.session)
        .map(|root_execution| root_execution.variables)
        .unwrap_or_default();
    let scope_execution = sub
        .scope_execution_id
        .as_deref()
        .and_then(|scope_execution_id| {
            command_context
                .runtime_store
                .find_execution(scope_execution_id, &mut command_context.session)
        });

    // For non-interrupting event subprocess paths, use the process instance as parent
    // to avoid being counted as a sibling of the triggering execution
    let parent_id = if non_interrupting_path {
        process_instance.id.clone()
    } else {
        scope_execution
            .as_ref()
            .map(|execution| execution.id.clone())
            .unwrap_or_else(|| process_instance.id.clone())
    };

    let mut start_event_execution = Execution {
        id: Uuid::new_v4().to_string(),
        parent_id: Some(parent_id),
        super_execution_id: None,
        root_process_instance_id: Some(process_instance.id.clone()),
        process_instance_id: Some(process_instance.id.clone()),
        process_definition_id: Some(process_definition_id),
        process_definition_key: Some(process_instance.process_definition_key.clone()),
        process_definition_name: None,
        process_definition_version: Some(process_instance.process_definition_version),
        activity_id: Some(sub.start_event_id.clone()),
        activity_name: None,
        name: None,
        description: None,
        is_suspended: false,
        is_ended: false,
        is_active: true,
        is_concurrent: false,
        is_scope: true,
        is_multi_instance_root: false,
        tenant_id: process_instance.tenant_id.clone(),
        variables: process_variables,
        ..Default::default()
    };
    if non_interrupting_path {
        // Structural flag (not a variable): must survive commit so a later
        // end-event command still knows this path is non-interrupting host-safe.
        // P41 briefly used transient_variables; P45 strips those on commit.
        start_event_execution.non_interrupting_event_subprocess_path = true;
    }

    command_context
        .execution_entity_manager
        .insert(&start_event_execution, &mut command_context.session);

    command_context
        .agenda
        .plan_continue_process_operation(start_event_execution);
}
