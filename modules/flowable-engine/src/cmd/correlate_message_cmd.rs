use crate::cmd::trigger_boundary_event_cmd::TriggerBoundaryEventByEventRefCmd;
use crate::cmd::trigger_intermediate_catch_event_cmd::TriggerEventIntermediateCatchCmd;
use crate::cmd::trigger_start_event_subscription_cmd::TriggerProcessStartByEventCmd;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{EventSubscriptionKind, RuntimeEventWaitKind};
use crate::runtime::execution::Execution;
use crate::runtime::process_instance::ProcessInstance;
use serde_json::Value;
use std::collections::HashMap;

fn is_message_wait_kind(kind: &RuntimeEventWaitKind) -> bool {
    matches!(
        kind,
        RuntimeEventWaitKind::MessageIntermediateCatchEvent | RuntimeEventWaitKind::ReceiveTask
    )
}

fn matches_message_ref(
    wait_state: &crate::persistence::runtime_store::RuntimeEventWaitState,
    message_name: &str,
) -> bool {
    wait_state.event_subscription.as_ref().is_some_and(|sub| {
        sub.kind == EventSubscriptionKind::Message && sub.event_ref == message_name
    })
}

struct CorrelateMessageTarget {
    execution_id: String,
    process_instance_id: String,
}

fn find_wait_state_target(
    command_context: &mut CommandContext,
    message_name: &str,
    process_instance_id: Option<&str>,
    business_key: Option<&str>,
    tenant_id: Option<&str>,
) -> Option<CorrelateMessageTarget> {
    let mut candidates: Vec<_> = command_context
        .runtime_store
        .snapshot_event_wait_states(&mut command_context.session)
        .into_values()
        .filter(|ws| is_message_wait_kind(&ws.wait_kind))
        .filter(|ws| matches_message_ref(ws, message_name))
        .filter(|ws| {
            if let Some(pi_id) = process_instance_id {
                ws.process_instance_id == pi_id
            } else {
                true
            }
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Apply business_key / tenant_id filters by looking up the process instance
    if business_key.is_some() || tenant_id.is_some() {
        candidates.retain(|ws| {
            if let Some(pi) = command_context
                .runtime_store
                .find_process_instance(&ws.process_instance_id, &mut command_context.session)
            {
                if let Some(bk) = business_key
                    && pi.business_key.as_deref() != Some(bk)
                {
                    return false;
                }
                if let Some(tid) = tenant_id
                    && pi.tenant_id.as_deref() != Some(tid)
                {
                    return false;
                }
                true
            } else {
                false
            }
        });
    }

    if candidates.is_empty() {
        return None;
    }

    // Deterministic: sort by execution_id, take the first deterministically
    candidates.sort_by(|a, b| a.execution_id.cmp(&b.execution_id));

    candidates.first().map(|ws| CorrelateMessageTarget {
        execution_id: ws.execution_id.clone(),
        process_instance_id: ws.process_instance_id.clone(),
    })
}

fn write_correlation_variables(
    command_context: &mut CommandContext,
    execution: &Execution,
    variables: &HashMap<String, Value>,
) {
    if variables.is_empty() {
        return;
    }

    let process_instance_id = execution
        .process_instance_id
        .clone()
        .unwrap_or_else(|| execution.id.clone());

    for (name, value) in variables {
        let variable_id = format!("{}:{}", execution.id, name);
        if command_context
            .runtime_store
            .get_historic_variable_instance(&variable_id, &mut command_context.session)
            .is_some()
        {
            command_context.history_manager.record_variable_updated(
                &variable_id,
                value.clone(),
                &mut command_context.session,
            );
        } else {
            command_context.history_manager.record_variable_created(
                &variable_id,
                name,
                crate::engine::variable_service::variable_type_name(value),
                value.clone(),
                &process_instance_id,
                Some(&execution.id),
                None,
                &mut command_context.session,
            );
        }
    }
}

/// Options for message correlation.
#[derive(Clone, Default)]
pub struct CorrelateMessageOptions {
    pub process_instance_id: Option<String>,
    pub business_key: Option<String>,
    pub tenant_id: Option<String>,
    pub variables: HashMap<String, Value>,
    /// If true and no match is found among running instances, start a new process instance.
    pub start_new_if_no_match: bool,
}

/// Unified message correlation command.
///
/// Searches across all running process instances for a matching message wait state
/// (intermediate catch event or receive task) and triggers it.
/// Supports optional filters: process_instance_id, business_key, tenant_id.
/// Falls back to starting a new process instance if `start_new_if_no_match` is set.
pub struct CorrelateMessageCmd {
    message_name: String,
    options: CorrelateMessageOptions,
}

impl CorrelateMessageCmd {
    pub fn new(message_name: String, options: CorrelateMessageOptions) -> Self {
        Self {
            message_name,
            options,
        }
    }
}

/// Result of a message correlation.
#[allow(clippy::large_enum_variant)]
pub enum CorrelateMessageResult {
    /// A waiting execution was found and triggered.
    MatchedExecution {
        execution_id: String,
        process_instance_id: String,
    },
    /// No match found; a new process instance was started.
    StartedProcess(ProcessInstance),
    /// No match found and start_new_if_no_match was false.
    NoMatch,
}

impl Command<CorrelateMessageResult> for CorrelateMessageCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<CorrelateMessageResult, crate::error::FlowableError> {
        // Phase 1: Search running instances for matching message wait state
        let target = find_wait_state_target(
            command_context,
            &self.message_name,
            self.options.process_instance_id.as_deref(),
            self.options.business_key.as_deref(),
            self.options.tenant_id.as_deref(),
        );

        if let Some(target) = target {
            // Write correlation variables to the matched execution
            if !self.options.variables.is_empty()
                && let Some(execution) = command_context
                    .execution_entity_manager
                    .find_by_id(&target.execution_id, &mut command_context.session)
            {
                let mut exec = execution.clone();
                for (name, value) in &self.options.variables {
                    exec.set_process_variable(name.clone(), value.clone());
                }
                command_context
                    .execution_entity_manager
                    .update(&exec, &mut command_context.session);
                write_correlation_variables(command_context, &exec, &self.options.variables);
            }

            // Check if this is a receive task (needs task completion) or intermediate catch
            let wait_state = command_context
                .runtime_store
                .find_event_wait_state_by_execution_id(
                    &target.execution_id,
                    &mut command_context.session,
                );

            match wait_state.as_ref().map(|ws| &ws.wait_kind) {
                Some(RuntimeEventWaitKind::ReceiveTask) => {
                    // For receive tasks, complete the associated task
                    if let Some(task_id) = wait_state.as_ref().and_then(|ws| ws.task_id.as_deref())
                        && let Some(task) = command_context
                            .task_entity_manager
                            .find_task_by_id(task_id, &mut command_context.session)
                    {
                        crate::engine::task_service::complete_task_internal(command_context, task)?;
                    }
                }
                _ => {
                    // For intermediate catch events, trigger via the unified command
                    let cmd = TriggerEventIntermediateCatchCmd::with_variables(
                        EventSubscriptionKind::Message,
                        self.message_name.clone(),
                        target.execution_id.clone(),
                        self.options.variables.clone(),
                    );
                    cmd.execute(command_context)?;
                }
            }

            return Ok(CorrelateMessageResult::MatchedExecution {
                execution_id: target.execution_id,
                process_instance_id: target.process_instance_id,
            });
        }

        // Phase 2: No match in running instances — check boundary events and event subprocesses
        let all_pis: Vec<String> = command_context
            .runtime_store
            .snapshot_process_instances(&mut command_context.session)
            .into_iter()
            .filter(|(_, pi)| !pi.is_ended)
            .filter(|(_, pi)| {
                if let Some(pi_id) = &self.options.process_instance_id {
                    pi.id == *pi_id
                } else {
                    true
                }
            })
            .filter(|(_, pi)| {
                if let Some(bk) = &self.options.business_key {
                    pi.business_key.as_deref() == Some(bk.as_str())
                } else {
                    true
                }
            })
            .filter(|(_, pi)| {
                if let Some(tid) = &self.options.tenant_id {
                    pi.tenant_id.as_deref() == Some(tid.as_str())
                } else {
                    true
                }
            })
            .map(|(id, _)| id)
            .collect();

        // Try boundary events first
        for pi_id in &all_pis {
            let boundary_cmd = TriggerBoundaryEventByEventRefCmd::new(
                EventSubscriptionKind::Message,
                self.message_name.clone(),
                pi_id.clone(),
            );
            let triggered = boundary_cmd
                .execute_with_trigger_result(command_context)
                .unwrap_or(false);
            if triggered {
                return Ok(CorrelateMessageResult::MatchedExecution {
                    execution_id: String::new(),
                    process_instance_id: pi_id.clone(),
                });
            }
        }

        // Try event subprocesses
        for pi_id in &all_pis {
            let event_sub_cmd = crate::cmd::trigger_start_event_subscription_cmd::TriggerEventSubprocessByEventCmd::new(
                EventSubscriptionKind::Message,
                self.message_name.clone(),
                pi_id.clone(),
            );
            let triggered_ids = event_sub_cmd.execute(command_context).unwrap_or_default();
            if !triggered_ids.is_empty() {
                return Ok(CorrelateMessageResult::MatchedExecution {
                    execution_id: String::new(),
                    process_instance_id: pi_id.clone(),
                });
            }
        }

        // Phase 3: Start new process instance if allowed
        if self.options.start_new_if_no_match {
            let mut cmd = TriggerProcessStartByEventCmd::new(
                EventSubscriptionKind::Message,
                self.message_name.clone(),
            )
            .with_variables(self.options.variables.clone());
            if let Some(tenant_id) = &self.options.tenant_id {
                cmd = cmd.with_tenant_id(tenant_id.clone());
            }
            if let Some(business_key) = &self.options.business_key {
                cmd = cmd.with_business_key(business_key.clone());
            }
            let result = cmd.execute(command_context)?;
            return Ok(CorrelateMessageResult::StartedProcess(result));
        }

        Ok(CorrelateMessageResult::NoMatch)
    }
}
