use crate::cache::AppDefinitionCacheEntry;
use crate::catalog::{DefinitionCatalog, TenantResolutionPolicy};
use crate::convert::{models_semantically_equal, parse_resource_bytes_to_engine_model};
use crate::deployment_manager::AppDeploymentManager;
use crate::error::AppError;
use crate::models::{
    AppDefinition, AppDefinitionRecord, AppDeployment, AppDeploymentRequest,
    AppDeploymentResourceData, AppPage, PagedResult, ResolvedAppComposition, ResolvedAppReference,
};
use crate::store::AppStore;
use chrono::Utc;
use flowable_persistence::entity::app_definition::{AppDefinitionDataManager, AppDefinitionEntity};
use flowable_persistence::entity::app_deployment::{AppDeploymentDataManager, AppDeploymentEntity};
use flowable_persistence::entity::app_deployment_resource::{
    AppDeploymentResourceDataManager, AppDeploymentResourceEntity,
};
use flowable_persistence::entity::app_resolved_composition::{
    AppResolvedCompositionDataManager, AppResolvedCompositionEntity,
};
use flowable_persistence::statement::{RenderedStatement, StatementId};
use flowable_persistence::value::DbParams;
use std::collections::BTreeSet;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppRepositoryService {
    store: AppStore,
    catalog: Arc<dyn DefinitionCatalog>,
    deployment_manager: AppDeploymentManager,
    tenant_resolution_policy: TenantResolutionPolicy,
}

impl AppRepositoryService {
    pub(crate) fn new(
        store: AppStore,
        catalog: Arc<dyn DefinitionCatalog>,
        deployment_manager: AppDeploymentManager,
    ) -> Self {
        Self {
            store,
            catalog,
            deployment_manager,
            tenant_resolution_policy: TenantResolutionPolicy::FallbackToDefault,
        }
    }

    /// Override the tenant resolution policy used when resolving referenced
    /// BPMN/DMN/CMMN/Event definitions at deployment time.
    pub fn with_tenant_resolution_policy(mut self, policy: TenantResolutionPolicy) -> Self {
        self.tenant_resolution_policy = policy;
        self
    }

    pub fn deploy(&self, request: AppDeploymentRequest) -> Result<AppDeployment, AppError> {
        let request = normalize_and_validate_deployment_request(request)?;

        let deployment_id = format!("app-deployment:{}", Uuid::new_v4());
        let deployed_at = Utc::now();
        let deployment = AppDeployment {
            id: deployment_id.clone(),
            name: request.name.clone(),
            category: request.category.clone(),
            tenant_id: request.tenant_id.clone(),
            resource_names: request
                .resources
                .iter()
                .map(|resource| resource.resource_name.clone())
                .collect(),
            deployed_at,
        };

        let mut session = self.store.create_session()?;
        let deployment_manager = AppDeploymentDataManager::new();
        let resource_manager = AppDeploymentResourceDataManager::new();
        let definition_manager = AppDefinitionDataManager::new();
        let composition_manager = AppResolvedCompositionDataManager::new();

        let mut deployment_entity = AppDeploymentEntity::new(
            deployment.id.clone(),
            deployment.name.clone(),
            deployment.deployed_at.to_rfc3339(),
            serde_json::to_string(&deployment)?,
        );
        deployment_entity.set_category(deployment.category.clone());
        deployment_entity.set_tenant_id(deployment.tenant_id.clone());
        deployment_manager.insert(&mut session, deployment_entity)?;

        for resource_name in &deployment.resource_names {
            let bytes = request
                .resource_bytes
                .get(resource_name)
                .cloned()
                .unwrap_or_default();
            let resource = AppDeploymentResourceData::new(
                deployment.id.clone(),
                resource_name.clone(),
                bytes,
                deployment.deployed_at.timestamp_millis(),
            );
            let resource_entity = AppDeploymentResourceEntity::new(
                resource.deployment_id,
                resource.resource_name,
                resource.resource_type,
                resource.content_type,
                resource.bytes,
                resource.created_at,
            );
            resource_manager.insert(&mut session, resource_entity)?;
        }

        let mut seen_keys = BTreeSet::new();
        let mut cache_entries = Vec::new();
        for resource in request.resources {
            for app_definition in resource.model.app_definitions {
                if !seen_keys.insert(app_definition.key.clone()) {
                    return Err(AppError::validation(format!(
                        "Duplicate app definition key '{}' in deployment '{}'",
                        app_definition.key, deployment.name
                    )));
                }

                let version = next_version(
                    &mut session,
                    &app_definition.key,
                    request.tenant_id.as_deref(),
                )?;
                let definition = AppDefinitionRecord {
                    id: format!("app-definition:{}:{}", deployment_id, app_definition.key),
                    app_id: app_definition.id.clone(),
                    deployment_id: deployment_id.clone(),
                    key: app_definition.key.clone(),
                    name: app_definition.name.clone(),
                    category: app_definition.category.clone(),
                    version,
                    tenant_id: request.tenant_id.clone(),
                    resource_name: resource.resource_name.clone(),
                    model: app_definition.clone(),
                };
                let composition = resolve_composition(
                    &definition,
                    &self.catalog,
                    request.tenant_id.as_deref(),
                    self.tenant_resolution_policy,
                )?;

                let mut definition_entity = AppDefinitionEntity::new(
                    definition.id.clone(),
                    definition.key.clone(),
                    definition.deployment_id.clone(),
                    definition.version,
                    definition.resource_name.clone(),
                    serde_json::to_string(&definition)?,
                );
                definition_entity.set_tenant_id(definition.tenant_id.clone());
                definition_manager.insert(&mut session, definition_entity)?;

                let mut composition_entity = AppResolvedCompositionEntity::new(
                    composition.id.clone(),
                    composition.app_definition_id.clone(),
                    composition.app_definition_key.clone(),
                    composition.deployment_id.clone(),
                    serde_json::to_string(&composition)?,
                );
                composition_entity.set_tenant_id(composition.tenant_id.clone());
                composition_manager.insert(&mut session, composition_entity)?;

                cache_entries.push(AppDefinitionCacheEntry::new(definition, composition));
            }
        }

        session.commit()?;
        // Populate cache only after the durable write succeeds.
        for entry in cache_entries {
            self.deployment_manager.put_entry(entry);
        }
        Ok(deployment)
    }

    pub fn create_deployment_query(&self) -> AppDeploymentQuery {
        AppDeploymentQuery::new(self.store.clone())
    }

    pub fn get_deployment(&self, deployment_id: &str) -> Result<AppDeployment, AppError> {
        let mut session = self.store.create_session()?;
        let manager = AppDeploymentDataManager::new();
        let entity = manager
            .find_by_id(&mut session, deployment_id)?
            .ok_or_else(|| {
                AppError::not_found(format!("App deployment '{deployment_id}' was not found"))
            })?;
        deployment_from_entity(&entity)
    }

    pub fn delete_deployment(&self, deployment_id: &str) -> Result<(), AppError> {
        self.get_deployment(deployment_id)?;

        let mut session = self.store.create_session()?;
        let definition_manager = AppDefinitionDataManager::new();
        let definition_ids = definition_manager
            .find_by_deployment_id(&mut session, deployment_id)?
            .into_iter()
            .map(|entity| entity.id)
            .collect::<Vec<_>>();

        let mut p = DbParams::new();
        p.push(deployment_id);
        session.execute(
            StatementId::DeleteAppResolvedCompositionsByDeploymentId,
            p.clone(),
        )?;
        session.execute(StatementId::DeleteAppDefinitionsByDeploymentId, p.clone())?;
        session.execute(
            StatementId::DeleteAppDeploymentResourcesByDeploymentId,
            p.clone(),
        )?;
        session.execute(StatementId::DeleteAppDeployment, p)?;
        session.commit()?;
        // Invalidate only after the durable delete commits.
        self.deployment_manager
            .invalidate_definitions(&definition_ids);
        Ok(())
    }

    pub fn get_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<AppDeploymentResourceData>, AppError> {
        self.get_deployment(deployment_id)?;
        let mut session = self.store.create_session()?;
        let manager = AppDeploymentResourceDataManager::new();
        let resources = manager.find_by_deployment_id(&mut session, deployment_id)?;
        Ok(resources.into_iter().map(resource_entity_to_data).collect())
    }

    pub fn get_deployment_resource(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<AppDeploymentResourceData, AppError> {
        self.get_deployment(deployment_id)?;
        let mut session = self.store.create_session()?;
        let manager = AppDeploymentResourceDataManager::new();
        let resource = manager
            .find_by_id(&mut session, deployment_id, resource_name)?
            .map(resource_entity_to_data)
            .ok_or_else(|| {
                AppError::not_found(format!(
                    "App deployment resource '{resource_name}' was not found in deployment '{deployment_id}'"
                ))
            })?;
        Ok(resource)
    }

    pub fn create_app_definition_query(&self) -> AppDefinitionQuery {
        AppDefinitionQuery::new(self.store.clone())
    }

    pub fn get_app_definition(
        &self,
        app_definition_id: &str,
    ) -> Result<AppDefinitionRecord, AppError> {
        self.deployment_manager
            .get_app_definition(app_definition_id)
    }

    pub fn set_app_definition_category(
        &self,
        app_definition_id: &str,
        category: Option<&str>,
    ) -> Result<AppDefinitionRecord, AppError> {
        let mut session = self.store.create_session()?;
        let manager = AppDefinitionDataManager::new();
        let entity = manager
            .find_by_id(&mut session, app_definition_id)?
            .ok_or_else(|| {
                AppError::not_found(format!(
                    "App definition '{app_definition_id}' was not found"
                ))
            })?;
        let mut definition: AppDefinitionRecord = serde_json::from_str(&entity.data)?;
        definition.category = category.map(str::to_string);
        definition.model.category = category.map(str::to_string);

        let mut params = DbParams::new();
        params.push(serde_json::to_string(&definition)?);
        params.push(app_definition_id);
        session.execute_raw(RenderedStatement::new(
            "UPDATE ACT_APP_DEFINITION SET DATA_ = ?1 WHERE ID_ = ?2".to_string(),
            params,
        ))?;
        session.commit()?;
        // Invalidate only after the durable update commits.
        self.deployment_manager
            .invalidate_definition(app_definition_id);
        Ok(definition)
    }

    pub(crate) fn latest_app_definition_by_key(
        &self,
        app_definition_key: &str,
        tenant_id: Option<&str>,
    ) -> Result<AppDefinitionRecord, AppError> {
        self.create_app_definition_query()
            .key(app_definition_key)
            .tenant_id_optional(tenant_id)
            .single_result()?
            .ok_or_else(|| {
                AppError::not_found(format!(
                    "App definition '{app_definition_key}' was not found"
                ))
            })
    }
}

fn deployment_from_entity(entity: &AppDeploymentEntity) -> Result<AppDeployment, AppError> {
    serde_json::from_str(&entity.data).map_err(AppError::from)
}

fn resource_entity_to_data(entity: AppDeploymentResourceEntity) -> AppDeploymentResourceData {
    AppDeploymentResourceData {
        deployment_id: entity.deployment_id,
        resource_name: entity.resource_name,
        resource_type: entity.resource_type,
        content_type: entity.content_type,
        bytes: entity.bytes,
        created_at: entity.created_at,
    }
}

pub struct AppDeploymentQuery {
    store: AppStore,
    id: Option<String>,
    name: Option<String>,
    category: Option<String>,
    tenant_id: Option<String>,
    resource_name: Option<String>,
    start: usize,
    size: Option<usize>,
}

impl AppDeploymentQuery {
    fn new(store: AppStore) -> Self {
        Self {
            store,
            id: None,
            name: None,
            category: None,
            tenant_id: None,
            resource_name: None,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn resource_name(mut self, resource_name: impl Into<String>) -> Self {
        self.resource_name = Some(resource_name.into());
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<AppDeployment>, AppError> {
        let mut session = self.store.create_session()?;
        let mut sql = String::from(
            "SELECT ID_, NAME_, CATEGORY_, TENANT_ID_, DEPLOYED_AT_, DATA_\n             FROM ACT_APP_DEPLOYMENT WHERE 1=1",
        );
        let mut params = DbParams::new();
        if let Some(value) = &self.id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND ID_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.name {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND NAME_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.category {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND CATEGORY_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.tenant_id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND TENANT_ID_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.resource_name {
            let index = params.len() + 1;
            sql.push_str(&format!(
                " AND ID_ IN (SELECT DEPLOYMENT_ID_ FROM ACT_APP_DEPLOYMENT_RESOURCE WHERE RESOURCE_NAME_ = ?{index})"
            ));
            params.push(value.clone());
        }
        sql.push_str(" ORDER BY DEPLOYED_AT_ ASC, ID_ ASC");

        let rendered = RenderedStatement::new(sql, params);
        let rows = session.select_raw(rendered)?;
        let mut deployments: Vec<AppDeployment> = rows
            .into_iter()
            .map(|row| {
                let data = row.get_text("DATA_").ok_or_else(|| {
                    AppError::storage("Missing DATA_ in app deployment query result")
                })?;
                let deployment: AppDeployment = serde_json::from_str(&data)?;
                Ok(AppDeployment {
                    id: row.get_text("ID_").unwrap_or_default(),
                    name: row.get_text("NAME_").unwrap_or_default(),
                    category: row.get_text("CATEGORY_"),
                    tenant_id: row.get_text("TENANT_ID_"),
                    resource_names: deployment.resource_names,
                    deployed_at: deployment.deployed_at,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;

        deployments.retain(|item| matches_optional(&self.id, &item.id));
        deployments.retain(|item| matches_optional(&self.name, &item.name));
        deployments
            .retain(|item| matches_optional_option(&self.category, item.category.as_deref()));
        deployments
            .retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));
        deployments.retain(|item| {
            self.resource_name.as_ref().is_none_or(|resource_name| {
                item.resource_names.iter().any(|name| name == resource_name)
            })
        });

        Ok(deployments)
    }

    pub fn single_result(&self) -> Result<Option<AppDeployment>, AppError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<AppDeployment>, AppError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

/// Tenant filter for app definition queries (Java query semantics):
/// no filter at all, an exact tenant, or explicitly "without tenant"
/// (`TENANT_ID_ IS NULL` — `appDefinitionWithoutTenantId` in Java).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum TenantFilter {
    #[default]
    Any,
    Exact(String),
    WithoutTenant,
}

pub struct AppDefinitionQuery {
    store: AppStore,
    id: Option<String>,
    key: Option<String>,
    deployment_id: Option<String>,
    tenant_filter: TenantFilter,
    resource_name: Option<String>,
    version: Option<i32>,
    start: usize,
    size: Option<usize>,
}

impl AppDefinitionQuery {
    fn new(store: AppStore) -> Self {
        Self {
            store,
            id: None,
            key: None,
            deployment_id: None,
            tenant_filter: TenantFilter::Any,
            resource_name: None,
            version: None,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_filter = TenantFilter::Exact(tenant_id.into());
        self
    }

    /// Only match definitions that have no tenant (`TENANT_ID_ IS NULL`).
    pub fn without_tenant_id(mut self) -> Self {
        self.tenant_filter = TenantFilter::WithoutTenant;
        self
    }

    /// Three-state mapping (P1 tenant fix): `Some(tenant)` filters on that
    /// exact tenant; `None` means "without tenant" and MUST NOT fall back to
    /// "any tenant" — otherwise a tenantless lookup could leak another
    /// tenant's latest definition.
    pub(crate) fn tenant_id_optional(self, tenant_id: Option<&str>) -> Self {
        match tenant_id {
            Some(tenant) => self.tenant_id(tenant),
            None => self.without_tenant_id(),
        }
    }

    pub fn resource_name(mut self, resource_name: impl Into<String>) -> Self {
        self.resource_name = Some(resource_name.into());
        self
    }

    pub fn version(mut self, version: i32) -> Self {
        self.version = Some(version);
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<AppDefinitionRecord>, AppError> {
        let mut session = self.store.create_session()?;
        let mut sql = String::from(
            "SELECT ID_, APP_KEY_, DEPLOYMENT_ID_, TENANT_ID_, VERSION_, RESOURCE_NAME_, DATA_\n             FROM ACT_APP_DEFINITION WHERE 1=1",
        );
        let mut params = DbParams::new();
        if let Some(value) = &self.id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND ID_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.key {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND APP_KEY_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.deployment_id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND DEPLOYMENT_ID_ = ?{index}"));
            params.push(value.clone());
        }
        match &self.tenant_filter {
            TenantFilter::Any => {}
            TenantFilter::Exact(value) => {
                let index = params.len() + 1;
                sql.push_str(&format!(" AND TENANT_ID_ = ?{index}"));
                params.push(value.clone());
            }
            TenantFilter::WithoutTenant => {
                sql.push_str(" AND (TENANT_ID_ IS NULL OR TENANT_ID_ = '')");
            }
        }
        if let Some(value) = &self.resource_name {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND RESOURCE_NAME_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.version {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND VERSION_ = ?{index}"));
            params.push(*value as i64);
        }
        sql.push_str(" ORDER BY APP_KEY_ ASC, VERSION_ DESC, ID_ ASC");

        let rendered = RenderedStatement::new(sql, params);
        let rows = session.select_raw(rendered)?;
        let mut definitions: Vec<AppDefinitionRecord> = rows
            .into_iter()
            .map(|row| {
                let data = row.get_text("DATA_").ok_or_else(|| {
                    AppError::storage("Missing DATA_ in app definition query result")
                })?;
                serde_json::from_str(&data).map_err(AppError::from)
            })
            .collect::<Result<Vec<_>, AppError>>()?;

        definitions.retain(|item| matches_optional(&self.id, &item.id));
        definitions.retain(|item| matches_optional(&self.key, &item.key));
        definitions.retain(|item| matches_optional(&self.deployment_id, &item.deployment_id));
        definitions.retain(|item| match &self.tenant_filter {
            TenantFilter::Any => true,
            TenantFilter::Exact(tenant) => item.tenant_id.as_deref() == Some(tenant.as_str()),
            TenantFilter::WithoutTenant => {
                item.tenant_id.as_deref().is_none_or(str::is_empty)
            }
        });
        definitions.retain(|item| matches_optional(&self.resource_name, &item.resource_name));
        definitions.retain(|item| self.version.is_none_or(|version| item.version == version));

        Ok(definitions)
    }

    pub fn single_result(&self) -> Result<Option<AppDefinitionRecord>, AppError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<AppDefinitionRecord>, AppError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

fn resolve_composition(
    definition: &AppDefinitionRecord,
    catalog: &Arc<dyn DefinitionCatalog>,
    tenant_id: Option<&str>,
    policy: TenantResolutionPolicy,
) -> Result<ResolvedAppComposition, AppError> {
    let mut references = Vec::new();
    for page in &definition.model.pages {
        resolve_page_references(page, catalog, tenant_id, policy, &mut references)?;
    }

    Ok(ResolvedAppComposition {
        id: format!("app-composition:{}", definition.id),
        app_definition_id: definition.id.clone(),
        app_definition_key: definition.key.clone(),
        app_definition_name: definition.name.clone(),
        deployment_id: definition.deployment_id.clone(),
        version: definition.version,
        tenant_id: definition.tenant_id.clone(),
        references,
    })
}

fn resolve_page_references(
    page: &AppPage,
    catalog: &Arc<dyn DefinitionCatalog>,
    tenant_id: Option<&str>,
    policy: TenantResolutionPolicy,
    target: &mut Vec<ResolvedAppReference>,
) -> Result<(), AppError> {
    for reference in &page.references {
        let resolved = if let Some(pinned_id) = reference
            .definition_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            // Exact pin: resolve by id only — never fall back to latest-by-key.
            let resolved = catalog
                .resolve_definition_by_id(reference.definition_type, pinned_id)?
                .ok_or_else(|| {
                    AppError::validation(format!(
                        "App page '{}' pins missing {} definition id '{}'",
                        page.id,
                        reference.definition_type.label(),
                        pinned_id
                    ))
                })?;
            if resolved.definition_type != reference.definition_type {
                return Err(AppError::validation(format!(
                    "App page '{}' pins definition id '{}' which is a {} definition, not {}",
                    page.id,
                    pinned_id,
                    resolved.definition_type.label(),
                    reference.definition_type.label()
                )));
            }
            if !reference.definition_key.is_empty()
                && resolved.definition_key != reference.definition_key
            {
                return Err(AppError::validation(format!(
                    "App page '{}' pins definition id '{}' whose key '{}' does not match the declared key '{}'",
                    page.id, pinned_id, resolved.definition_key, reference.definition_key
                )));
            }
            if let Some(reference_tenant) = reference.tenant_id.as_deref()
                && resolved.tenant_id.as_deref() != Some(reference_tenant)
            {
                return Err(AppError::validation(format!(
                    "App page '{}' pins definition id '{}' in tenant '{}' but the definition belongs to tenant '{}'",
                    page.id,
                    pinned_id,
                    reference_tenant,
                    resolved.tenant_id.as_deref().unwrap_or("")
                )));
            }
            resolved
        } else {
            // The per-reference tenant overrides the App definition tenant.
            let effective_tenant = reference.tenant_id.as_deref().or(tenant_id);
            catalog
                .resolve_definition_with_policy(
                    reference.definition_type,
                    &reference.definition_key,
                    effective_tenant,
                    policy,
                )?
                .ok_or_else(|| {
                    AppError::validation(format!(
                        "App page '{}' references missing {} definition key '{}'",
                        page.id,
                        reference.definition_type.label(),
                        reference.definition_key
                    ))
                })?
        };

        target.push(ResolvedAppReference {
            page_id: page.id.clone(),
            page_name: page.name.clone(),
            reference_id: reference.id.clone(),
            reference_name: reference.name.clone(),
            definition_type: reference.definition_type,
            requested_definition_key: reference.definition_key.clone(),
            resolved_definition_id: resolved.definition_id,
            resolved_definition_key: resolved.definition_key,
            resolved_definition_name: resolved.definition_name,
            resolved_definition_version: resolved.version,
            resolved_deployment_id: resolved.deployment_id,
            tenant_id: resolved.tenant_id,
        });
    }

    Ok(())
}

fn normalize_and_validate_deployment_request(
    mut request: AppDeploymentRequest,
) -> Result<AppDeploymentRequest, AppError> {
    if request.name.trim().is_empty() {
        return Err(AppError::validation("App deployment name is required"));
    }

    if request.resources.is_empty() {
        return Err(AppError::validation(
            "App deployment requires at least one resource",
        ));
    }

    let mut resource_names = BTreeSet::new();
    let mut durable_bytes = std::collections::BTreeMap::new();

    for resource in &mut request.resources {
        if resource.resource_name.trim().is_empty() {
            return Err(AppError::validation(
                "App deployment resource name is required",
            ));
        }

        if !resource_names.insert(resource.resource_name.clone()) {
            return Err(AppError::validation(format!(
                "Duplicate app deployment resource '{}'",
                resource.resource_name
            )));
        }

        let existing_bytes = request
            .resource_bytes
            .get(&resource.resource_name)
            .cloned()
            .unwrap_or_default();

        if !existing_bytes.is_empty() {
            let parsed = parse_resource_bytes_to_engine_model(&existing_bytes).map_err(|error| {
                AppError::validation(format!(
                    "App deployment resource '{}' is not a valid app definition: {error}",
                    resource.resource_name
                ))
            })?;
            if resource.model.app_definitions.is_empty() {
                resource.model = parsed;
            } else if !models_semantically_equal(&resource.model, &parsed) {
                return Err(AppError::validation(format!(
                    "App deployment resource '{}' model does not match supplied resource bytes",
                    resource.resource_name
                )));
            }
            durable_bytes.insert(resource.resource_name.clone(), existing_bytes);
        } else {
            let durable = crate::convert::serialize_engine_model_as_durable_bytes(&resource.model)
                .unwrap_or_else(|_| serde_json::to_vec(&resource.model).unwrap_or_default());
            durable_bytes.insert(resource.resource_name.clone(), durable);
        }

        if resource.model.app_definitions.is_empty() {
            return Err(AppError::validation(format!(
                "App deployment resource '{}' must contain at least one app definition",
                resource.resource_name
            )));
        }

        for app_definition in &resource.model.app_definitions {
            validate_app_definition(app_definition)?;
        }
    }

    request.resource_bytes = durable_bytes;
    Ok(request)
}

fn validate_app_definition(app_definition: &AppDefinition) -> Result<(), AppError> {
    if app_definition.id.trim().is_empty() {
        return Err(AppError::validation("App definition id is required"));
    }
    if app_definition.key.trim().is_empty() {
        return Err(AppError::validation("App definition key is required"));
    }
    if app_definition.name.trim().is_empty() {
        return Err(AppError::validation("App definition name is required"));
    }

    let mut page_ids = BTreeSet::new();
    for page in &app_definition.pages {
        if page.id.trim().is_empty() {
            return Err(AppError::validation(format!(
                "App definition '{}' contains a page without an id",
                app_definition.key
            )));
        }
        if page.name.trim().is_empty() {
            return Err(AppError::validation(format!(
                "App page '{}' in app definition '{}' requires a name",
                page.id, app_definition.key
            )));
        }
        if !page_ids.insert(page.id.clone()) {
            return Err(AppError::validation(format!(
                "App definition '{}' contains duplicate page id '{}'",
                app_definition.key, page.id
            )));
        }

        let mut reference_ids = BTreeSet::new();
        for reference in &page.references {
            if reference.id.trim().is_empty() {
                return Err(AppError::validation(format!(
                    "App page '{}' in app definition '{}' contains a reference without an id",
                    page.id, app_definition.key
                )));
            }
            if reference.definition_key.trim().is_empty() {
                return Err(AppError::validation(format!(
                    "App reference '{}' in page '{}' must declare a definition key",
                    reference.id, page.id
                )));
            }
            if !reference_ids.insert(reference.id.clone()) {
                return Err(AppError::validation(format!(
                    "App page '{}' in app definition '{}' contains duplicate reference id '{}'",
                    page.id, app_definition.key, reference.id
                )));
            }
        }
    }

    Ok(())
}

fn next_version(
    session: &mut flowable_persistence::db_session::DbSession,
    app_key: &str,
    tenant_id: Option<&str>,
) -> Result<i32, AppError> {
    let manager = AppDefinitionDataManager::new();
    let definitions = manager.find_by_key(session, app_key)?;
    let version = definitions
        .into_iter()
        .filter(|d| match tenant_id {
            Some(t) => d.tenant_id.as_deref() == Some(t),
            None => d.tenant_id.is_none(),
        })
        .map(|d| d.version)
        .max()
        .unwrap_or(0);
    Ok(version + 1)
}

fn matches_optional(filter: &Option<String>, value: &str) -> bool {
    filter.as_ref().is_none_or(|filter| filter == value)
}

fn matches_optional_option(filter: &Option<String>, value: Option<&str>) -> bool {
    filter
        .as_ref()
        .is_none_or(|filter| value.is_some_and(|value| value == filter))
}

fn page_items<T>(items: Vec<T>, start: usize, size: Option<usize>) -> PagedResult<T> {
    let total = items.len();
    let size = size.unwrap_or(total.saturating_sub(start));
    let data = items.into_iter().skip(start).take(size).collect();
    PagedResult {
        start,
        size,
        total,
        data,
    }
}
