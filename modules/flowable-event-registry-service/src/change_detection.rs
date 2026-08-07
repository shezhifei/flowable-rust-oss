//! Cross-instance Event Registry definition change detection.
//!
//! Polls the durable change log after the last observed revision and reconciles
//! the local definition cache. Cache updates are applied only for committed
//! change records (written in the same transaction as deploy/delete).

use crate::cache::DefinitionCache;
use crate::models::{ChannelDefinition, EventDefinition};
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::runtime_store::{EventRegistryChangeRecord, RuntimeStore};

/// Default page size for bounded change log polling.
pub const DEFAULT_CHANGE_POLL_LIMIT: usize = 100;

#[derive(Debug, Clone)]
pub struct ChangeDetectionResult {
    pub applied: usize,
    pub last_revision: u64,
    pub exhausted: bool,
}

/// Apply a single committed change record against the local cache, rehydrating
/// entity bodies from the shared store when needed.
pub fn apply_change_record(
    store: &RuntimeStore,
    cache: &mut DefinitionCache,
    record: &EventRegistryChangeRecord,
) -> Result<(), FlowableError> {
    let mut session = store.create_session().map_err(|error| {
        FlowableError::Internal(format!("failed to open session for change detection: {error}"))
    })?;

    match (record.change_type.as_str(), record.entity_type.as_str()) {
        ("deploy" | "update", "channel") => {
            if let Some(definition) =
                store.find_event_registry_channel_definition(&record.entity_id, &mut session)
            {
                cache.register_channel(definition);
            }
        }
        ("deploy" | "update", "event") => {
            if let Some(definition) =
                store.find_event_registry_event_definition(&record.entity_id, &mut session)
            {
                cache.register_event(definition);
            }
        }
        ("delete", "channel") => {
            cache.unregister_channel_id(&record.entity_id);
            // After deleting the latest, repoint from store if a previous version remains.
            if let Some(previous) = previous_channel_version(
                store,
                &mut session,
                &record.entity_key,
                record.tenant_id.as_deref(),
                record.version,
            ) {
                cache.register_channel(previous);
            }
        }
        ("delete", "event") => {
            cache.unregister_event_id(&record.entity_id);
            if let Some(previous) = previous_event_version(
                store,
                &mut session,
                &record.entity_key,
                record.tenant_id.as_deref(),
                record.version,
            ) {
                cache.register_event(previous);
            }
        }
        ("delete", "deployment") => {
            // Deployment deletes cascade; individual entity deletes are also recorded.
        }
        _ => {}
    }

    Ok(())
}

/// Poll changes after `after_revision` (exclusive) and reconcile the cache.
/// Returns how many records were applied and the new high-water mark.
pub fn detect_and_reconcile_changes(
    store: &RuntimeStore,
    cache: &mut DefinitionCache,
    after_revision: u64,
    limit: usize,
) -> Result<ChangeDetectionResult, FlowableError> {
    let mut session = store.create_session().map_err(|error| {
        FlowableError::Internal(format!("failed to open session for change detection: {error}"))
    })?;
    let records =
        store.list_event_registry_change_records_after(after_revision, limit, &mut session);
    // Drop the read session before re-opening for entity loads inside apply.
    drop(session);

    let mut last_revision = after_revision;
    let mut applied = 0usize;
    let batch_len = records.len();
    for record in &records {
        apply_change_record(store, cache, record)?;
        last_revision = record.revision;
        applied += 1;
    }

    Ok(ChangeDetectionResult {
        applied,
        last_revision,
        exhausted: batch_len < limit,
    })
}

fn previous_channel_version(
    store: &RuntimeStore,
    session: &mut flowable_engine::persistence::db_session::DbSession,
    key: &str,
    tenant_id: Option<&str>,
    deleted_version: Option<i32>,
) -> Option<ChannelDefinition> {
    store
        .list_event_registry_channel_definitions(session)
        .into_iter()
        .filter(|definition| {
            definition.key == key
                && definition.tenant_id.as_deref() == tenant_id
                && deleted_version.is_none_or(|version| definition.version < version)
        })
        .max_by_key(|definition| definition.version)
}

fn previous_event_version(
    store: &RuntimeStore,
    session: &mut flowable_engine::persistence::db_session::DbSession,
    key: &str,
    tenant_id: Option<&str>,
    deleted_version: Option<i32>,
) -> Option<EventDefinition> {
    store
        .list_event_registry_event_definitions(session)
        .into_iter()
        .filter(|definition| {
            definition.key == key
                && definition.tenant_id.as_deref() == tenant_id
                && deleted_version.is_none_or(|version| definition.version < version)
        })
        .max_by_key(|definition| definition.version)
}
