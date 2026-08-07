use crate::cache::{AppDefinitionCache, AppDefinitionCacheEntry};
use crate::error::AppError;
use crate::models::{AppDefinitionRecord, ResolvedAppComposition};
use crate::store::AppStore;
use flowable_persistence::entity::app_definition::AppDefinitionDataManager;
use flowable_persistence::entity::app_resolved_composition::AppResolvedCompositionDataManager;
use std::sync::Arc;

/// Resolves deployed app definitions through an engine-local bounded cache.
///
/// Cache misses rehydrate from durable definition and composition records.
/// The cache lock is never held while reading storage.
#[derive(Clone)]
pub struct AppDeploymentManager {
    store: AppStore,
    cache: Arc<AppDefinitionCache>,
}

impl std::fmt::Debug for AppDeploymentManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppDeploymentManager")
            .field("cache_size", &self.cache.size())
            .finish()
    }
}

impl AppDeploymentManager {
    pub(crate) fn new(store: AppStore, cache_limit: Option<usize>) -> Self {
        Self {
            store,
            cache: Arc::new(AppDefinitionCache::new(cache_limit)),
        }
    }

    pub fn cache_size(&self) -> usize {
        self.cache.size()
    }

    pub fn is_cached(&self, app_definition_id: &str) -> Result<bool, AppError> {
        Ok(self.cache.contains(app_definition_id))
    }

    pub fn evict_app_definition(&self, app_definition_id: &str) {
        self.cache.remove(app_definition_id);
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    pub(crate) fn put_entry(&self, entry: AppDefinitionCacheEntry) {
        let definition_id = entry.definition.id.clone();
        self.cache.put(definition_id, entry);
    }

    pub(crate) fn invalidate_definition(&self, app_definition_id: &str) {
        self.cache.remove(app_definition_id);
    }

    pub(crate) fn invalidate_definitions(&self, app_definition_ids: &[String]) {
        for id in app_definition_ids {
            self.cache.remove(id);
        }
    }

    /// Resolve an app definition by id with double-checked cache lookup:
    /// cache → durable definition/composition → cache insert.
    pub fn resolve_app_definition(
        &self,
        app_definition_id: &str,
    ) -> Result<Arc<AppDefinitionCacheEntry>, AppError> {
        if let Some(entry) = self.cache.get(app_definition_id) {
            return Ok(entry);
        }

        // Load outside the cache lock.
        let entry = self.load_entry_from_store(app_definition_id)?;
        Ok(self.cache.put_if_absent(app_definition_id, entry))
    }

    pub fn get_resolved_composition(
        &self,
        app_definition_id: &str,
    ) -> Result<ResolvedAppComposition, AppError> {
        Ok(self
            .resolve_app_definition(app_definition_id)?
            .composition
            .clone())
    }

    pub fn get_app_definition(
        &self,
        app_definition_id: &str,
    ) -> Result<AppDefinitionRecord, AppError> {
        Ok(self
            .resolve_app_definition(app_definition_id)?
            .definition
            .clone())
    }

    fn load_entry_from_store(
        &self,
        app_definition_id: &str,
    ) -> Result<AppDefinitionCacheEntry, AppError> {
        let mut session = self.store.create_session()?;
        let definition_manager = AppDefinitionDataManager::new();
        let composition_manager = AppResolvedCompositionDataManager::new();

        let definition_entity = definition_manager
            .find_by_id(&mut session, app_definition_id)?
            .ok_or_else(|| {
                AppError::not_found(format!(
                    "App definition '{app_definition_id}' was not found"
                ))
            })?;
        let definition: AppDefinitionRecord = serde_json::from_str(&definition_entity.data)?;

        let composition_entity = composition_manager
            .find_by_app_definition_id(&mut session, app_definition_id)?
            .ok_or_else(|| {
                AppError::not_found(format!(
                    "Resolved app composition for definition '{app_definition_id}' was not found"
                ))
            })?;
        let composition: ResolvedAppComposition =
            serde_json::from_str(&composition_entity.data)?;

        Ok(AppDefinitionCacheEntry::new(definition, composition))
    }
}
