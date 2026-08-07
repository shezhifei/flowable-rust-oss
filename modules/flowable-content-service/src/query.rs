use crate::models::{ContentItem, PagedResult};
use crate::repository;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContentItemSort {
    CreatedDate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SortOrder {
    Asc,
    Desc,
}

pub struct ContentItemQuery {
    engine: Arc<ProcessEngine>,
    id: Option<String>,
    name: Option<String>,
    mime_type: Option<String>,
    task_id: Option<String>,
    process_instance_id: Option<String>,
    scope_type: Option<String>,
    scope_id: Option<String>,
    created_by: Option<String>,
    sort: ContentItemSort,
    order: SortOrder,
    start: usize,
    size: Option<usize>,
    unsupported_filters: BTreeMap<String, String>,
}

impl ContentItemQuery {
    pub(crate) fn new(engine: Arc<ProcessEngine>) -> Self {
        Self {
            engine,
            id: None,
            name: None,
            mime_type: None,
            task_id: None,
            process_instance_id: None,
            scope_type: None,
            scope_id: None,
            created_by: None,
            sort: ContentItemSort::CreatedDate,
            order: SortOrder::Asc,
            start: 0,
            size: None,
            unsupported_filters: BTreeMap::new(),
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

    pub fn mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    pub fn task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn process_instance_id(mut self, process_instance_id: impl Into<String>) -> Self {
        self.process_instance_id = Some(process_instance_id.into());
        self
    }

    pub fn scope_type(mut self, scope_type: impl Into<String>) -> Self {
        self.scope_type = Some(scope_type.into());
        self
    }

    pub fn scope_id(mut self, scope_id: impl Into<String>) -> Self {
        self.scope_id = Some(scope_id.into());
        self
    }

    pub fn created_by(mut self, created_by: impl Into<String>) -> Self {
        self.created_by = Some(created_by.into());
        self
    }

    pub fn order_by_created_date(mut self) -> Self {
        self.sort = ContentItemSort::CreatedDate;
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

    pub fn unsupported_filter(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.unsupported_filters.insert(name.into(), value.into());
        self
    }

    pub fn list(&self) -> Result<Vec<ContentItem>, FlowableError> {
        validate_unsupported_filters("content item", &self.unsupported_filters)?;

        let store = self.engine.get_runtime_store();
        let mut items = repository::list_content_items(&store);
        items.retain(|item| matches_optional(&self.id, &item.id));
        items.retain(|item| matches_optional(&self.name, &item.name));
        items.retain(|item| matches_optional_option(&self.mime_type, &item.mime_type));
        items.retain(|item| matches_optional_option(&self.task_id, &item.task_id));
        items.retain(|item| {
            matches_optional_option(&self.process_instance_id, &item.process_instance_id)
        });
        items.retain(|item| matches_optional_option(&self.scope_type, &item.scope_type));
        items.retain(|item| matches_optional_option(&self.scope_id, &item.scope_id));
        items.retain(|item| matches_optional_option(&self.created_by, &item.created_by));
        items.sort_by(|left, right| {
            let ordering = match (self.sort, self.order) {
                (ContentItemSort::CreatedDate, SortOrder::Asc) => {
                    left.created_at.cmp(&right.created_at)
                }
                (ContentItemSort::CreatedDate, SortOrder::Desc) => {
                    right.created_at.cmp(&left.created_at)
                }
            };
            ordering
                .then(left.name.cmp(&right.name))
                .then(left.id.cmp(&right.id))
        });
        Ok(items)
    }

    pub fn list_page(&self) -> Result<PagedResult<ContentItem>, FlowableError> {
        let items = self.list()?;
        Ok(page_items(items, self.start, self.size))
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

fn matches_optional_option(filter: &Option<String>, actual: &Option<String>) -> bool {
    filter.as_ref().is_none_or(|value| {
        actual
            .as_ref()
            .is_some_and(|actual_value| actual_value == value)
    })
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
