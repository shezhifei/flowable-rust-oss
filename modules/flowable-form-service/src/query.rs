use crate::models::{FormDefinition, FormInstance, PagedResult};
use crate::repository;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct FormDefinitionQuery {
    engine: Arc<ProcessEngine>,
    id: Option<String>,
    key: Option<String>,
    name: Option<String>,
    deployment_id: Option<String>,
    resource_name: Option<String>,
    version: Option<i32>,
    start: usize,
    size: Option<usize>,
    unsupported_filters: BTreeMap<String, String>,
}

impl FormDefinitionQuery {
    pub(crate) fn new(engine: Arc<ProcessEngine>) -> Self {
        Self {
            engine,
            id: None,
            key: None,
            name: None,
            deployment_id: None,
            resource_name: None,
            version: None,
            start: 0,
            size: None,
            unsupported_filters: BTreeMap::new(),
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

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
        self
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

    pub fn unsupported_filter(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.unsupported_filters.insert(name.into(), value.into());
        self
    }

    pub fn list(&self) -> Result<Vec<FormDefinition>, FlowableError> {
        validate_unsupported_filters("form definition", &self.unsupported_filters)?;

        let store = self.engine.get_runtime_store();
        let mut definitions = repository::list_form_definitions(&store);
        definitions.retain(|item| matches_optional(&self.id, &item.id));
        definitions.retain(|item| matches_optional(&self.key, &item.key));
        definitions.retain(|item| matches_optional(&self.name, &item.name));
        definitions.retain(|item| matches_optional(&self.deployment_id, &item.deployment_id));
        definitions.retain(|item| matches_optional(&self.resource_name, &item.resource_name));
        definitions.retain(|item| self.version.is_none_or(|value| item.version == value));
        definitions.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then(right.version.cmp(&left.version))
                .then(left.id.cmp(&right.id))
        });
        Ok(definitions)
    }

    pub fn list_page(&self) -> Result<PagedResult<FormDefinition>, FlowableError> {
        let definitions = self.list()?;
        Ok(page_items(definitions, self.start, self.size))
    }
}

fn validate_unsupported_filters(
    query_name: &str,
    unsupported_filters: &BTreeMap<String, String>,
) -> Result<(), FlowableError> {
    if unsupported_filters.is_empty() {
        return Ok(());
    }

    let names = unsupported_filters
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    Err(FlowableError::ExecutionError(format!(
        "Unsupported {} filter(s): {}",
        query_name, names
    )))
}

fn matches_optional(filter: &Option<String>, actual: &str) -> bool {
    filter.as_ref().is_none_or(|value| value == actual)
}

fn page_items<T>(items: Vec<T>, start: usize, size: Option<usize>) -> PagedResult<T> {
    let total = items.len();
    let page_size = size.unwrap_or(total.saturating_sub(start));
    let data = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    PagedResult {
        start,
        size: data.len(),
        total,
        data,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormInstanceSort {
    SubmittedDate,
    TenantId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SortOrder {
    Asc,
    Desc,
}

pub struct FormInstanceQuery {
    engine: Arc<ProcessEngine>,
    id: Option<String>,
    ids: Option<BTreeMap<String, ()>>,
    form_definition_id: Option<String>,
    form_definition_id_like: Option<String>,
    form_definition_key: Option<String>,
    process_definition_id: Option<String>,
    process_definition_id_like: Option<String>,
    process_instance_id: Option<String>,
    process_instance_id_like: Option<String>,
    task_id: Option<String>,
    task_id_like: Option<String>,
    without_task_id: bool,
    scope_id: Option<String>,
    scope_type: Option<String>,
    scope_definition_id: Option<String>,
    submitted_date: Option<i64>,
    submitted_date_before: Option<i64>,
    submitted_date_after: Option<i64>,
    submitted_by: Option<String>,
    submitted_by_like: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
    sort: FormInstanceSort,
    order: SortOrder,
    start: usize,
    size: Option<usize>,
}

impl FormInstanceQuery {
    pub(crate) fn new(engine: Arc<ProcessEngine>) -> Self {
        Self {
            engine,
            id: None,
            ids: None,
            form_definition_id: None,
            form_definition_id_like: None,
            form_definition_key: None,
            process_definition_id: None,
            process_definition_id_like: None,
            process_instance_id: None,
            process_instance_id_like: None,
            task_id: None,
            task_id_like: None,
            without_task_id: false,
            scope_id: None,
            scope_type: None,
            scope_definition_id: None,
            submitted_date: None,
            submitted_date_before: None,
            submitted_date_after: None,
            submitted_by: None,
            submitted_by_like: None,
            tenant_id: None,
            tenant_id_like: None,
            without_tenant_id: false,
            sort: FormInstanceSort::SubmittedDate,
            order: SortOrder::Asc,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Only select form instances whose id is in the given set (Java `ids`).
    pub fn ids(mut self, ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut map = BTreeMap::new();
        for id in ids {
            map.insert(id.into(), ());
        }
        self.ids = Some(map);
        self
    }

    pub fn form_definition_id(mut self, form_definition_id: impl Into<String>) -> Self {
        self.form_definition_id = Some(form_definition_id.into());
        self
    }

    pub fn form_definition_id_like(mut self, form_definition_id_like: impl Into<String>) -> Self {
        self.form_definition_id_like = Some(form_definition_id_like.into());
        self
    }

    pub fn form_definition_key(mut self, form_definition_key: impl Into<String>) -> Self {
        self.form_definition_key = Some(form_definition_key.into());
        self
    }

    pub fn process_definition_id(mut self, process_definition_id: impl Into<String>) -> Self {
        self.process_definition_id = Some(process_definition_id.into());
        self
    }

    pub fn process_definition_id_like(
        mut self,
        process_definition_id_like: impl Into<String>,
    ) -> Self {
        self.process_definition_id_like = Some(process_definition_id_like.into());
        self
    }

    pub fn process_instance_id(mut self, process_instance_id: impl Into<String>) -> Self {
        self.process_instance_id = Some(process_instance_id.into());
        self
    }

    pub fn process_instance_id_like(mut self, process_instance_id_like: impl Into<String>) -> Self {
        self.process_instance_id_like = Some(process_instance_id_like.into());
        self
    }

    pub fn task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn task_id_like(mut self, task_id_like: impl Into<String>) -> Self {
        self.task_id_like = Some(task_id_like.into());
        self
    }

    /// Only select submitted forms that do not have a task id (Java `withoutTaskId`).
    pub fn without_task_id(mut self) -> Self {
        self.without_task_id = true;
        self
    }

    pub fn scope_id(mut self, scope_id: impl Into<String>) -> Self {
        self.scope_id = Some(scope_id.into());
        self
    }

    pub fn scope_type(mut self, scope_type: impl Into<String>) -> Self {
        self.scope_type = Some(scope_type.into());
        self
    }

    pub fn scope_definition_id(mut self, scope_definition_id: impl Into<String>) -> Self {
        self.scope_definition_id = Some(scope_definition_id.into());
        self
    }

    pub fn submitted_date(mut self, submitted_date: i64) -> Self {
        self.submitted_date = Some(submitted_date);
        self
    }

    pub fn submitted_date_before(mut self, submitted_date_before: i64) -> Self {
        self.submitted_date_before = Some(submitted_date_before);
        self
    }

    pub fn submitted_date_after(mut self, submitted_date_after: i64) -> Self {
        self.submitted_date_after = Some(submitted_date_after);
        self
    }

    pub fn submitted_by(mut self, submitted_by: impl Into<String>) -> Self {
        self.submitted_by = Some(submitted_by.into());
        self
    }

    pub fn submitted_by_like(mut self, submitted_by_like: impl Into<String>) -> Self {
        self.submitted_by_like = Some(submitted_by_like.into());
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn tenant_id_like(mut self, tenant_id_like: impl Into<String>) -> Self {
        self.tenant_id_like = Some(tenant_id_like.into());
        self
    }

    pub fn without_tenant_id(mut self) -> Self {
        self.without_tenant_id = true;
        self
    }

    pub fn order_by_submitted_date(mut self) -> Self {
        self.sort = FormInstanceSort::SubmittedDate;
        self
    }

    pub fn order_by_tenant_id(mut self) -> Self {
        self.sort = FormInstanceSort::TenantId;
        self
    }

    pub fn asc(mut self) -> Self {
        self.order = SortOrder::Asc;
        self
    }

    pub fn desc(mut self) -> Self {
        self.order = SortOrder::Desc;
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<FormInstance>, FlowableError> {
        let store = self.engine.get_runtime_store();
        // Repository applies physical-column filters when possible; remaining
        // like/scope filters and sorting still run here for correctness.
        let mut instances = repository::list_form_instances_filtered(
            &store,
            repository::FormInstanceListFilter {
                id: self.id.as_deref(),
                form_definition_id: self.form_definition_id.as_deref(),
                form_definition_key: self.form_definition_key.as_deref(),
                process_definition_id: self.process_definition_id.as_deref(),
                process_instance_id: self.process_instance_id.as_deref(),
                task_id: self.task_id.as_deref(),
                without_task_id: self.without_task_id,
                scope_id: self.scope_id.as_deref(),
                scope_type: self.scope_type.as_deref(),
                scope_definition_id: self.scope_definition_id.as_deref(),
                tenant_id: self.tenant_id.as_deref(),
                without_tenant_id: self.without_tenant_id,
                submitted_date: self.submitted_date,
                submitted_date_before: self.submitted_date_before,
                submitted_date_after: self.submitted_date_after,
                submitted_by: self.submitted_by.as_deref(),
            },
        );

        if let Some(ids) = self.ids.as_ref() {
            instances.retain(|item| ids.contains_key(&item.id));
        }
        instances.retain(|item| {
            self.form_definition_id_like
                .as_ref()
                .is_none_or(|pattern| sql_like_matches(&item.form_definition_id, pattern))
        });
        instances.retain(|item| {
            self.process_definition_id_like.as_ref().is_none_or(|pattern| {
                matches_optional_like(pattern, item.process_definition_id.as_deref())
            })
        });
        instances.retain(|item| {
            self.process_instance_id_like.as_ref().is_none_or(|pattern| {
                matches_optional_like(pattern, item.process_instance_id.as_deref())
            })
        });
        instances.retain(|item| {
            self.task_id_like
                .as_ref()
                .is_none_or(|pattern| matches_optional_like(pattern, item.task_id.as_deref()))
        });
        instances.retain(|item| {
            self.submitted_by_like
                .as_ref()
                .is_none_or(|value| matches_optional_like(value, item.submitted_by.as_deref()))
        });
        instances.retain(|item| {
            self.tenant_id_like
                .as_ref()
                .is_none_or(|pattern| matches_optional_like(pattern, item.tenant_id.as_deref()))
        });

        instances.sort_by(|left, right| {
            let primary = match (self.sort, self.order) {
                (FormInstanceSort::SubmittedDate, SortOrder::Asc) => {
                    left.submitted_at.cmp(&right.submitted_at)
                }
                (FormInstanceSort::SubmittedDate, SortOrder::Desc) => {
                    right.submitted_at.cmp(&left.submitted_at)
                }
                (FormInstanceSort::TenantId, SortOrder::Asc) => {
                    left.tenant_id.cmp(&right.tenant_id)
                }
                (FormInstanceSort::TenantId, SortOrder::Desc) => {
                    right.tenant_id.cmp(&left.tenant_id)
                }
            };
            primary.then(left.id.cmp(&right.id))
        });
        Ok(instances)
    }

    pub fn list_page(&self) -> Result<PagedResult<FormInstance>, FlowableError> {
        let instances = self.list()?;
        Ok(page_items(instances, self.start, self.size))
    }

    pub fn count(&self) -> Result<usize, FlowableError> {
        Ok(self.list()?.len())
    }
}

fn matches_optional_like(pattern: &str, actual: Option<&str>) -> bool {
    actual.is_some_and(|value| sql_like_matches(value, pattern))
}

/// Local signature is `(value, pattern)`; shared impl is `(pattern, value)`.
fn sql_like_matches(value: &str, pattern: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, value)
}
