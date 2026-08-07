//! Unified runtime definition resolution.
//!
//! Runtime request paths never read the store directly for channel/event
//! definition resolution. Every resolver entry point performs a bounded
//! change-log reconcile first (so committed cross-instance changes become
//! visible without an explicit `detect_and_reconcile_changes()` call), then a
//! cache lookup; a cache miss rehydrates from the shared store and inserts the
//! result so subsequent lookups are served from the engine-local cache.
//!
//! Tenant fallback is gated by `EventRegistryConfiguration.fallback_to_default_tenant`
//! (Java `AbstractEngineConfiguration.java:324` / `GetEventModelCmd.java:82-90` /
//! `GetChannelModelCmd.java:82-90`).

use crate::models::{ChannelDefinition, EventDefinition};
use crate::query::{
    latest_channel_definition_matching_tenant, latest_event_definition_for_tenant_with_policy,
    latest_event_definition_matching_tenant,
};
use crate::tenant_fallback::resolve_definition_with_fallback;
use crate::FlowableEventRegistryService;
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::db_session::DbSession;

impl FlowableEventRegistryService {
    /// Bounded reconcile of committed changes before any definition resolution.
    fn reconcile_before_resolve(&self) -> Result<(), FlowableError> {
        self.detect_and_reconcile_changes()?;
        Ok(())
    }

    fn open_resolver_session(&self) -> Result<DbSession, FlowableError> {
        self.engine
            .get_runtime_store()
            .create_session()
            .map_err(|error| {
                FlowableError::Internal(format!(
                    "failed to open session for definition resolution: {error}"
                ))
            })
    }

    pub(crate) fn resolve_channel_definition_by_id(
        &self,
        id: &str,
    ) -> Result<Option<ChannelDefinition>, FlowableError> {
        self.reconcile_before_resolve()?;
        if let Some(hit) = self
            .definition_cache
            .lock()
            .unwrap()
            .channel_by_id(id)
            .cloned()
        {
            return Ok(Some(hit));
        }
        let store = self.engine.get_runtime_store();
        let mut session = self.open_resolver_session()?;
        let loaded = store.find_event_registry_channel_definition(id, &mut session);
        if let Some(definition) = &loaded {
            self.definition_cache
                .lock()
                .unwrap()
                .register_channel(definition.clone());
        }
        Ok(loaded)
    }

    pub(crate) fn resolve_event_definition_by_id(
        &self,
        id: &str,
    ) -> Result<Option<EventDefinition>, FlowableError> {
        self.reconcile_before_resolve()?;
        if let Some(hit) = self
            .definition_cache
            .lock()
            .unwrap()
            .event_by_id(id)
            .cloned()
        {
            return Ok(Some(hit));
        }
        let store = self.engine.get_runtime_store();
        let mut session = self.open_resolver_session()?;
        let loaded = store.find_event_registry_event_definition(id, &mut session);
        if let Some(definition) = &loaded {
            self.definition_cache
                .lock()
                .unwrap()
                .register_event(definition.clone());
        }
        Ok(loaded)
    }

    /// Latest channel for tenant+key with policy-gated default-tenant fallback.
    ///
    /// Java: `GetChannelModelCmd.java:82-90`.
    pub(crate) fn resolve_latest_channel_definition(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<ChannelDefinition>, FlowableError> {
        self.reconcile_before_resolve()?;
        let policy = self.configuration.tenant_fallback_policy();

        {
            let cache = self.definition_cache.lock().unwrap();
            let hit = resolve_definition_with_fallback(tenant_id, &policy, |lookup_tenant| {
                cache.latest_channel(key, lookup_tenant).cloned()
            });
            if let Some(definition) = hit {
                return Ok(Some(definition));
            }
        }

        // Exact-tenant / tenantless lookups only — do not use store helpers that
        // always fall back (those predate the fallback switch).
        let store = self.engine.get_runtime_store();
        let mut session = self.open_resolver_session()?;
        let candidates: Vec<ChannelDefinition> = store
            .list_event_registry_channel_definitions(&mut session)
            .into_iter()
            .filter(|definition| definition.key == key)
            .collect();

        let loaded = resolve_definition_with_fallback(tenant_id, &policy, |lookup_tenant| {
            latest_channel_definition_matching_tenant(candidates.iter().cloned(), lookup_tenant)
        });
        if let Some(definition) = &loaded {
            self.definition_cache
                .lock()
                .unwrap()
                .register_channel(definition.clone());
        }
        Ok(loaded)
    }

    /// Latest event definition for tenant+key with policy-gated fallback.
    ///
    /// Java: `GetEventModelCmd.java:82-90`.
    pub(crate) fn resolve_latest_event_definition(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<EventDefinition>, FlowableError> {
        self.reconcile_before_resolve()?;
        let policy = self.configuration.tenant_fallback_policy();

        {
            let cache = self.definition_cache.lock().unwrap();
            let hit = resolve_definition_with_fallback(tenant_id, &policy, |lookup_tenant| {
                cache.latest_event(key, lookup_tenant).cloned()
            });
            if let Some(definition) = hit {
                return Ok(Some(definition));
            }
        }

        let store = self.engine.get_runtime_store();
        let mut session = self.open_resolver_session()?;
        let candidates: Vec<EventDefinition> = store
            .list_event_registry_event_definitions(&mut session)
            .into_iter()
            .filter(|definition| definition.key == key)
            .collect();

        let loaded = resolve_definition_with_fallback(tenant_id, &policy, |lookup_tenant| {
            latest_event_definition_matching_tenant(candidates.iter().cloned(), lookup_tenant)
        });
        if let Some(definition) = &loaded {
            self.definition_cache
                .lock()
                .unwrap()
                .register_event(definition.clone());
        }
        Ok(loaded)
    }

    /// Event definitions bound to an event type (compatibility inbound adapter).
    /// The cache has no event-type index, so this is a store-backed lookup that
    /// still runs behind the bounded reconcile.
    pub(crate) fn resolve_event_definitions_by_event_type(
        &self,
        event_type: &str,
    ) -> Result<Vec<EventDefinition>, FlowableError> {
        self.reconcile_before_resolve()?;
        let store = self.engine.get_runtime_store();
        let mut session = self.open_resolver_session()?;
        Ok(store.find_event_registry_event_definitions_by_event_type(event_type, &mut session))
    }

    /// Inbound key-detection resolution: cache fast path on the definition key,
    /// store scan fallback that also matches on event type.
    ///
    /// Tenant order follows `DefaultInboundEventProcessingPipeline.java:120-136`.
    pub(crate) fn resolve_inbound_event_definition(
        &self,
        event_key: &str,
        channel_key: &str,
        tenant_id: Option<&str>,
    ) -> Result<EventDefinition, FlowableError> {
        self.reconcile_before_resolve()?;
        let policy = self.configuration.tenant_fallback_policy();
        {
            let cache = self.definition_cache.lock().unwrap();
            let hit = resolve_definition_with_fallback(tenant_id, &policy, |lookup_tenant| {
                cache
                    .latest_event(event_key, lookup_tenant)
                    .filter(|definition| definition.channel_key == channel_key)
                    .cloned()
            });
            if let Some(definition) = hit {
                return Ok(definition);
            }
        }

        let store = self.engine.get_runtime_store();
        let mut session = self.open_resolver_session()?;
        let mut candidates = store.list_event_registry_event_definitions(&mut session);
        candidates.retain(|definition| {
            (definition.key == event_key || definition.event_type == event_key)
                && definition.channel_key == channel_key
        });

        if candidates.is_empty() {
            return Err(FlowableError::NotFound(format!(
                "No inbound event definition found for key '{}' on channel '{}'",
                event_key, channel_key
            )));
        }

        let resolved = latest_event_definition_for_tenant_with_policy(
            candidates,
            tenant_id,
            &policy,
        )
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "No inbound event definition found for key '{}' and tenant '{}' on channel '{}'",
                event_key,
                tenant_id.unwrap_or(""),
                channel_key
            ))
        })?;
        self.definition_cache
            .lock()
            .unwrap()
            .register_event(resolved.clone());
        Ok(resolved)
    }
}
