use crate::models::{ChannelDefinition, EventDefinition};
use std::collections::HashMap;

type TenantKey = (Option<String>, String);
type TenantKeyVersion = (Option<String>, String, i32);

/// Engine-local definition cache. Versions are stored by tenant+key+version;
/// latest pointers are tracked separately so rollbacks can repoint without
/// discarding unrelated versions.
#[derive(Debug, Default, Clone)]
pub struct DefinitionCache {
    channels_by_id: HashMap<String, ChannelDefinition>,
    events_by_id: HashMap<String, EventDefinition>,
    channel_versions: HashMap<TenantKeyVersion, String>,
    event_versions: HashMap<TenantKeyVersion, String>,
    channel_latest: HashMap<TenantKey, String>,
    event_latest: HashMap<TenantKey, String>,
}

impl DefinitionCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_channel(&mut self, definition: ChannelDefinition) {
        let tenant_key = (definition.tenant_id.clone(), definition.key.clone());
        let version_key = (
            definition.tenant_id.clone(),
            definition.key.clone(),
            definition.version,
        );
        self.channel_versions
            .insert(version_key, definition.id.clone());
        self.channels_by_id
            .insert(definition.id.clone(), definition.clone());

        let should_update_latest = self
            .channel_latest
            .get(&tenant_key)
            .and_then(|id| self.channels_by_id.get(id))
            .map(|existing| definition.version >= existing.version)
            .unwrap_or(true);
        if should_update_latest {
            self.channel_latest
                .insert(tenant_key, definition.id.clone());
        }
    }

    pub fn register_event(&mut self, definition: EventDefinition) {
        let tenant_key = (definition.tenant_id.clone(), definition.key.clone());
        let version_key = (
            definition.tenant_id.clone(),
            definition.key.clone(),
            definition.version,
        );
        self.event_versions
            .insert(version_key, definition.id.clone());
        self.events_by_id
            .insert(definition.id.clone(), definition.clone());

        let should_update_latest = self
            .event_latest
            .get(&tenant_key)
            .and_then(|id| self.events_by_id.get(id))
            .map(|existing| definition.version >= existing.version)
            .unwrap_or(true);
        if should_update_latest {
            self.event_latest.insert(tenant_key, definition.id.clone());
        }
    }

    pub fn unregister_channel_id(&mut self, id: &str) {
        let Some(definition) = self.channels_by_id.remove(id) else {
            return;
        };
        let tenant_key = (definition.tenant_id.clone(), definition.key.clone());
        let version_key = (
            definition.tenant_id.clone(),
            definition.key.clone(),
            definition.version,
        );
        self.channel_versions.remove(&version_key);

        if self.channel_latest.get(&tenant_key).map(String::as_str) == Some(id) {
            // Repoint latest to highest remaining version for this key, if any.
            let previous = self
                .channels_by_id
                .values()
                .filter(|candidate| {
                    candidate.key == definition.key && candidate.tenant_id == definition.tenant_id
                })
                .max_by_key(|candidate| candidate.version)
                .map(|candidate| candidate.id.clone());
            match previous {
                Some(previous_id) => {
                    self.channel_latest.insert(tenant_key, previous_id);
                }
                None => {
                    self.channel_latest.remove(&tenant_key);
                }
            }
        }
    }

    pub fn unregister_event_id(&mut self, id: &str) {
        let Some(definition) = self.events_by_id.remove(id) else {
            return;
        };
        let tenant_key = (definition.tenant_id.clone(), definition.key.clone());
        let version_key = (
            definition.tenant_id.clone(),
            definition.key.clone(),
            definition.version,
        );
        self.event_versions.remove(&version_key);

        if self.event_latest.get(&tenant_key).map(String::as_str) == Some(id) {
            let previous = self
                .events_by_id
                .values()
                .filter(|candidate| {
                    candidate.key == definition.key && candidate.tenant_id == definition.tenant_id
                })
                .max_by_key(|candidate| candidate.version)
                .map(|candidate| candidate.id.clone());
            match previous {
                Some(previous_id) => {
                    self.event_latest.insert(tenant_key, previous_id);
                }
                None => {
                    self.event_latest.remove(&tenant_key);
                }
            }
        }
    }

    pub fn channel_by_id(&self, id: &str) -> Option<&ChannelDefinition> {
        self.channels_by_id.get(id)
    }

    pub fn event_by_id(&self, id: &str) -> Option<&EventDefinition> {
        self.events_by_id.get(id)
    }

    pub fn latest_channel(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> Option<&ChannelDefinition> {
        let tenant_key = (tenant_id.map(str::to_string), key.to_string());
        self.channel_latest
            .get(&tenant_key)
            .and_then(|id| self.channels_by_id.get(id))
    }

    pub fn latest_event(&self, key: &str, tenant_id: Option<&str>) -> Option<&EventDefinition> {
        let tenant_key = (tenant_id.map(str::to_string), key.to_string());
        self.event_latest
            .get(&tenant_key)
            .and_then(|id| self.events_by_id.get(id))
    }

    pub fn channel_version(
        &self,
        key: &str,
        tenant_id: Option<&str>,
        version: i32,
    ) -> Option<&ChannelDefinition> {
        let version_key = (tenant_id.map(str::to_string), key.to_string(), version);
        self.channel_versions
            .get(&version_key)
            .and_then(|id| self.channels_by_id.get(id))
    }

    pub fn event_version(
        &self,
        key: &str,
        tenant_id: Option<&str>,
        version: i32,
    ) -> Option<&EventDefinition> {
        let version_key = (tenant_id.map(str::to_string), key.to_string(), version);
        self.event_versions
            .get(&version_key)
            .and_then(|id| self.events_by_id.get(id))
    }

    pub fn channel_count(&self) -> usize {
        self.channels_by_id.len()
    }

    pub fn event_count(&self) -> usize {
        self.events_by_id.len()
    }

    pub fn latest_channel_keys(&self) -> Vec<(Option<String>, String)> {
        self.channel_latest.keys().cloned().collect()
    }

    pub fn latest_event_keys(&self) -> Vec<(Option<String>, String)> {
        self.event_latest.keys().cloned().collect()
    }
}
