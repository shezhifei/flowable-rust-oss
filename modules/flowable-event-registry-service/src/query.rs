use crate::models::{
    ChannelDefinition, EventDefinition, EventDirection, EventInstanceDelivery, EventInstanceStatus,
    EventRegistryDeployment, PagedResult, page_items,
};
use crate::tenant_fallback::{resolve_definition_with_fallback, TenantFallbackPolicy};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct EventRegistryDeploymentQuery {
    engine: Arc<ProcessEngine>,
    name: Option<String>,
    name_like: Option<String>,
    category: Option<String>,
    category_not_equals: Option<String>,
    parent_deployment_id: Option<String>,
    parent_deployment_id_like: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
    event_definition_key: Option<String>,
    event_definition_key_like: Option<String>,
    channel_definition_key: Option<String>,
    channel_definition_key_like: Option<String>,
    sort: Option<String>,
    descending: bool,
    start: usize,
    size: Option<usize>,
}

impl EventRegistryDeploymentQuery {
    pub(crate) fn new(engine: Arc<ProcessEngine>) -> Self {
        Self {
            engine,
            name: None,
            name_like: None,
            category: None,
            category_not_equals: None,
            parent_deployment_id: None,
            parent_deployment_id_like: None,
            tenant_id: None,
            tenant_id_like: None,
            without_tenant_id: false,
            event_definition_key: None,
            event_definition_key_like: None,
            channel_definition_key: None,
            channel_definition_key_like: None,
            sort: None,
            descending: false,
            start: 0,
            size: None,
        }
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn name_like(mut self, name_like: impl Into<String>) -> Self {
        self.name_like = Some(name_like.into());
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn category_not_equals(mut self, category_not_equals: impl Into<String>) -> Self {
        self.category_not_equals = Some(category_not_equals.into());
        self
    }

    pub fn parent_deployment_id(mut self, parent_deployment_id: impl Into<String>) -> Self {
        self.parent_deployment_id = Some(parent_deployment_id.into());
        self
    }

    pub fn parent_deployment_id_like(
        mut self,
        parent_deployment_id_like: impl Into<String>,
    ) -> Self {
        self.parent_deployment_id_like = Some(parent_deployment_id_like.into());
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

    pub fn event_definition_key(mut self, key: impl Into<String>) -> Self {
        self.event_definition_key = Some(key.into());
        self
    }

    pub fn event_definition_key_like(mut self, key_like: impl Into<String>) -> Self {
        self.event_definition_key_like = Some(key_like.into());
        self
    }

    pub fn channel_definition_key(mut self, key: impl Into<String>) -> Self {
        self.channel_definition_key = Some(key.into());
        self
    }

    pub fn channel_definition_key_like(mut self, key_like: impl Into<String>) -> Self {
        self.channel_definition_key_like = Some(key_like.into());
        self
    }

    pub fn order_by(mut self, sort: impl Into<String>, descending: bool) -> Self {
        self.sort = Some(sort.into());
        self.descending = descending;
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list_page(&self) -> Result<PagedResult<EventRegistryDeployment>, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        let mut deployments = store.list_event_registry_deployments(&mut session);
        deployments.retain(|item| matches_optional(&self.name, &item.name));
        deployments.retain(|item| matches_like_optional(&self.name_like, &item.name));
        deployments
            .retain(|item| matches_optional_option(&self.category, item.category.as_deref()));
        deployments.retain(|item| {
            self.category_not_equals
                .as_ref()
                .is_none_or(|value| item.category.as_deref() != Some(value.as_str()))
        });
        deployments.retain(|item| {
            matches_optional_option(
                &self.parent_deployment_id,
                item.parent_deployment_id.as_deref(),
            )
        });
        deployments.retain(|item| {
            matches_like_optional_option(
                &self.parent_deployment_id_like,
                item.parent_deployment_id.as_deref(),
            )
        });
        deployments
            .retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));
        deployments.retain(|item| {
            matches_like_optional_option(&self.tenant_id_like, item.tenant_id.as_deref())
        });
        deployments.retain(|item| !self.without_tenant_id || item.tenant_id.is_none());

        if self.event_definition_key.is_some() || self.event_definition_key_like.is_some() {
            let event_definitions = store.list_event_registry_event_definitions(&mut session);
            deployments.retain(|deployment| {
                event_definitions.iter().any(|definition| {
                    definition.deployment_id == deployment.id
                        && matches_optional(&self.event_definition_key, &definition.key)
                        && matches_like_optional(&self.event_definition_key_like, &definition.key)
                })
            });
        }
        if self.channel_definition_key.is_some() || self.channel_definition_key_like.is_some() {
            let channel_definitions = store.list_event_registry_channel_definitions(&mut session);
            deployments.retain(|deployment| {
                channel_definitions.iter().any(|definition| {
                    definition.deployment_id == deployment.id
                        && matches_optional(&self.channel_definition_key, &definition.key)
                        && matches_like_optional(&self.channel_definition_key_like, &definition.key)
                })
            });
        }
        deployments.sort_by(|left, right| {
            let ordering = match self.sort.as_deref().unwrap_or("id") {
                "name" => left.name.cmp(&right.name),
                "deployTime" => left.deployed_at.cmp(&right.deployed_at),
                "tenantId" => left.tenant_id.cmp(&right.tenant_id),
                _ => left.id.cmp(&right.id),
            }
            .then(left.id.cmp(&right.id));
            if self.descending {
                ordering.reverse()
            } else {
                ordering
            }
        });

        Ok(page_items(deployments, self.start, self.size))
    }
}

pub struct ChannelDefinitionQuery {
    engine: Arc<ProcessEngine>,
    id: Option<String>,
    key: Option<String>,
    key_like: Option<String>,
    key_like_ignore_case: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    category: Option<String>,
    category_like: Option<String>,
    category_not_equals: Option<String>,
    deployment_id: Option<String>,
    channel_type: Option<String>,
    resource_name: Option<String>,
    resource_name_like: Option<String>,
    implementation: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    parent_deployment_id: Option<String>,
    version: Option<i32>,
    latest: bool,
    /// P133: ChannelDefinitionCollectionResource.java:126-133 (create_time ms)
    create_time: Option<i64>,
    create_time_after: Option<i64>,
    create_time_before: Option<i64>,
    sort: Option<String>,
    descending: bool,
    start: usize,
    size: Option<usize>,
    unsupported_filters: BTreeMap<String, String>,
}

impl ChannelDefinitionQuery {
    pub(crate) fn new(engine: Arc<ProcessEngine>) -> Self {
        Self {
            engine,
            id: None,
            key: None,
            key_like: None,
            key_like_ignore_case: None,
            name: None,
            name_like: None,
            name_like_ignore_case: None,
            category: None,
            category_like: None,
            category_not_equals: None,
            deployment_id: None,
            channel_type: None,
            resource_name: None,
            resource_name_like: None,
            implementation: None,
            tenant_id: None,
            tenant_id_like: None,
            parent_deployment_id: None,
            version: None,
            latest: false,
            create_time: None,
            create_time_after: None,
            create_time_before: None,
            sort: None,
            descending: false,
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

    pub fn key_like(mut self, key_like: impl Into<String>) -> Self {
        self.key_like = Some(key_like.into());
        self
    }

    pub fn key_like_ignore_case(mut self, key_like_ignore_case: impl Into<String>) -> Self {
        self.key_like_ignore_case = Some(key_like_ignore_case.into());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn name_like(mut self, name_like: impl Into<String>) -> Self {
        self.name_like = Some(name_like.into());
        self
    }

    pub fn name_like_ignore_case(mut self, name_like_ignore_case: impl Into<String>) -> Self {
        self.name_like_ignore_case = Some(name_like_ignore_case.into());
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn category_like(mut self, category_like: impl Into<String>) -> Self {
        self.category_like = Some(category_like.into());
        self
    }

    pub fn category_not_equals(mut self, category_not_equals: impl Into<String>) -> Self {
        self.category_not_equals = Some(category_not_equals.into());
        self
    }

    pub fn deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
        self
    }

    pub fn channel_type(mut self, channel_type: impl Into<String>) -> Self {
        self.channel_type = Some(channel_type.into());
        self
    }

    pub fn resource_name(mut self, resource_name: impl Into<String>) -> Self {
        self.resource_name = Some(resource_name.into());
        self
    }

    pub fn resource_name_like(mut self, resource_name_like: impl Into<String>) -> Self {
        self.resource_name_like = Some(resource_name_like.into());
        self
    }

    pub fn implementation(mut self, implementation: impl Into<String>) -> Self {
        self.implementation = Some(implementation.into());
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

    pub fn parent_deployment_id(mut self, parent_deployment_id: impl Into<String>) -> Self {
        self.parent_deployment_id = Some(parent_deployment_id.into());
        self
    }

    pub fn version(mut self, version: i32) -> Self {
        self.version = Some(version);
        self
    }

    pub fn latest(mut self) -> Self {
        self.latest = true;
        self
    }

    /// P133: ChannelDefinitionCollectionResource.java:126-127
    pub fn create_time(mut self, create_time: i64) -> Self {
        self.create_time = Some(create_time);
        self
    }

    /// P133: ChannelDefinitionCollectionResource.java:129-130
    pub fn create_time_after(mut self, create_time_after: i64) -> Self {
        self.create_time_after = Some(create_time_after);
        self
    }

    /// P133: ChannelDefinitionCollectionResource.java:132-133
    pub fn create_time_before(mut self, create_time_before: i64) -> Self {
        self.create_time_before = Some(create_time_before);
        self
    }

    pub fn order_by(mut self, sort: impl Into<String>, descending: bool) -> Self {
        self.sort = Some(sort.into());
        self.descending = descending;
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

    pub fn list(&self) -> Result<Vec<ChannelDefinition>, FlowableError> {
        validate_unsupported_filters("channel definition", &self.unsupported_filters)?;

        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        let mut definitions = store.list_event_registry_channel_definitions(&mut session);
        definitions.retain(|item| matches_optional(&self.id, &item.id));
        definitions.retain(|item| matches_optional(&self.key, &item.key));
        definitions.retain(|item| matches_like_optional(&self.key_like, &item.key));
        definitions.retain(|item| {
            matches_like_ignore_case_optional(&self.key_like_ignore_case, &item.key)
        });
        definitions.retain(|item| matches_optional(&self.name, &item.name));
        definitions.retain(|item| matches_like_optional(&self.name_like, &item.name));
        definitions.retain(|item| {
            matches_like_ignore_case_optional(&self.name_like_ignore_case, &item.name)
        });
        definitions
            .retain(|item| matches_optional_option(&self.category, item.category.as_deref()));
        definitions.retain(|item| {
            matches_like_optional_option(&self.category_like, item.category.as_deref())
        });
        definitions.retain(|item| {
            self.category_not_equals
                .as_ref()
                .is_none_or(|value| item.category.as_deref() != Some(value.as_str()))
        });
        definitions.retain(|item| matches_optional(&self.deployment_id, &item.deployment_id));
        definitions.retain(|item| matches_optional(&self.channel_type, &item.channel_type));
        definitions.retain(|item| matches_optional(&self.resource_name, &item.resource_name));
        definitions
            .retain(|item| matches_like_optional(&self.resource_name_like, &item.resource_name));
        definitions.retain(|item| {
            self.implementation.as_ref().is_none_or(|implementation| {
                item.configuration
                    .get("type")
                    .and_then(|value| value.as_str())
                    == Some(implementation.as_str())
            })
        });
        definitions
            .retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));
        definitions.retain(|item| {
            matches_like_optional_option(&self.tenant_id_like, item.tenant_id.as_deref())
        });
        definitions.retain(|item| {
            matches_optional_option(
                &self.parent_deployment_id,
                item.parent_deployment_id.as_deref(),
            )
        });
        definitions.retain(|item| self.version.is_none_or(|value| item.version == value));
        // P133: create_time filters (ChannelDefinitionCollectionResource.java:126-133)
        if let Some(create_time) = self.create_time {
            definitions.retain(|item| item.create_time == create_time);
        }
        if let Some(after) = self.create_time_after {
            definitions.retain(|item| item.create_time > after);
        }
        if let Some(before) = self.create_time_before {
            definitions.retain(|item| item.create_time < before);
        }
        if self.latest {
            definitions = latest_channel_definitions(definitions);
        }
        definitions.sort_by(|left, right| {
            let ordering = match self.sort.as_deref().unwrap_or("name") {
                "id" => left.id.cmp(&right.id),
                "key" => left.key.cmp(&right.key),
                "category" => left.category.cmp(&right.category),
                "deploymentId" => left.deployment_id.cmp(&right.deployment_id),
                "tenantId" => left.tenant_id.cmp(&right.tenant_id),
                "version" => left.version.cmp(&right.version),
                "createTime" => left.create_time.cmp(&right.create_time),
                _ => left.name.cmp(&right.name),
            }
            .then(left.id.cmp(&right.id));
            if self.descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
        Ok(definitions)
    }

    pub fn list_page(&self) -> Result<PagedResult<ChannelDefinition>, FlowableError> {
        let definitions = self.list()?;
        Ok(page_items(definitions, self.start, self.size))
    }
}

pub struct EventDefinitionQuery {
    engine: Arc<ProcessEngine>,
    id: Option<String>,
    key: Option<String>,
    key_like: Option<String>,
    key_like_ignore_case: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    category: Option<String>,
    category_like: Option<String>,
    category_not_equals: Option<String>,
    deployment_id: Option<String>,
    event_type: Option<String>,
    channel_key: Option<String>,
    resource_name: Option<String>,
    resource_name_like: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    parent_deployment_id: Option<String>,
    version: Option<i32>,
    latest: bool,
    sort: Option<String>,
    descending: bool,
    start: usize,
    size: Option<usize>,
    unsupported_filters: BTreeMap<String, String>,
}

impl EventDefinitionQuery {
    pub(crate) fn new(engine: Arc<ProcessEngine>) -> Self {
        Self {
            engine,
            id: None,
            key: None,
            key_like: None,
            key_like_ignore_case: None,
            name: None,
            name_like: None,
            name_like_ignore_case: None,
            category: None,
            category_like: None,
            category_not_equals: None,
            deployment_id: None,
            event_type: None,
            channel_key: None,
            resource_name: None,
            resource_name_like: None,
            tenant_id: None,
            tenant_id_like: None,
            parent_deployment_id: None,
            version: None,
            latest: false,
            sort: None,
            descending: false,
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

    pub fn key_like(mut self, key_like: impl Into<String>) -> Self {
        self.key_like = Some(key_like.into());
        self
    }

    pub fn key_like_ignore_case(mut self, key_like_ignore_case: impl Into<String>) -> Self {
        self.key_like_ignore_case = Some(key_like_ignore_case.into());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn name_like(mut self, name_like: impl Into<String>) -> Self {
        self.name_like = Some(name_like.into());
        self
    }

    pub fn name_like_ignore_case(mut self, name_like_ignore_case: impl Into<String>) -> Self {
        self.name_like_ignore_case = Some(name_like_ignore_case.into());
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn category_like(mut self, category_like: impl Into<String>) -> Self {
        self.category_like = Some(category_like.into());
        self
    }

    pub fn category_not_equals(mut self, category_not_equals: impl Into<String>) -> Self {
        self.category_not_equals = Some(category_not_equals.into());
        self
    }

    pub fn deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
        self
    }

    pub fn event_type(mut self, event_type: impl Into<String>) -> Self {
        self.event_type = Some(event_type.into());
        self
    }

    pub fn channel_key(mut self, channel_key: impl Into<String>) -> Self {
        self.channel_key = Some(channel_key.into());
        self
    }

    pub fn resource_name(mut self, resource_name: impl Into<String>) -> Self {
        self.resource_name = Some(resource_name.into());
        self
    }

    pub fn resource_name_like(mut self, resource_name_like: impl Into<String>) -> Self {
        self.resource_name_like = Some(resource_name_like.into());
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

    pub fn parent_deployment_id(mut self, parent_deployment_id: impl Into<String>) -> Self {
        self.parent_deployment_id = Some(parent_deployment_id.into());
        self
    }

    pub fn version(mut self, version: i32) -> Self {
        self.version = Some(version);
        self
    }

    pub fn latest(mut self) -> Self {
        self.latest = true;
        self
    }

    pub fn order_by(mut self, sort: impl Into<String>, descending: bool) -> Self {
        self.sort = Some(sort.into());
        self.descending = descending;
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

    pub fn list(&self) -> Result<Vec<EventDefinition>, FlowableError> {
        validate_unsupported_filters("event definition", &self.unsupported_filters)?;

        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        let mut definitions = store.list_event_registry_event_definitions(&mut session);
        definitions.retain(|item| matches_optional(&self.id, &item.id));
        definitions.retain(|item| matches_optional(&self.key, &item.key));
        definitions.retain(|item| matches_like_optional(&self.key_like, &item.key));
        definitions.retain(|item| {
            matches_like_ignore_case_optional(&self.key_like_ignore_case, &item.key)
        });
        definitions.retain(|item| matches_optional(&self.name, &item.name));
        definitions.retain(|item| matches_like_optional(&self.name_like, &item.name));
        definitions.retain(|item| {
            matches_like_ignore_case_optional(&self.name_like_ignore_case, &item.name)
        });
        definitions
            .retain(|item| matches_optional_option(&self.category, item.category.as_deref()));
        definitions.retain(|item| {
            matches_like_optional_option(&self.category_like, item.category.as_deref())
        });
        definitions.retain(|item| {
            self.category_not_equals
                .as_ref()
                .is_none_or(|value| item.category.as_deref() != Some(value.as_str()))
        });
        definitions.retain(|item| matches_optional(&self.deployment_id, &item.deployment_id));
        definitions.retain(|item| matches_optional(&self.event_type, &item.event_type));
        definitions.retain(|item| matches_optional(&self.channel_key, &item.channel_key));
        definitions.retain(|item| matches_optional(&self.resource_name, &item.resource_name));
        definitions
            .retain(|item| matches_like_optional(&self.resource_name_like, &item.resource_name));
        definitions
            .retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));
        definitions.retain(|item| {
            matches_like_optional_option(&self.tenant_id_like, item.tenant_id.as_deref())
        });
        definitions.retain(|item| {
            matches_optional_option(
                &self.parent_deployment_id,
                item.parent_deployment_id.as_deref(),
            )
        });
        definitions.retain(|item| self.version.is_none_or(|value| item.version == value));
        if self.latest {
            definitions = latest_event_definitions(definitions);
        }
        definitions.sort_by(|left, right| {
            let ordering = match self.sort.as_deref().unwrap_or("name") {
                "id" => left.id.cmp(&right.id),
                "key" => left.key.cmp(&right.key),
                "category" => left.category.cmp(&right.category),
                "deploymentId" => left.deployment_id.cmp(&right.deployment_id),
                "tenantId" => left.tenant_id.cmp(&right.tenant_id),
                "version" => left.version.cmp(&right.version),
                _ => left.name.cmp(&right.name),
            }
            .then(left.id.cmp(&right.id));
            if self.descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
        Ok(definitions)
    }

    pub fn list_page(&self) -> Result<PagedResult<EventDefinition>, FlowableError> {
        let definitions = self.list()?;
        Ok(page_items(definitions, self.start, self.size))
    }
}

pub struct EventInstanceDeliveryQuery {
    engine: Arc<ProcessEngine>,
    direction: Option<EventDirection>,
    status: Option<EventInstanceStatus>,
    event_type: Option<String>,
    channel_key: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
    start: usize,
    size: Option<usize>,
    unsupported_filters: BTreeMap<String, String>,
}

impl EventInstanceDeliveryQuery {
    pub(crate) fn new(engine: Arc<ProcessEngine>) -> Self {
        Self {
            engine,
            direction: None,
            status: None,
            event_type: None,
            channel_key: None,
            tenant_id: None,
            tenant_id_like: None,
            without_tenant_id: false,
            start: 0,
            size: None,
            unsupported_filters: BTreeMap::new(),
        }
    }

    pub fn direction(mut self, direction: EventDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    pub fn status(mut self, status: EventInstanceStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn event_type(mut self, event_type: impl Into<String>) -> Self {
        self.event_type = Some(event_type.into());
        self
    }

    pub fn channel_key(mut self, channel_key: impl Into<String>) -> Self {
        self.channel_key = Some(channel_key.into());
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

    pub fn without_tenant_id(mut self, without_tenant_id: bool) -> Self {
        self.without_tenant_id = without_tenant_id;
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

    pub fn list_page(&self) -> Result<PagedResult<EventInstanceDelivery>, FlowableError> {
        validate_unsupported_filters("event instance delivery", &self.unsupported_filters)?;

        let store = self.engine.get_runtime_store();
        let mut session = store.create_session()?;
        let mut deliveries = store.list_event_registry_event_instance_deliveries(&mut session)?;
        deliveries.retain(|item| {
            self.direction
                .as_ref()
                .is_none_or(|value| item.direction == *value)
        });
        deliveries.retain(|item| {
            self.status
                .as_ref()
                .is_none_or(|value| item.status == *value)
        });
        deliveries.retain(|item| matches_optional(&self.event_type, &item.event_type));
        deliveries.retain(|item| matches_optional(&self.channel_key, &item.channel_key));
        if self.without_tenant_id {
            deliveries.retain(|item| item.tenant_id.is_none());
        } else if let Some(tenant_id) = self.tenant_id.as_deref() {
            deliveries.retain(|item| item.tenant_id.as_deref() == Some(tenant_id));
        } else if let Some(tenant_id_like) = self.tenant_id_like.as_deref() {
            deliveries.retain(|item| {
                item.tenant_id
                    .as_deref()
                    .is_some_and(|value| sql_like_matches(value, tenant_id_like))
            });
        }
        deliveries.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then(left.id.cmp(&right.id))
        });

        Ok(page_items(deliveries, self.start, self.size))
    }
}

pub(crate) fn validate_unsupported_filters(
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

pub(crate) fn matches_optional(filter: &Option<String>, actual: &str) -> bool {
    filter.as_ref().is_none_or(|value| value == actual)
}

pub(crate) fn matches_optional_option(filter: &Option<String>, actual: Option<&str>) -> bool {
    filter
        .as_ref()
        .is_none_or(|value| actual == Some(value.as_str()))
}

pub(crate) fn matches_like_optional(pattern: &Option<String>, actual: &str) -> bool {
    pattern
        .as_ref()
        .is_none_or(|pattern| sql_like_matches(actual, pattern))
}

pub(crate) fn matches_like_optional_option(pattern: &Option<String>, actual: Option<&str>) -> bool {
    pattern
        .as_ref()
        .is_none_or(|pattern| actual.is_some_and(|actual| sql_like_matches(actual, pattern)))
}

pub(crate) fn matches_like_ignore_case_optional(pattern: &Option<String>, actual: &str) -> bool {
    pattern.as_ref().is_none_or(|pattern| {
        sql_like_matches(&actual.to_ascii_lowercase(), &pattern.to_ascii_lowercase())
    })
}

/// Local signature is `(value, pattern)`; shared impl is `(pattern, value)`.
pub(crate) fn sql_like_matches(value: &str, pattern: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, value)
}

pub(crate) fn latest_channel_definitions(
    definitions: Vec<ChannelDefinition>,
) -> Vec<ChannelDefinition> {
    let mut latest = Vec::<ChannelDefinition>::new();
    for definition in definitions {
        if let Some(existing) = latest.iter_mut().find(|candidate| {
            candidate.key == definition.key && candidate.tenant_id == definition.tenant_id
        }) {
            if definition.version > existing.version {
                *existing = definition;
            }
        } else {
            latest.push(definition);
        }
    }
    latest
}

pub(crate) fn latest_event_definitions(
    definitions: Vec<EventDefinition>,
) -> Vec<EventDefinition> {
    let mut latest = Vec::<EventDefinition>::new();
    for definition in definitions {
        if let Some(existing) = latest.iter_mut().find(|candidate| {
            candidate.key == definition.key && candidate.tenant_id == definition.tenant_id
        }) {
            if definition.version > existing.version {
                *existing = definition;
            }
        } else {
            latest.push(definition);
        }
    }
    latest
}

/// Latest event definition honouring Java tenant-fallback policy
/// (`GetEventModelCmd.java:82-90` / `DefaultInboundEventProcessingPipeline.java:120-136`).
pub(crate) fn latest_event_definition_for_tenant_with_policy(
    definitions: Vec<EventDefinition>,
    tenant_id: Option<&str>,
    policy: &TenantFallbackPolicy,
) -> Option<EventDefinition> {
    resolve_definition_with_fallback(tenant_id, policy, |lookup_tenant| {
        latest_event_definition_matching_tenant(definitions.iter().cloned(), lookup_tenant)
    })
}

pub(crate) fn latest_event_definition_matching_tenant<I>(
    definitions: I,
    tenant_id: Option<&str>,
) -> Option<EventDefinition>
where
    I: IntoIterator<Item = EventDefinition>,
{
    definitions
        .into_iter()
        .filter(|definition| definition.tenant_id.as_deref() == tenant_id)
        .max_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then(left.key.cmp(&right.key))
                .then(left.id.cmp(&right.id))
        })
}

pub(crate) fn latest_channel_definition_matching_tenant<I>(
    definitions: I,
    tenant_id: Option<&str>,
) -> Option<ChannelDefinition>
where
    I: IntoIterator<Item = ChannelDefinition>,
{
    definitions
        .into_iter()
        .filter(|definition| definition.tenant_id.as_deref() == tenant_id)
        .max_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then(left.key.cmp(&right.key))
                .then(left.id.cmp(&right.id))
        })
}
