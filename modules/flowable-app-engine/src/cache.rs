use crate::models::{AppDefinitionRecord, AppModel, ResolvedAppComposition};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Engine-local cache entry for a deployed app definition.
///
/// Holds the durable definition record, the app model snapshot used at deploy
/// time, and the immutable resolved composition for that definition version.
#[derive(Clone, Debug)]
pub struct AppDefinitionCacheEntry {
    pub definition: AppDefinitionRecord,
    pub app_model: AppModel,
    pub composition: ResolvedAppComposition,
}

impl AppDefinitionCacheEntry {
    pub fn new(
        definition: AppDefinitionRecord,
        composition: ResolvedAppComposition,
    ) -> Self {
        let app_model = AppModel::new().with_app_definition(definition.model.clone());
        Self {
            definition,
            app_model,
            composition,
        }
    }
}

/// Bounded, engine-local definition cache keyed by app definition id.
///
/// Never process-global. Callers must not hold the cache lock while reading
/// storage or resolving catalog references.
#[derive(Debug)]
pub struct AppDefinitionCache {
    inner: Mutex<AppDefinitionCacheInner>,
}

#[derive(Debug)]
struct AppDefinitionCacheInner {
    /// `None` means unbounded (Java default when limit <= 0).
    limit: Option<usize>,
    entries: HashMap<String, Arc<AppDefinitionCacheEntry>>,
    /// Front = least recently used, back = most recently used.
    order: VecDeque<String>,
}

impl AppDefinitionCache {
    pub fn new(limit: Option<usize>) -> Self {
        let limit = match limit {
            Some(0) => None,
            other => other,
        };
        Self {
            inner: Mutex::new(AppDefinitionCacheInner {
                limit,
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
        }
    }

    pub fn get(&self, definition_id: &str) -> Option<Arc<AppDefinitionCacheEntry>> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !guard.entries.contains_key(definition_id) {
            return None;
        }
        touch_order(&mut guard.order, definition_id);
        guard.entries.get(definition_id).cloned()
    }

    pub fn contains(&self, definition_id: &str) -> bool {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.entries.contains_key(definition_id)
    }

    pub fn size(&self) -> usize {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.entries.len()
    }

    /// Insert or replace an entry. Returns the cached arc.
    pub fn put(
        &self,
        definition_id: impl Into<String>,
        entry: AppDefinitionCacheEntry,
    ) -> Arc<AppDefinitionCacheEntry> {
        let definition_id = definition_id.into();
        let entry = Arc::new(entry);
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.entries.contains_key(&definition_id) {
            remove_order(&mut guard.order, &definition_id);
        }
        guard
            .entries
            .insert(definition_id.clone(), Arc::clone(&entry));
        guard.order.push_back(definition_id);
        evict_if_needed(&mut guard);
        entry
    }

    /// Double-checked insert: if another thread already cached the id, keep it.
    pub fn put_if_absent(
        &self,
        definition_id: impl Into<String>,
        entry: AppDefinitionCacheEntry,
    ) -> Arc<AppDefinitionCacheEntry> {
        let definition_id = definition_id.into();
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = guard.entries.get(&definition_id).cloned() {
            touch_order(&mut guard.order, &definition_id);
            return existing;
        }
        let entry = Arc::new(entry);
        guard
            .entries
            .insert(definition_id.clone(), Arc::clone(&entry));
        guard.order.push_back(definition_id);
        evict_if_needed(&mut guard);
        entry
    }

    pub fn remove(&self, definition_id: &str) -> Option<Arc<AppDefinitionCacheEntry>> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        remove_order(&mut guard.order, definition_id);
        guard.entries.remove(definition_id)
    }

    pub fn clear(&self) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.entries.clear();
        guard.order.clear();
    }
}

fn touch_order(order: &mut VecDeque<String>, definition_id: &str) {
    if let Some(index) = order.iter().position(|id| id == definition_id) {
        order.remove(index);
    }
    order.push_back(definition_id.to_string());
}

fn remove_order(order: &mut VecDeque<String>, definition_id: &str) {
    if let Some(index) = order.iter().position(|id| id == definition_id) {
        order.remove(index);
    }
}

fn evict_if_needed(guard: &mut AppDefinitionCacheInner) {
    let Some(limit) = guard.limit else {
        return;
    };
    while guard.entries.len() > limit {
        let Some(oldest) = guard.order.pop_front() else {
            break;
        };
        guard.entries.remove(&oldest);
    }
}
