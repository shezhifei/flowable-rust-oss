mod cache;
mod catalog;
mod convert;
mod deployment_manager;
mod error;
mod models;
mod repository;
mod runtime;
mod store;

pub use cache::AppDefinitionCacheEntry;
pub use catalog::{
    DefinitionCatalog, InMemoryDefinitionCatalog, InMemoryDefinitionCatalogBuilder,
    TenantResolutionPolicy,
};
pub use convert::{
    canonical_definition_to_engine, engine_definition_to_canonical, engine_model_to_canonical,
    models_semantically_equal, parse_resource_bytes_to_engine_model,
    serialize_engine_model_as_durable_bytes,
};
pub use deployment_manager::AppDeploymentManager;
pub use error::AppError;
pub use models::{
    AppDefinition, AppDefinitionRecord, AppDeployment, AppDeploymentRequest, AppDeploymentResource,
    AppDeploymentResourceData, AppModel, AppPage, AppReference, DefinitionType, PagedResult,
    ResolvedAppComposition, ResolvedAppReference, ResolvedDefinition,
};
pub use repository::{AppDefinitionQuery, AppDeploymentQuery, AppRepositoryService};
pub use runtime::{AppRuntimeService, ResolvedAppCompositionQuery};
use store::AppStore;

pub use flowable_persistence::{DatabaseConfig, DatabaseKind, SchemaMode};

use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppEngine {
    repository_service: AppRepositoryService,
    runtime_service: AppRuntimeService,
    deployment_manager: AppDeploymentManager,
}

impl std::fmt::Debug for AppEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AppEngine { .. }")
    }
}

impl AppEngine {
    pub fn new_in_memory() -> Result<Self, AppError> {
        Self::new_in_memory_with_catalog(Arc::new(InMemoryDefinitionCatalog::new()))
    }

    pub fn new_in_memory_with_catalog(
        catalog: Arc<dyn DefinitionCatalog>,
    ) -> Result<Self, AppError> {
        Self::from_store(AppStore::in_memory()?, catalog, None)
    }

    pub fn new_in_memory_with_catalog_and_cache_limit(
        catalog: Arc<dyn DefinitionCatalog>,
        cache_limit: usize,
    ) -> Result<Self, AppError> {
        Self::from_store(AppStore::in_memory()?, catalog, Some(cache_limit))
    }

    pub fn new_sqlite(path: impl AsRef<Path>) -> Result<Self, AppError> {
        Self::new_sqlite_with_catalog(path, Arc::new(InMemoryDefinitionCatalog::new()))
    }

    pub fn new_sqlite_with_catalog(
        path: impl AsRef<Path>,
        catalog: Arc<dyn DefinitionCatalog>,
    ) -> Result<Self, AppError> {
        Self::from_store(AppStore::sqlite(path)?, catalog, None)
    }

    pub fn new_sqlite_with_catalog_and_cache_limit(
        path: impl AsRef<Path>,
        catalog: Arc<dyn DefinitionCatalog>,
        cache_limit: usize,
    ) -> Result<Self, AppError> {
        Self::from_store(AppStore::sqlite(path)?, catalog, Some(cache_limit))
    }

    /// Build an App engine on an explicitly configured backend (the production
    /// `FLOWABLE_DATABASE_URL` path — MySQL/Postgres as well as SQLite).
    pub fn from_database_config_with_catalog(
        config: DatabaseConfig,
        catalog: Arc<dyn DefinitionCatalog>,
    ) -> Result<Self, AppError> {
        Self::from_store(AppStore::from_config(config)?, catalog, None)
    }

    pub fn repository_service(&self) -> AppRepositoryService {
        self.repository_service.clone()
    }

    pub fn runtime_service(&self) -> AppRuntimeService {
        self.runtime_service.clone()
    }

    pub fn deployment_manager(&self) -> AppDeploymentManager {
        self.deployment_manager.clone()
    }

    pub fn deploy(&self, request: AppDeploymentRequest) -> Result<AppDeployment, AppError> {
        self.repository_service.deploy(request)
    }

    /// Configure tenant resolution policy used when resolving referenced definitions
    /// at deployment time. Default is [`TenantResolutionPolicy::FallbackToDefault`].
    pub fn with_tenant_resolution_policy(mut self, policy: TenantResolutionPolicy) -> Self {
        self.repository_service = self
            .repository_service
            .with_tenant_resolution_policy(policy);
        // Runtime keeps a repository clone for key lookup; keep them in sync.
        self.runtime_service = AppRuntimeService::new(
            self.runtime_service.store_handle(),
            self.repository_service.clone(),
            self.deployment_manager.clone(),
        );
        self
    }

    fn from_store(
        store: AppStore,
        catalog: Arc<dyn DefinitionCatalog>,
        cache_limit: Option<usize>,
    ) -> Result<Self, AppError> {
        let deployment_manager = AppDeploymentManager::new(store.clone(), cache_limit);
        let repository_service =
            AppRepositoryService::new(store.clone(), catalog, deployment_manager.clone());
        let runtime_service =
            AppRuntimeService::new(store, repository_service.clone(), deployment_manager.clone());

        Ok(Self {
            repository_service,
            runtime_service,
            deployment_manager,
        })
    }
}
