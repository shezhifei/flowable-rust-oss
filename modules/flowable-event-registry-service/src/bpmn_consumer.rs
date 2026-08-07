//! BPMN Event Registry inbound consumer.
//!
//! Java: `BpmnEventRegistryEventConsumer.java:62-64` (consumer key),
//! `:78-117` (eventReceived → handleEventSubscription),
//! registered from `ProcessEngineConfigurationImpl.java:1608-1616` into
//! `DefaultEventRegistry.java:132-134`.
//!
//! Rust maps each matching `EventSubscriptionKind::EventRegistry` wait-state
//! onto the existing typed trigger commands (no generic `runtimeService.trigger`).
//!
//! Tenant filter for wait-states / process instances follows
//! `BaseEventRegistryEventConsumer.java:177-265` via the shared helper (P122).

use crate::models::{EventDefinition, EventInstanceDelivery};
use crate::pipeline::InboundEventConsumer;
use crate::tenant_fallback::{
    dedup_definition_level_subscriptions_by_key, subscription_matches_event_tenant,
    TenantFallbackPolicy,
};
use flowable_engine::bpmn::event_registry_correlation::{
    correlation_params_from_payload, generate_event_correlation_keys,
    matches_subscription_configuration,
};
use flowable_engine::cmd::trigger_start_event_subscription_cmd::{
    TriggerEventSubprocessByEventCmd, TriggerProcessStartByEventCmd,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use flowable_engine::persistence::runtime_store::{
    EventSubscriptionKind, ProcessEventStartSubscription, RuntimeEventWaitKind,
};
use std::sync::Arc;

/// Java consumer key: `BpmnEventRegistryEventConsumer.getConsumerKey()` → `"bpmnEventConsumer"`.
pub const BPMN_EVENT_CONSUMER_KEY: &str = "bpmnEventConsumer";

/// Inbound consumer that bridges Event Registry deliveries into BPMN wait-states.
pub struct BpmnEventRegistryConsumer {
    engine: Arc<ProcessEngine>,
    tenant_fallback: TenantFallbackPolicy,
}

impl BpmnEventRegistryConsumer {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        Self::with_tenant_fallback(engine, TenantFallbackPolicy::default())
    }

    pub fn with_tenant_fallback(
        engine: Arc<ProcessEngine>,
        tenant_fallback: TenantFallbackPolicy,
    ) -> Self {
        Self {
            engine,
            tenant_fallback,
        }
    }

    pub fn engine(&self) -> &Arc<ProcessEngine> {
        &self.engine
    }
}

impl InboundEventConsumer for BpmnEventRegistryConsumer {
    fn consume(
        &self,
        delivery: &EventInstanceDelivery,
        _definition: &EventDefinition,
    ) -> Result<(), FlowableError> {
        // Match subscriptions by event definition key (the `flowable:eventType` value).
        // Java BpmnEventRegistryEventConsumer finds subscriptions by event type.
        let event_key = delivery.event_definition_key.as_str();
        if event_key.is_empty() {
            return Ok(());
        }
        let tenant_id = delivery.tenant_id.as_deref();
        let policy = &self.tenant_fallback;

        // Correlation keys from inbound payload
        // (BaseEventRegistryEventConsumer.generateCorrelationKeys:76-131 /
        // findEventSubscriptions:163-174 — configuration IS NULL OR IN keys).
        // P134: required for send-event triggerEventCorrelationParameter matching.
        let correlation_params = correlation_params_from_payload(&delivery.payload);
        let correlation_keys = generate_event_correlation_keys(&correlation_params);

        // Snapshot wait-states and process instances outside command transactions
        // (each trigger is its own command — Java also handles one subscription
        // per transaction: BpmnEventRegistryEventConsumer.java:81-84).
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session().map_err(FlowableError::from)?;
        let mut wait_states: Vec<_> = store
            .snapshot_event_wait_states(&mut session)
            .into_values()
            .filter(|ws| {
                ws.event_subscription.as_ref().is_some_and(|sub| {
                    sub.kind == EventSubscriptionKind::EventRegistry && sub.event_ref == event_key
                }) && matches_subscription_configuration(
                    ws.configuration.as_deref(),
                    &correlation_keys,
                )
            })
            .collect();
        wait_states.sort_by(|a, b| a.execution_id.cmp(&b.execution_id));

        let mut process_instance_ids: Vec<String> = store
            .snapshot_process_instances(&mut session)
            .into_iter()
            .filter(|(_, pi)| !pi.is_ended)
            .filter(|(_, pi)| {
                // Boundary / event-subprocess targets are instance-level
                // (BaseEventRegistryEventConsumer.java:198-201).
                subscription_matches_event_tenant(
                    tenant_id,
                    pi.tenant_id.as_deref(),
                    true,
                    policy,
                )
            })
            .map(|(id, _)| id)
            .collect();
        process_instance_ids.sort();
        drop(session);

        let runtime = self.engine.get_runtime_service();
        let task_service = self.engine.get_task_service();
        let executor = self.engine.get_command_executor();

        // 1) Intermediate catch + receive task wait-states
        //    (correlate_message_cmd.rs:40-53 wait-state scan paradigm).
        for wait_state in wait_states {
            // Instance-level: PI tenant must match event tenant under the policy
            // (BaseEventRegistryEventConsumer.java:198-201).
            let store = self.engine.get_runtime_store();
            let mut session = store.create_session().map_err(FlowableError::from)?;
            let pi_tenant = store
                .find_process_instance(&wait_state.process_instance_id, &mut session)
                .and_then(|pi| pi.tenant_id);
            drop(session);
            if !subscription_matches_event_tenant(
                tenant_id,
                pi_tenant.as_deref(),
                true,
                policy,
            ) {
                continue;
            }

            match wait_state.wait_kind {
                RuntimeEventWaitKind::ReceiveTask => {
                    // correlate_message_cmd.rs:221-242 — complete path when a Task exists.
                    if let Some(task_id) = wait_state.task_id.clone() {
                        if task_service.complete_task_by_id(task_id).is_ok() {
                            continue;
                        }
                    }
                    // Event-registry receive has no Task (ReceiveEventTaskActivityBehavior).
                    runtime.trigger_event_intermediate_catch(
                        EventSubscriptionKind::EventRegistry,
                        event_key.to_string(),
                        wait_state.execution_id.clone(),
                    );
                }
                // P130: send-event triggerable wait → TriggerCmd/TriggerExecutionOperation
                // (Java BpmnEventRegistryEventConsumer.java:103,116 → runtimeService.trigger
                // with EVENT_INSTANCE payload for SendEventTaskActivityBehavior#trigger).
                // P134: pass pipeline delivery id so trigger updates that row
                // instead of inserting a second event-instance delivery.
                RuntimeEventWaitKind::SendEventTask => {
                    let _ = runtime.trigger_send_event_service_task_with_delivery(
                        wait_state.execution_id.clone(),
                        event_key.to_string(),
                        delivery.payload.clone(),
                        delivery.id.clone(),
                    );
                }
                _ => {
                    // Intermediate catch (EventRegistryIntermediateCatchEvent) and any
                    // other wait kinds carrying an EventRegistry subscription.
                    runtime.trigger_event_intermediate_catch(
                        EventSubscriptionKind::EventRegistry,
                        event_key.to_string(),
                        wait_state.execution_id.clone(),
                    );
                }
            }
        }

        // 2) Boundary events on running process instances
        for pi_id in &process_instance_ids {
            runtime.trigger_boundary_event_by_event_ref(
                EventSubscriptionKind::EventRegistry,
                event_key.to_string(),
                pi_id.clone(),
            );
        }

        // 3) Event subprocesses
        for pi_id in &process_instance_ids {
            let esp_cmd = TriggerEventSubprocessByEventCmd::new(
                EventSubscriptionKind::EventRegistry,
                event_key.to_string(),
                pi_id.clone(),
            );
            let _ = executor.execute(&esp_cmd);
        }

        // 4) Process-level start subscriptions
        //    (BpmnEventRegistryEventConsumer.java:228-270 start path).
        //    Unique-instance correlation is out of P92 scope (P93).
        //
        // P136: collect [event-tenant, tenantless-default] matching start subs →
        // definition-key dedup (drop tenantless same-key) → one start per surviving
        // subscription (Java :81-84 one sub one tx). Fixes prior under-delivery that
        // short-circuited after the first exact-tenant hit.
        trigger_process_start(
            self.engine.as_ref(),
            executor.as_ref(),
            event_key,
            tenant_id,
            policy,
            &correlation_keys,
        );

        Ok(())
    }
}

/// Collect matching process start subscriptions, apply tenant filter + key dedup,
/// then trigger each surviving subscription once
/// (BaseEventRegistryEventConsumer.java:177-268 + BpmnEventRegistryEventConsumer start path).
fn trigger_process_start(
    engine: &ProcessEngine,
    executor: &DefaultCommandExecutor,
    event_key: &str,
    tenant_id: Option<&str>,
    policy: &TenantFallbackPolicy,
    correlation_keys: &[String],
) {
    let mut subs: Vec<ProcessEventStartSubscription> = engine
        .get_event_start_subscriptions()
        .into_iter()
        .filter(|sub| {
            sub.event_kind == EventSubscriptionKind::EventRegistry && sub.event_ref == event_key
        })
        .filter(|sub| {
            matches_subscription_configuration(sub.configuration.as_deref(), correlation_keys)
        })
        .collect();

    // Tenant match for definition-level start subs (always definition-level here).
    let event_tenant = policy.normalize_tenant(tenant_id);
    subs.retain(|sub| {
        subscription_matches_event_tenant(
            event_tenant,
            sub.tenant_id.as_deref(),
            false, // definition-level
            policy,
        )
    });

    // Key-based dedup only when fallback + tenantless default
    // (BaseEventRegistryEventConsumer.java:186-255). When defaultTenant is a real
    // tenant (:257-260) both tenants are queried but no key dedup applies.
    if policy.fallback_to_default_tenant && policy.default_is_tenantless() && event_tenant.is_some()
    {
        subs = dedup_definition_level_subscriptions_by_key(
            subs,
            |sub| (true, sub.tenant_id.clone()),
            |sub| Some(sub.process_definition_key.clone()),
        );
    }

    // Stable order for determinism.
    subs.sort_by(|a, b| {
        a.process_definition_id
            .cmp(&b.process_definition_id)
            .then(a.start_event_id.cmp(&b.start_event_id))
    });

    // One command per subscription (Java :81-84).
    for sub in subs {
        let mut cmd = TriggerProcessStartByEventCmd::new(
            EventSubscriptionKind::EventRegistry,
            event_key.to_string(),
        )
        .with_process_definition_id(sub.process_definition_id.clone());
        // Instance tenant override: use event tenant when present
        // (BpmnEventRegistryEventConsumer.startProcessInstance.java:238-240).
        if let Some(tenant) = event_tenant {
            cmd = cmd.with_tenant_id(tenant.to_string());
        } else if let Some(sub_tenant) = sub.tenant_id.clone().filter(|t| !t.is_empty()) {
            cmd = cmd.with_tenant_id(sub_tenant);
        }
        let _ = executor.execute(&cmd);
    }
}
