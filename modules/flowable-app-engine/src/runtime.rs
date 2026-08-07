use crate::deployment_manager::AppDeploymentManager;
use crate::error::AppError;
use crate::models::{DefinitionType, PagedResult, ResolvedAppComposition};
use crate::repository::AppRepositoryService;
use crate::store::AppStore;
use flowable_persistence::statement::RenderedStatement;
use flowable_persistence::value::DbParams;

#[derive(Clone)]
pub struct AppRuntimeService {
    store: AppStore,
    repository_service: AppRepositoryService,
    deployment_manager: AppDeploymentManager,
}

impl AppRuntimeService {
    pub(crate) fn new(
        store: AppStore,
        repository_service: AppRepositoryService,
        deployment_manager: AppDeploymentManager,
    ) -> Self {
        Self {
            store,
            repository_service,
            deployment_manager,
        }
    }

    pub(crate) fn store_handle(&self) -> AppStore {
        self.store.clone()
    }

    pub fn resolve_app_definition_by_key(
        &self,
        app_definition_key: &str,
        tenant_id: Option<&str>,
    ) -> Result<ResolvedAppComposition, AppError> {
        let definition = self
            .repository_service
            .latest_app_definition_by_key(app_definition_key, tenant_id)?;
        self.get_resolved_composition(&definition.id)
    }

    pub fn get_resolved_composition(
        &self,
        app_definition_id: &str,
    ) -> Result<ResolvedAppComposition, AppError> {
        self.deployment_manager
            .get_resolved_composition(app_definition_id)
    }

    pub fn create_resolved_composition_query(&self) -> ResolvedAppCompositionQuery {
        ResolvedAppCompositionQuery::new(self.store.clone())
    }
}

pub struct ResolvedAppCompositionQuery {
    store: AppStore,
    app_definition_id: Option<String>,
    app_definition_key: Option<String>,
    deployment_id: Option<String>,
    tenant_id: Option<String>,
    definition_type: Option<DefinitionType>,
    resolved_definition_key: Option<String>,
    start: usize,
    size: Option<usize>,
}

impl ResolvedAppCompositionQuery {
    fn new(store: AppStore) -> Self {
        Self {
            store,
            app_definition_id: None,
            app_definition_key: None,
            deployment_id: None,
            tenant_id: None,
            definition_type: None,
            resolved_definition_key: None,
            start: 0,
            size: None,
        }
    }

    pub fn app_definition_id(mut self, app_definition_id: impl Into<String>) -> Self {
        self.app_definition_id = Some(app_definition_id.into());
        self
    }

    pub fn app_definition_key(mut self, app_definition_key: impl Into<String>) -> Self {
        self.app_definition_key = Some(app_definition_key.into());
        self
    }

    pub fn deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn definition_type(mut self, definition_type: DefinitionType) -> Self {
        self.definition_type = Some(definition_type);
        self
    }

    pub fn resolved_definition_key(mut self, definition_key: impl Into<String>) -> Self {
        self.resolved_definition_key = Some(definition_key.into());
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<ResolvedAppComposition>, AppError> {
        let mut session = self.store.create_session()?;
        let mut sql = String::from(
            "SELECT ID_, APP_DEFINITION_ID_, APP_KEY_, DEPLOYMENT_ID_, TENANT_ID_, DATA_\n             FROM ACT_APP_RESOLVED_COMPOSITION WHERE 1=1",
        );
        let mut params = DbParams::new();
        if let Some(value) = &self.app_definition_id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND APP_DEFINITION_ID_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.app_definition_key {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND APP_KEY_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.deployment_id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND DEPLOYMENT_ID_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.tenant_id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND TENANT_ID_ = ?{index}"));
            params.push(value.clone());
        }
        sql.push_str(" ORDER BY APP_KEY_ ASC, APP_DEFINITION_ID_ ASC");

        let rendered = RenderedStatement::new(sql, params);
        let rows = session.select_raw(rendered)?;
        let mut compositions: Vec<ResolvedAppComposition> = rows
            .into_iter()
            .map(|row| {
                let data = row.get_text("DATA_").ok_or_else(|| {
                    AppError::storage("Missing DATA_ in resolved composition query result")
                })?;
                serde_json::from_str(&data).map_err(AppError::from)
            })
            .collect::<Result<Vec<_>, AppError>>()?;

        compositions
            .retain(|item| matches_optional(&self.app_definition_id, &item.app_definition_id));
        compositions
            .retain(|item| matches_optional(&self.app_definition_key, &item.app_definition_key));
        compositions.retain(|item| matches_optional(&self.deployment_id, &item.deployment_id));
        compositions
            .retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));

        if self.definition_type.is_some() || self.resolved_definition_key.is_some() {
            let definition_type = self.definition_type;
            let resolved_definition_key = self.resolved_definition_key.clone();
            compositions = compositions
                .into_iter()
                .filter_map(|mut composition| {
                    composition.references.retain(|reference| {
                        definition_type.is_none_or(|expected| reference.definition_type == expected)
                            && resolved_definition_key.as_ref().is_none_or(|expected| {
                                &reference.resolved_definition_key == expected
                            })
                    });
                    (!composition.references.is_empty()).then_some(composition)
                })
                .collect();
        }

        Ok(compositions)
    }

    pub fn single_result(&self) -> Result<Option<ResolvedAppComposition>, AppError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<ResolvedAppComposition>, AppError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
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
