//! CMMN Event Registry inbound consumer.
//!
//! Java: `CmmnEventRegistryEventConsumer.java:63-66` (consumer key),
//! `:80-106` (eventReceived → find subscriptions → handle each),
//! `:108-136` (plan-item trigger path via occur),
//! `:138-278` (definition-level case start path — P136),
//! registered from `CmmnEngineConfiguration.java:1358-1365` into the
//! event-registry consumer map (`DefaultEventRegistry.java:132-134`).
//!
//! Subscription match keys (`BaseEventRegistryEventConsumer.findEventSubscriptions:156-175`):
//! - eventType = event definition key
//! - scopeType = CMMN (implicit: we query CMMN subscription store only)
//! - configuration IS NULL (broadcast) OR configuration IN power-set correlation keys
//!
//! Tenant filter: shared helper implementing
//! `BaseEventRegistryEventConsumer.java:177-265` (P122).
//! Tenant definition-key dedup: `BaseEventRegistryEventConsumer.java:203-253` (P136).
//!
//! No-match → silent discard (Java returns empty EventRegistryProcessingInfo).
//! Each subscription is handled independently (Java :82-85: one per subscription,
//! no overarching transaction).

use crate::models::{EventDefinition, EventInstanceDelivery};
use crate::pipeline::InboundEventConsumer;
use crate::tenant_fallback::{
    dedup_definition_level_subscriptions_by_key, subscription_matches_event_tenant,
    TenantFallbackPolicy,
};
use flowable_cmmn_engine::{
    correlation_params_from_payload, generate_correlation_key, generate_event_correlation_keys,
    matches_subscription_configuration, CmmnCaseInstanceStartRequest, CmmnEngine, CmmnError,
    CmmnEventSubscription, REFERENCE_TYPE_EVENT_CASE,
    START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID,
};
use flowable_engine::error::FlowableError;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Java consumer key: `CmmnEventRegistryEventConsumer.getConsumerKey()` → `"cmmnEventConsumer"`.
pub const CMMN_EVENT_CONSUMER_KEY: &str = "cmmnEventConsumer";

/// Inbound consumer that bridges Event Registry deliveries into CMMN event subscriptions.
pub struct CmmnEventRegistryConsumer {
    cmmn_engine: Arc<CmmnEngine>,
    tenant_fallback: TenantFallbackPolicy,
}

impl CmmnEventRegistryConsumer {
    pub fn new(cmmn_engine: Arc<CmmnEngine>) -> Self {
        Self::with_tenant_fallback(cmmn_engine, TenantFallbackPolicy::default())
    }

    pub fn with_tenant_fallback(
        cmmn_engine: Arc<CmmnEngine>,
        tenant_fallback: TenantFallbackPolicy,
    ) -> Self {
        Self {
            cmmn_engine,
            tenant_fallback,
        }
    }

    pub fn cmmn_engine(&self) -> &Arc<CmmnEngine> {
        &self.cmmn_engine
    }
}

impl InboundEventConsumer for CmmnEventRegistryConsumer {
    fn consume(
        &self,
        delivery: &EventInstanceDelivery,
        _definition: &EventDefinition,
    ) -> Result<(), FlowableError> {
        // Match key = event definition key (Java eventInstance.getEventKey(),
        // EventRegistryEventListenerActivityBehaviour.java:146 stores that as eventType).
        let event_key = delivery.event_definition_key.as_str();
        if event_key.is_empty() {
            return Ok(());
        }

        // Correlation keys from inbound payload
        // (BaseEventRegistryEventConsumer.generateCorrelationKeys:76-131).
        // Without event-model correlationParameter flags we use all payload fields;
        // power-set matching still hits subscriptions that used a subset of params.
        let correlation_params = correlation_params_from_payload(&delivery.payload);
        let correlation_keys = generate_event_correlation_keys(&correlation_params);

        let runtime = self.cmmn_engine.runtime_service();
        // Snapshot matching subscriptions, then trigger one-by-one
        // (CmmnEventRegistryEventConsumer.java:82-85 / :95-103).
        let mut subscriptions = runtime
            .create_event_subscription_query()
            .event_type(event_key)
            .list()
            .map_err(map_cmmn_error)?;

        // Tenant filter (BaseEventRegistryEventConsumer.java:177-265 via shared helper).
        // Instance-level = case or plan-item bound; definition-level = neither.
        let event_tenant = delivery.tenant_id.as_deref();
        subscriptions.retain(|sub| {
            let is_instance_level =
                sub.plan_item_instance_id.is_some() || sub.case_instance_id.is_some();
            subscription_matches_event_tenant(
                event_tenant,
                sub.tenant_id.as_deref(),
                is_instance_level,
                &self.tenant_fallback,
            )
        });

        subscriptions.retain(|sub| {
            matches_subscription_configuration(sub.configuration.as_deref(), &correlation_keys)
        });

        // P136: definition-key dedup when fallback is on and default is tenantless
        // (BaseEventRegistryEventConsumer.java:203-253). Drop tenantless def-level
        // subs whose case definition key is already covered by a tenant-exact sub.
        if self.tenant_fallback.fallback_to_default_tenant
            && self.tenant_fallback.default_is_tenantless()
            && event_tenant.is_some()
        {
            let engine = self.cmmn_engine.clone();
            subscriptions = dedup_definition_level_subscriptions_by_key(
                subscriptions,
                |sub| {
                    let is_instance =
                        sub.plan_item_instance_id.is_some() || sub.case_instance_id.is_some();
                    (!is_instance, sub.tenant_id.clone())
                },
                |sub| {
                    sub.case_definition_id.as_ref().and_then(|def_id| {
                        engine
                            .repository_service()
                            .get_case_definition(def_id)
                            .ok()
                            .map(|d| d.key)
                    })
                },
            );
        }

        // Stable order for determinism.
        subscriptions.sort_by(|a, b| a.id.cmp(&b.id));

        for subscription in subscriptions {
            // Definition-level start path: case_definition_id set, no instance scope
            // (CmmnEventRegistryEventConsumer.java:138-221).
            if subscription.plan_item_instance_id.is_none()
                && subscription.case_instance_id.is_none()
            {
                if subscription.case_definition_id.is_some() {
                    handle_definition_level_start(
                        &runtime,
                        &self.cmmn_engine,
                        &subscription,
                        delivery,
                        &correlation_params,
                    )?;
                }
                continue;
            }

            // Plan-item waiting path: subScopeId present
            // (CmmnEventRegistryEventConsumer.java:111-136).
            // Apply payload → variables via out-params, then occur.
            // Java: transientVariable(EVENT_INSTANCE) + trigger()
            // (CmmnEventRegistryEventConsumer.java:128-129).
            match runtime.occur_event_subscription_with_payload(
                &subscription.id,
                Some(&delivery.payload),
            ) {
                Ok(()) => {}
                // Already deleted / concurrent occur: treat as no-op (idempotent).
                Err(CmmnError::NotFound { .. }) => {}
                Err(err) => return Err(map_cmmn_error(err)),
            }
        }

        Ok(())
    }
}

/// Java `CmmnEventRegistryEventConsumer` definition-level start
/// (CmmnEventRegistryEventConsumer.java:138-278).
///
/// Divergences (intentional):
/// - No LockManager unique-instance check (Java :158-210) — Rust engine is single-writer.
/// - No startAsync (Java :257-267) — only synchronous start.
/// - Payload is not mapped to case variables; Java only exposes EVENT_INSTANCE as a
///   transient variable (:245). Rust has no transient-variable concept on start, so
///   we start with empty variables (same observable persistent state).
fn handle_definition_level_start(
    runtime: &flowable_cmmn_engine::CmmnRuntimeService,
    cmmn_engine: &CmmnEngine,
    subscription: &CmmnEventSubscription,
    delivery: &EventInstanceDelivery,
    correlation_params: &BTreeMap<String, Option<String>>,
) -> Result<(), FlowableError> {
    let case_definition_id = subscription
        .case_definition_id
        .as_deref()
        .ok_or_else(|| FlowableError::Internal("definition-level sub missing case_definition_id".into()))?;

    let case_definition = cmmn_engine
        .repository_service()
        .get_case_definition(case_definition_id)
        .map_err(map_cmmn_error)?;

    // storeAsUniqueReferenceId: count existing instances by key + referenceId/type
    // (CmmnEventRegistryEventConsumer.java:142-217, :226-239).
    let start_cfg = case_definition
        .model
        .start_correlation_configuration
        .as_deref();
    let unique_ref = start_cfg == Some(START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID);

    let correlation_key_for_ref = if unique_ref && !correlation_params.is_empty() {
        Some(generate_correlation_key(correlation_params))
    } else {
        None
    };

    if unique_ref {
        if let Some(ref_id) = correlation_key_for_ref.as_deref() {
            let mut query = runtime
                .create_case_instance_query()
                .case_definition_key(case_definition.key.clone())
                .reference_id(ref_id.to_string())
                .reference_type(REFERENCE_TYPE_EVENT_CASE.to_string());
            if let Some(tenant) = delivery.tenant_id.as_deref().filter(|t| !t.is_empty()) {
                query = query.tenant_id(tenant.to_string());
            }
            let count = query.count().map_err(map_cmmn_error)?;
            if count > 0 {
                // Existing unique instance — do not start another
                // (CmmnEventRegistryEventConsumer.java:152-156).
                return Ok(());
            }
        }
    }

    let mut request = CmmnCaseInstanceStartRequest::new();
    // Tenant override on the instance (Java :247-249 overrideCaseDefinitionTenantId).
    if let Some(tenant) = delivery.tenant_id.as_deref().filter(|t| !t.is_empty()) {
        request = request.with_override_definition_tenant_id(tenant.to_string());
    }
    if let Some(ref_id) = correlation_key_for_ref {
        request = request
            .with_reference_id(ref_id)
            .with_reference_type(REFERENCE_TYPE_EVENT_CASE.to_string());
    }

    runtime
        .start_case_instance_by_id(case_definition_id, request)
        .map_err(map_cmmn_error)?;
    Ok(())
}

fn map_cmmn_error(error: CmmnError) -> FlowableError {
    match error {
        CmmnError::NotFound { message } => FlowableError::NotFound(message),
        CmmnError::Storage { message } => FlowableError::Internal(message),
        CmmnError::Validation { message }
        | CmmnError::UnsupportedModel { message, .. }
        | CmmnError::Execution { message } => FlowableError::BadRequest(message),
        CmmnError::Conflict { message } => FlowableError::Forbidden(message),
        CmmnError::NonUniqueResult { query, count } => {
            FlowableError::Internal(format!("non-unique result for {query}: found {count}"))
        }
    }
}
