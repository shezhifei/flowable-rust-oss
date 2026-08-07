//! Production trigger for send-event service task wait states.
//!
//! Java chain:
//! - `BpmnEventRegistryEventConsumer.java:78,103,116` →
//!   `runtimeService.trigger(executionId, null, transientVariableMap)`
//! - `TriggerCmd.java:78` → `TriggerExecutionOperation.java:51-59`
//!   (current FlowNode behavior as `TriggerableActivityBehavior#trigger`)
//! - `SendEventTaskActivityBehavior.java:230-265` (out params + delete subscription + leave)
//!
//! Rust maps this onto [`ServiceTaskActivityBehavior::trigger`], which already
//! implements `trigger_send_event_service_task` (map out params, record inbound,
//! delete wait state, take outgoing).

use crate::bpmn::behavior::service_task_activity_behavior::ServiceTaskActivityBehavior;
use crate::cmd::trigger_intermediate_catch_event_cmd::require_active_execution;
use crate::delegate::activity_behavior::TriggerableActivityBehavior;
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{EventSubscriptionKind, RuntimeEventWaitKind};
use serde_json::Value;

/// Session-cache key for the inbound Event Registry delivery id that owns this
/// trigger (P134 dual-record merge). When set, the send-event trigger path
/// updates that delivery instead of inserting a second row.
pub const INBOUND_EVENT_DELIVERY_ID_CACHE_KEY: &str =
    "flowable.eventRegistry.inboundDeliveryId";

/// Triggers a waiting send-event (triggerable) service task with inbound payload.
///
/// Corresponds to Java `TriggerCmd` + `TriggerExecutionOperation` for the
/// send-event activity behavior path.
pub struct TriggerSendEventServiceTaskCmd {
    execution_id: String,
    event_key: String,
    payload: Value,
    /// Pipeline delivery id when called from `BpmnEventRegistryConsumer`.
    /// Absent for direct `behavior.trigger` / unit-test paths.
    inbound_delivery_id: Option<String>,
}

impl TriggerSendEventServiceTaskCmd {
    pub fn new(execution_id: impl Into<String>, event_key: impl Into<String>, payload: Value) -> Self {
        Self {
            execution_id: execution_id.into(),
            event_key: event_key.into(),
            payload,
            inbound_delivery_id: None,
        }
    }

    /// Attach the inbound pipeline delivery so the trigger path reuses it
    /// (P134: one delivery record per logical event — Java has no dual insert).
    pub fn with_inbound_delivery_id(mut self, delivery_id: impl Into<String>) -> Self {
        self.inbound_delivery_id = Some(delivery_id.into());
        self
    }
}

impl Command<()> for TriggerSendEventServiceTaskCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        let wait_state = command_context
            .runtime_store
            .find_event_wait_state_by_execution_id(&self.execution_id, &mut command_context.session);

        let Some(wait_state) = wait_state else {
            tracing::warn!(
                "No send-event wait state for execution id {}",
                self.execution_id
            );
            return Ok(());
        };

        if wait_state.wait_kind != RuntimeEventWaitKind::SendEventTask {
            tracing::warn!(
                "Execution {} wait kind is {:?}, expected SendEventTask",
                self.execution_id,
                wait_state.wait_kind
            );
            return Ok(());
        }

        let subscription_ok = wait_state.event_subscription.as_ref().is_some_and(|sub| {
            sub.kind == EventSubscriptionKind::EventRegistry && sub.event_ref == self.event_key
        });
        if !subscription_ok {
            tracing::warn!(
                "Send-event wait on execution {} does not match event key {}",
                self.execution_id,
                self.event_key
            );
            return Ok(());
        }

        let mut execution = command_context
            .execution_entity_manager
            .find_by_id(&self.execution_id, &mut command_context.session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "No execution could be found for id {}",
                    self.execution_id
                ))
            })?;

        // Java NeedsActiveExecutionCmd / TriggerCmd — reject suspended executions.
        require_active_execution(&execution)?;

        // P134: stash pipeline delivery id for record_inbound_event_registry_delivery.
        if let Some(delivery_id) = self.inbound_delivery_id.as_ref() {
            command_context.session_caches.insert(
                INBOUND_EVENT_DELIVERY_ID_CACHE_KEY.to_string(),
                Box::new(delivery_id.clone()),
            );
        }

        // Java TriggerExecutionOperation: behavior.trigger(execution, signalName, signalData)
        // with EVENT_INSTANCE carried as transient data; Rust passes event key + payload.
        let trigger_result = ServiceTaskActivityBehavior::new().trigger(
            &mut execution,
            command_context,
            Some(self.event_key.clone()),
            Some(self.payload.clone()),
        );

        // Always clear so a failed trigger does not leak into later commands
        // on a reused context (session is command-scoped in practice).
        command_context
            .session_caches
            .remove(INBOUND_EVENT_DELIVERY_ID_CACHE_KEY);

        trigger_result
    }
}
