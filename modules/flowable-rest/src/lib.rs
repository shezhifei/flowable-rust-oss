use axum::{
    Router,
    extract::Extension,
    http::{HeaderName, Request},
    middleware,
    routing::{delete, get, post},
};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use flowable_app_converter::parse_app_definition;
use flowable_app_engine::{
    AppDefinitionRecord as EngineAppDefinitionRecord, AppDeployment as EngineAppDeployment,
    AppDeploymentRequest as EngineAppDeploymentRequest, AppEngine, AppModel as EngineAppModel,
    DefinitionCatalog, DefinitionType, ResolvedAppComposition, ResolvedDefinition,
    canonical_definition_to_engine,
};
use flowable_cmmn_engine::{
    CMMN_ENGINE_VERSION, CmmnCase,
    CmmnCaseInstanceStartRequest as EngineCmmnCaseInstanceStartRequest, CmmnCaseInstanceState,
    CmmnChangePlanItemStateRequest as EngineCmmnChangePlanItemStateRequest, CmmnDecisionResolver,
    CmmnEngine, CmmnEventSubscription, CmmnFormResolver, CmmnHistoricCaseInstance,
    CmmnHistoricHumanTaskInstance, CmmnHistoricMilestoneInstance, CmmnHumanTask,
    CmmnHumanTaskCompletionRequest as EngineCmmnHumanTaskCompletionRequest, CmmnHumanTaskState,
    CmmnHumanTaskUpdate as EngineCmmnHumanTaskUpdate, CmmnIdentityLink, CmmnMigrationDocument,
    CmmnPlanItemDefinitionWithTargetIds, CmmnStage, CmmnUserGroupResolver,
    CmmnStageOverview, ReferencedDecision, ReferencedFormDefinition,
};
use flowable_cmmn_image_generator::CmmnSvgGenerator;
use flowable_content_service::{CreateContentItemRequest, FlowableContentService};
use flowable_dmn_converter::parse_dmn_definition;
use flowable_dmn_engine::{
    DmnDecisionDefinition, DmnDeploymentRequest as EngineDmnDeploymentRequest, DmnEngine,
    DmnExecutionRequest, DmnHitPolicy, DmnModel, DmnRuleExecutionAudit,
};
use flowable_dmn_image_generator::DmnSvgGenerator;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::{Group, Membership, User};
use flowable_engine::runtime::process_instance::ProcessInstance;
use flowable_event_registry_service::FlowableEventRegistryService;
use flowable_form_service::{
    FlowableFormService, FormDefinition, FormDeploymentRequest, FormDeploymentResource,
    FormManagementService,
};
use flowable_image_generator::{
    DefaultProcessDiagramGenerator, ProcessDiagramRenderOptions, generate_process_svg,
};
use flowable_platform_bootstrap::{
    BoundedLdapLiveDirectoryProvider, DirectoryReadSnapshot, FlowablePlatform,
    LiveDirectoryMutationError,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

pub mod common;
pub mod config;
pub mod error;
pub mod routes;
pub mod security;
pub(crate) mod query_variable;
pub(crate) mod variable_types;

#[derive(Clone)]
pub(crate) struct DirectoryReadState {
    live_provider: Option<Arc<BoundedLdapLiveDirectoryProvider>>,
}

impl DirectoryReadState {
    fn internal() -> Arc<Self> {
        Arc::new(Self {
            live_provider: None,
        })
    }

    fn from_platform(platform: &FlowablePlatform) -> Arc<Self> {
        Arc::new(Self {
            live_provider: platform.live_directory_provider(),
        })
    }

    pub(crate) fn load_live_snapshot(
        &self,
    ) -> Result<Option<DirectoryReadSnapshot>, crate::error::ApiError> {
        self.live_provider
            .as_ref()
            .map(|provider| {
                provider
                    .load_snapshot()
                    .map_err(|error| crate::error::ApiError::InternalServerError(error.to_string()))
            })
            .transpose()
    }

    pub(crate) fn has_live_provider(&self) -> bool {
        self.live_provider.is_some()
    }

    pub(crate) fn save_live_user(
        &self,
        user: User,
    ) -> Result<Option<User>, crate::error::ApiError> {
        self.live_provider
            .as_ref()
            .map(|provider| provider.save_user(user).map_err(map_live_mutation_error))
            .transpose()
    }

    pub(crate) fn delete_live_user(
        &self,
        user_id: &str,
    ) -> Result<Option<bool>, crate::error::ApiError> {
        self.live_provider
            .as_ref()
            .map(|provider| {
                provider
                    .delete_user(user_id)
                    .map_err(map_live_mutation_error)
            })
            .transpose()
    }

    pub(crate) fn save_live_group(
        &self,
        group: Group,
    ) -> Result<Option<Group>, crate::error::ApiError> {
        self.live_provider
            .as_ref()
            .map(|provider| provider.save_group(group).map_err(map_live_mutation_error))
            .transpose()
    }

    pub(crate) fn delete_live_group(
        &self,
        group_id: &str,
    ) -> Result<Option<bool>, crate::error::ApiError> {
        self.live_provider
            .as_ref()
            .map(|provider| {
                provider
                    .delete_group(group_id)
                    .map_err(map_live_mutation_error)
            })
            .transpose()
    }

    pub(crate) fn create_live_membership(
        &self,
        user_id: &str,
        group_id: &str,
    ) -> Result<Option<Membership>, crate::error::ApiError> {
        self.live_provider
            .as_ref()
            .map(|provider| {
                provider
                    .create_membership(user_id, group_id)
                    .map_err(map_live_mutation_error)
            })
            .transpose()
    }

    pub(crate) fn delete_live_membership(
        &self,
        user_id: &str,
        group_id: &str,
    ) -> Result<Option<bool>, crate::error::ApiError> {
        self.live_provider
            .as_ref()
            .map(|provider| {
                provider
                    .delete_membership(user_id, group_id)
                    .map_err(map_live_mutation_error)
            })
            .transpose()
    }
}

fn map_live_mutation_error(error: LiveDirectoryMutationError) -> crate::error::ApiError {
    match error {
        LiveDirectoryMutationError::NotFound(message) => crate::error::ApiError::NotFound(message),
        LiveDirectoryMutationError::Conflict(message)
        | LiveDirectoryMutationError::InvalidReference(message) => {
            crate::error::ApiError::BadRequest(message)
        }
        LiveDirectoryMutationError::Storage(message) => {
            crate::error::ApiError::InternalServerError(message)
        }
    }
}

pub(crate) fn merge_users(
    stored_users: Vec<User>,
    live_users: impl IntoIterator<Item = User>,
) -> Vec<User> {
    let mut merged = BTreeMap::new();
    for user in stored_users {
        merged.insert(user.id.clone(), user);
    }
    for user in live_users {
        merged.insert(user.id.clone(), user);
    }
    merged.into_values().collect()
}

pub(crate) fn merge_groups(
    stored_groups: Vec<Group>,
    live_groups: impl IntoIterator<Item = Group>,
) -> Vec<Group> {
    let mut merged = BTreeMap::new();
    for group in stored_groups {
        merged.insert(group.id.clone(), group);
    }
    for group in live_groups {
        merged.insert(group.id.clone(), group);
    }
    merged.into_values().collect()
}

pub(crate) fn merge_memberships(
    stored_memberships: Vec<Membership>,
    live_memberships: impl IntoIterator<Item = Membership>,
) -> Vec<Membership> {
    let mut merged = BTreeMap::new();
    for membership in stored_memberships {
        merged.insert(
            (membership.user_id.clone(), membership.group_id.clone()),
            membership,
        );
    }
    for membership in live_memberships {
        merged.insert(
            (membership.user_id.clone(), membership.group_id.clone()),
            membership,
        );
    }
    merged.into_values().collect()
}

#[derive(Clone)]
struct FormRepositoryAdapter {
    service: FlowableFormService,
    engine: Arc<ProcessEngine>,
}

impl routes::forms::FormRepositoryApi for FormRepositoryAdapter {
    fn deploy_form_definitions(
        &self,
        command: routes::forms::FormDeploymentCommand,
    ) -> Result<routes::forms::FormDeploymentRecord, crate::error::ApiError> {
        let deployment = self.service.deploy(FormDeploymentRequest {
            name: command.name,
            resources: command
                .resources
                .into_iter()
                .map(|resource| FormDeploymentResource {
                    resource_name: resource.resource_name,
                    resource: resource.resource,
                })
                .collect(),
        })?;
        Ok(routes::forms::FormDeploymentRecord {
            id: deployment.id,
            name: deployment.name,
            deployed_at: deployment.deployed_at,
            resource_names: deployment.resource_names,
        })
    }

    fn list_form_definitions(
        &self,
        query: routes::forms::FormDefinitionQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::forms::FormDefinitionRecord>,
        crate::error::ApiError,
    > {
        let mut service_query = self.service.create_form_definition_query();
        if let Some(id) = query.id {
            service_query = service_query.id(id);
        }
        if let Some(key) = query.key {
            service_query = service_query.key(key);
        }
        if let Some(name) = query.name {
            service_query = service_query.name(name);
        }
        if let Some(deployment_id) = query.deployment_id {
            service_query = service_query.deployment_id(deployment_id);
        }
        if let Some(size) = query.paging.size {
            service_query = service_query.page(query.paging.start, size);
        } else if query.paging.start > 0 {
            service_query = service_query.page(query.paging.start, usize::MAX);
        }
        let page = service_query.list_page()?;
        Ok(crate::common::PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page
                .data
                .into_iter()
                .map(|definition| routes::forms::FormDefinitionRecord {
                    id: definition.id,
                    key: definition.key,
                    name: definition.name,
                    version: definition.version,
                    deployment_id: definition.deployment_id,
                    resource_name: definition.resource_name,
                    tenant_id: None,
                    active: definition.active,
                })
                .collect(),
            sort: None,
            order: None,
        })
    }

    fn get_form_definition(
        &self,
        form_definition_id: &str,
    ) -> Result<routes::forms::FormDefinitionRecord, crate::error::ApiError> {
        let definition = self.service.get_form_definition(form_definition_id)?;
        Ok(routes::forms::FormDefinitionRecord {
            id: definition.id,
            key: definition.key,
            name: definition.name,
            version: definition.version,
            deployment_id: definition.deployment_id,
            resource_name: definition.resource_name,
            tenant_id: None,
            active: definition.active,
        })
    }

    // ── M41: Breadth methods ──────────────────────────────────────

    fn list_form_definition_versions(
        &self,
        form_definition_id: &str,
    ) -> Result<Vec<routes::forms::FormDefinitionVersionRecord>, crate::error::ApiError> {
        let definition = self.service.get_form_definition(form_definition_id)?;
        let mgmt = FormManagementService::new(Arc::clone(&self.engine));
        let versions = mgmt.list_versions(&definition.key)?;
        Ok(versions
            .into_iter()
            .map(|v| routes::forms::FormDefinitionVersionRecord {
                id: v.id,
                key: v.key,
                name: v.name,
                version: v.version,
                deployment_id: v.deployment_id,
                resource_name: v.resource_name,
                tenant_id: None,
                active: v.active,
            })
            .collect())
    }

    fn get_form_definition_layout(
        &self,
        form_definition_id: &str,
    ) -> Result<Value, crate::error::ApiError> {
        let definition = self.service.get_form_definition(form_definition_id)?;
        Ok(definition
            .layout
            .unwrap_or(Value::Object(serde_json::Map::new())))
    }

    fn get_form_definition_outcomes(
        &self,
        form_definition_id: &str,
    ) -> Result<Vec<flowable_form_service::FormOutcome>, crate::error::ApiError> {
        let definition = self.service.get_form_definition(form_definition_id)?;
        Ok(definition.outcomes.unwrap_or_default())
    }

    fn delete_form_definitions(
        &self,
        query: routes::forms::FormDeleteQuery,
    ) -> Result<usize, crate::error::ApiError> {
        let mgmt = FormManagementService::new(Arc::clone(&self.engine));
        if let Some(deployment_id) = query.deployment_id {
            Ok(mgmt.delete_definitions_by_deployment_id(&deployment_id)?)
        } else if let Some(key) = query.key {
            Ok(mgmt.delete_definitions_by_key(&key)?)
        } else {
            Err(crate::error::ApiError::bad_request(
                "Either deploymentId or key query parameter is required",
            ))
        }
    }

    fn set_form_definition_activation(
        &self,
        form_definition_id: &str,
        active: bool,
    ) -> Result<routes::forms::FormDefinitionRecord, crate::error::ApiError> {
        let mgmt = FormManagementService::new(Arc::clone(&self.engine));
        let definition = mgmt.set_activation(form_definition_id, active)?;
        Ok(routes::forms::FormDefinitionRecord {
            id: definition.id,
            key: definition.key,
            name: definition.name,
            version: definition.version,
            deployment_id: definition.deployment_id,
            resource_name: definition.resource_name,
            tenant_id: None,
            active: definition.active,
        })
    }
}

#[derive(Clone)]
struct ContentServiceAdapter {
    service: FlowableContentService,
    engine: Arc<ProcessEngine>,
}

impl routes::content::ContentServiceApi for ContentServiceAdapter {
    fn create_content_item(
        &self,
        command: routes::content::ContentItemCreateCommand,
        authenticated_user_id: Option<&str>,
    ) -> Result<routes::content::ContentItemRecord, crate::error::ApiError> {
        // Tenant ownership comes from the authenticated principal's identity
        // record — never from the request payload — so tenant users get a
        // legitimate same-tenant pre-upload path for tenant-scoped form claims.
        let tenant_id = authenticated_user_id.and_then(|user_id| {
            self.engine
                .get_identity_service()
                .find_user_by_id(user_id)
                .and_then(|user| user.tenant_id)
        });
        let item = self.service.create_content_item_for_tenant(
            CreateContentItemRequest {
                name: command.name,
                mime_type: command.mime_type,
                description: command.description,
                attachment_type: command.attachment_type,
                external_url: command.external_url,
                content: command.content,
                task_id: command.task_id,
                process_instance_id: command.process_instance_id,
                scope_type: command.scope_type,
                scope_id: command.scope_id,
                created_by: authenticated_user_id.map(str::to_string),
                expires_in_seconds: command.expires_in_seconds,
            },
            tenant_id.as_deref(),
        )?;
        Ok(routes::content::ContentItemRecord {
            id: item.id,
            name: item.name,
            mime_type: item.mime_type,
            description: item.description,
            attachment_type: item.attachment_type,
            external_url: item.external_url,
            task_id: item.task_id,
            process_instance_id: item.process_instance_id,
            scope_type: item.scope_type,
            scope_id: item.scope_id,
            created: item.created_at,
            modified: item.updated_at,
            content_size: item.content_size,
        })
    }

    fn list_content_items(
        &self,
        query: routes::content::ContentItemQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::content::ContentItemRecord>,
        crate::error::ApiError,
    > {
        let mut service_query = self.service.create_content_item_query();
        if let Some(name) = query.name {
            service_query = service_query.name(name);
        }
        if let Some(mime_type) = query.mime_type {
            service_query = service_query.mime_type(mime_type);
        }
        if let Some(task_id) = query.task_id {
            service_query = service_query.task_id(task_id);
        }
        if let Some(process_instance_id) = query.process_instance_id {
            service_query = service_query.process_instance_id(process_instance_id);
        }
        if let Some(scope_type) = query.scope_type {
            service_query = service_query.scope_type(scope_type);
        }
        if let Some(scope_id) = query.scope_id {
            service_query = service_query.scope_id(scope_id);
        }
        if query.sort == Some(routes::content::ContentItemSort::Created) || query.order.is_some() {
            service_query = service_query.order_by_created_date();
            match query.order.unwrap_or(routes::content::SortOrder::Asc) {
                routes::content::SortOrder::Asc => {
                    service_query = service_query.asc();
                }
                routes::content::SortOrder::Desc => {
                    service_query = service_query.desc();
                }
            }
        }
        if let Some(size) = query.paging.size {
            service_query = service_query.page(query.paging.start, size);
        } else if query.paging.start > 0 {
            service_query = service_query.page(query.paging.start, usize::MAX);
        }
        let page = service_query.list_page()?;
        Ok(crate::common::PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page
                .data
                .into_iter()
                .map(|item| routes::content::ContentItemRecord {
                    id: item.id,
                    name: item.name,
                    mime_type: item.mime_type,
                    description: item.description,
                    attachment_type: item.attachment_type,
                    external_url: item.external_url,
                    task_id: item.task_id,
                    process_instance_id: item.process_instance_id,
                    scope_type: item.scope_type,
                    scope_id: item.scope_id,
                    created: item.created_at,
                    modified: item.updated_at,
                    content_size: item.content_size,
                })
                .collect(),
            sort: None,
            order: None,
        })
    }

    fn get_content_item(
        &self,
        content_item_id: &str,
    ) -> Result<routes::content::ContentItemRecord, crate::error::ApiError> {
        let item = self.service.get_content_item(content_item_id)?;
        Ok(routes::content::ContentItemRecord {
            id: item.id,
            name: item.name,
            mime_type: item.mime_type,
            description: item.description,
            attachment_type: item.attachment_type,
            external_url: item.external_url,
            task_id: item.task_id,
            process_instance_id: item.process_instance_id,
            scope_type: item.scope_type,
            scope_id: item.scope_id,
            created: item.created_at,
            modified: item.updated_at,
            content_size: item.content_size,
        })
    }

    fn get_content_item_data(
        &self,
        content_item_id: &str,
    ) -> Result<routes::content::ContentItemDataRecord, crate::error::ApiError> {
        let item = self.service.get_content_item_data(content_item_id)?;
        Ok(routes::content::ContentItemDataRecord {
            mime_type: item.mime_type,
            content: item.content,
        })
    }

    fn delete_content_item(&self, content_item_id: &str) -> Result<(), crate::error::ApiError> {
        self.service.delete_content_item(content_item_id)?;
        Ok(())
    }

    fn get_content_item_object_metadata(
        &self,
        content_item_id: &str,
    ) -> Result<flowable_content_service::ContentObjectStorageMetadata, crate::error::ApiError>
    {
        Ok(self
            .service
            .get_content_item_object_metadata(content_item_id)?)
    }

    fn get_content_item_object_data(
        &self,
        content_item_id: &str,
    ) -> Result<routes::content::ContentItemDataRecord, crate::error::ApiError> {
        let item = self.service.get_content_item_data(content_item_id)?;
        Ok(routes::content::ContentItemDataRecord {
            mime_type: item.mime_type,
            content: item.content,
        })
    }

    fn get_storage_status(&self) -> Result<Value, crate::error::ApiError> {
        Ok(self.service.get_storage_status())
    }

    fn create_task_attachment(
        &self,
        task_id: String,
        name: String,
        description: Option<String>,
        attachment_type: Option<String>,
        external_url: Option<String>,
        content: Option<Vec<u8>>,
        user_id: Option<String>,
        process_instance_id: Option<String>,
    ) -> Result<routes::content::TaskAttachmentRecord, crate::error::ApiError> {
        let item = self.service.create_task_attachment(
            flowable_content_service::CreateTaskAttachmentInput {
                task_id,
                name,
                description,
                attachment_type,
                external_url,
                content,
                user_id,
                process_instance_id,
            },
        )?;
        Ok(content_item_to_attachment_record(item))
    }

    fn list_task_attachments(
        &self,
        task_id: &str,
    ) -> Result<Vec<routes::content::TaskAttachmentRecord>, crate::error::ApiError> {
        Ok(self
            .service
            .list_task_attachments(task_id)?
            .into_iter()
            .map(content_item_to_attachment_record)
            .collect())
    }

    fn get_task_attachment(
        &self,
        task_id: &str,
        attachment_id: &str,
    ) -> Result<routes::content::TaskAttachmentRecord, crate::error::ApiError> {
        Ok(content_item_to_attachment_record(
            self.service.get_task_attachment(task_id, attachment_id)?,
        ))
    }

    fn get_task_attachment_content(
        &self,
        task_id: &str,
        attachment_id: &str,
    ) -> Result<routes::content::TaskAttachmentContentRecord, crate::error::ApiError> {
        let content = self
            .service
            .get_task_attachment_content(task_id, attachment_id)?;
        Ok(routes::content::TaskAttachmentContentRecord {
            bytes: content.bytes,
            mime_type: content.item.mime_type,
            attachment_type: content.item.attachment_type,
        })
    }

    fn delete_task_attachment(
        &self,
        task_id: &str,
        attachment_id: &str,
        user_id: Option<&str>,
    ) -> Result<(), crate::error::ApiError> {
        self.service
            .delete_task_attachment(task_id, attachment_id, user_id)?;
        Ok(())
    }

    fn create_process_attachment(
        &self,
        process_instance_id: String,
        task_id: Option<String>,
        name: String,
        description: Option<String>,
        attachment_type: Option<String>,
        external_url: Option<String>,
        content: Option<Vec<u8>>,
        user_id: Option<String>,
    ) -> Result<routes::content::TaskAttachmentRecord, crate::error::ApiError> {
        let item = self.service.create_process_attachment(
            flowable_content_service::CreateProcessAttachmentInput {
                process_instance_id,
                task_id,
                name,
                description,
                attachment_type,
                external_url,
                content,
                user_id,
            },
        )?;
        Ok(content_item_to_attachment_record(item))
    }

    fn list_process_attachments(
        &self,
        process_instance_id: &str,
    ) -> Result<Vec<routes::content::TaskAttachmentRecord>, crate::error::ApiError> {
        Ok(self
            .service
            .list_process_attachments(process_instance_id)?
            .into_iter()
            .map(content_item_to_attachment_record)
            .collect())
    }

    fn get_process_attachment(
        &self,
        process_instance_id: &str,
        attachment_id: &str,
    ) -> Result<routes::content::TaskAttachmentRecord, crate::error::ApiError> {
        Ok(content_item_to_attachment_record(
            self.service
                .get_process_attachment(process_instance_id, attachment_id)?,
        ))
    }

    fn get_process_attachment_content(
        &self,
        process_instance_id: &str,
        attachment_id: &str,
    ) -> Result<routes::content::TaskAttachmentContentRecord, crate::error::ApiError> {
        let content = self
            .service
            .get_process_attachment_content(process_instance_id, attachment_id)?;
        Ok(routes::content::TaskAttachmentContentRecord {
            bytes: content.bytes,
            mime_type: content.item.mime_type,
            attachment_type: content.item.attachment_type,
        })
    }

    fn delete_process_attachment(
        &self,
        process_instance_id: &str,
        attachment_id: &str,
        user_id: Option<&str>,
    ) -> Result<(), crate::error::ApiError> {
        self.service
            .delete_process_attachment(process_instance_id, attachment_id, user_id)?;
        Ok(())
    }
}

fn content_item_to_attachment_record(
    item: flowable_content_service::ContentItem,
) -> routes::content::TaskAttachmentRecord {
    routes::content::TaskAttachmentRecord {
        id: item.id,
        name: item.name,
        mime_type: item.mime_type,
        description: item.description,
        attachment_type: item.attachment_type,
        external_url: item.external_url,
        task_id: item.task_id,
        process_instance_id: item.process_instance_id,
        user_id: item.created_by,
        content_size: item.content_size,
        created: Some(item.created_at),
    }
}

#[derive(Clone)]
struct DmnApiAdapter {
    engine: Arc<DmnEngine>,
}

impl DmnApiAdapter {
    fn new(engine: Arc<DmnEngine>) -> Self {
        Self { engine }
    }
}

impl routes::dmn::DmnRepositoryApi for DmnApiAdapter {
    fn deploy_decision_tables(
        &self,
        command: routes::dmn::DmnDeploymentCommand,
    ) -> Result<routes::dmn::DmnDeploymentRecord, crate::error::ApiError> {
        let mut request = EngineDmnDeploymentRequest::new(command.name);
        if let Some(category) = command.category {
            request = request.with_category(category);
        }
        if let Some(parent_deployment_id) = command.parent_deployment_id {
            request = request.with_parent_deployment_id(parent_deployment_id);
        }
        if let Some(tenant_id) = command.tenant_id {
            request = request.with_tenant_id(tenant_id);
        }
        for resource in command.resources {
            let definition = parse_dmn_definition(&resource.resource)
                .map_err(|error| crate::error::ApiError::bad_request(error.to_string()))?;
            let model = DmnModel::try_from(definition)
                .map_err(|error| crate::error::ApiError::bad_request(error.to_string()))?;
            request = request.with_resource_bytes(
                resource.resource_name,
                model,
                resource.resource.into_bytes(),
            );
        }
        let deployment = self.engine.deploy(request)?;
        Ok(routes::dmn::DmnDeploymentRecord {
            id: deployment.id,
            name: deployment.name,
            category: deployment.category,
            parent_deployment_id: deployment.parent_deployment_id,
            deployed_at: deployment.deployed_at.timestamp_millis(),
            resource_names: deployment.resource_names,
            tenant_id: deployment.tenant_id,
        })
    }

    fn get_deployment(
        &self,
        deployment_id: &str,
    ) -> Result<routes::dmn::DmnDeploymentRecord, crate::error::ApiError> {
        let deployment = self
            .engine
            .repository_service()
            .get_deployment(deployment_id)?;
        Ok(routes::dmn::DmnDeploymentRecord {
            id: deployment.id,
            name: deployment.name,
            category: deployment.category,
            parent_deployment_id: deployment.parent_deployment_id,
            deployed_at: deployment.deployed_at.timestamp_millis(),
            resource_names: deployment.resource_names,
            tenant_id: deployment.tenant_id,
        })
    }

    fn list_deployments(
        &self,
        query: routes::dmn::DmnDeploymentQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::dmn::DmnDeploymentRecord>,
        crate::error::ApiError,
    > {
        let paging = query.paging;
        let mut deployment_query = self.engine.repository_service().create_deployment_query();
        if let Some(id) = query.id {
            deployment_query = deployment_query.id(id);
        }
        if let Some(name) = query.name {
            deployment_query = deployment_query.name(name);
        }
        if let Some(category) = query.category {
            deployment_query = deployment_query.category(category);
        }
        if let Some(category_not_equals) = query.category_not_equals {
            deployment_query = deployment_query.category_not_equals(category_not_equals);
        }
        if let Some(parent_deployment_id) = query.parent_deployment_id {
            deployment_query = deployment_query.parent_deployment_id(parent_deployment_id);
        }
        if let Some(parent_deployment_id_like) = query.parent_deployment_id_like {
            deployment_query =
                deployment_query.parent_deployment_id_like(parent_deployment_id_like);
        }
        if let Some(tenant_id) = query.tenant_id {
            deployment_query = deployment_query.tenant_id(tenant_id);
        }
        if let Some(resource_name) = query.resource_name {
            deployment_query = deployment_query.resource_name(resource_name);
        }
        let tenant_id_like = query.tenant_id_like;
        let sort = query.sort;
        let order = query.order;
        let without_tenant_id = query.without_tenant_id;
        let deployments = deployment_query.list()?;
        let mut records = deployments
            .into_iter()
            .map(|deployment| routes::dmn::DmnDeploymentRecord {
                id: deployment.id,
                name: deployment.name,
                category: deployment.category,
                parent_deployment_id: deployment.parent_deployment_id,
                deployed_at: deployment.deployed_at.timestamp_millis(),
                resource_names: deployment.resource_names,
                tenant_id: deployment.tenant_id,
            })
            .collect::<Vec<_>>();
        if let Some(name_like) = query.name_like {
            records.retain(|deployment| deployment.name.contains(&name_like));
        }
        if let Some(tenant_id_like) = tenant_id_like {
            records.retain(|deployment| {
                deployment
                    .tenant_id
                    .as_deref()
                    .is_some_and(|tenant_id| tenant_id.contains(&tenant_id_like))
            });
        }
        if without_tenant_id {
            records.retain(|deployment| deployment.tenant_id.is_none());
        }
        routes::dmn::sort_deployments(&mut records, sort.as_deref(), order.as_deref());
        Ok(paging.paginate(records))
    }

    fn delete_deployment(
        &self,
        deployment_id: &str,
        cascade: bool,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .repository_service()
            .delete_deployment(deployment_id, cascade)?;
        Ok(())
    }

    fn get_deployment_resource_data(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<routes::dmn::DmnResourceDataRecord, crate::error::ApiError> {
        let resource = self
            .engine
            .repository_service()
            .get_deployment_resource_data(deployment_id, resource_name)?;
        Ok(routes::dmn::DmnResourceDataRecord {
            mime_type: resource.content_type,
            bytes: resource.bytes,
        })
    }

    fn list_decision_tables(
        &self,
        query: routes::dmn::DecisionTableQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::dmn::DecisionTableRecord>,
        crate::error::ApiError,
    > {
        let paging = query.paging;
        let mut decision_query = self.engine.repository_service().create_decision_query();
        if let Some(id) = query.id {
            decision_query = decision_query.id(id);
        }
        if let Some(key) = query.key {
            decision_query = decision_query.key(key);
        }
        if let Some(resource_name) = query.resource_name {
            decision_query = decision_query.resource_name(resource_name);
        }
        if let Some(tenant_id) = query.tenant_id {
            decision_query = decision_query.tenant_id(tenant_id);
        }
        if let Some(deployment_id) = query.deployment_id {
            decision_query = decision_query.deployment_id(deployment_id);
        }
        if let Some(version) = query.version {
            decision_query = decision_query.version(version);
        }
        let mut records = decision_query
            .list()?
            .into_iter()
            .map(|decision| routes::dmn::DecisionTableRecord {
                id: decision.id,
                key: decision.key,
                name: decision.name,
                version: decision.version,
                deployment_id: decision.deployment_id,
                resource_name: decision.resource_name,
                category: decision.category,
                description: None,
                tenant_id: decision.tenant_id,
                parent_deployment_id: decision.parent_deployment_id,
            })
            .collect::<Vec<_>>();
        if let Some(name_filter) = query.name {
            records.retain(|decision| decision.name == name_filter);
        }
        if let Some(key_like) = query.key_like {
            records.retain(|decision| dmn_rest_like(&decision.key, &key_like));
        }
        if let Some(name_like) = query.name_like {
            records.retain(|decision| dmn_rest_like(&decision.name, &name_like));
        }
        if let Some(resource_name_like) = query.resource_name_like {
            records.retain(|decision| dmn_rest_like(&decision.resource_name, &resource_name_like));
        }
        if let Some(tenant_id_like) = query.tenant_id_like {
            records.retain(|decision| {
                decision
                    .tenant_id
                    .as_deref()
                    .is_some_and(|tenant_id| dmn_rest_like(tenant_id, &tenant_id_like))
            });
        }
        if let Some(parent_deployment_id) = query.parent_deployment_id {
            records.retain(|decision| {
                decision.parent_deployment_id.as_deref() == Some(parent_deployment_id.as_str())
            });
        }
        if let Some(category) = query.category {
            records.retain(|decision| decision.category.as_deref() == Some(category.as_str()));
        }
        if let Some(category_like) = query.category_like {
            records.retain(|decision| {
                decision
                    .category
                    .as_deref()
                    .is_some_and(|category| dmn_rest_like(category, &category_like))
            });
        }
        if let Some(category_not_equals) = query.category_not_equals {
            records.retain(|decision| {
                decision.category.as_deref() != Some(category_not_equals.as_str())
            });
        }

        if query.latest {
            records = latest_dmn_decisions(records);
        }
        sort_dmn_decision_records(&mut records, query.sort.as_deref(), query.order.as_deref());
        Ok(paging.paginate(records))
    }

    fn get_decision_table(
        &self,
        decision_table_id: &str,
    ) -> Result<routes::dmn::DecisionTableRecord, crate::error::ApiError> {
        let decision = self
            .engine
            .repository_service()
            .get_decision(decision_table_id)?;
        Ok(routes::dmn::DecisionTableRecord {
            id: decision.id,
            key: decision.key,
            name: decision.name,
            version: decision.version,
            deployment_id: decision.deployment_id,
            resource_name: decision.resource_name,
            category: decision.category,
            description: None,
            tenant_id: decision.tenant_id,
            parent_deployment_id: decision.parent_deployment_id,
        })
    }

    fn get_decision_table_resource_data(
        &self,
        decision_table_id: &str,
    ) -> Result<routes::dmn::DmnResourceDataRecord, crate::error::ApiError> {
        let decision = self
            .engine
            .repository_service()
            .get_decision(decision_table_id)?;
        let resource = self
            .engine
            .repository_service()
            .get_deployment_resource_data(&decision.deployment_id, &decision.resource_name)?;
        Ok(routes::dmn::DmnResourceDataRecord {
            mime_type: resource.content_type,
            bytes: resource.bytes,
        })
    }

    fn get_decision_table_model(
        &self,
        decision_table_id: &str,
    ) -> Result<Value, crate::error::ApiError> {
        serde_json::to_value(
            self.engine
                .repository_service()
                .get_decision(decision_table_id)?,
        )
        .map_err(|error| crate::error::ApiError::InternalServerError(error.to_string()))
    }

    fn get_drd(&self, drd_id: &str) -> Result<Value, crate::error::ApiError> {
        let drd = self.engine.repository_service().get_drd(drd_id)?;
        serde_json::to_value(&drd)
            .map_err(|error| crate::error::ApiError::InternalServerError(error.to_string()))
    }

    fn list_drds(&self) -> Result<crate::common::PagedResponse<Value>, crate::error::ApiError> {
        let drds = self.engine.repository_service().list_drds()?;
        let data = drds
            .into_iter()
            .map(|drd| serde_json::to_value(&drd).unwrap())
            .collect::<Vec<_>>();
        let total = data.len();
        Ok(crate::common::PagedResponse {
            start: 0,
            size: total,
            total,
            data,
            sort: None,
            order: None,
        })
    }

    fn get_drd_resource_data(
        &self,
        drd_id: &str,
    ) -> Result<routes::dmn::DmnResourceDataRecord, crate::error::ApiError> {
        let (deployment, _model) = self
            .engine
            .repository_service()
            .get_drd_with_deployment_info(drd_id)?;
        let resource_name = deployment.resource_names.first().ok_or_else(|| {
            crate::error::ApiError::NotFound(format!("DRD '{}' has no resources", drd_id))
        })?;
        let resource = self
            .engine
            .repository_service()
            .get_deployment_resource_data(&deployment.id, resource_name)?;
        Ok(routes::dmn::DmnResourceDataRecord {
            mime_type: resource.content_type,
            bytes: resource.bytes,
        })
    }

    fn get_decision_image(&self, decision_id: &str) -> Result<Vec<u8>, crate::error::ApiError> {
        let decision = self.engine.repository_service().get_decision(decision_id)?;
        let svg = flowable_dmn_image_generator::DmnSvgGenerator::new()
            .generate_engine_definition_svg(&decision)
            .map_err(|err| {
                crate::error::ApiError::InternalServerError(format!(
                    "Failed to generate decision image: {err}"
                ))
            })?;
        Ok(svg.into_bytes())
    }

    fn list_decision_services(
        &self,
        query: routes::dmn::DecisionServiceQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::dmn::DecisionServiceRecord>,
        crate::error::ApiError,
    > {
        let paging = query.paging;
        let mut records = Vec::new();

        for drd in self.engine.repository_service().list_drds()? {
            let (deployment, _model) = self
                .engine
                .repository_service()
                .get_drd_with_deployment_info(&drd.id)?;
            let resource_name = deployment
                .resource_names
                .first()
                .cloned()
                .unwrap_or_default();

            for service in drd.decision_services {
                records.push(routes::dmn::DecisionServiceRecord {
                    id: format!("dmn-decision-service:{}:{}", deployment.id, service.id),
                    key: service.id,
                    name: service.name,
                    deployment_id: deployment.id.clone(),
                    resource_name: resource_name.clone(),
                    tenant_id: deployment.tenant_id.clone(),
                    parent_deployment_id: deployment.parent_deployment_id.clone(),
                    required_decision_keys: service.required_decisions,
                    output_decision_keys: service.output_decisions,
                });
            }
        }

        if let Some(id) = query.id {
            records.retain(|service| service.id == id);
        }
        if let Some(key) = query.key {
            records.retain(|service| service.key == key);
        }
        if let Some(key_like) = query.key_like {
            records.retain(|service| dmn_rest_like(&service.key, &key_like));
        }
        if let Some(name) = query.name {
            records.retain(|service| service.name == name);
        }
        if let Some(name_like) = query.name_like {
            records.retain(|service| dmn_rest_like(&service.name, &name_like));
        }
        if let Some(deployment_id) = query.deployment_id {
            records.retain(|service| service.deployment_id == deployment_id);
        }
        if let Some(parent_deployment_id) = query.parent_deployment_id {
            records.retain(|service| {
                service.parent_deployment_id.as_deref() == Some(parent_deployment_id.as_str())
            });
        }
        if let Some(resource_name) = query.resource_name {
            records.retain(|service| service.resource_name == resource_name);
        }
        // P133: resourceNameLike (DecisionService has resource_name; no version field)
        if let Some(resource_name_like) = query.resource_name_like {
            records.retain(|service| dmn_rest_like(&service.resource_name, &resource_name_like));
        }
        if let Some(tenant_id) = query.tenant_id {
            records.retain(|service| service.tenant_id.as_deref() == Some(tenant_id.as_str()));
        }
        if let Some(tenant_id_like) = query.tenant_id_like {
            records.retain(|service| {
                service
                    .tenant_id
                    .as_deref()
                    .is_some_and(|tenant_id| dmn_rest_like(tenant_id, &tenant_id_like))
            });
        }

        sort_dmn_decision_service_records(
            &mut records,
            query.sort.as_deref(),
            query.order.as_deref(),
        );
        Ok(paging.paginate(records))
    }

    fn get_decision_service(
        &self,
        decision_service_id: &str,
    ) -> Result<routes::dmn::DecisionServiceRecord, crate::error::ApiError> {
        self.list_decision_services(routes::dmn::DecisionServiceQuery {
            paging: crate::common::PagingQuery {
                start: 0,
                size: None,
            },
            id: Some(decision_service_id.to_string()),
            ..routes::dmn::DecisionServiceQuery::default()
        })?
        .data
        .into_iter()
        .next()
        .ok_or_else(|| {
            crate::error::ApiError::NotFound(format!(
                "DMN decision service '{decision_service_id}' was not found"
            ))
        })
    }
}

fn dmn_rest_like(candidate: &str, pattern: &str) -> bool {
    match (pattern.strip_prefix('%'), pattern.strip_suffix('%')) {
        (Some(_), Some(_)) if pattern.len() >= 2 => {
            candidate.contains(&pattern[1..pattern.len() - 1])
        }
        (Some(suffix), _) => candidate.ends_with(suffix),
        (_, Some(prefix)) => candidate.starts_with(prefix),
        _ => candidate.contains(pattern),
    }
}

fn latest_dmn_decisions(
    mut records: Vec<routes::dmn::DecisionTableRecord>,
) -> Vec<routes::dmn::DecisionTableRecord> {
    records.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| right.version.cmp(&left.version))
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut seen_keys = std::collections::BTreeSet::new();
    records.retain(|record| seen_keys.insert(record.key.clone()));
    records
}

fn sort_dmn_decision_records(
    records: &mut [routes::dmn::DecisionTableRecord],
    sort: Option<&str>,
    order: Option<&str>,
) {
    let descending = matches!(order, Some("desc"));
    records.sort_by(|left, right| {
        let ordering = match sort.unwrap_or("name") {
            "id" => left.id.cmp(&right.id),
            "key" => left.key.cmp(&right.key),
            "category" => left.category.cmp(&right.category),
            "deploymentId" => left.deployment_id.cmp(&right.deployment_id),
            "tenantId" => left.tenant_id.cmp(&right.tenant_id),
            "version" => left.version.cmp(&right.version),
            _ => left.name.cmp(&right.name),
        };
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
        .then_with(|| left.id.cmp(&right.id))
    });
}

fn sort_dmn_decision_service_records(
    records: &mut [routes::dmn::DecisionServiceRecord],
    sort: Option<&str>,
    order: Option<&str>,
) {
    let descending = matches!(order, Some("desc"));
    records.sort_by(|left, right| {
        let ordering = match sort.unwrap_or("name") {
            "id" => left.id.cmp(&right.id),
            "key" => left.key.cmp(&right.key),
            "deploymentId" => left.deployment_id.cmp(&right.deployment_id),
            "tenantId" => left.tenant_id.cmp(&right.tenant_id),
            "resourceName" => left.resource_name.cmp(&right.resource_name),
            _ => left.name.cmp(&right.name),
        };
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
        .then_with(|| left.id.cmp(&right.id))
    });
}

impl routes::dmn::DmnRuntimeApi for DmnApiAdapter {
    fn execute_decision(
        &self,
        command: routes::dmn::DecisionExecutionCommand,
    ) -> Result<routes::dmn::DecisionExecutionRecord, crate::error::ApiError> {
        let variables = Value::Object(
            command
                .variables
                .into_iter()
                .collect::<serde_json::Map<String, Value>>(),
        );
        let mut request = DmnExecutionRequest::new(variables);
        if let Some(tenant_id) = command.tenant_id.clone() {
            request = request.with_tenant_id(tenant_id);
        }
        if let Some(parent_deployment_id) = command.parent_deployment_id.clone() {
            request.parent_deployment_id = Some(parent_deployment_id);
        }
        if let Some(business_key) = command.business_key.clone() {
            request = request.with_business_key(business_key);
        }
        if command.disable_history {
            request = request.disable_history();
        }
        let result = self.engine.execute_by_key(&command.decision_key, request)?;
        Ok(routes::dmn::DecisionExecutionRecord {
            id: result.execution_id,
            decision_table_id: result.decision_definition_id,
            deployment_id: result.deployment_id,
            decision_key: result.decision_key,
            tenant_id: command.tenant_id,
            business_key: result.business_key,
            hit_policy: dmn_hit_policy_name(&result.hit_policy).to_string(),
            executed_at: result.executed_at.timestamp_millis(),
            rule_hit_count: result.matched_rule_count,
            input_variables: result.inputs.into_iter().collect::<BTreeMap<_, _>>(),
            // P79 row shape + P85 EngineRestVariable wrapper (Java
            // DmnRuleServiceResponse.resultVariables: List<List<EngineRestVariable>>)
            result_variables: dmn_wrapped_result_variables(&result.decision_result),
            multiple_results: result.multiple_results,
            rule_executions: result
                .rule_executions
                .into_iter()
                .map(dmn_rule_execution_record)
                .collect(),
        })
    }
}

/// Map DMN row results to REST `resultVariables`: list of output maps.
///
/// Raw (unwrapped) shape, used by the historic endpoints — Java serves those
/// from the stored execution JSON rather than `DmnRestResponseFactory`
/// (`BaseHistoricDecisionExecutionResource.java:65-75`).
fn dmn_row_result_variables(
    decision_result: &[serde_json::Map<String, Value>],
) -> Vec<BTreeMap<String, Value>> {
    decision_result
        .iter()
        .map(|row| row.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .collect()
}

/// Map DMN row results to `EngineRestVariable`-wrapped `resultVariables`, the
/// shape the Java rule-service endpoints return
/// (`DmnRestResponseFactory.java:128-144`: one inner list per result map, one
/// `EngineRestVariable` per output).
fn dmn_wrapped_result_variables(
    decision_result: &[serde_json::Map<String, Value>],
) -> Vec<Vec<routes::dmn::EngineRestVariable>> {
    dmn_row_result_variables(decision_result)
        .iter()
        .map(routes::dmn::engine_rest_variable_row)
        .collect()
}

fn dmn_hit_policy_name(hit_policy: &DmnHitPolicy) -> &'static str {
    match hit_policy {
        DmnHitPolicy::First => "FIRST",
        DmnHitPolicy::Unique => "UNIQUE",
        DmnHitPolicy::Any => "ANY",
        DmnHitPolicy::RuleOrder => "RULE_ORDER",
        DmnHitPolicy::OutputOrder => "OUTPUT_ORDER",
        DmnHitPolicy::Priority => "PRIORITY",
        DmnHitPolicy::Collect => "COLLECT",
        DmnHitPolicy::Complete => "COMPLETE",
        DmnHitPolicy::Batch => "BATCH",
    }
}

fn dmn_rule_execution_record(
    audit: DmnRuleExecutionAudit,
) -> routes::dmn::DecisionRuleExecutionRecord {
    routes::dmn::DecisionRuleExecutionRecord {
        rule_number: audit.rule_number,
        rule_id: audit.rule_id,
        valid: audit.valid,
        condition_results: audit
            .condition_results
            .into_iter()
            .map(|result| routes::dmn::DecisionExpressionExecutionRecord {
                id: result.id,
                result: result.result,
            })
            .collect(),
        conclusion_results: audit
            .conclusion_results
            .into_iter()
            .map(|result| routes::dmn::DecisionExpressionExecutionRecord {
                id: result.id,
                result: result.result,
            })
            .collect(),
    }
}

impl routes::dmn::DmnHistoryApi for DmnApiAdapter {
    fn list_historic_decision_executions(
        &self,
        query: routes::dmn::HistoricDecisionExecutionQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::dmn::HistoricDecisionExecutionRecord>,
        crate::error::ApiError,
    > {
        let mut history_query = self
            .engine
            .history_service()
            .create_execution_history_query();
        if let Some(id) = query.id {
            history_query = history_query.execution_id(id);
        }
        if let Some(decision_key) = query.decision_key {
            history_query = history_query.decision_key(decision_key);
        }
        if let Some(decision_table_id) = query.decision_table_id {
            history_query = history_query.decision_definition_id(decision_table_id);
        }
        if let Some(deployment_id) = query.deployment_id {
            history_query = history_query.deployment_id(deployment_id);
        }
        if let Some(business_key) = query.business_key {
            history_query = history_query.business_key(business_key);
        }
        if let Some(tenant_id) = query.tenant_id {
            history_query = history_query.tenant_id(tenant_id);
        }
        if let Some(tenant_id_like) = query.tenant_id_like {
            history_query = history_query.tenant_id_like(tenant_id_like);
        }
        if let Some(failed) = query.failed {
            history_query = history_query.failed(failed);
        }
        history_query = match query.sort.as_deref() {
            Some("id") | Some("decisionExecutionId") => history_query.order_by_execution_id(),
            Some("decisionKey") => history_query.order_by_decision_key(),
            Some("decisionTableId") | Some("decisionDefinitionId") => {
                history_query.order_by_decision_definition_id()
            }
            Some("deploymentId") => history_query.order_by_deployment_id(),
            Some("businessKey") => history_query.order_by_business_key(),
            Some("tenantId") => history_query.order_by_tenant_id(),
            Some("startTime") | Some("executionTime") | Some("executedAt") | None => {
                history_query.order_by_execution_time()
            }
            Some(_) => history_query,
        };
        if query.order.as_deref() == Some("desc") {
            history_query = history_query.desc();
        } else {
            history_query = history_query.asc();
        }
        let page = if let Some(size) = query.paging.size {
            history_query.page(query.paging.start, size).list_page()?
        } else {
            history_query.list_page()?
        };
        Ok(crate::common::PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page
                .data
                .into_iter()
                .map(|item| routes::dmn::HistoricDecisionExecutionRecord {
                    id: item.execution_id,
                    decision_table_id: item.decision_definition_id,
                    deployment_id: item.deployment_id,
                    decision_key: item.decision_key,
                    tenant_id: item.tenant_id,
                    business_key: item.business_key,
                    executed_at: item.executed_at.timestamp_millis(),
                    rule_hit_count: item.matched_rule_count,
                    input_variables: item.inputs.into_iter().collect::<BTreeMap<_, _>>(),
                    result_variables: dmn_row_result_variables(&item.decision_result),
                    multiple_results: item.multiple_results,
                    rule_executions: item
                        .rule_executions
                        .into_iter()
                        .map(dmn_rule_execution_record)
                        .collect(),
                    // P83 — Java `HistoricDecisionExecutionResponse.java:28-31`.
                    instance_id: item.instance_id,
                    execution_id: item.scope_execution_id,
                    activity_id: item.activity_id,
                    scope_type: item.scope_type,
                })
                .collect(),
            sort: None,
            order: None,
        })
    }

    fn delete_historic_decision_execution(
        &self,
        historic_decision_execution_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .history_service()
            .delete_historic_decision_execution(historic_decision_execution_id)?;
        Ok(())
    }

    fn bulk_delete_historic_decision_executions(
        &self,
        historic_decision_execution_ids: Vec<String>,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .history_service()
            .bulk_delete_historic_decision_executions(&historic_decision_execution_ids)?;
        Ok(())
    }
}

#[derive(Clone)]
struct CmmnApiAdapter {
    engine: Arc<CmmnEngine>,
    dmn_engine: Arc<DmnEngine>,
    form_service: FlowableFormService,
    /// Supplies the user→groups expansion for candidateUser / candidateOrAssigned
    /// (Java TaskQueryImpl.getGroupsForCandidateUser, TaskQueryImpl.java:2021-2032).
    /// Backed by the ProcessEngine identity service in the running server; None in
    /// adapters built without one (candidateUser then matches direct links only).
    user_group_resolver: Option<CmmnUserGroupResolver>,
}

impl CmmnApiAdapter {
    fn new(
        engine: Arc<CmmnEngine>,
        dmn_engine: Arc<DmnEngine>,
        form_service: FlowableFormService,
        user_group_resolver: Option<CmmnUserGroupResolver>,
    ) -> Self {
        Self {
            engine,
            dmn_engine,
            form_service,
            user_group_resolver,
        }
    }
}

fn list_cmmn_identity_links(
    engine: &CmmnEngine,
    scope_type: &str,
    scope_id: &str,
) -> Result<Vec<routes::cmmn::CmmnIdentityLinkRecord>, crate::error::ApiError> {
    Ok(engine
        .identity_link_service()
        .list_identity_links(scope_type, scope_id)?
        .into_iter()
        .map(to_cmmn_identity_link_record)
        .collect())
}

fn create_cmmn_identity_link(
    engine: &CmmnEngine,
    scope_type: &str,
    scope_id: &str,
    command: routes::cmmn::CmmnIdentityLinkCreateCommand,
) -> Result<routes::cmmn::CmmnIdentityLinkRecord, crate::error::ApiError> {
    let (family, identity_id) = match (&command.user, &command.group) {
        (Some(user), None) => ("users", user.as_str()),
        (None, Some(group)) => ("groups", group.as_str()),
        _ => {
            return Err(crate::error::ApiError::bad_request(
                "Exactly one of user or group is required",
            ));
        }
    };

    let link = CmmnIdentityLink {
        id: format!(
            "cmmn:{scope_type}:{scope_id}:{family}:{identity_id}:type:{}",
            command.link_type
        ),
        scope_type: scope_type.to_string(),
        scope_id: scope_id.to_string(),
        link_type: command.link_type,
        user_id: command.user,
        group_id: command.group,
    };
    engine
        .identity_link_service()
        .add_identity_link(link.clone())?;
    Ok(to_cmmn_identity_link_record(link))
}

fn delete_cmmn_identity_links_by_family(
    engine: &CmmnEngine,
    scope_type: &str,
    scope_id: &str,
    family: &str,
    identity_id: &str,
    link_type: Option<&str>,
) -> Result<(), crate::error::ApiError> {
    let matching_links = engine
        .identity_link_service()
        .list_identity_links(scope_type, scope_id)?
        .into_iter()
        .filter(|link| cmmn_identity_link_matches(link, family, identity_id, link_type))
        .collect::<Vec<_>>();

    if matching_links.is_empty() {
        return Err(crate::error::ApiError::NotFound(format!(
            "CMMN identity link '{scope_type}:{scope_id}:{family}:{identity_id}' was not found"
        )));
    }

    let service = engine.identity_link_service();
    for link in matching_links {
        service.delete_identity_link(&link.id)?;
    }
    Ok(())
}

fn cmmn_identity_link_matches(
    link: &CmmnIdentityLink,
    family: &str,
    identity_id: &str,
    link_type: Option<&str>,
) -> bool {
    let family_matches = match family {
        "users" => link.user_id.as_deref() == Some(identity_id),
        "groups" => link.group_id.as_deref() == Some(identity_id),
        _ => false,
    };
    family_matches && link_type.is_none_or(|value| link.link_type == value)
}

fn to_cmmn_identity_link_record(link: CmmnIdentityLink) -> routes::cmmn::CmmnIdentityLinkRecord {
    routes::cmmn::CmmnIdentityLinkRecord {
        user: link.user_id,
        group: link.group_id,
        link_type: link.link_type,
    }
}

/// Adapter that implements [`CmmnDecisionResolver`] over the DMN engine.
/// Applies parent-deployment scoping via `latest_decision_by_key`.
struct DmnDecisionResolverAdapter {
    dmn_engine: Arc<DmnEngine>,
}

impl DmnDecisionResolverAdapter {
    fn new(dmn_engine: Arc<DmnEngine>) -> Self {
        Self { dmn_engine }
    }
}

impl CmmnDecisionResolver for DmnDecisionResolverAdapter {
    fn resolve_decision(
        &self,
        key: &str,
        tenant_id: Option<&str>,
        parent_deployment_id: Option<&str>,
    ) -> Result<Option<ReferencedDecision>, flowable_cmmn_engine::CmmnError> {
        match self.dmn_engine.repository_service().latest_decision_by_key(
            key,
            tenant_id,
            parent_deployment_id,
        ) {
            Ok(decision) => Ok(Some(ReferencedDecision {
                id: decision.id,
                key: decision.key,
                name: decision.name,
                version: decision.version,
                deployment_id: decision.deployment_id,
                tenant_id: decision.tenant_id,
                resource_name: decision.resource_name,
            })),
            Err(flowable_dmn_engine::DmnError::NotFound { .. }) => Ok(None),
            Err(error) => Err(flowable_cmmn_engine::CmmnError::storage(error.to_string())),
        }
    }
}

/// Adapter that implements [`CmmnFormResolver`] over the form service.
/// Returns the latest active form definition by key. Form deployments do
/// not carry a parent-deployment ID, so parent-deployment scoping is not
/// applied.
struct FormResolverAdapter {
    form_service: FlowableFormService,
}

impl FormResolverAdapter {
    fn new(form_service: FlowableFormService) -> Self {
        Self { form_service }
    }
}

impl CmmnFormResolver for FormResolverAdapter {
    fn resolve_form(
        &self,
        key: &str,
        _tenant_id: Option<&str>,
        _parent_deployment_id: Option<&str>,
    ) -> Result<Option<ReferencedFormDefinition>, flowable_cmmn_engine::CmmnError> {
        let definitions = self
            .form_service
            .create_form_definition_query()
            .key(key)
            .list()
            .map_err(|error| flowable_cmmn_engine::CmmnError::storage(error.to_string()))?;

        let latest = definitions
            .into_iter()
            .filter(|d| d.active.unwrap_or(true))
            .max_by(|a, b| a.version.cmp(&b.version).then(a.id.cmp(&b.id)));

        Ok(latest.map(|d| ReferencedFormDefinition {
            id: d.id,
            key: d.key,
            name: d.name,
            version: d.version,
            deployment_id: d.deployment_id,
            resource_name: d.resource_name,
        }))
    }
}

#[allow(dead_code)]
fn to_decision_table_record(decision: DmnDecisionDefinition) -> routes::dmn::DecisionTableRecord {
    routes::dmn::DecisionTableRecord {
        id: decision.id,
        key: decision.key,
        name: decision.name,
        version: decision.version,
        deployment_id: decision.deployment_id,
        resource_name: decision.resource_name,
        category: None,
        description: None,
        tenant_id: decision.tenant_id,
        parent_deployment_id: None,
    }
}

fn to_form_definition_record(definition: FormDefinition) -> routes::forms::FormDefinitionRecord {
    routes::forms::FormDefinitionRecord {
        id: definition.id,
        key: definition.key,
        name: definition.name,
        version: definition.version,
        deployment_id: definition.deployment_id,
        resource_name: definition.resource_name,
        tenant_id: None,
        active: definition.active,
    }
}

fn to_decision_table_record_from_referenced(
    decision: ReferencedDecision,
) -> routes::dmn::DecisionTableRecord {
    routes::dmn::DecisionTableRecord {
        id: decision.id,
        key: decision.key,
        name: decision.name,
        version: decision.version,
        deployment_id: decision.deployment_id,
        resource_name: decision.resource_name,
        category: None,
        description: None,
        tenant_id: decision.tenant_id,
        parent_deployment_id: None,
    }
}

fn to_form_definition_record_from_referenced(
    form: ReferencedFormDefinition,
) -> routes::forms::FormDefinitionRecord {
    routes::forms::FormDefinitionRecord {
        id: form.id,
        key: form.key,
        name: form.name,
        version: form.version,
        deployment_id: form.deployment_id,
        resource_name: form.resource_name,
        tenant_id: None,
        active: Some(true),
    }
}

impl routes::cmmn::CmmnRepositoryApi for CmmnApiAdapter {
    fn deploy_case_definitions(
        &self,
        command: routes::cmmn::CmmnDeploymentCommand,
    ) -> Result<routes::cmmn::CmmnDeploymentRecord, crate::error::ApiError> {
        let mut builder =
            flowable_cmmn_engine::CmmnDeploymentBuilder::new(self.engine.repository_service())
                .name(command.name);
        if let Some(tenant_id) = command.tenant_id {
            builder = builder.tenant_id(tenant_id);
        }
        for resource in command.resources {
            builder = builder.add_string(resource.resource_name, resource.resource)?;
        }
        let deployment = builder.deploy()?;
        Ok(routes::cmmn::CmmnDeploymentRecord {
            id: deployment.id,
            name: deployment.name.unwrap_or_default(),
            deployed_at: deployment.deployed_at.timestamp_millis(),
            resource_names: deployment.resource_names,
            tenant_id: deployment.tenant_id,
        })
    }

    fn get_engine_info(
        &self,
    ) -> Result<routes::cmmn::CmmnEngineInfoRecord, crate::error::ApiError> {
        Ok(routes::cmmn::CmmnEngineInfoRecord {
            name: "cmmn-engine".to_string(),
            version: CMMN_ENGINE_VERSION.to_string(),
            resource_url: None,
            exception: None,
        })
    }

    fn get_deployment(
        &self,
        deployment_id: &str,
    ) -> Result<routes::cmmn::CmmnDeploymentRecord, crate::error::ApiError> {
        let deployment = self
            .engine
            .repository_service()
            .get_deployment(deployment_id)?;
        Ok(routes::cmmn::CmmnDeploymentRecord {
            id: deployment.id,
            name: deployment.name.unwrap_or_default(),
            deployed_at: deployment.deployed_at.timestamp_millis(),
            resource_names: deployment.resource_names,
            tenant_id: deployment.tenant_id,
        })
    }

    fn list_deployments(
        &self,
        query: routes::cmmn::CmmnDeploymentQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::CmmnDeploymentRecord>,
        crate::error::ApiError,
    > {
        let paging = query.paging;
        // P133: wire category/parentDeploymentId/tenantIdLike via engine query
        // (CmmnDeploymentQuery / Java DeploymentCollectionResource)
        let mut deployment_query = self.engine.repository_service().create_deployment_query();
        if let Some(id) = query.id {
            deployment_query = deployment_query.id(id);
        }
        if let Some(name) = query.name {
            deployment_query = deployment_query.name(name);
        }
        if let Some(name_like) = query.name_like {
            deployment_query = deployment_query.name_like(name_like);
        }
        if let Some(tenant_id) = query.tenant_id {
            deployment_query = deployment_query.tenant_id(tenant_id);
        }
        if let Some(tenant_id_like) = query.tenant_id_like {
            deployment_query = deployment_query.tenant_id_like(tenant_id_like);
        }
        if query.without_tenant_id {
            deployment_query = deployment_query.without_tenant_id();
        }
        if let Some(resource_name) = query.resource_name {
            deployment_query = deployment_query.resource_name(resource_name);
        }
        if let Some(category) = query.category {
            deployment_query = deployment_query.category(category);
        }
        if let Some(category_not_equals) = query.category_not_equals {
            deployment_query = deployment_query.category_not_equals(category_not_equals);
        }
        if let Some(parent_deployment_id) = query.parent_deployment_id {
            deployment_query = deployment_query.parent_deployment_id(parent_deployment_id);
        }
        if let Some(parent_deployment_id_like) = query.parent_deployment_id_like {
            deployment_query =
                deployment_query.parent_deployment_id_like(parent_deployment_id_like);
        }
        let deployments = deployment_query.list()?;
        let records = deployments
            .into_iter()
            .map(|deployment| routes::cmmn::CmmnDeploymentRecord {
                id: deployment.id,
                name: deployment.name.unwrap_or_default(),
                deployed_at: deployment.deployed_at.timestamp_millis(),
                resource_names: deployment.resource_names,
                tenant_id: deployment.tenant_id,
            })
            .collect::<Vec<_>>();
        Ok(paging.paginate(records))
    }

    fn delete_deployment(
        &self,
        deployment_id: &str,
        cascade: bool,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .repository_service()
            .delete_deployment(deployment_id, cascade)?;
        Ok(())
    }

    fn get_deployment_resource_data(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<routes::cmmn::CmmnResourceDataRecord, crate::error::ApiError> {
        let resource = self
            .engine
            .repository_service()
            .get_deployment_resource_data(deployment_id, resource_name)?;
        Ok(routes::cmmn::CmmnResourceDataRecord {
            mime_type: resource.content_type,
            bytes: resource.bytes,
        })
    }

    fn list_case_definitions(
        &self,
        query: routes::cmmn::CaseDefinitionQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::CaseDefinitionRecord>,
        crate::error::ApiError,
    > {
        // P133: CaseDefinitionCollectionResource.java:103-153 filters
        let mut case_query = self
            .engine
            .repository_service()
            .create_case_definition_query();
        if let Some(id) = query.id {
            case_query = case_query.id(id);
        }
        if let Some(key) = query.key {
            case_query = case_query.key(key);
        }
        if let Some(key_like) = query.key_like {
            case_query = case_query.key_like(key_like);
        }
        if let Some(name) = query.name {
            case_query = case_query.name(name);
        }
        if let Some(name_like) = query.name_like {
            case_query = case_query.name_like(name_like);
        }
        if let Some(name_like_ignore_case) = query.name_like_ignore_case {
            case_query = case_query.name_like_ignore_case(name_like_ignore_case);
        }
        if let Some(deployment_id) = query.deployment_id {
            case_query = case_query.deployment_id(deployment_id);
        }
        if let Some(version) = query.version {
            case_query = case_query.version(version);
        }
        if let Some(category) = query.category {
            case_query = case_query.category(category);
        }
        if let Some(category_like) = query.category_like {
            case_query = case_query.category_like(category_like);
        }
        if let Some(category_not_equals) = query.category_not_equals {
            case_query = case_query.category_not_equals(category_not_equals);
        }
        if let Some(resource_name) = query.resource_name {
            case_query = case_query.resource_name(resource_name);
        }
        if let Some(resource_name_like) = query.resource_name_like {
            case_query = case_query.resource_name_like(resource_name_like);
        }
        if let Some(tenant_id) = query.tenant_id {
            case_query = case_query.tenant_id(tenant_id);
        }
        if let Some(tenant_id_like) = query.tenant_id_like {
            case_query = case_query.tenant_id_like(tenant_id_like);
        }
        if query.latest {
            case_query = case_query.latest_version();
        }
        let page = if let Some(size) = query.paging.size {
            case_query.page(query.paging.start, size).list_page()?
        } else if query.paging.start > 0 {
            case_query
                .page(query.paging.start, usize::MAX)
                .list_page()?
        } else {
            case_query.list_page()?
        };
        Ok(crate::common::PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page
                .data
                .into_iter()
                .map(to_case_definition_record)
                .collect(),
            sort: None,
            order: None,
        })
    }

    fn get_case_definition(
        &self,
        case_definition_id: &str,
    ) -> Result<routes::cmmn::CaseDefinitionRecord, crate::error::ApiError> {
        Ok(to_case_definition_record(
            self.engine
                .repository_service()
                .get_case_definition(case_definition_id)?,
        ))
    }

    /// Java `CmmnRepositoryService.setCaseDefinitionCategory`
    /// (CaseDefinitionResource.java:100). Java does not re-fetch the entity, it
    /// patches the category onto the already-loaded response
    /// (CaseDefinitionResource.java:102-105); doing the same here keeps the
    /// response consistent under a concurrent redeploy.
    fn set_case_definition_category(
        &self,
        case_definition_id: &str,
        category: &str,
    ) -> Result<routes::cmmn::CaseDefinitionRecord, crate::error::ApiError> {
        let repository_service = self.engine.repository_service();
        let definition = repository_service.get_case_definition(case_definition_id)?;
        repository_service.set_case_definition_category(case_definition_id, Some(category))?;
        let mut record = to_case_definition_record(definition);
        record.category = Some(category.to_string());
        Ok(record)
    }

    fn get_case_definition_resource_data(
        &self,
        case_definition_id: &str,
    ) -> Result<routes::cmmn::CmmnResourceDataRecord, crate::error::ApiError> {
        Ok(routes::cmmn::CmmnResourceDataRecord {
            mime_type: "application/xml".to_string(),
            bytes: self
                .engine
                .repository_service()
                .get_case_definition_resource_bytes(case_definition_id)?,
        })
    }

    fn get_case_definition_model(
        &self,
        case_definition_id: &str,
    ) -> Result<Value, crate::error::ApiError> {
        serde_json::to_value(
            self.engine
                .repository_service()
                .get_case_definition(case_definition_id)?,
        )
        .map_err(|error| crate::error::ApiError::InternalServerError(error.to_string()))
    }

    fn list_case_definition_decision_tables(
        &self,
        case_definition_id: &str,
        paging: crate::common::PagingQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::dmn::DecisionTableRecord>,
        crate::error::ApiError,
    > {
        let resolver = DmnDecisionResolverAdapter::new(Arc::clone(&self.dmn_engine));
        let decisions = self
            .engine
            .repository_service()
            .list_referenced_decisions(case_definition_id, &resolver)?;
        let records: Vec<_> = decisions
            .into_iter()
            .map(to_decision_table_record_from_referenced)
            .collect();
        Ok(paging.paginate(records))
    }

    fn list_case_definition_decisions(
        &self,
        case_definition_id: &str,
        paging: crate::common::PagingQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::dmn::DecisionTableRecord>,
        crate::error::ApiError,
    > {
        self.list_case_definition_decision_tables(case_definition_id, paging)
    }

    fn list_case_definition_form_definitions(
        &self,
        case_definition_id: &str,
        paging: crate::common::PagingQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::forms::FormDefinitionRecord>,
        crate::error::ApiError,
    > {
        let resolver = FormResolverAdapter::new(self.form_service.clone());
        let forms = self
            .engine
            .repository_service()
            .list_referenced_form_definitions(case_definition_id, &resolver)?;
        let records: Vec<_> = forms
            .into_iter()
            .map(to_form_definition_record_from_referenced)
            .collect();
        Ok(paging.paginate(records))
    }

    fn get_case_definition_start_form(
        &self,
        case_definition_id: &str,
    ) -> Result<Value, crate::error::ApiError> {
        let form_key = self
            .engine
            .repository_service()
            .get_case_definition_start_form_key(case_definition_id)?
            .ok_or_else(|| {
                crate::error::ApiError::NotFound(format!(
                    "CMMN case definition '{case_definition_id}' start form was not found"
                ))
            })?;
        let definition = self
            .form_service
            .create_form_definition_query()
            .key(form_key.clone())
            .list()?
            .into_iter()
            .next()
            .ok_or_else(|| {
                crate::error::ApiError::NotFound(format!(
                    "CMMN case definition '{case_definition_id}' start form '{form_key}' was not found"
                ))
            })?;
        serde_json::to_value(to_form_definition_record(definition))
            .map_err(|error| crate::error::ApiError::InternalServerError(error.to_string()))
    }

    fn list_case_definition_identity_links(
        &self,
        case_definition_id: &str,
    ) -> Result<Vec<routes::cmmn::CmmnIdentityLinkRecord>, crate::error::ApiError> {
        self.engine
            .repository_service()
            .get_case_definition(case_definition_id)?;
        list_cmmn_identity_links(&self.engine, "caseDefinition", case_definition_id)
    }

    fn create_case_definition_identity_link(
        &self,
        case_definition_id: &str,
        command: routes::cmmn::CmmnIdentityLinkCreateCommand,
    ) -> Result<routes::cmmn::CmmnIdentityLinkRecord, crate::error::ApiError> {
        self.engine
            .repository_service()
            .get_case_definition(case_definition_id)?;
        create_cmmn_identity_link(&self.engine, "caseDefinition", case_definition_id, command)
    }

    fn delete_case_definition_identity_links(
        &self,
        case_definition_id: &str,
        family: &str,
        identity_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .repository_service()
            .get_case_definition(case_definition_id)?;
        delete_cmmn_identity_links_by_family(
            &self.engine,
            "caseDefinition",
            case_definition_id,
            family,
            identity_id,
            None,
        )
    }

    fn migrate_case_definition_instances(
        &self,
        case_definition_id: &str,
        command: routes::cmmn::CmmnMigrationCommand,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .migrate_case_instances_of_case_definition(
                case_definition_id,
                to_cmmn_migration_document(command),
            )?;
        Ok(())
    }

    fn batch_migrate_case_definition_instances(
        &self,
        case_definition_id: &str,
        command: routes::cmmn::CmmnMigrationCommand,
    ) -> Result<(), crate::error::ApiError> {
        self.migrate_case_definition_instances(case_definition_id, command)
    }

    fn migrate_historic_case_definition_instances(
        &self,
        case_definition_id: &str,
        command: routes::cmmn::CmmnMigrationCommand,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .history_service()
            .migrate_historic_case_instances_of_case_definition(
                case_definition_id,
                to_cmmn_migration_document(command),
            )?;
        Ok(())
    }

    fn batch_migrate_historic_case_definition_instances(
        &self,
        case_definition_id: &str,
        command: routes::cmmn::CmmnMigrationCommand,
    ) -> Result<(), crate::error::ApiError> {
        self.migrate_historic_case_definition_instances(case_definition_id, command)
    }
}

impl routes::cmmn::CmmnRuntimeApi for CmmnApiAdapter {
    fn start_case_instance(
        &self,
        command: routes::cmmn::StartCaseInstanceCommand,
    ) -> Result<routes::cmmn::CaseInstanceRecord, crate::error::ApiError> {
        let mut request = EngineCmmnCaseInstanceStartRequest::new();
        if let Some(business_key) = command.business_key {
            request = request.with_business_key(business_key);
        }
        if let Some(name) = command.name {
            request = request.with_name(name);
        }
        if let Some(tenant_id) = command.tenant_id {
            request = request.with_tenant_id(tenant_id);
        }
        if !command.variables.is_empty() {
            request = request.with_variables(Value::Object(
                command
                    .variables
                    .into_iter()
                    .collect::<serde_json::Map<_, _>>(),
            ));
        }
        if !command.transient_variables.is_empty() {
            request = request.with_transient_variables(Value::Object(
                command
                    .transient_variables
                    .into_iter()
                    .collect::<serde_json::Map<_, _>>(),
            ));
        }
        if let Some(outcome) = command.outcome {
            // Java CaseInstanceCreateRequest.outcome — only used through the form
            // engine; accepted and dropped here (P102 acceptance).
            request = request.with_outcome(outcome);
        }
        if let Some(override_definition_tenant_id) = command.override_definition_tenant_id {
            request = request.with_override_definition_tenant_id(override_definition_tenant_id);
        }
        let return_variables = command.return_variables;

        let case_key = if let Some(case_definition_id) = command.case_definition_id {
            self.engine
                .repository_service()
                .get_case_definition(&case_definition_id)?
                .key
        } else if let Some(case_definition_key) = command.case_definition_key {
            case_definition_key
        } else {
            return Err(crate::error::ApiError::bad_request(
                "Either caseDefinitionId or caseDefinitionKey is required",
            ));
        };

        let instance = self
            .engine
            .runtime_service()
            .start_case_instance_by_key(&case_key, request)?;
        let mut record = to_case_instance_record(instance);
        if return_variables {
            // Java CaseInstanceCollectionResource.java:410-416 — returnVariables
            // includes the case variables in the response.
            let case = self
                .engine
                .runtime_service()
                .get_case_instance(&record.id)?;
            record.variables = case
                .variables
                .into_iter()
                .map(|(name, value)| {
                    serde_json::json!({
                        "name": name,
                        "type": cmmn_variable_type(&value),
                        "value": value,
                        "scope": "local",
                    })
                })
                .collect();
        }
        Ok(record)
    }

    fn list_case_instances(
        &self,
        query: routes::cmmn::CaseInstanceQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::CaseInstanceRecord>,
        crate::error::ApiError,
    > {
        let mut case_query = self.engine.runtime_service().create_case_instance_query();
        if let Some(id) = query.id {
            case_query = case_query.id(id);
        }
        if !query.ids.is_empty() {
            case_query = case_query.ids(query.ids);
        }
        if let Some(case_definition_id) = query.case_definition_id {
            case_query = case_query.case_definition_id(case_definition_id);
        }
        if let Some(key) = query.case_definition_key {
            case_query = case_query.case_definition_key(key);
        }
        if let Some(pattern) = query.case_definition_key_like {
            case_query = case_query.case_definition_key_like(pattern);
        }
        if let Some(pattern) = query.case_definition_key_like_ignore_case {
            case_query = case_query.case_definition_key_like_ignore_case(pattern);
        }
        if !query.case_definition_keys.is_empty() {
            case_query = case_query.case_definition_keys(query.case_definition_keys);
        }
        if !query.exclude_case_definition_keys.is_empty() {
            case_query =
                case_query.exclude_case_definition_keys(query.exclude_case_definition_keys);
        }
        if let Some(name) = query.case_definition_name {
            case_query = case_query.case_definition_name(name);
        }
        if let Some(pattern) = query.case_definition_name_like {
            case_query = case_query.case_definition_name_like(pattern);
        }
        if let Some(pattern) = query.case_definition_name_like_ignore_case {
            case_query = case_query.case_definition_name_like_ignore_case(pattern);
        }
        if let Some(name) = query.name {
            case_query = case_query.name(name);
        }
        if let Some(pattern) = query.name_like {
            case_query = case_query.name_like(pattern);
        }
        if let Some(pattern) = query.name_like_ignore_case {
            case_query = case_query.name_like_ignore_case(pattern);
        }
        if let Some(business_key) = query.business_key {
            case_query = case_query.business_key(business_key);
        }
        if let Some(pattern) = query.business_key_like {
            case_query = case_query.business_key_like(pattern);
        }
        if let Some(pattern) = query.business_key_like_ignore_case {
            case_query = case_query.business_key_like_ignore_case(pattern);
        }
        if let Some(business_status) = query.business_status {
            case_query = case_query.business_status(business_status);
        }
        if let Some(pattern) = query.business_status_like {
            case_query = case_query.business_status_like(pattern);
        }
        if let Some(pattern) = query.business_status_like_ignore_case {
            case_query = case_query.business_status_like_ignore_case(pattern);
        }
        if let Some(started_by) = query.started_by {
            case_query = case_query.started_by(started_by);
        }
        // Java `BaseCaseInstanceResource.java:181-185` forwards both exact filters.
        if let Some(reference_id) = query.reference_id {
            case_query = case_query.reference_id(reference_id);
        }
        if let Some(reference_type) = query.reference_type {
            case_query = case_query.reference_type(reference_type);
        }
        if let Some(started_before) = query.started_before {
            case_query = case_query.started_before(started_before);
        }
        if let Some(started_after) = query.started_after {
            case_query = case_query.started_after(started_after);
        }
        if let Some(callback_id) = query.callback_id {
            case_query = case_query.callback_id(callback_id);
        }
        if !query.callback_ids.is_empty() {
            case_query = case_query.callback_ids(query.callback_ids);
        }
        if let Some(callback_type) = query.callback_type {
            case_query = case_query.callback_type(callback_type);
        }
        if let Some(tenant_id) = query.tenant_id {
            case_query = case_query.tenant_id(tenant_id);
        }
        if let Some(pattern) = query.tenant_id_like {
            case_query = case_query.tenant_id_like(pattern);
        }
        if let Some(pattern) = query.tenant_id_like_ignore_case {
            case_query = case_query.tenant_id_like_ignore_case(pattern);
        }
        if query.without_tenant_id {
            case_query = case_query.without_tenant_id();
        }
        if let Some(state) = query.state {
            case_query = case_query.state(parse_case_state(&state)?);
        }
        // Java BaseCaseInstanceResource.java:204-206 — variable conditions AND-ed
        // against case-instance variables (P103).
        if !query.variable_conditions.is_empty() {
            case_query = case_query.variable_conditions(query.variable_conditions);
        }

        let mut records = case_query
            .list()?
            .into_iter()
            .map(to_case_instance_record)
            .collect::<Vec<_>>();

        let case_definition_names = self
            .engine
            .repository_service()
            .create_case_definition_query()
            .list()?
            .into_iter()
            .map(|definition| (definition.id, definition.name))
            .collect::<std::collections::HashMap<_, _>>();
        for record in &mut records {
            // Java BaseCaseInstanceResource.java:246-260 — set the definition name
            // from the deployed definition.
            if let Some(name) = case_definition_names.get(&record.case_definition_id) {
                record.case_definition_name = Some(name.clone());
            }
        }

        // Java `paginateList` (PaginateListUtil.java:117-131) sorts before paging.
        sort_case_instance_records(&mut records, query.sort.as_deref(), query.order.as_deref())?;

        // Java CaseInstanceResponse.variables — populated when includeCaseVariables
        // (or the names variant) is requested (BaseCaseInstanceResource.java:196-203).
        if query.include_case_variables || !query.include_case_variables_names.is_empty() {
            for record in &mut records {
                let case = self
                    .engine
                    .runtime_service()
                    .create_case_instance_query()
                    .id(&record.id)
                    .single_result()?
                    .ok_or_else(|| {
                        crate::error::ApiError::NotFound(format!(
                            "CMMN case instance '{}' was not found",
                            record.id
                        ))
                    })?;
                let names = query.include_case_variables_names.as_slice();
                record.variables = case
                    .variables
                    .into_iter()
                    .filter(|(name, _)| names.is_empty() || names.contains(name))
                    .map(|(name, value)| {
                        serde_json::json!({
                            "name": name,
                            "type": cmmn_variable_type(&value),
                            "value": value,
                            "scope": "local",
                        })
                    })
                    .collect();
            }
        }

        let total = records.len();
        let start = query.paging.start.min(total);
        let size = query.paging.size.unwrap_or(total.saturating_sub(start));
        let data = records
            .into_iter()
            .skip(start)
            .take(size)
            .collect::<Vec<_>>();
        Ok(crate::common::PagedResponse {
            start,
            size: data.len(),
            total,
            data,
            sort: query.sort,
            order: query.order,
        })
    }

    fn terminate_case_instance(
        &self,
        case_instance_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        // Java reads a thread-local authenticated user at case end
        // (DefaultCmmnHistoryManager.java:89-90). Rust REST has no equivalent
        // auth context, so this compatibility entry point intentionally passes None.
        self.engine
            .runtime_service()
            .terminate_case_instance(case_instance_id)?;
        Ok(())
    }

    fn delete_case_instance(&self, case_instance_id: &str) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .delete_case_instance(case_instance_id)?;
        Ok(())
    }

    fn bulk_delete_case_instances(
        &self,
        case_instance_ids: Vec<String>,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .bulk_delete_case_instances(&case_instance_ids)?;
        Ok(())
    }

    fn bulk_terminate_case_instances(
        &self,
        case_instance_ids: Vec<String>,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .bulk_terminate_case_instances(&case_instance_ids)?;
        Ok(())
    }

    fn list_plan_item_instances(
        &self,
        query: routes::cmmn::PlanItemInstanceQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::PlanItemInstanceRecord>,
        crate::error::ApiError,
    > {
        // P116: merge the unified plan-item-instance mirror (stage / milestone / event
        // listener) into the response. When a task-specific filter is present (assignee,
        // priority, candidate, …) the non-task sources are skipped — only human-task plan
        // items carry those fields, so no non-task row could match anyway (Java
        // PlanItemInstanceQuery assigns the same semantics to its ASSIGNEE_/identity-link
        // filters). The common plan-item filters (caseInstanceId, planItemDefinitionId,
        // planItemDefinitionType, state, name, stageInstanceId, elementId) are applied to
        // both sources below.
        let mut records: Vec<routes::cmmn::PlanItemInstanceRecord> = Vec::new();
        if !query.task_only && !plan_item_query_has_task_specific_filters(&query) {
            let mut plan_item_query = self
                .engine
                .runtime_service()
                .create_plan_item_instance_query();
            if query.include_ended {
                plan_item_query = plan_item_query.include_ended();
            }
            if let Some(id) = &query.id {
                plan_item_query = plan_item_query.id(id);
            }
            if let Some(case_instance_id) = &query.case_instance_id {
                plan_item_query = plan_item_query.case_instance_id(case_instance_id);
            } else if let Some(scope_id) = &query.scope_id {
                // Java TaskCollectionResource.java:237-239 — scopeId → caseInstanceId.
                plan_item_query = plan_item_query.case_instance_id(scope_id);
            }
            if !query.case_instance_ids.is_empty() {
                plan_item_query =
                    plan_item_query.case_instance_ids(query.case_instance_ids.clone());
            }
            if let Some(case_definition_id) = &query.case_definition_id {
                plan_item_query = plan_item_query.case_definition_id(case_definition_id);
            }
            if let Some(stage_instance_id) = &query.stage_instance_id {
                plan_item_query = plan_item_query.stage_instance_id(stage_instance_id);
            }
            if let Some(plan_item_definition_type) = &query.plan_item_definition_type {
                plan_item_query =
                    plan_item_query.plan_item_definition_type(plan_item_definition_type);
            }
            if !query.plan_item_definition_types.is_empty() {
                plan_item_query = plan_item_query
                    .plan_item_definition_types(query.plan_item_definition_types.clone());
            }
            if let Some(element_id) = &query.element_id {
                plan_item_query = plan_item_query.element_id(element_id);
            }
            if let Some(name) = &query.name {
                plan_item_query = plan_item_query.name(name);
            }
            if let Some(name_like) = &query.name_like {
                plan_item_query = plan_item_query.name_like(name_like);
            }
            if let Some(name_like_ignore_case) = &query.name_like_ignore_case {
                plan_item_query = plan_item_query.name_like_ignore_case(name_like_ignore_case);
            }
            if let Some(state) = &query.state {
                plan_item_query = plan_item_query.state(state);
            }
            records.extend(
                plan_item_query
                    .list()?
                    .into_iter()
                    .map(to_plan_item_instance_record_from_mirror),
            );
        }

        let mut task_query = self.engine.runtime_service().create_human_task_query();
        if let Some(id) = query.id {
            task_query = task_query.id(id);
        }
        if let Some(case_instance_id) = query.case_instance_id {
            task_query = task_query.case_instance_id(case_instance_id);
        }
        if !query.case_instance_ids.is_empty() {
            task_query = task_query.case_instance_ids(query.case_instance_ids);
        }
        if let Some(scope_id) = query.scope_id {
            // Java TaskCollectionResource.java:237-239 — scopeId → caseInstanceId.
            task_query = task_query.case_instance_id(scope_id);
        }
        if let Some(stage_instance_id) = query.stage_instance_id {
            // Java PlanItemInstanceBaseResource.java:76-78.
            task_query = task_query.stage_instance_id(stage_instance_id);
        }
        if let Some(element_id) = query.element_id {
            // Java planItemInstanceElementId (PlanItemInstanceBaseResource.java:91-93)
            // — the plan item id, stored as the task entity's plan_item_id.
            task_query = task_query.element_id(element_id);
        }
        if let Some(plan_item_definition_type) = query.plan_item_definition_type {
            task_query = task_query.plan_item_definition_type(plan_item_definition_type);
        }
        if !query.plan_item_definition_types.is_empty() {
            task_query = task_query.plan_item_definition_types(query.plan_item_definition_types);
        }
        if let Some(state) = query.state {
            task_query = task_query.state(parse_human_task_state(&state)?);
        }
        if let Some(case_definition_id) = query.case_definition_id {
            task_query = task_query.case_definition_id(case_definition_id);
        }
        if let Some(case_definition_key) = query.case_definition_key {
            task_query = task_query.case_definition_key(case_definition_key);
        }
        if let Some(pattern) = query.case_definition_key_like {
            task_query = task_query.case_definition_key_like(pattern);
        }
        if let Some(pattern) = query.case_definition_key_like_ignore_case {
            task_query = task_query.case_definition_key_like_ignore_case(pattern);
        }
        if let Some(name) = query.name {
            task_query = task_query.name(name);
        }
        if let Some(pattern) = query.name_like {
            task_query = task_query.name_like(pattern);
        }
        if let Some(pattern) = query.name_like_ignore_case {
            task_query = task_query.name_like_ignore_case(pattern);
        }
        if let Some(assignee) = query.assignee {
            task_query = task_query.assignee(assignee);
        }
        if let Some(pattern) = query.assignee_like {
            task_query = task_query.assignee_like(pattern);
        }
        if let Some(owner) = query.owner {
            task_query = task_query.owner(owner);
        }
        if let Some(pattern) = query.owner_like {
            task_query = task_query.owner_like(pattern);
        }
        if query.unassigned.is_some() {
            // Java applies taskUnassigned() whenever the param is present
            // (TaskBaseResource.java:182-184), regardless of the boolean value.
            task_query = task_query.unassigned();
        }
        if let Some(delegation_state) = query.delegation_state {
            task_query = task_query.delegation_state(match delegation_state.as_str() {
                "pending" => flowable_cmmn_engine::CmmnDelegationState::Pending,
                "resolved" => flowable_cmmn_engine::CmmnDelegationState::Resolved,
                other => {
                    return Err(crate::error::ApiError::bad_request(format!(
                        "Illegal value for delegationState: {other}"
                    )));
                }
            });
        }
        if let Some(category) = query.category {
            task_query = task_query.category(category);
        }
        if !query.category_in.is_empty() {
            task_query = task_query.category_in(query.category_in);
        }
        if !query.category_not_in.is_empty() {
            task_query = task_query.category_not_in(query.category_not_in);
        }
        if query.without_category {
            task_query = task_query.without_category();
        }
        if let Some(task_definition_key) = query.task_definition_key {
            task_query = task_query.task_definition_id(task_definition_key);
        }
        if let Some(pattern) = query.task_definition_key_like {
            task_query = task_query.task_definition_id_like(pattern);
        }
        if let Some(priority) = query.priority {
            task_query = task_query.priority(priority);
        }
        if let Some(min_priority) = query.min_priority {
            task_query = task_query.min_priority(min_priority);
        }
        if let Some(max_priority) = query.max_priority {
            task_query = task_query.max_priority(max_priority);
        }
        if let Some(created_on) = query.created_on {
            task_query = task_query.created_on(created_on);
        }
        if let Some(created_before) = query.created_before {
            task_query = task_query.created_before(created_before);
        }
        if let Some(created_after) = query.created_after {
            task_query = task_query.created_after(created_after);
        }
        if let Some(due_date) = query.due_date {
            task_query = task_query.due_date(due_date);
        }
        if let Some(due_before) = query.due_before {
            task_query = task_query.due_before(due_before);
        }
        if let Some(due_after) = query.due_after {
            task_query = task_query.due_after(due_after);
        }
        if query.without_due_date {
            task_query = task_query.without_due_date();
        }
        if let Some(active) = query.active {
            // Java active()/suspended() (TaskBaseResource.java:268-274). The Rust
            // engine never suspends cases/tasks, so Active retains all and
            // Suspended none (P100 acceptance).
            task_query = task_query.suspension_state(if active {
                flowable_cmmn_engine::TaskSuspensionState::Active
            } else {
                flowable_cmmn_engine::TaskSuspensionState::Suspended
            });
        }
        // P114 candidate filters (Java TaskBaseResource.java:191-205, 328-330).
        // candidateUser/candidateOrAssigned need the user→groups expansion from
        // the ProcessEngine identity service (see `user_group_resolver`).
        if (query.candidate_user.is_some() || query.candidate_or_assigned.is_some())
            && let Some(resolver) = &self.user_group_resolver
        {
            task_query = task_query.user_group_resolver(Arc::clone(resolver));
        }
        if let Some(candidate_user) = &query.candidate_user {
            task_query = task_query.candidate_user(candidate_user);
        }
        if let Some(candidate_group) = &query.candidate_group {
            task_query = task_query.candidate_group(candidate_group);
        }
        if !query.candidate_group_in.is_empty() {
            task_query = task_query.candidate_group_in(query.candidate_group_in.clone());
        }
        if let Some(candidate_or_assigned) = &query.candidate_or_assigned {
            task_query = task_query.candidate_or_assigned(candidate_or_assigned);
        }
        if query.ignore_assignee == Some(true) {
            task_query = task_query.ignore_assignee_value();
        }

        records.extend(
            task_query
                .list()?
                .into_iter()
                .map(to_plan_item_instance_record),
        );

        // Java `planItemDefinitionId` (PlanItemInstanceBaseResource.java:80-81) — applied
        // uniformly across the merged sources.
        if let Some(plan_item_definition_id) = &query.plan_item_definition_id {
            records.retain(|record| record.plan_item_definition_id == *plan_item_definition_id);
        }

        // P103 variable conditions (before sort/page):
        // - local (taskVariables / plan-item variables): empty-local convention
        //   → any non-empty condition set yields zero results.
        // - caseInstanceVariables: join the owning case's variables
        //   (PlanItemInstanceBaseResource.java:122-124).
        if !query.local_variable_conditions.is_empty() {
            // Empty local store: no plan-item/task can satisfy a local condition.
            records.clear();
        } else if !query.case_instance_variable_conditions.is_empty() {
            let case_ids = records
                .iter()
                .map(|record| record.case_instance_id.clone())
                .collect::<std::collections::BTreeSet<_>>();
            let case_vars_by_id = self
                .engine
                .runtime_service()
                .create_case_instance_query()
                .list()?
                .into_iter()
                .filter(|instance| case_ids.contains(&instance.id))
                .map(|instance| (instance.id, instance.variables))
                .collect::<std::collections::HashMap<_, _>>();
            let conditions = &query.case_instance_variable_conditions;
            records.retain(|record| {
                case_vars_by_id
                    .get(&record.case_instance_id)
                    .is_some_and(|variables| {
                        flowable_cmmn_engine::variables_match_conditions(variables, conditions)
                    })
            });
        }

        // Java `paginateList` (PaginateListUtil.java:117-131) sorts the full
        // result in memory before paging.
        sort_plan_item_records(&mut records, query.sort.as_deref(), query.order.as_deref())?;

        // Java TaskResponse.variables: process (= case) variables join when
        // includeProcessVariables; task-local variables are always empty by the
        // documented convention (TaskBaseResource.java:276-286).
        if query.include_process_variables {
            let case_ids = records
                .iter()
                .map(|record| record.case_instance_id.clone())
                .collect::<std::collections::BTreeSet<_>>();
            let case_variables = self
                .engine
                .runtime_service()
                .create_case_instance_query()
                .list()?
                .into_iter()
                .filter(|instance| case_ids.contains(&instance.id))
                .flat_map(case_instance_variables)
                .collect::<Vec<_>>();
            for record in &mut records {
                record.variables = case_variables
                    .iter()
                    .filter(|variable| variable.case_instance_id == record.case_instance_id)
                    .map(|variable| {
                        serde_json::json!({
                            "name": variable.name,
                            "type": variable.variable_type,
                            "value": variable.value,
                            "scope": "global",
                        })
                    })
                    .collect();
            }
        }

        let total = records.len();
        let start = query.paging.start.min(total);
        let size = query.paging.size.unwrap_or(total.saturating_sub(start));
        let data = records
            .into_iter()
            .skip(start)
            .take(size)
            .collect::<Vec<_>>();
        Ok(crate::common::PagedResponse {
            start,
            size: data.len(),
            total,
            data,
            sort: query.sort,
            order: query.order,
        })
    }

    fn complete_plan_item_instance(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine.runtime_service().complete_human_task(
            plan_item_instance_id,
            EngineCmmnHumanTaskCompletionRequest::new(),
        )?;
        Ok(())
    }

    fn reactivate_plan_item_instance(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .reactivate_plan_item_instance(plan_item_instance_id)?;
        Ok(())
    }

    fn disable_plan_item_instance(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .disable_plan_item_instance(plan_item_instance_id)?;
        Ok(())
    }

    fn enable_plan_item_instance(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .enable_plan_item_instance(plan_item_instance_id)?;
        Ok(())
    }

    fn get_stage_overview(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<routes::cmmn::StageOverviewRecord>, crate::error::ApiError> {
        Ok(self
            .engine
            .runtime_service()
            .get_stage_overview(case_instance_id)?
            .into_iter()
            .map(to_stage_overview_record)
            .collect())
    }

    fn list_variable_instances(
        &self,
        query: routes::cmmn::VariableInstanceQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::VariableInstanceRecord>,
        crate::error::ApiError,
    > {
        let mut case_query = self.engine.runtime_service().create_case_instance_query();
        if let Some(case_instance_id) = query
            .case_instance_id
            .clone()
            .or_else(|| query.scope_id.clone())
        {
            case_query = case_query.id(case_instance_id);
        }

        let mut variables = case_query
            .list()?
            .into_iter()
            .flat_map(case_instance_variables)
            .collect::<Vec<_>>();
        if let Some(id) = query.id {
            variables.retain(|variable| variable.id == id);
        }
        if let Some(case_instance_id) = query.case_instance_id {
            variables.retain(|variable| variable.case_instance_id == case_instance_id);
        }
        if let Some(scope_id) = query.scope_id {
            variables.retain(|variable| variable.scope_id == scope_id);
        }
        if let Some(variable_name) = query.variable_name {
            variables.retain(|variable| variable.name == variable_name);
        }
        // P133: VariableInstanceCollectionResource.java:80-85
        if let Some(variable_name_like) = query.variable_name_like.as_deref() {
            variables.retain(|variable| {
                routes::tasks::sql_like_matches(variable_name_like, &variable.name)
            });
        }
        if query.exclude_task_variables {
            // Case-scoped variables only; task-scoped have scope_type cmmn-task
            variables.retain(|variable| variable.scope_type != "cmmn-task");
        }
        if query.exclude_local_variables {
            // Local = non-case (plan-item / task) scope. Case scope_type is "cmmn".
            variables.retain(|variable| variable.scope_type == "cmmn");
        }

        Ok(query.paging.paginate(variables))
    }

    fn get_variable_instance(
        &self,
        variable_instance_id: &str,
    ) -> Result<routes::cmmn::VariableInstanceRecord, crate::error::ApiError> {
        self.list_variable_instances(routes::cmmn::VariableInstanceQuery {
            id: Some(variable_instance_id.to_string()),
            ..routes::cmmn::VariableInstanceQuery::default()
        })?
        .data
        .into_iter()
        .next()
        .ok_or_else(|| {
            crate::error::ApiError::NotFound(format!(
                "CMMN variable instance '{variable_instance_id}' was not found"
            ))
        })
    }

    fn set_case_instance_variables(
        &self,
        case_instance_id: &str,
        variables: Vec<routes::cmmn::CmmnVariableUpdate>,
    ) -> Result<(), crate::error::ApiError> {
        self.engine.runtime_service().set_case_instance_variables(
            case_instance_id,
            variables
                .into_iter()
                .map(|variable| (variable.name, variable.value))
                .collect(),
        )?;
        Ok(())
    }

    // Java: CaseInstanceResource.java:88-130
    fn update_case_instance(
        &self,
        case_instance_id: &str,
        command: routes::cmmn::CmmnCaseInstanceUpdateCommand,
    ) -> Result<Option<routes::cmmn::CaseInstanceRecord>, crate::error::ApiError> {
        // Ensure the case exists before applying mutations.
        self.engine
            .runtime_service()
            .get_case_instance(case_instance_id)?;

        if let Some(action) = command.action.as_deref().filter(|a| !a.is_empty()) {
            // Java: RestActionRequest.EVALUATE_CRITERIA (CaseInstanceResource.java:101)
            if action == "evaluateCriteria" {
                self.engine
                    .runtime_service()
                    .evaluate_criteria(case_instance_id)?;
            } else {
                return Err(crate::error::ApiError::bad_request(format!(
                    "Invalid action: '{action}'."
                )));
            }
        } else {
            // Java: CaseInstanceResource.java:114-117
            if let Some(name) = command.name.as_deref().filter(|n| !n.is_empty()) {
                self.engine
                    .runtime_service()
                    .set_case_instance_name(case_instance_id, name)?;
            }
            if let Some(business_key) = command.business_key.as_deref().filter(|k| !k.is_empty()) {
                self.engine
                    .runtime_service()
                    .update_business_key(case_instance_id, business_key)?;
            }
        }

        // Java: re-fetch; null means case ended → HTTP 204 (CaseInstanceResource.java:122-128)
        match self.engine.runtime_service().get_case_instance(case_instance_id) {
            Ok(instance) => Ok(Some(to_case_instance_record(instance))),
            Err(flowable_cmmn_engine::CmmnError::NotFound { .. }) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    // Java: PlanItemInstanceResource.java:59-95
    fn perform_plan_item_instance_action(
        &self,
        plan_item_instance_id: &str,
        action: &str,
    ) -> Result<Option<routes::cmmn::PlanItemInstanceRecord>, crate::error::ApiError> {
        match action {
            // Java: triggerPlanItemInstance (PlanItemInstanceResource.java:69-70)
            "trigger" => {
                self.engine
                    .runtime_service()
                    .trigger_plan_item_instance(plan_item_instance_id)?;
            }
            // Existing service enable (Disabled → Available). Java REST maps enable→start;
            // Rust keeps enable as the true enable transition; start is separate below.
            // POST /enable extension remains available.
            "enable" => {
                self.engine
                    .runtime_service()
                    .enable_plan_item_instance(plan_item_instance_id)?;
            }
            // Java: disablePlanItemInstance (PlanItemInstanceResource.java:75-76)
            "disable" => {
                self.engine
                    .runtime_service()
                    .disable_plan_item_instance(plan_item_instance_id)?;
            }
            // Java: startPlanItemInstance (PlanItemInstanceResource.java:78-79)
            "start" => {
                self.engine
                    .runtime_service()
                    .start_plan_item_instance(plan_item_instance_id)?;
            }
            other => {
                return Err(crate::error::ApiError::bad_request(format!(
                    "Invalid action: '{other}'."
                )));
            }
        }

        // Java: re-fetch; null → 204 (PlanItemInstanceResource.java:87-94)
        match self.engine.runtime_service().get_human_task(plan_item_instance_id) {
            Ok(task) => Ok(Some(to_plan_item_instance_record(task))),
            Err(flowable_cmmn_engine::CmmnError::NotFound { .. }) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    // Java: CaseInstanceVariableResource.java:176 / CmmnRuntimeService#removeVariable
    fn remove_case_instance_variable(
        &self,
        case_instance_id: &str,
        variable_name: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .remove_variable(case_instance_id, variable_name)?;
        Ok(())
    }

    // Java: CaseInstanceVariableCollectionResource.java:180 / BaseVariableResource#deleteAllVariables
    fn remove_case_instance_variables(
        &self,
        case_instance_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        let case_instance = self
            .engine
            .runtime_service()
            .get_case_instance(case_instance_id)?;
        let names: Vec<String> = case_instance.variables.keys().cloned().collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.engine
            .runtime_service()
            .remove_variables(case_instance_id, &name_refs)?;
        Ok(())
    }

    fn list_case_instance_identity_links(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<routes::cmmn::CmmnIdentityLinkRecord>, crate::error::ApiError> {
        self.engine
            .runtime_service()
            .get_case_instance(case_instance_id)?;
        list_cmmn_identity_links(&self.engine, "caseInstance", case_instance_id)
    }

    fn create_case_instance_identity_link(
        &self,
        case_instance_id: &str,
        command: routes::cmmn::CmmnIdentityLinkCreateCommand,
    ) -> Result<routes::cmmn::CmmnIdentityLinkRecord, crate::error::ApiError> {
        self.engine
            .runtime_service()
            .get_case_instance(case_instance_id)?;
        create_cmmn_identity_link(&self.engine, "caseInstance", case_instance_id, command)
    }

    fn delete_case_instance_identity_link(
        &self,
        case_instance_id: &str,
        identity_id: &str,
        link_type: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .get_case_instance(case_instance_id)?;
        delete_cmmn_identity_links_by_family(
            &self.engine,
            "caseInstance",
            case_instance_id,
            "users",
            identity_id,
            Some(link_type),
        )
    }

    fn get_task_form(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<serde_json::Value, crate::error::ApiError> {
        let task = self
            .engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        let definition = self
            .engine
            .repository_service()
            .get_case_definition(&task.case_definition_id)?;
        let form_key = find_cmmn_human_task_form_key(&definition.model, &task.task_definition_id)
            .ok_or_else(|| {
            crate::error::ApiError::NotFound(format!(
                "CMMN task '{plan_item_instance_id}' form was not found"
            ))
        })?;
        let form_definition = self
            .form_service
            .create_form_definition_query()
            .key(form_key.clone())
            .list()?
            .into_iter()
            .next()
            .ok_or_else(|| {
                crate::error::ApiError::NotFound(format!(
                    "CMMN task '{plan_item_instance_id}' form '{form_key}' was not found"
                ))
            })?;
        Ok(form_definition.form_payload)
    }

    fn list_task_identity_links(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<Vec<routes::cmmn::CmmnIdentityLinkRecord>, crate::error::ApiError> {
        self.engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        list_cmmn_identity_links(&self.engine, "humanTask", plan_item_instance_id)
    }

    fn create_task_identity_link(
        &self,
        plan_item_instance_id: &str,
        command: routes::cmmn::CmmnIdentityLinkCreateCommand,
    ) -> Result<routes::cmmn::CmmnIdentityLinkRecord, crate::error::ApiError> {
        self.engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        create_cmmn_identity_link(&self.engine, "humanTask", plan_item_instance_id, command)
    }

    fn delete_task_identity_link(
        &self,
        plan_item_instance_id: &str,
        family: &str,
        identity_id: &str,
        link_type: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        delete_cmmn_identity_links_by_family(
            &self.engine,
            "humanTask",
            plan_item_instance_id,
            family,
            identity_id,
            Some(link_type),
        )
    }

    // Java: TaskResource.java:76-99 — PUT update task (null clears).
    fn update_task(
        &self,
        plan_item_instance_id: &str,
        update: EngineCmmnHumanTaskUpdate,
    ) -> Result<routes::cmmn::PlanItemInstanceRecord, crate::error::ApiError> {
        let task = self
            .engine
            .runtime_service()
            .update_human_task(plan_item_instance_id, update)?;
        Ok(to_plan_item_instance_record(task))
    }

    // Java: TaskResource.java:109-137 — POST task action (complete/claim/delegate/resolve).
    fn execute_task_action(
        &self,
        plan_item_instance_id: &str,
        action_request: routes::cmmn::CmmnTaskActionRequest,
    ) -> Result<(), crate::error::ApiError> {
        let action = action_request
            .action
            .as_deref()
            .ok_or_else(|| crate::error::ApiError::bad_request("action is required".to_string()))?;
        match action {
            // Java: completeTask (TaskResource.java:197-236).
            "complete" => {
                let mut completion = EngineCmmnHumanTaskCompletionRequest::new();
                completion.outcome = action_request.outcome.clone();
                for variable in &action_request.variables {
                    let name = variable.name.clone().ok_or_else(|| {
                        crate::error::ApiError::bad_request(
                            "Variable name is required".to_string(),
                        )
                    })?;
                    // Java: only an explicit LOCAL scope is task-local; every other
                    // value (null included) is GLOBAL (TaskResource.java:207-211).
                    // Rust has no task-local storage, so LOCAL completion variables
                    // are dropped (completion still succeeds) and GLOBAL ones are
                    // written to the case.
                    if completion_variable_scope_is_local(variable.scope.as_deref())? {
                        continue;
                    }
                    completion.variables.push((name, variable.value.clone()));
                }
                self.engine
                    .runtime_service()
                    .complete_human_task(plan_item_instance_id, completion)?;
            }
            // Java: claimTask — assignee required, already claimed → 409
            // (ClaimTaskCmd.java:51).
            "claim" => {
                let assignee = action_request.assignee.clone().ok_or_else(|| {
                    crate::error::ApiError::bad_request(
                        "An assignee is required when claiming a task.".to_string(),
                    )
                })?;
                self.engine
                    .runtime_service()
                    .claim_human_task(plan_item_instance_id, &assignee)?;
            }
            // Java: delegateTask (DelegateTaskCmd.java:37-47).
            "delegate" => {
                let assignee = action_request.assignee.clone().ok_or_else(|| {
                    crate::error::ApiError::bad_request(
                        "An assignee is required when delegating a task.".to_string(),
                    )
                })?;
                self.engine
                    .runtime_service()
                    .delegate_human_task(plan_item_instance_id, &assignee)?;
            }
            // Java: resolveTask (ResolveTaskCmd.java:55-57).
            "resolve" => {
                self.engine
                    .runtime_service()
                    .resolve_human_task(plan_item_instance_id)?;
            }
            other => {
                return Err(crate::error::ApiError::bad_request(format!(
                    "Invalid action: '{other}'."
                )));
            }
        }
        Ok(())
    }

    // Java: TaskResource.java:149-174 — CMMN task deletion is always forbidden.
    fn delete_task(&self, plan_item_instance_id: &str) -> Result<(), crate::error::ApiError> {
        // 404 first (getTaskFromRequestWithoutAccessCheck), then the CMMN guard.
        self.engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        Err(crate::error::ApiError::Forbidden(
            "Cannot delete a task that is part of a case instance.".to_string(),
        ))
    }

    // Java: TaskVariableCollectionResource.java:182-208 — GLOBAL scope → case variables.
    fn create_task_variables(
        &self,
        plan_item_instance_id: &str,
        variables: Vec<routes::cmmn::CmmnVariableUpdate>,
    ) -> Result<(), crate::error::ApiError> {
        let task = self
            .engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        self.engine.runtime_service().set_case_instance_variables(
            &task.case_instance_id,
            variables
                .into_iter()
                .map(|variable| (variable.name, variable.value))
                .collect(),
        )?;
        Ok(())
    }

    // Java: TaskVariableCollectionResource.java:219-228 — delete ALL local
    // variables (removeVariablesLocal with the current names).
    fn delete_task_variables(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        let task = self
            .engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        let names: Vec<&str> = task
            .task_local_variables
            .keys()
            .map(String::as_str)
            .collect();
        self.engine
            .runtime_service()
            .remove_task_variables_local(plan_item_instance_id, &names)?;
        Ok(())
    }

    // P115: task-local variables — the task's own scope, keyed by task id
    // (TaskService.getVariablesLocal → VariableScopeImpl.java:455-470).
    fn list_task_variables_local(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<Vec<routes::cmmn::VariableInstanceRecord>, crate::error::ApiError> {
        let task = self
            .engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        Ok(task_local_variables(task))
    }

    // Java: TaskService.setVariablesLocal (TaskServiceImpl.java:445-447) →
    // SetTaskVariablesCmd.java:42-47 → task.setVariableLocal — writes land on
    // the task's own scope only.
    fn set_task_variables_local(
        &self,
        plan_item_instance_id: &str,
        variables: Vec<routes::cmmn::CmmnVariableUpdate>,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .set_task_variables_local(
                plan_item_instance_id,
                variables
                    .into_iter()
                    .map(|variable| (variable.name, variable.value))
                    .collect(),
            )?;
        Ok(())
    }

    // Java: TaskService.removeVariableLocal (TaskServiceImpl.java:457-461) →
    // RemoveTaskVariablesCmd.java:38-42.
    fn remove_task_variable_local(
        &self,
        plan_item_instance_id: &str,
        variable_name: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .remove_task_variable_local(plan_item_instance_id, variable_name)?;
        Ok(())
    }

    // Java: TaskVariableResource.java:94-130 — PUT single variable (GLOBAL → case).
    // The handler already validated the body name against the URL path and that
    // the variable exists on the case, so only the value write happens here.
    fn update_task_variable(
        &self,
        plan_item_instance_id: &str,
        _variable_name: &str,
        variable: routes::cmmn::CmmnVariableUpdate,
    ) -> Result<(), crate::error::ApiError> {
        let task = self
            .engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        self.engine.runtime_service().set_case_instance_variables(
            &task.case_instance_id,
            vec![(variable.name, variable.value)],
        )?;
        Ok(())
    }

    // Java: TaskVariableResource.java:138-167 — DELETE single variable (GLOBAL → case).
    fn delete_task_variable(
        &self,
        plan_item_instance_id: &str,
        variable_name: &str,
    ) -> Result<(), crate::error::ApiError> {
        let task = self
            .engine
            .runtime_service()
            .get_human_task(plan_item_instance_id)?;
        self.engine
            .runtime_service()
            .remove_variable(&task.case_instance_id, variable_name)?;
        Ok(())
    }

    fn validate_case_instance_migration(
        &self,
        case_instance_id: &str,
        command: routes::cmmn::CmmnMigrationCommand,
    ) -> Result<routes::cmmn::CmmnMigrationValidationRecord, crate::error::ApiError> {
        let result = self
            .engine
            .runtime_service()
            .validate_case_instance_migration(
                case_instance_id,
                to_cmmn_migration_document(command),
            )?;
        Ok(routes::cmmn::CmmnMigrationValidationRecord {
            valid: result.valid,
            validation_messages: result.validation_messages,
        })
    }

    fn migrate_case_instance(
        &self,
        case_instance_id: &str,
        command: routes::cmmn::CmmnMigrationCommand,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .runtime_service()
            .migrate_case_instance(case_instance_id, to_cmmn_migration_document(command))?;
        Ok(())
    }

    fn change_plan_item_state(
        &self,
        case_instance_id: &str,
        command: routes::cmmn::CmmnChangePlanItemStateCommand,
    ) -> Result<(), crate::error::ApiError> {
        self.engine.runtime_service().change_plan_item_state(
            case_instance_id,
            to_cmmn_change_plan_item_state_request(command),
        )?;
        Ok(())
    }

    fn trigger_case_event(
        &self,
        case_instance_id: &str,
        command: routes::cmmn::CmmnTriggerEventCommand,
    ) -> Result<(), crate::error::ApiError> {
        // Find event subscriptions for this case instance
        let mut event_query = self
            .engine
            .runtime_service()
            .create_event_subscription_query()
            .case_instance_id(case_instance_id.to_string());

        if let Some(ref event_name) = command.event_name {
            event_query = event_query.event_name(event_name.clone());
        }
        if let Some(ref event_type) = command.event_type {
            event_query = event_query.event_type(event_type.clone());
        }

        let page = event_query.list_page()?;
        let subscription = page.data.into_iter().next().ok_or_else(|| {
            crate::error::ApiError::NotFound(format!(
                "No event subscription found for case instance '{case_instance_id}'"
            ))
        })?;

        if !command.variables.is_empty() {
            self.engine.runtime_service().set_case_instance_variables(
                case_instance_id,
                command
                    .variables
                    .into_iter()
                    .map(|variable| (variable.name, variable.value))
                    .collect(),
            )?;
        }

        self.engine
            .runtime_service()
            .occur_event_subscription(&subscription.id)?;
        Ok(())
    }

    fn list_event_subscriptions(
        &self,
        query: routes::cmmn::CmmnEventSubscriptionQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::CmmnEventSubscriptionRecord>,
        crate::error::ApiError,
    > {
        let mut event_query = self
            .engine
            .runtime_service()
            .create_event_subscription_query();
        if let Some(id) = query.id {
            event_query = event_query.id(id);
        }
        if let Some(event_type) = query.event_type {
            event_query = event_query.event_type(event_type);
        }
        if let Some(event_name) = query.event_name {
            event_query = event_query.event_name(event_name);
        }
        if let Some(activity_id) = query.activity_id {
            event_query = event_query.activity_id(activity_id);
        }
        if let Some(case_instance_id) = query.case_instance_id {
            event_query = event_query.case_instance_id(case_instance_id);
        }
        if let Some(case_definition_id) = query.case_definition_id {
            event_query = event_query.case_definition_id(case_definition_id);
        }
        if let Some(plan_item_instance_id) = query.plan_item_instance_id {
            event_query = event_query.plan_item_instance_id(plan_item_instance_id);
        }
        if let Some(tenant_id) = query.tenant_id {
            event_query = event_query.tenant_id(tenant_id);
        }
        if let Some(configuration) = query.configuration {
            event_query = event_query.configuration(configuration);
        }
        if query.without_scope_id {
            event_query = event_query.without_scope_id();
        }
        if query.without_scope_definition_id {
            event_query = event_query.without_scope_definition_id();
        }
        if query.without_tenant_id {
            event_query = event_query.without_tenant_id();
        }
        if query.without_configuration {
            event_query = event_query.without_configuration();
        }
        // P133: createdAfter/createdBefore on CmmnEventSubscription.created_at
        if let Some(created_after) = query.created_after {
            event_query = event_query.created_after(created_after);
        }
        if let Some(created_before) = query.created_before {
            event_query = event_query.created_before(created_before);
        }

        let page = if let Some(size) = query.paging.size {
            event_query.page(query.paging.start, size).list_page()?
        } else {
            event_query.list_page()?
        };
        Ok(crate::common::PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page
                .data
                .into_iter()
                .map(to_cmmn_event_subscription_record)
                .collect(),
            sort: None,
            order: None,
        })
    }
}

impl routes::cmmn::CmmnHistoryApi for CmmnApiAdapter {
    fn list_historic_case_instances(
        &self,
        query: routes::cmmn::HistoricCaseInstanceQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::HistoricCaseInstanceRecord>,
        crate::error::ApiError,
    > {
        let mut case_query = self
            .engine
            .history_service()
            .create_historic_case_instance_query();
        if let Some(id) = query.id {
            case_query = case_query.case_instance_id(id);
        }
        if let Some(ids) = query.ids {
            case_query = case_query.case_instance_ids(ids);
        }
        if let Some(case_definition_id) = query.case_definition_id {
            case_query = case_query.case_definition_id(case_definition_id);
        }
        if let Some(key) = query.case_definition_key {
            case_query = case_query.case_definition_key(key);
        }
        if let Some(pattern) = query.case_definition_key_like {
            case_query = case_query.case_definition_key_like(pattern);
        }
        if let Some(pattern) = query.case_definition_key_like_ignore_case {
            case_query = case_query.case_definition_key_like_ignore_case(pattern);
        }
        if let Some(category) = query.case_definition_category {
            case_query = case_query.case_definition_category(category);
        }
        if let Some(pattern) = query.case_definition_category_like {
            case_query = case_query.case_definition_category_like(pattern);
        }
        if let Some(pattern) = query.case_definition_category_like_ignore_case {
            case_query = case_query.case_definition_category_like_ignore_case(pattern);
        }
        if let Some(name) = query.case_definition_name {
            case_query = case_query.case_definition_name(name);
        }
        if let Some(pattern) = query.case_definition_name_like {
            case_query = case_query.case_definition_name_like(pattern);
        }
        if let Some(pattern) = query.case_definition_name_like_ignore_case {
            case_query = case_query.case_definition_name_like_ignore_case(pattern);
        }
        if let Some(name) = query.name {
            case_query = case_query.name(name);
        }
        if let Some(pattern) = query.name_like {
            case_query = case_query.name_like(pattern);
        }
        if let Some(pattern) = query.name_like_ignore_case {
            case_query = case_query.name_like_ignore_case(pattern);
        }
        if let Some(business_key) = query.business_key {
            case_query = case_query.business_key(business_key);
        }
        if let Some(pattern) = query.business_key_like {
            case_query = case_query.business_key_like(pattern);
        }
        if let Some(pattern) = query.business_key_like_ignore_case {
            case_query = case_query.business_key_like_ignore_case(pattern);
        }
        if let Some(business_status) = query.business_status {
            case_query = case_query.business_status(business_status);
        }
        if let Some(pattern) = query.business_status_like {
            case_query = case_query.business_status_like(pattern);
        }
        if let Some(pattern) = query.business_status_like_ignore_case {
            case_query = case_query.business_status_like_ignore_case(pattern);
        }
        if let Some(started_by) = query.started_by {
            case_query = case_query.started_by(started_by);
        }
        // Java `HistoricCaseInstanceBaseResource.java:167-172` forwards both predicates.
        if let Some(reference_id) = query.reference_id {
            case_query = case_query.reference_id(reference_id);
        }
        if let Some(reference_type) = query.reference_type {
            case_query = case_query.reference_type(reference_type);
        }
        if let Some(bound) = query.started_before {
            case_query = case_query.started_before(bound);
        }
        if let Some(bound) = query.started_after {
            case_query = case_query.started_after(bound);
        }
        if let Some(finished) = query.finished {
            case_query = case_query.finished(finished);
        }
        if let Some(bound) = query.finished_before {
            case_query = case_query.finished_before(bound);
        }
        if let Some(bound) = query.finished_after {
            case_query = case_query.finished_after(bound);
        }
        // Java `HistoricCaseInstanceBaseResource.java:188-189` forwards finishedBy.
        if let Some(finished_by) = query.finished_by {
            case_query = case_query.finished_by(finished_by);
        }
        if let Some(tenant_id) = query.tenant_id {
            case_query = case_query.tenant_id(tenant_id);
        }
        if let Some(pattern) = query.tenant_id_like {
            case_query = case_query.tenant_id_like(pattern);
        }
        if let Some(pattern) = query.tenant_id_like_ignore_case {
            case_query = case_query.tenant_id_like_ignore_case(pattern);
        }
        if query.without_tenant_id {
            case_query = case_query.without_tenant_id();
        }
        if let Some(callback_id) = query.callback_id {
            case_query = case_query.callback_id(callback_id);
        }
        if let Some(callback_ids) = query.callback_ids {
            case_query = case_query.callback_ids(callback_ids);
        }
        if let Some(callback_type) = query.callback_type {
            case_query = case_query.callback_type(callback_type);
        }
        if query.without_callback_id {
            case_query = case_query.without_callback_id();
        }
        if let Some(involved_user) = query.involved_user {
            case_query = case_query.involved_user(involved_user);
        }
        if let Some(plan_item_definition_id) = query.active_plan_item_definition_id {
            case_query = case_query.active_plan_item_definition_id(plan_item_definition_id);
        }
        if let Some(state) = query.state {
            case_query = case_query.state(parse_case_state(&state)?);
        }
        // Java sorts inside the query (`paginateList`), so ordering precedes the
        // page window (HistoricCaseInstanceBaseResource.java:48-52).
        let mut records: Vec<_> = case_query
            .list()?
            .into_iter()
            .map(|instance| {
                to_historic_case_instance_record(
                    instance,
                    query.include_case_variables,
                    &query.include_case_variables_names,
                )
            })
            .collect();
        let total = records.len();
        sort_historic_case_instance_records(
            &mut records,
            query.sort.as_deref(),
            query.order.as_deref(),
        )?;
        let start = query.paging.start.min(records.len());
        let size = query.paging.size.unwrap_or(records.len() - start);
        let data: Vec<_> = records.into_iter().skip(start).take(size).collect();
        Ok(crate::common::PagedResponse {
            start,
            size: data.len(),
            total,
            data,
            sort: query.sort,
            order: query.order,
        })
    }

    fn delete_historic_case_instance(
        &self,
        case_instance_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .history_service()
            .delete_historic_case_instance(case_instance_id)?;
        Ok(())
    }

    fn bulk_delete_historic_case_instances(
        &self,
        case_instance_ids: Vec<String>,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .history_service()
            .bulk_delete_historic_case_instances(&case_instance_ids)?;
        Ok(())
    }

    fn list_historic_plan_item_instances(
        &self,
        query: routes::cmmn::HistoricPlanItemInstanceQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::HistoricPlanItemInstanceRecord>,
        crate::error::ApiError,
    > {
        let mirror_filter = query.clone();
        let mut task_query = self
            .engine
            .history_service()
            .create_historic_human_task_query();
        if let Some(id) = query.id {
            task_query = task_query.task_id(id);
        }
        if let Some(case_instance_id) = query.case_instance_id {
            task_query = task_query.case_instance_id(case_instance_id);
        }
        if let Some(case_definition_id) = query.case_definition_id {
            task_query = task_query.case_definition_id(case_definition_id);
        }
        if let Some(name) = query.name {
            task_query = task_query.name(name);
        }
        if let Some(pattern) = query.name_like {
            task_query = task_query.name_like(pattern);
        }
        if let Some(pattern) = query.name_like_ignore_case {
            task_query = task_query.name_like_ignore_case(pattern);
        }
        if let Some(key) = query.task_definition_key {
            task_query = task_query.task_definition_key(key);
        }
        if let Some(pattern) = query.task_definition_key_like {
            task_query = task_query.task_definition_key_like(pattern);
        }
        if let Some(assignee) = query.assignee {
            task_query = task_query.assignee(assignee);
        }
        if let Some(pattern) = query.assignee_like {
            task_query = task_query.assignee_like(pattern);
        }
        if let Some(owner) = query.owner {
            task_query = task_query.owner(owner);
        }
        if let Some(pattern) = query.owner_like {
            task_query = task_query.owner_like(pattern);
        }
        if let Some(category) = query.category {
            task_query = task_query.category(category);
        }
        if let Some(delete_reason) = query.delete_reason {
            task_query = task_query.delete_reason(delete_reason);
        }
        if let Some(bound) = query.created_before {
            task_query = task_query.created_before(bound);
        }
        if let Some(bound) = query.created_after {
            task_query = task_query.created_after(bound);
        }
        if let Some(bound) = query.completed_before {
            task_query = task_query.completed_before(bound);
        }
        if let Some(bound) = query.completed_after {
            task_query = task_query.completed_after(bound);
        }
        if let Some(finished) = query.finished {
            task_query = task_query.finished(finished);
        }
        if let Some(candidate_group) = query.candidate_group {
            task_query = task_query.candidate_group(candidate_group);
        }
        if let Some(involved_user) = query.involved_user {
            task_query = task_query.involved_user(involved_user);
        }
        if query.ignore_assignee {
            task_query = task_query.ignore_assignee_value();
        }
        if let Some(state) = query.state {
            task_query = task_query.state(parse_human_task_state(&state)?);
        }
        let mut records = task_query
            .list()?
            .into_iter()
            .map(to_historic_plan_item_instance_record)
            .collect::<Vec<_>>();

        if historic_plan_item_query_accepts_mirrors(&mirror_filter) {
            let mut mirror_query = self
                .engine
                .runtime_service()
                .create_plan_item_instance_query()
                .include_ended();
            if let Some(id) = &mirror_filter.id {
                mirror_query = mirror_query.id(id.clone());
            }
            if let Some(case_instance_id) = &mirror_filter.case_instance_id {
                mirror_query = mirror_query.case_instance_id(case_instance_id.clone());
            }
            if let Some(case_definition_id) = &mirror_filter.case_definition_id {
                mirror_query = mirror_query.case_definition_id(case_definition_id.clone());
            }
            if let Some(plan_item_definition_id) = &mirror_filter.plan_item_definition_id {
                mirror_query =
                    mirror_query.plan_item_definition_id(plan_item_definition_id.clone());
            }
            if let Some(state) = &mirror_filter.state {
                mirror_query = mirror_query.state(state.clone());
            }
            if let Some(name) = &mirror_filter.name {
                mirror_query = mirror_query.name(name.clone());
            }
            if let Some(pattern) = &mirror_filter.name_like {
                mirror_query = mirror_query.name_like(pattern.clone());
            }
            if let Some(pattern) = &mirror_filter.name_like_ignore_case {
                mirror_query = mirror_query.name_like_ignore_case(pattern.clone());
            }
            records.extend(
                mirror_query
                    .list()?
                    .into_iter()
                    .filter(|instance| instance.ended_at.is_some())
                    .filter(|instance| {
                        mirror_filter
                            .created_before
                            .is_none_or(|bound| instance.created_at < bound)
                            && mirror_filter
                                .created_after
                                .is_none_or(|bound| instance.created_at > bound)
                            && mirror_filter
                                .completed_before
                                .is_none_or(|bound| instance.ended_at.is_some_and(|at| at < bound))
                            && mirror_filter
                                .completed_after
                                .is_none_or(|bound| instance.ended_at.is_some_and(|at| at > bound))
                    })
                    .map(to_historic_plan_item_instance_record_from_mirror),
            );
        }
        if let Some(plan_item_definition_id) = query.plan_item_definition_id {
            records.retain(|record| record.plan_item_definition_id == plan_item_definition_id);
        }
        let total = records.len();
        // Java sorts before paging (HistoricTaskInstanceBaseResource.java:43-62).
        sort_historic_task_records(&mut records, query.sort.as_deref(), query.order.as_deref())?;
        let start = query.paging.start.min(total);
        let size = query.paging.size.unwrap_or(total.saturating_sub(start));
        let data = records
            .into_iter()
            .skip(start)
            .take(size)
            .collect::<Vec<_>>();
        Ok(crate::common::PagedResponse {
            start,
            size: data.len(),
            total,
            data,
            sort: query.sort,
            order: query.order,
        })
    }

    fn list_historic_milestone_instances(
        &self,
        query: routes::cmmn::HistoricMilestoneInstanceQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::HistoricMilestoneInstanceRecord>,
        crate::error::ApiError,
    > {
        let mut milestone_query = self
            .engine
            .history_service()
            .create_historic_milestone_query();
        if let Some(id) = query.id {
            milestone_query = milestone_query.id(id);
        }
        if let Some(case_instance_id) = query.case_instance_id {
            milestone_query = milestone_query.case_instance_id(case_instance_id);
        }
        if let Some(case_definition_id) = query.case_definition_id {
            milestone_query = milestone_query.case_definition_id(case_definition_id);
        }
        if let Some(case_definition_key) = query.case_definition_key {
            milestone_query = milestone_query.case_definition_key(case_definition_key);
        }
        if let Some(milestone_id) = query.milestone_id {
            milestone_query = milestone_query.milestone_id(milestone_id);
        }
        let page = if let Some(size) = query.paging.size {
            milestone_query.page(query.paging.start, size).list_page()?
        } else {
            milestone_query.list_page()?
        };
        Ok(crate::common::PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page
                .data
                .into_iter()
                .map(to_historic_milestone_instance_record)
                .collect(),
            sort: None,
            order: None,
        })
    }

    fn get_historic_stage_overview(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<routes::cmmn::StageOverviewRecord>, crate::error::ApiError> {
        Ok(self
            .engine
            .history_service()
            .get_stage_overview(case_instance_id)?
            .into_iter()
            .map(to_stage_overview_record)
            .collect())
    }

    fn list_historic_variable_instances(
        &self,
        query: routes::cmmn::HistoricVariableInstanceQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::HistoricVariableInstanceRecord>,
        crate::error::ApiError,
    > {
        let mut case_query = self
            .engine
            .history_service()
            .create_historic_case_instance_query();
        if let Some(case_instance_id) = query
            .case_instance_id
            .clone()
            .or_else(|| query.scope_id.clone())
        {
            case_query = case_query.case_instance_id(case_instance_id);
        }

        let mut variables = case_query
            .list()?
            .into_iter()
            .flat_map(historic_case_instance_variables)
            .collect::<Vec<_>>();
        if let Some(id) = query.id {
            variables.retain(|variable| variable.id == id);
        }
        if let Some(case_instance_id) = query.case_instance_id {
            variables.retain(|variable| variable.case_instance_id == case_instance_id);
        }
        if let Some(scope_id) = query.scope_id {
            variables.retain(|variable| variable.scope_id == scope_id);
        }
        if let Some(variable_name) = query.variable_name {
            variables.retain(|variable| variable.name == variable_name);
        }
        // P133: historic variable name like / exclude flags
        if let Some(variable_name_like) = query.variable_name_like.as_deref() {
            variables.retain(|variable| {
                routes::tasks::sql_like_matches(variable_name_like, &variable.name)
            });
        }
        if query.exclude_task_variables {
            variables.retain(|variable| variable.scope_type != "cmmn-task");
        }
        if query.exclude_local_variables {
            variables.retain(|variable| variable.scope_type == "cmmn");
        }

        Ok(query.paging.paginate(variables))
    }

    fn list_historic_case_instance_identity_links(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<routes::cmmn::CmmnIdentityLinkRecord>, crate::error::ApiError> {
        self.engine
            .history_service()
            .get_historic_case_instance(case_instance_id)?;
        list_cmmn_identity_links(&self.engine, "caseInstance", case_instance_id)
    }

    fn get_historic_task_form(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<serde_json::Value, crate::error::ApiError> {
        let task = self
            .engine
            .history_service()
            .get_historic_human_task(plan_item_instance_id)?;
        let definition = self
            .engine
            .repository_service()
            .get_case_definition(&task.case_definition_id)?;
        let form_key = find_cmmn_human_task_form_key(&definition.model, &task.task_definition_id)
            .ok_or_else(|| {
            crate::error::ApiError::NotFound(format!(
                "Historic CMMN task '{plan_item_instance_id}' form was not found"
            ))
        })?;
        let form_definition = self
            .form_service
            .create_form_definition_query()
            .key(form_key.clone())
            .list()?
            .into_iter()
            .next()
            .ok_or_else(|| {
                crate::error::ApiError::NotFound(format!(
                    "Historic CMMN task '{plan_item_instance_id}' form '{form_key}' was not found"
                ))
            })?;
        Ok(form_definition.form_payload)
    }

    fn list_historic_task_identity_links(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<Vec<routes::cmmn::CmmnIdentityLinkRecord>, crate::error::ApiError> {
        self.engine
            .history_service()
            .get_historic_human_task(plan_item_instance_id)?;
        list_cmmn_identity_links(&self.engine, "humanTask", plan_item_instance_id)
    }

    fn migrate_historic_case_instance(
        &self,
        case_instance_id: &str,
        command: routes::cmmn::CmmnMigrationCommand,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .history_service()
            .migrate_historic_case_instance(
                case_instance_id,
                to_cmmn_migration_document(command),
            )?;
        Ok(())
    }
}

impl routes::cmmn::CmmnManagementApi for CmmnApiAdapter {
    fn list_jobs(
        &self,
        query: routes::cmmn::CmmnManagementJobQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::cmmn::CmmnManagementJobRecord>,
        crate::error::ApiError,
    > {
        let mut job_query = self.engine.management_service().create_job_query();
        if let Some(family) = query.family {
            job_query = job_query.family(to_engine_cmmn_job_family(family));
        }
        if let Some(id) = query.id {
            job_query = job_query.id(id);
        }
        // P123: the CMMN job collection filters (JobCollectionResource.java:112-182) now
        // reach the engine query instead of being parsed and dropped.
        if let Some(scope_id) = query.scope_id {
            job_query = job_query.scope_id(scope_id);
        }
        if let Some(sub_scope_id) = query.sub_scope_id {
            job_query = job_query.sub_scope_id(sub_scope_id);
        }
        if let Some(scope_definition_id) = query.scope_definition_id {
            job_query = job_query.scope_definition_id(scope_definition_id);
        }
        if let Some(scope_type) = query.scope_type {
            job_query = job_query.scope_type(scope_type);
        }
        if let Some(element_id) = query.element_id {
            job_query = job_query.element_id(element_id);
        }
        if query.without_scope_id {
            job_query = job_query.without_scope_id();
        }
        if query.timers_only {
            job_query = job_query.timers();
        }
        if query.messages_only {
            job_query = job_query.messages();
        }
        if query.with_exception {
            job_query = job_query.with_exception();
        }
        if let Some(exception_message) = query.exception_message {
            job_query = job_query.exception_message(exception_message);
        }
        if let Some(due_before) = query.due_before {
            job_query = job_query.due_before(due_before);
        }
        if let Some(due_after) = query.due_after {
            job_query = job_query.due_after(due_after);
        }
        if let Some(tenant_id) = query.tenant_id {
            job_query = job_query.tenant_id(tenant_id);
        }
        if let Some(tenant_id_like) = query.tenant_id_like {
            job_query = job_query.tenant_id_like(tenant_id_like);
        }
        if query.without_tenant_id {
            job_query = job_query.without_tenant_id();
        }
        let page = if let Some(size) = query.paging.size {
            job_query.page(query.paging.start, size).list_page()?
        } else {
            job_query.list_page()?
        };
        Ok(crate::common::PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page.data.into_iter().map(to_cmmn_job_record).collect(),
            sort: None,
            order: None,
        })
    }

    fn get_job(
        &self,
        family: routes::cmmn::CmmnManagementJobFamily,
        job_id: &str,
    ) -> Result<routes::cmmn::CmmnManagementJobRecord, crate::error::ApiError> {
        let job = self.engine.management_service().get_job(job_id)?;
        if job.family != to_engine_cmmn_job_family(family) {
            return Err(crate::error::ApiError::NotFound(format!(
                "CMMN {} job '{}' was not found",
                cmmn_job_family_name(family),
                job_id
            )));
        }
        Ok(to_cmmn_job_record(job))
    }

    fn get_job_exception_stacktrace(
        &self,
        family: routes::cmmn::CmmnManagementJobFamily,
        job_id: &str,
    ) -> Result<String, crate::error::ApiError> {
        let job = self.engine.management_service().get_job(job_id)?;
        if job.family != to_engine_cmmn_job_family(family) {
            return Err(crate::error::ApiError::NotFound(format!(
                "CMMN {} job '{}' was not found",
                cmmn_job_family_name(family),
                job_id
            )));
        }
        job.exception_stacktrace
            .filter(|stacktrace| !stacktrace.is_empty())
            .ok_or_else(|| {
                crate::error::ApiError::NotFound(format!(
                    "CMMN {} job '{}' does not have an exception stacktrace",
                    cmmn_job_family_name(family),
                    job_id
                ))
            })
    }

    fn delete_job(
        &self,
        family: routes::cmmn::CmmnManagementJobFamily,
        job_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        // Family validation lives in the management service; REST only maps HTTP.
        match family {
            routes::cmmn::CmmnManagementJobFamily::Suspended => {
                self.engine
                    .management_service()
                    .delete_suspended_job(job_id)?;
                Ok(())
            }
            other => {
                // Keep executable/timer/deadletter/history delete via generic
                // path with family-typed 404 when mismatch.
                let job = self.engine.management_service().get_job(job_id)?;
                if job.family != to_engine_cmmn_job_family(other) {
                    return Err(crate::error::ApiError::NotFound(format!(
                        "CMMN {} job '{}' was not found",
                        cmmn_job_family_name(other),
                        job_id
                    )));
                }
                self.engine.management_service().delete_job(job_id)?;
                Ok(())
            }
        }
    }

    /// Java `JobResource.executeJobAction` (JobResource.java:216-231) resolves the job first
    /// (404 when absent, JobBaseResource.java:34-72) and then calls
    /// `managementService.executeJob`. In Rust the execute path lives on the engine
    /// (cmmn-engine/src/lib.rs:194) rather than on the management service, so the family
    /// check is done here before delegating.
    fn execute_job(&self, job_id: &str) -> Result<(), crate::error::ApiError> {
        let job = self.engine.management_service().get_job(job_id)?;
        if job.family != flowable_cmmn_engine::CmmnJobFamily::Executable {
            return Err(crate::error::ApiError::NotFound(format!(
                "CMMN executable job '{job_id}' was not found"
            )));
        }
        self.engine.execute_job(job_id)?;
        Ok(())
    }

    /// Java `JobResource.executeTimerJobAction` `move` (JobResource.java:248-254) calls
    /// `managementService.moveTimerToExecutableJob`, which lands on
    /// `DefaultJobManager.moveTimerJobToExecutableJob` (DefaultJobManager.java:126-139):
    /// an executable row is created from the timer row via `copyJobInfo` and the timer row
    /// is deleted. Rust keeps one `ACT_CMMN_JOB` table, so the family/state columns are
    /// rewritten in place instead of delete+insert.
    fn move_timer_job_to_executable(&self, job_id: &str) -> Result<(), crate::error::ApiError> {
        let management = self.engine.management_service();
        let job = management.get_job(job_id)?;
        if job.family != flowable_cmmn_engine::CmmnJobFamily::Timer {
            return Err(crate::error::ApiError::NotFound(format!(
                "CMMN timer job '{job_id}' was not found"
            )));
        }
        let mut executable = job;
        executable.family = flowable_cmmn_engine::CmmnJobFamily::Executable;
        executable.state = flowable_cmmn_engine::CmmnJobFamily::Executable
            .as_str()
            .to_string();
        // `copyJobInfo` (DefaultJobManager.java:769-796) carries the due date, retries,
        // scope and handler fields across unchanged. The lock is only stamped when the
        // async executor is active (DefaultJobManager.java:709-715); Rust has no async
        // executor behind this REST call, so the lock is cleared instead.
        executable.lock_owner = None;
        management.update_job(&executable)?;
        Ok(())
    }

    /// Java `JobResource.executeTimerJobAction` reschedule branch calls
    /// `rescheduleTimeDateValueJob` (JobResource.java:255-264). The CMMN management
    /// command performs the plan-item lookup and delete/recreate transaction.
    fn reschedule_timer_job(
        &self,
        job_id: &str,
        due_date: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .management_service()
            .reschedule_time_date_value_job(job_id, due_date)?;
        Ok(())
    }

    /// Java `JobResource.executeDeadLetterJobAction` `move` chooses executable/history
    /// from the deadletter's jobType (JobResource.java:306-323).
    fn move_deadletter_job(&self, job_id: &str) -> Result<(), crate::error::ApiError> {
        self.engine
            .management_service()
            .move_deadletter_job_by_type(job_id, 3)?;
        Ok(())
    }

    /// Java `moveToHistoryJob` forces history and uses async-history retries
    /// (JobResource.java:330-339). Rust CMMN has one default retry setting (3).
    fn move_deadletter_job_to_history(
        &self,
        job_id: &str,
    ) -> Result<(), crate::error::ApiError> {
        self.engine
            .management_service()
            .move_deadletter_job_to_history_job(job_id, 3)?;
        Ok(())
    }

    /// Java `JobResource.executeHistoryJob` delegates an existing history row to
    /// `CmmnManagementService.executeHistoryJob` (JobResource.java:281-288).
    fn execute_history_job(&self, job_id: &str) -> Result<(), crate::error::ApiError> {
        self.engine.execute_history_job(job_id)?;
        Ok(())
    }
}

/// Java `CaseInstanceState` spells the states in lower case (CaseInstanceState.java:28-33)
/// and the query filters on that literal, so `state=completed` is a legal Java
/// request. The pre-P120 Rust surface only accepted the upper-case rendering used
/// by `format_case_state`; P120 accepts either casing rather than 400-ing on the
/// Java spelling. `failed`/`closed` have no Rust `CmmnCaseInstanceState` variant
/// and stay rejected.
fn parse_case_state(state: &str) -> Result<CmmnCaseInstanceState, crate::error::ApiError> {
    match state.to_ascii_uppercase().as_str() {
        "ACTIVE" => Ok(CmmnCaseInstanceState::Active),
        "COMPLETED" => Ok(CmmnCaseInstanceState::Completed),
        "TERMINATED" => Ok(CmmnCaseInstanceState::Terminated),
        "SUSPENDED" => Ok(CmmnCaseInstanceState::Suspended),
        _ => Err(crate::error::ApiError::bad_request(format!(
            "Unsupported CMMN case state filter '{state}'"
        ))),
    }
}

fn find_cmmn_human_task_form_key(case: &CmmnCase, task_definition_id: &str) -> Option<String> {
    find_form_key_in_container(
        &case.case_plan_model.human_tasks,
        &case.case_plan_model.stages,
        task_definition_id,
    )
}

fn find_form_key_in_container(
    human_tasks: &[CmmnHumanTask],
    stages: &[CmmnStage],
    task_definition_id: &str,
) -> Option<String> {
    for human_task in human_tasks {
        if human_task.id == task_definition_id
            && let Some(form_key) = &human_task.form_key
            && !form_key.trim().is_empty()
        {
            return Some(form_key.clone());
        }
    }
    for stage in stages {
        if let Some(key) =
            find_form_key_in_container(&stage.human_tasks, &stage.stages, task_definition_id)
        {
            return Some(key);
        }
    }
    None
}

fn parse_human_task_state(state: &str) -> Result<CmmnHumanTaskState, crate::error::ApiError> {
    match state {
        "AVAILABLE" => Ok(CmmnHumanTaskState::Available),
        "ENABLED" => Ok(CmmnHumanTaskState::Enabled),
        "ACTIVE" => Ok(CmmnHumanTaskState::Active),
        "DISABLED" => Ok(CmmnHumanTaskState::Disabled),
        "COMPLETED" => Ok(CmmnHumanTaskState::Completed),
        "TERMINATED" => Ok(CmmnHumanTaskState::Terminated),
        other => Err(crate::error::ApiError::bad_request(format!(
            "Unsupported CMMN plan item state filter '{other}'"
        ))),
    }
}

fn format_case_state(state: &CmmnCaseInstanceState) -> &'static str {
    match state {
        CmmnCaseInstanceState::Active => "ACTIVE",
        CmmnCaseInstanceState::Completed => "COMPLETED",
        CmmnCaseInstanceState::Terminated => "TERMINATED",
        CmmnCaseInstanceState::Suspended => "SUSPENDED",
    }
}

fn format_human_task_state(state: &CmmnHumanTaskState) -> &'static str {
    match state {
        CmmnHumanTaskState::Available => "AVAILABLE",
        CmmnHumanTaskState::Enabled => "ENABLED",
        CmmnHumanTaskState::Active => "ACTIVE",
        CmmnHumanTaskState::Disabled => "DISABLED",
        CmmnHumanTaskState::Completed => "COMPLETED",
        CmmnHumanTaskState::Terminated => "TERMINATED",
    }
}

fn to_engine_cmmn_job_family(
    family: routes::cmmn::CmmnManagementJobFamily,
) -> flowable_cmmn_engine::CmmnJobFamily {
    match family {
        routes::cmmn::CmmnManagementJobFamily::Executable => {
            flowable_cmmn_engine::CmmnJobFamily::Executable
        }
        routes::cmmn::CmmnManagementJobFamily::Timer => flowable_cmmn_engine::CmmnJobFamily::Timer,
        routes::cmmn::CmmnManagementJobFamily::Deadletter => {
            flowable_cmmn_engine::CmmnJobFamily::Deadletter
        }
        routes::cmmn::CmmnManagementJobFamily::History => {
            flowable_cmmn_engine::CmmnJobFamily::History
        }
        routes::cmmn::CmmnManagementJobFamily::Suspended => {
            flowable_cmmn_engine::CmmnJobFamily::Suspended
        }
    }
}

fn cmmn_job_family_name(family: routes::cmmn::CmmnManagementJobFamily) -> &'static str {
    match family {
        routes::cmmn::CmmnManagementJobFamily::Executable => "executable",
        routes::cmmn::CmmnManagementJobFamily::Timer => "timer",
        routes::cmmn::CmmnManagementJobFamily::Deadletter => "deadletter",
        routes::cmmn::CmmnManagementJobFamily::History => "history",
        routes::cmmn::CmmnManagementJobFamily::Suspended => "suspended",
    }
}

fn to_cmmn_migration_document(
    command: routes::cmmn::CmmnMigrationCommand,
) -> CmmnMigrationDocument {
    CmmnMigrationDocument {
        target_case_definition_id: command.target_case_definition_id,
    }
}

fn to_cmmn_change_plan_item_state_request(
    command: routes::cmmn::CmmnChangePlanItemStateCommand,
) -> EngineCmmnChangePlanItemStateRequest {
    EngineCmmnChangePlanItemStateRequest {
        activate_plan_item_definition_ids: command.activate_plan_item_definition_ids,
        move_to_available_plan_item_definition_ids: command
            .move_to_available_plan_item_definition_ids,
        terminate_plan_item_definition_ids: command.terminate_plan_item_definition_ids,
        add_waiting_for_repetition_plan_item_definition_ids: command
            .add_waiting_for_repetition_plan_item_definition_ids,
        remove_waiting_for_repetition_plan_item_definition_ids: command
            .remove_waiting_for_repetition_plan_item_definition_ids,
        change_plan_item_ids: command.change_plan_item_ids.into_iter().collect(),
        change_plan_item_ids_with_definition_id: command
            .change_plan_item_ids_with_definition_id
            .into_iter()
            .collect(),
        change_plan_item_definitions_with_new_target_ids: command
            .change_plan_item_definitions_with_new_target_ids
            .into_iter()
            .map(|item| CmmnPlanItemDefinitionWithTargetIds {
                existing_plan_item_definition_id: item.existing_plan_item_definition_id,
                new_plan_item_id: item.new_plan_item_id,
                new_plan_item_definition_id: item.new_plan_item_definition_id,
            })
            .collect(),
    }
}

fn to_cmmn_event_subscription_record(
    subscription: CmmnEventSubscription,
) -> routes::cmmn::CmmnEventSubscriptionRecord {
    routes::cmmn::CmmnEventSubscriptionRecord {
        id: subscription.id,
        event_type: subscription.event_type,
        event_name: subscription.event_name,
        activity_id: subscription.activity_id,
        case_instance_id: subscription.case_instance_id,
        case_definition_id: subscription.case_definition_id,
        plan_item_instance_id: subscription.plan_item_instance_id,
        tenant_id: subscription.tenant_id,
        configuration: subscription.configuration,
        created: subscription.created_at.to_rfc3339(),
    }
}

fn to_case_definition_record(
    definition: flowable_cmmn_engine::CmmnCaseDefinition,
) -> routes::cmmn::CaseDefinitionRecord {
    routes::cmmn::CaseDefinitionRecord {
        id: definition.id,
        key: definition.key,
        name: definition.name,
        version: definition.version,
        deployment_id: definition.deployment_id,
        resource_name: definition.resource_name,
        category: None,
        description: definition.model.description,
        tenant_id: definition.tenant_id,
        parent_deployment_id: None,
    }
}

fn to_case_instance_record(
    instance: flowable_cmmn_engine::CmmnCaseInstance,
) -> routes::cmmn::CaseInstanceRecord {
    routes::cmmn::CaseInstanceRecord {
        id: instance.id,
        case_definition_id: instance.case_definition_id,
        case_definition_key: instance.case_definition_key,
        business_key: instance.business_key,
        name: Some(instance.name),
        state: format_case_state(&instance.state).to_string(),
        business_status: instance.business_status,
        started_by: instance.started_by,
        callback_id: instance.callback_id,
        callback_type: instance.callback_type,
        // Java `CaseInstanceResponse.java:51-52` exposes persisted reference metadata.
        reference_id: instance.reference_id,
        reference_type: instance.reference_type,
        case_definition_name: None,
        variables: Vec::new(),
        tenant_id: instance.tenant_id,
        started_at: instance.started_at.to_rfc3339(),
    }
}

/// Whether the plan-item query carries a filter that only human-task plan items
/// can satisfy (assignee/owner/priority/dueDate/category/delegation/candidate/…).
/// When present, the non-task mirror sources (stage / milestone / event listener)
/// are skipped — those rows carry none of these fields, so no mirror row could
/// match anyway. Java's PlanItemInstanceQuery supports `assignee` etc. with the
/// same effect (only task plan item instances carry an assignee).
fn plan_item_query_has_task_specific_filters(
    query: &routes::cmmn::PlanItemInstanceQuery,
) -> bool {
    query.assignee.is_some()
        || query.assignee_like.is_some()
        || query.owner.is_some()
        || query.owner_like.is_some()
        || query.unassigned.is_some()
        || query.delegation_state.is_some()
        || query.category.is_some()
        || !query.category_in.is_empty()
        || !query.category_not_in.is_empty()
        || query.without_category
        || query.task_definition_key.is_some()
        || query.task_definition_key_like.is_some()
        || query.priority.is_some()
        || query.min_priority.is_some()
        || query.max_priority.is_some()
        || query.created_on.is_some()
        || query.created_before.is_some()
        || query.created_after.is_some()
        || query.due_date.is_some()
        || query.due_before.is_some()
        || query.due_after.is_some()
        || query.without_due_date
        || query.active.is_some()
        || query.candidate_user.is_some()
        || query.candidate_group.is_some()
        || !query.candidate_group_in.is_empty()
        || query.candidate_or_assigned.is_some()
        || query.case_definition_key.is_some()
        || query.case_definition_key_like.is_some()
        || query.case_definition_key_like_ignore_case.is_some()
}

fn to_plan_item_instance_record(
    task: flowable_cmmn_engine::CmmnHumanTaskInstance,
) -> routes::cmmn::PlanItemInstanceRecord {
    routes::cmmn::PlanItemInstanceRecord {
        id: task.id,
        case_instance_id: task.case_instance_id,
        case_definition_id: task.case_definition_id,
        // Java PlanItemInstanceEntityManagerImpl.java:92-95 stores the plan item
        // XML id as elementId and the definitionRef target as planItemDefinitionId.
        plan_item_definition_id: task.task_definition_id,
        plan_item_definition_type: "humantask".to_string(),
        element_id: task.plan_item_id,
        stage_instance_id: task.stage_instance_id,
        stage: false,
        name: task.name,
        state: format_human_task_state(&task.state).to_string(),
        occurred_time: None,
        assignee: task.assignee,
        owner: task.owner,
        priority: task.priority,
        due_date: task.due_date,
        category: task.category,
        delegation_state: task
            .delegation_state
            .as_ref()
            .map(format_delegation_state)
            .map(str::to_string),
        variables: Vec::new(),
        tenant_id: None,
        created_at: task.activated_at.to_rfc3339(),
        ended_at: task.completed_at.map(|value| value.to_rfc3339()),
    }
}

/// P116: map a unified plan-item-instance mirror row (stage / milestone / event
/// listener) into the REST record. Java `CmmnRestResponseFactory.createPlanItemInstanceResponse`
/// (CmmnRestResponseFactory.java:536-591).
fn to_plan_item_instance_record_from_mirror(
    instance: flowable_cmmn_engine::CmmnPlanItemInstance,
) -> routes::cmmn::PlanItemInstanceRecord {
    routes::cmmn::PlanItemInstanceRecord {
        id: instance.id,
        case_instance_id: instance.case_instance_id,
        case_definition_id: instance.case_definition_id,
        plan_item_definition_id: instance.plan_item_definition_id,
        plan_item_definition_type: instance.plan_item_definition_type.clone(),
        element_id: instance.plan_item_id,
        stage_instance_id: instance.stage_instance_id,
        stage: instance.plan_item_definition_type == "stage",
        name: instance.name,
        state: instance.state,
        occurred_time: instance.occurred_at.map(|value| value.to_rfc3339()),
        assignee: instance.assignee,
        owner: None,
        priority: None,
        due_date: None,
        category: None,
        delegation_state: None,
        variables: Vec::new(),
        tenant_id: instance.tenant_id,
        created_at: instance.created_at.to_rfc3339(),
        ended_at: instance.ended_at.map(|value| value.to_rfc3339()),
    }
}

/// Java `paginateList` sort/order over case instance records
/// (PaginateListUtil.java:117-131, BaseCaseInstanceResource.java:45-54).
/// Unknown sort/order → 400, mirroring Java's `FlowableIllegalArgumentException`.
fn sort_case_instance_records(
    records: &mut [routes::cmmn::CaseInstanceRecord],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), crate::error::ApiError> {
    match sort {
        None => {}
        Some("id") => {
            records.sort_by(|left, right| left.id.cmp(&right.id));
        }
        Some("caseDefinitionId") => {
            records.sort_by(|left, right| {
                left.case_definition_id
                    .cmp(&right.case_definition_id)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("caseDefinitionKey") => {
            records.sort_by(|left, right| {
                left.case_definition_key
                    .cmp(&right.case_definition_key)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("startTime") => {
            records.sort_by(|left, right| {
                left.started_at
                    .cmp(&right.started_at)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("tenantId") => {
            records.sort_by(|left, right| {
                left.tenant_id
                    .cmp(&right.tenant_id)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("businessKey") => {
            records.sort_by(|left, right| {
                left.business_key
                    .cmp(&right.business_key)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some(other) => {
            return Err(crate::error::ApiError::bad_request(format!(
                "Value for param 'sort' is not valid, '{other}' is not a valid property"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => records.reverse(),
        Some(other) => {
            return Err(crate::error::ApiError::bad_request(format!(
                "Value for param 'order' is not valid : '{other}', must be 'asc' or 'desc'"
            )));
        }
    }

    Ok(())
}

/// Java `paginateList` sort/order over historic case instance records; the
/// allowed properties come from `HistoricCaseInstanceBaseResource`'s
/// `allowedSortProperties` map (HistoricCaseInstanceBaseResource.java:48-52).
/// Unknown sort/order → 400, mirroring Java's `FlowableIllegalArgumentException`.
fn sort_historic_case_instance_records(
    records: &mut [routes::cmmn::HistoricCaseInstanceRecord],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), crate::error::ApiError> {
    match sort {
        None => {}
        Some("caseInstanceId") => {
            records.sort_by(|left, right| left.id.cmp(&right.id));
        }
        Some("caseDefinitionId") => {
            records.sort_by(|left, right| {
                left.case_definition_id
                    .cmp(&right.case_definition_id)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("startTime") => {
            records.sort_by(|left, right| {
                left.started_at
                    .cmp(&right.started_at)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("endTime") => {
            // Unfinished cases carry a null END_TIME_; SQL sorts nulls first on the
            // engines Flowable targets, and `None` already orders before `Some`.
            records.sort_by(|left, right| {
                left.ended_at
                    .cmp(&right.ended_at)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("tenantId") => {
            records.sort_by(|left, right| {
                left.tenant_id
                    .cmp(&right.tenant_id)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some(other) => {
            return Err(crate::error::ApiError::bad_request(format!(
                "Value for param 'sort' is not valid, '{other}' is not a valid property"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => records.reverse(),
        Some(other) => {
            return Err(crate::error::ApiError::bad_request(format!(
                "Value for param 'order' is not valid : '{other}', must be 'asc' or 'desc'"
            )));
        }
    }

    Ok(())
}

/// Java `paginateList` sort/order over historic task records; the allowed
/// properties come from `HistoricTaskInstanceBaseResource`'s
/// `allowedSortProperties` map (HistoricTaskInstanceBaseResource.java:43-62).
/// Properties the Rust record cannot express (`deleteReason`, `duration`,
/// `executionId`, `description`, `dueDate`, `priority`, `owner`,
/// `taskDefinitionKey`) are rejected rather than silently ignored.
fn sort_historic_task_records(
    records: &mut [routes::cmmn::HistoricPlanItemInstanceRecord],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), crate::error::ApiError> {
    match sort {
        None => {}
        Some("taskInstanceId") => {
            records.sort_by(|left, right| left.id.cmp(&right.id));
        }
        Some("caseInstanceId") => {
            records.sort_by(|left, right| {
                left.case_instance_id
                    .cmp(&right.case_instance_id)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("caseDefinitionId") => {
            records.sort_by(|left, right| {
                left.case_definition_id
                    .cmp(&right.case_definition_id)
                    .then(left.id.cmp(&right.id))
            });
        }
        // Java maps both "start" and "startTime" onto the same START property.
        Some("start") | Some("startTime") => {
            records.sort_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("endTime") => {
            records.sort_by(|left, right| {
                left.ended_at
                    .cmp(&right.ended_at)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("name") => {
            records.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        }
        Some("assignee") => {
            records.sort_by(|left, right| {
                left.assignee
                    .cmp(&right.assignee)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("tenantId") => {
            records.sort_by(|left, right| {
                left.tenant_id
                    .cmp(&right.tenant_id)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some(other) => {
            return Err(crate::error::ApiError::bad_request(format!(
                "Value for param 'sort' is not valid, '{other}' is not a valid property"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => records.reverse(),
        Some(other) => {
            return Err(crate::error::ApiError::bad_request(format!(
                "Value for param 'order' is not valid : '{other}', must be 'asc' or 'desc'"
            )));
        }
    }

    Ok(())
}

fn format_delegation_state(state: &flowable_cmmn_engine::CmmnDelegationState) -> &'static str {
    // Java TaskResponse.getDelegationStateString (TaskResponse.java:105-111):
    // `state.toString().toLowerCase()`.
    match state {
        flowable_cmmn_engine::CmmnDelegationState::Pending => "pending",
        flowable_cmmn_engine::CmmnDelegationState::Resolved => "resolved",
    }
}

/// Java `paginateList` sort/order over task records (PaginateListUtil.java:117-131,
/// TaskBaseResource.java:46-60): the allowed sort properties are the CMMN task
/// fields that carry data. Unknown sort/order → 400, mirroring Java's
/// `FlowableIllegalArgumentException`.
fn sort_plan_item_records(
    records: &mut [routes::cmmn::PlanItemInstanceRecord],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), crate::error::ApiError> {
    match sort {
        None => {}
        Some("id") => {
            records.sort_by(|left, right| left.id.cmp(&right.id));
        }
        Some("name") => {
            records.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        }
        Some("dueDate") => {
            records.sort_by(|left, right| {
                left.due_date
                    .cmp(&right.due_date)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("createTime") => {
            records.sort_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("priority") => {
            records.sort_by(|left, right| {
                left.priority
                    .cmp(&right.priority)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("assignee") => {
            records.sort_by(|left, right| {
                left.assignee
                    .cmp(&right.assignee)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("owner") => {
            records.sort_by(|left, right| {
                left.owner
                    .cmp(&right.owner)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("category") => {
            records.sort_by(|left, right| {
                left.category
                    .cmp(&right.category)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some("taskDefinitionKey") => {
            records.sort_by(|left, right| {
                left.plan_item_definition_id
                    .cmp(&right.plan_item_definition_id)
                    .then(left.id.cmp(&right.id))
            });
        }
        Some(other) => {
            return Err(crate::error::ApiError::bad_request(format!(
                "Value for param 'sort' is not valid, '{other}' is not a valid property"
            )));
        }
    }

    match order {
        None | Some("asc") => {}
        Some("desc") => records.reverse(),
        Some(other) => {
            return Err(crate::error::ApiError::bad_request(format!(
                "Value for param 'order' is not valid : '{other}', must be 'asc' or 'desc'"
            )));
        }
    }

    Ok(())
}

/// Java: only an explicit LOCAL `RestVariableScope` routes a completion variable
/// to the task's local scope; null and GLOBAL are case-scoped
/// (TaskResource.java:207-211).
fn completion_variable_scope_is_local(scope: Option<&str>) -> Result<bool, crate::error::ApiError> {
    match scope {
        Some(s) if s.eq_ignore_ascii_case("local") => Ok(true),
        Some(s) if s.eq_ignore_ascii_case("global") => Ok(false),
        Some(s) => Err(crate::error::ApiError::bad_request(format!(
            "Unsupported variable scope '{s}'"
        ))),
        None => Ok(false),
    }
}

fn to_stage_overview_record(stage: CmmnStageOverview) -> routes::cmmn::StageOverviewRecord {
    routes::cmmn::StageOverviewRecord {
        id: stage.id,
        name: stage.name,
        current: stage.current,
        ended: stage.ended,
        end_time: stage.end_time.map(|value| value.to_rfc3339()),
    }
}

fn to_cmmn_job_record(job: flowable_cmmn_engine::CmmnJob) -> routes::cmmn::CmmnManagementJobRecord {
    routes::cmmn::CmmnManagementJobRecord {
        id: job.id,
        job_type: job.family.as_str().to_string(),
        scope_id: job.scope_id,
        sub_scope_id: job.sub_scope_id,
        scope_type: "cmmn".to_string(),
        scope_definition_id: job.scope_definition_id,
        element_id: job.element_id,
        tenant_id: job.tenant_id,
        create_time: job.created_at.to_rfc3339(),
        due_date: job.due_date.map(|value| value.to_rfc3339()),
        lock_owner: job.lock_owner,
        retries: job.retries,
        exception_message: job.exception_message,
    }
}

fn to_historic_case_instance_record(
    instance: CmmnHistoricCaseInstance,
    include_case_variables: bool,
    include_case_variable_names: &[String],
) -> routes::cmmn::HistoricCaseInstanceRecord {
    let variables = if include_case_variables || !include_case_variable_names.is_empty() {
        instance
            .variables
            .iter()
            .filter(|(name, _)| {
                include_case_variable_names.is_empty() || include_case_variable_names.contains(name)
            })
            .map(|(name, value)| {
                serde_json::json!({
                    "name": name,
                    "type": cmmn_variable_type(value),
                    "value": value,
                    "scope": "local",
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    routes::cmmn::HistoricCaseInstanceRecord {
        id: instance.case_instance_id,
        case_definition_id: instance.case_definition_id,
        case_definition_key: instance.case_definition_key,
        business_key: instance.business_key,
        name: Some(instance.name),
        state: format_case_state(&instance.state).to_string(),
        tenant_id: instance.tenant_id,
        started_at: instance.started_at.to_rfc3339(),
        ended_at: instance.completed_at.map(|value| value.to_rfc3339()),
        // Java `HistoricCaseInstanceResponse.java:53-54` exposes both fields.
        reference_id: instance.reference_id,
        reference_type: instance.reference_type,
        // Java `HistoricCaseInstanceResponse.java:46` names the actor `endUserId`.
        end_user_id: instance.finished_by,
        variables,
    }
}

fn to_historic_plan_item_instance_record(
    task: CmmnHistoricHumanTaskInstance,
) -> routes::cmmn::HistoricPlanItemInstanceRecord {
    routes::cmmn::HistoricPlanItemInstanceRecord {
        id: task.task_id,
        case_instance_id: task.case_instance_id,
        case_definition_id: task.case_definition_id,
        // Java CmmnRestResponseFactory.java:901-902 serialises the definitionRef
        // target, not the plan item XML id.
        plan_item_definition_id: task.task_definition_id,
        // HistoricPlanItemInstanceResponse.java:43 — human-task rows are always
        // the lowercased type string `"humantask"` (runtime parity at cmmn.rs:1231).
        plan_item_definition_type: "humantask".to_string(),
        // HistoricPlanItemInstanceResponse.java:41 — plan item XML id; the value
        // P131 moved off planItemDefinitionId (migration path for old clients).
        element_id: task.plan_item_id,
        // HistoricPlanItemInstanceResponse.java:39
        stage_instance_id: task.stage_instance_id,
        name: task.name,
        state: format_human_task_state(&task.state).to_string(),
        // Java `HistoricTaskInstanceResponse.assignee` is populated from the
        // historic task row (HistoricTaskInstanceResponse.java assignee field);
        // the Rust historic human task carries the same value. The CMMN human
        // task has no tenant column, so `tenantId` stays null (P100 acceptance).
        assignee: task.assignee,
        tenant_id: None,
        created_at: task.activated_at.to_rfc3339(),
        ended_at: task.completed_at.map(|value| value.to_rfc3339()),
    }
}

fn historic_plan_item_query_accepts_mirrors(
    query: &routes::cmmn::HistoricPlanItemInstanceQuery,
) -> bool {
    query.finished != Some(false)
        && query.task_definition_key.is_none()
        && query.task_definition_key_like.is_none()
        && query.assignee.is_none()
        && query.assignee_like.is_none()
        && query.owner.is_none()
        && query.owner_like.is_none()
        && query.category.is_none()
        && query.delete_reason.is_none()
        && query.candidate_group.is_none()
        && query.involved_user.is_none()
}

fn to_historic_plan_item_instance_record_from_mirror(
    instance: flowable_cmmn_engine::CmmnPlanItemInstance,
) -> routes::cmmn::HistoricPlanItemInstanceRecord {
    routes::cmmn::HistoricPlanItemInstanceRecord {
        id: instance.id,
        case_instance_id: instance.case_instance_id,
        case_definition_id: instance.case_definition_id,
        plan_item_definition_id: instance.plan_item_definition_id,
        // HistoricPlanItemInstanceResponse.java:43
        plan_item_definition_type: instance.plan_item_definition_type.clone(),
        // HistoricPlanItemInstanceResponse.java:41
        element_id: instance.plan_item_id,
        // HistoricPlanItemInstanceResponse.java:39
        stage_instance_id: instance.stage_instance_id,
        name: instance.name,
        state: instance.state,
        assignee: instance.assignee,
        tenant_id: instance.tenant_id,
        created_at: instance.created_at.to_rfc3339(),
        ended_at: instance.ended_at.map(|value| value.to_rfc3339()),
    }
}

fn to_historic_milestone_instance_record(
    milestone: CmmnHistoricMilestoneInstance,
) -> routes::cmmn::HistoricMilestoneInstanceRecord {
    routes::cmmn::HistoricMilestoneInstanceRecord {
        id: milestone.id,
        case_instance_id: milestone.case_instance_id,
        case_definition_id: milestone.case_definition_id,
        case_definition_key: milestone.case_definition_key,
        milestone_id: milestone.milestone_id,
        name: milestone.name,
        tenant_id: milestone.tenant_id,
        time: milestone.time.to_rfc3339(),
    }
}

fn historic_case_instance_variables(
    instance: CmmnHistoricCaseInstance,
) -> Vec<routes::cmmn::HistoricVariableInstanceRecord> {
    let case_instance_id = instance.case_instance_id;
    let tenant_id = instance.tenant_id;
    instance
        .variables
        .into_iter()
        .map(
            |(name, value)| routes::cmmn::HistoricVariableInstanceRecord {
                id: format!("cmmn-historic-variable:{case_instance_id}:{name}"),
                variable_type: cmmn_variable_type(&value).to_string(),
                value,
                name,
                case_instance_id: case_instance_id.clone(),
                scope_id: case_instance_id.clone(),
                scope_type: "cmmn".to_string(),
                tenant_id: tenant_id.clone(),
            },
        )
        .collect()
}

fn case_instance_variables(
    instance: flowable_cmmn_engine::CmmnCaseInstance,
) -> Vec<routes::cmmn::VariableInstanceRecord> {
    let case_instance_id = instance.id;
    let tenant_id = instance.tenant_id;
    instance
        .variables
        .into_iter()
        .map(|(name, value)| routes::cmmn::VariableInstanceRecord {
            id: format!("cmmn-variable:{case_instance_id}:{name}"),
            variable_type: cmmn_variable_type(&value).to_string(),
            value,
            name,
            case_instance_id: case_instance_id.clone(),
            scope_id: case_instance_id.clone(),
            scope_type: "cmmn".to_string(),
            tenant_id: tenant_id.clone(),
        })
        .collect()
}

/// Task-local variables as REST records. The task's own scope is the human task
/// (Java ACT_RU_VARIABLE rows with TASK_ID_ set); scope_id is the task id.
fn task_local_variables(
    task: flowable_cmmn_engine::CmmnHumanTaskInstance,
) -> Vec<routes::cmmn::VariableInstanceRecord> {
    let task_id = task.id;
    let case_instance_id = task.case_instance_id;
    task.task_local_variables
        .into_iter()
        .map(|(name, value)| routes::cmmn::VariableInstanceRecord {
            id: format!("cmmn-task-local-variable:{task_id}:{name}"),
            variable_type: cmmn_variable_type(&value).to_string(),
            value,
            name,
            case_instance_id: case_instance_id.clone(),
            scope_id: task_id.clone(),
            scope_type: "cmmn-task".to_string(),
            tenant_id: None,
        })
        .collect()
}

fn cmmn_variable_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "double",
        Value::String(_) => "string",
        Value::Array(_) => "json",
        Value::Object(_) => "json",
    }
}

#[derive(Clone)]
struct AppDefinitionCatalogAdapter {
    engine: Arc<ProcessEngine>,
    dmn_engine: Arc<DmnEngine>,
    cmmn_engine: Arc<CmmnEngine>,
    event_registry_service: FlowableEventRegistryService,
}

impl DefinitionCatalog for AppDefinitionCatalogAdapter {
    fn resolve_definition(
        &self,
        definition_type: DefinitionType,
        definition_key: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<ResolvedDefinition>, flowable_app_engine::AppError> {
        match definition_type {
            DefinitionType::BpmnProcess => {
                let definition = self
                    .engine
                    .get_repository_service()
                    .latest_process_definition_by_key(definition_key, tenant_id)
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name.unwrap_or_else(|| "Process".to_string()),
                    deployment_id: definition
                        .deployment_id
                        .unwrap_or_else(|| "process-deployment".to_string()),
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::DmnDecision => {
                let mut query = self
                    .dmn_engine
                    .repository_service()
                    .create_decision_query()
                    .key(definition_key);
                if let Some(tenant_id) = tenant_id {
                    query = query.tenant_id(tenant_id.to_string());
                }
                let definition = query
                    .single_result()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::CmmnCase => {
                let mut query = self
                    .cmmn_engine
                    .repository_service()
                    .create_case_definition_query()
                    .key(definition_key);
                if let Some(tenant_id) = tenant_id {
                    query = query.tenant_id(tenant_id.to_string());
                }
                let definition = query
                    .single_result()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::EventRegistry => {
                let mut query = self
                    .event_registry_service
                    .create_event_definition_query()
                    .key(definition_key)
                    .latest();
                if let Some(tenant_id) = tenant_id {
                    query = query.tenant_id(tenant_id.to_string());
                }
                let definition = query
                    .list()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?
                    .into_iter()
                    .next()
                    .or_else(|| {
                        if tenant_id.is_some() {
                            self.event_registry_service
                                .create_event_definition_query()
                                .key(definition_key)
                                .latest()
                                .list()
                                .ok()
                                .and_then(|definitions| {
                                    definitions
                                        .into_iter()
                                        .find(|definition| definition.tenant_id.is_none())
                                })
                        } else {
                            None
                        }
                    });
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
        }
    }

    fn resolve_definition_by_id(
        &self,
        definition_type: DefinitionType,
        definition_id: &str,
    ) -> Result<Option<ResolvedDefinition>, flowable_app_engine::AppError> {
        match definition_type {
            DefinitionType::BpmnProcess => {
                let definition = match self
                    .engine
                    .get_repository_service()
                    .get_process_definition(definition_id)
                {
                    Ok(definition) => Some(definition),
                    Err(flowable_engine::error::FlowableError::NotFound(_)) => None,
                    Err(error) => {
                        return Err(flowable_app_engine::AppError::execution(error.to_string()));
                    }
                };
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name.unwrap_or_else(|| "Process".to_string()),
                    deployment_id: definition
                        .deployment_id
                        .unwrap_or_else(|| "process-deployment".to_string()),
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::DmnDecision => {
                let definition = self
                    .dmn_engine
                    .repository_service()
                    .create_decision_query()
                    .id(definition_id)
                    .single_result()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::CmmnCase => {
                let definition = self
                    .cmmn_engine
                    .repository_service()
                    .create_case_definition_query()
                    .id(definition_id)
                    .single_result()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::EventRegistry => {
                let definition = match self
                    .event_registry_service
                    .get_event_definition(definition_id)
                {
                    Ok(definition) => Some(definition),
                    Err(flowable_engine::error::FlowableError::NotFound(_)) => None,
                    Err(error) => {
                        return Err(flowable_app_engine::AppError::execution(error.to_string()));
                    }
                };
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
        }
    }
}

#[derive(Clone)]
struct AppApiAdapter {
    engine: Arc<AppEngine>,
}

impl AppApiAdapter {
    fn new(engine: Arc<AppEngine>) -> Self {
        Self { engine }
    }
}

impl routes::apps::AppRepositoryApi for AppApiAdapter {
    fn deploy_applications(
        &self,
        command: routes::apps::AppDeploymentCommand,
    ) -> Result<routes::apps::AppDeploymentRecord, crate::error::ApiError> {
        let routes::apps::AppDeploymentCommand {
            name,
            category,
            tenant_id,
            resources,
        } = command;
        let mut request = EngineAppDeploymentRequest::new(name);
        if let Some(category) = category {
            request = request.with_category(category);
        }
        if let Some(tenant_id) = tenant_id {
            request = request.with_tenant_id(tenant_id);
        }
        for resource in resources {
            let definition = parse_app_definition(
                std::str::from_utf8(&resource.resource)
                    .map_err(|error| crate::error::ApiError::bad_request(error.to_string()))?,
            )
            .map_err(|error| crate::error::ApiError::bad_request(error.to_string()))?;
            // Engine owns model conversion and durable composition resolution.
            let model = EngineAppModel::new()
                .with_app_definition(canonical_definition_to_engine(definition));
            request = request.with_resource_bytes(resource.resource_name, model, resource.resource);
        }
        let deployment = self.engine.deploy(request)?;
        Ok(routes::apps::AppDeploymentRecord {
            id: deployment.id,
            name: deployment.name,
            category: deployment.category,
            deployed_at: deployment.deployed_at.timestamp_millis(),
            resource_names: deployment.resource_names,
            tenant_id: deployment.tenant_id,
        })
    }

    fn list_app_deployments(
        &self,
        query: routes::apps::AppDeploymentQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::apps::AppDeploymentRecord>,
        crate::error::ApiError,
    > {
        let mut deployments = self
            .engine
            .repository_service()
            .create_deployment_query()
            .list()?
            .into_iter()
            .map(to_app_deployment_record)
            .filter(|deployment| {
                query
                    .id
                    .as_ref()
                    .is_none_or(|value| deployment.id == *value)
                    && query
                        .name
                        .as_ref()
                        .is_none_or(|value| deployment.name == *value)
                    && query
                        .name_like
                        .as_ref()
                        .is_none_or(|value| matches_flowable_like(&deployment.name, value))
                    && query
                        .category
                        .as_ref()
                        .is_none_or(|value| deployment.category.as_deref() == Some(value))
                    && query
                        .category_not_equals
                        .as_ref()
                        .is_none_or(|value| deployment.category.as_deref() != Some(value))
                    && query
                        .tenant_id
                        .as_ref()
                        .is_none_or(|value| deployment.tenant_id.as_deref() == Some(value))
                    && query.tenant_id_like.as_ref().is_none_or(|value| {
                        deployment
                            .tenant_id
                            .as_deref()
                            .is_some_and(|tenant_id| matches_flowable_like(tenant_id, value))
                    })
                    && (!query.without_tenant_id || deployment.tenant_id.is_none())
            })
            .collect::<Vec<_>>();

        sort_app_deployments(
            &mut deployments,
            query.sort.as_deref(),
            query.order.as_deref(),
        );
        Ok(query.paging.paginate(deployments))
    }

    fn get_app_deployment(
        &self,
        deployment_id: &str,
    ) -> Result<routes::apps::AppDeploymentRecord, crate::error::ApiError> {
        let deployment = self
            .engine
            .repository_service()
            .get_deployment(deployment_id)?;
        Ok(to_app_deployment_record(deployment))
    }

    fn delete_app_deployment(&self, deployment_id: &str) -> Result<(), crate::error::ApiError> {
        self.engine
            .repository_service()
            .delete_deployment(deployment_id)
            .map_err(Into::into)
    }

    fn list_app_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<routes::apps::AppDeploymentResourceRecord>, crate::error::ApiError> {
        Ok(self
            .engine
            .repository_service()
            .get_deployment_resources(deployment_id)?
            .into_iter()
            .map(|resource| routes::apps::AppDeploymentResourceRecord {
                deployment_id: resource.deployment_id,
                resource_name: resource.resource_name,
                resource_type: resource.resource_type,
                content_type: resource.content_type,
                bytes: resource.bytes,
            })
            .collect())
    }

    fn get_app_deployment_resource(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<routes::apps::AppDeploymentResourceRecord, crate::error::ApiError> {
        let resource = self
            .engine
            .repository_service()
            .get_deployment_resource(deployment_id, resource_name)?;
        Ok(routes::apps::AppDeploymentResourceRecord {
            deployment_id: resource.deployment_id,
            resource_name: resource.resource_name,
            resource_type: resource.resource_type,
            content_type: resource.content_type,
            bytes: resource.bytes,
        })
    }

    fn list_app_definitions(
        &self,
        query: routes::apps::AppDefinitionQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::apps::AppDefinitionRecord>,
        crate::error::ApiError,
    > {
        let mut definitions =
            self.engine
                .repository_service()
                .create_app_definition_query()
                .list()?
                .into_iter()
                .map(to_app_definition_record)
                .filter(|definition| {
                    query
                        .id
                        .as_ref()
                        .is_none_or(|value| definition.id == *value)
                        && query
                            .key
                            .as_ref()
                            .is_none_or(|value| definition.key == *value)
                        && query
                            .key_like
                            .as_ref()
                            .is_none_or(|value| matches_flowable_like(&definition.key, value))
                        && query
                            .name
                            .as_ref()
                            .is_none_or(|value| definition.name == *value)
                        && query
                            .name_like
                            .as_ref()
                            .is_none_or(|value| matches_flowable_like(&definition.name, value))
                        && query
                            .category
                            .as_ref()
                            .is_none_or(|value| definition.category.as_deref() == Some(value))
                        && query.category_like.as_ref().is_none_or(|value| {
                            definition
                                .category
                                .as_deref()
                                .is_some_and(|category| matches_flowable_like(category, value))
                        })
                        && query
                            .category_not_equals
                            .as_ref()
                            .is_none_or(|value| definition.category.as_deref() != Some(value))
                        && query
                            .deployment_id
                            .as_ref()
                            .is_none_or(|value| definition.deployment_id == *value)
                        && query
                            .tenant_id
                            .as_ref()
                            .is_none_or(|value| definition.tenant_id.as_deref() == Some(value))
                        && query.tenant_id_like.as_ref().is_none_or(|value| {
                            definition
                                .tenant_id
                                .as_deref()
                                .is_some_and(|tenant_id| matches_flowable_like(tenant_id, value))
                        })
                        && (!query.without_tenant_id || definition.tenant_id.is_none())
                        && query
                            .resource_name
                            .as_ref()
                            .is_none_or(|value| definition.resource_name == *value)
                        && query.resource_name_like.as_ref().is_none_or(|value| {
                            matches_flowable_like(&definition.resource_name, value)
                        })
                        && query
                            .version
                            .is_none_or(|value| definition.version == value)
                        && query
                            .version_greater_than
                            .is_none_or(|value| definition.version > value)
                        && query
                            .version_greater_than_or_equals
                            .is_none_or(|value| definition.version >= value)
                        && query
                            .version_lower_than
                            .is_none_or(|value| definition.version < value)
                        && query
                            .version_lower_than_or_equals
                            .is_none_or(|value| definition.version <= value)
                })
                .collect::<Vec<_>>();

        if query.latest {
            definitions = latest_app_definitions(definitions);
        }

        sort_app_definitions(
            &mut definitions,
            query.sort.as_deref(),
            query.order.as_deref(),
        );
        Ok(query.paging.paginate(definitions))
    }

    fn get_app_definition(
        &self,
        app_definition_id: &str,
    ) -> Result<routes::apps::AppDefinitionRecord, crate::error::ApiError> {
        Ok(to_app_definition_record(
            self.engine
                .deployment_manager()
                .get_app_definition(app_definition_id)?,
        ))
    }

    fn get_app_definition_resource_data(
        &self,
        app_definition_id: &str,
    ) -> Result<routes::apps::AppDeploymentResourceRecord, crate::error::ApiError> {
        let definition = self
            .engine
            .deployment_manager()
            .get_app_definition(app_definition_id)?;
        self.get_app_deployment_resource(&definition.deployment_id, &definition.resource_name)
    }

    fn get_app_definition_model(
        &self,
        app_definition_id: &str,
    ) -> Result<Value, crate::error::ApiError> {
        let entry = self
            .engine
            .deployment_manager()
            .resolve_app_definition(app_definition_id)?;
        serde_json::to_value(&entry.definition.model)
            .map_err(|error| crate::error::ApiError::InternalServerError(error.to_string()))
    }
}

impl routes::apps::AppRuntimeApi for AppApiAdapter {
    fn list_app_compositions(
        &self,
        query: routes::apps::AppCompositionQuery,
    ) -> Result<
        crate::common::PagedResponse<routes::apps::AppCompositionRecord>,
        crate::error::ApiError,
    > {
        let mut composition_query = self
            .engine
            .runtime_service()
            .create_resolved_composition_query();
        if let Some(app_definition_id) = query.app_definition_id {
            composition_query = composition_query.app_definition_id(app_definition_id);
        }
        if let Some(app_definition_key) = query.app_definition_key {
            composition_query = composition_query.app_definition_key(app_definition_key);
        }
        if let Some(tenant_id) = query.tenant_id {
            composition_query = composition_query.tenant_id(tenant_id);
        }
        if let Some(definition_type) = query.definition_type {
            composition_query =
                composition_query.definition_type(parse_app_definition_type(&definition_type)?);
        }
        let page = if let Some(size) = query.paging.size {
            composition_query
                .page(query.paging.start, size)
                .list_page()?
        } else {
            composition_query.list_page()?
        };
        Ok(crate::common::PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page
                .data
                .into_iter()
                .map(to_app_composition_record)
                .collect(),
            sort: None,
            order: None,
        })
    }

    fn get_app_composition(
        &self,
        app_definition_id: &str,
        filter: routes::apps::AppCompositionFilter,
    ) -> Result<routes::apps::AppCompositionRecord, crate::error::ApiError> {
        // Always resolve through the engine deployment manager so cold-cache and
        // restart paths rehydrate the durable composition snapshot.
        let mut composition = self
            .engine
            .deployment_manager()
            .get_resolved_composition(app_definition_id)?;
        if let Some(definition_type) = filter.definition_type {
            let definition_type = parse_app_definition_type(&definition_type)?;
            composition
                .references
                .retain(|reference| reference.definition_type == definition_type);
        }
        Ok(to_app_composition_record(composition))
    }
}

#[derive(Clone)]
struct RenderingApiAdapter {
    engine: Arc<ProcessEngine>,
    dmn_engine: Arc<DmnEngine>,
    cmmn_engine: Arc<CmmnEngine>,
    app_engine: Arc<AppEngine>,
}

impl RenderingApiAdapter {
    fn new(
        engine: Arc<ProcessEngine>,
        dmn_engine: Arc<DmnEngine>,
        cmmn_engine: Arc<CmmnEngine>,
        app_engine: Arc<AppEngine>,
    ) -> Self {
        Self {
            engine,
            dmn_engine,
            cmmn_engine,
            app_engine,
        }
    }
}

impl routes::rendering::RenderingApi for RenderingApiAdapter {
    fn render_process_definition_image(
        &self,
        process_definition_id: &str,
        request: routes::rendering::RenderingRequest,
    ) -> Result<String, crate::error::ApiError> {
        let repository_service = self.engine.get_repository_service();
        let definition = repository_service.get_process_definition(process_definition_id)?;
        let model = repository_service.get_bpmn_model(process_definition_id)?;
        if request.highlight_activity_ids.is_empty() && request.highlight_flow_ids.is_empty() {
            return Ok(generate_process_svg(
                &model,
                definition.name.as_deref().unwrap_or(&definition.key),
            )?);
        }
        let options = ProcessDiagramRenderOptions {
            highlight_activity_ids: request.highlight_activity_ids,
            highlight_flow_ids: request.highlight_flow_ids,
            ..ProcessDiagramRenderOptions::default()
        };
        Ok(DefaultProcessDiagramGenerator::with_options(options).generate_svg(&model)?)
    }

    fn render_process_instance_diagram(
        &self,
        process_instance_id: &str,
        request: routes::rendering::RenderingRequest,
    ) -> Result<String, crate::error::ApiError> {
        let instance = self
            .engine
            .get_runtime_store()
            .db_store()
            .find_by_id::<ProcessInstance>("process_instances", process_instance_id)
            .unwrap()
            .ok_or_else(|| {
                crate::error::ApiError::NotFound(format!(
                    "Process instance '{process_instance_id}' was not found"
                ))
            })?;
        self.render_process_definition_image(&instance.process_definition_id, request)
    }

    fn render_decision_table_image(
        &self,
        decision_table_id: &str,
        _request: routes::rendering::RenderingRequest,
    ) -> Result<String, crate::error::ApiError> {
        let definition = self
            .dmn_engine
            .repository_service()
            .get_decision(decision_table_id)?;
        Ok(DmnSvgGenerator::new().generate_engine_definition_svg(&definition)?)
    }

    fn render_case_definition_image(
        &self,
        case_definition_id: &str,
        _request: routes::rendering::RenderingRequest,
    ) -> Result<String, crate::error::ApiError> {
        let definition = self
            .cmmn_engine
            .repository_service()
            .get_case_definition(case_definition_id)?;
        Ok(CmmnSvgGenerator::new().generate_engine_case_definition_svg(&definition)?)
    }

    fn render_case_instance_diagram(
        &self,
        case_instance_id: &str,
        request: routes::rendering::RenderingRequest,
    ) -> Result<String, crate::error::ApiError> {
        let instance = self
            .cmmn_engine
            .runtime_service()
            .get_case_instance(case_instance_id)?;
        self.render_case_definition_image(&instance.case_definition_id, request)
    }

    fn render_app_definition_image(
        &self,
        app_definition_id: &str,
        _request: routes::rendering::RenderingRequest,
    ) -> Result<String, crate::error::ApiError> {
        let entry = self
            .app_engine
            .deployment_manager()
            .resolve_app_definition(app_definition_id)?;
        Ok(render_app_definition_svg(
            &entry.definition,
            &entry.composition,
        ))
    }
}

fn parse_app_definition_type(value: &str) -> Result<DefinitionType, crate::error::ApiError> {
    match value {
        "bpmnProcess" => Ok(DefinitionType::BpmnProcess),
        "dmnDecision" => Ok(DefinitionType::DmnDecision),
        "cmmnCase" => Ok(DefinitionType::CmmnCase),
        "eventRegistry" => Ok(DefinitionType::EventRegistry),
        other => Err(crate::error::ApiError::bad_request(format!(
            "Unsupported app composition definitionType '{other}'"
        ))),
    }
}

fn to_app_deployment_record(deployment: EngineAppDeployment) -> routes::apps::AppDeploymentRecord {
    routes::apps::AppDeploymentRecord {
        id: deployment.id,
        name: deployment.name,
        category: deployment.category,
        deployed_at: deployment.deployed_at.timestamp_millis(),
        resource_names: deployment.resource_names,
        tenant_id: deployment.tenant_id,
    }
}

fn to_app_definition_record(
    definition: EngineAppDefinitionRecord,
) -> routes::apps::AppDefinitionRecord {
    routes::apps::AppDefinitionRecord {
        id: definition.id,
        key: definition.key,
        name: definition.name,
        description: definition.model.description,
        category: definition.category.or(definition.model.category),
        version: definition.version,
        deployment_id: definition.deployment_id,
        resource_name: definition.resource_name,
        tenant_id: definition.tenant_id,
    }
}

fn matches_flowable_like(value: &str, pattern: &str) -> bool {
    if pattern == "%" {
        return true;
    }

    match (pattern.strip_prefix('%'), pattern.strip_suffix('%')) {
        (Some(_), Some(_)) if pattern.len() >= 2 => value.contains(&pattern[1..pattern.len() - 1]),
        (Some(_), Some(_)) => value == pattern,
        (Some(suffix), None) => value.ends_with(suffix),
        (None, Some(prefix)) => value.starts_with(prefix),
        (None, None) => value == pattern,
    }
}

fn latest_app_definitions(
    definitions: Vec<routes::apps::AppDefinitionRecord>,
) -> Vec<routes::apps::AppDefinitionRecord> {
    let mut latest = Vec::<routes::apps::AppDefinitionRecord>::new();
    for definition in definitions {
        if let Some(existing) = latest
            .iter_mut()
            .find(|existing| existing.key == definition.key)
        {
            if definition.version > existing.version {
                *existing = definition;
            }
        } else {
            latest.push(definition);
        }
    }
    latest
}

fn sort_app_deployments(
    deployments: &mut [routes::apps::AppDeploymentRecord],
    sort: Option<&str>,
    order: Option<&str>,
) {
    match sort.unwrap_or("id") {
        "name" => deployments.sort_by(|left, right| left.name.cmp(&right.name)),
        "deployTime" => deployments.sort_by_key(|left| left.deployed_at),
        "tenantId" => deployments.sort_by(|left, right| left.tenant_id.cmp(&right.tenant_id)),
        _ => deployments.sort_by(|left, right| left.id.cmp(&right.id)),
    }
    if order == Some("desc") {
        deployments.reverse();
    }
}

fn sort_app_definitions(
    definitions: &mut [routes::apps::AppDefinitionRecord],
    sort: Option<&str>,
    order: Option<&str>,
) {
    match sort.unwrap_or("name") {
        "id" => definitions.sort_by(|left, right| left.id.cmp(&right.id)),
        "key" => definitions.sort_by(|left, right| left.key.cmp(&right.key)),
        "category" => definitions.sort_by(|left, right| left.category.cmp(&right.category)),
        "version" => definitions.sort_by_key(|left| left.version),
        "deploymentId" => {
            definitions.sort_by(|left, right| left.deployment_id.cmp(&right.deployment_id))
        }
        "tenantId" => definitions.sort_by(|left, right| left.tenant_id.cmp(&right.tenant_id)),
        _ => definitions.sort_by(|left, right| left.name.cmp(&right.name)),
    }
    if order == Some("desc") {
        definitions.reverse();
    }
}

fn to_app_composition_record(
    composition: ResolvedAppComposition,
) -> routes::apps::AppCompositionRecord {
    routes::apps::AppCompositionRecord {
        app_definition_id: composition.app_definition_id,
        app_definition_key: composition.app_definition_key,
        app_definition_name: composition.app_definition_name,
        app_definition_version: composition.version,
        deployment_id: composition.deployment_id,
        tenant_id: composition.tenant_id,
        references: composition
            .references
            .into_iter()
            .map(|reference| routes::apps::AppResolvedReferenceRecord {
                page_id: reference.page_id,
                page_name: Some(reference.page_name),
                reference_id: reference.reference_id,
                reference_name: reference.reference_name,
                definition_type: match reference.definition_type {
                    DefinitionType::BpmnProcess => "bpmnProcess".to_string(),
                    DefinitionType::DmnDecision => "dmnDecision".to_string(),
                    DefinitionType::CmmnCase => "cmmnCase".to_string(),
                    DefinitionType::EventRegistry => "eventRegistry".to_string(),
                },
                resolved_definition_id: reference.resolved_definition_id,
                resolved_definition_key: reference.resolved_definition_key,
                resolved_definition_name: reference.resolved_definition_name,
                resolved_definition_version: reference.resolved_definition_version,
                resolved_tenant_id: reference.tenant_id,
            })
            .collect(),
    }
}

fn render_app_definition_svg(
    definition: &EngineAppDefinitionRecord,
    composition: &ResolvedAppComposition,
) -> String {
    let mut bpmn_references = 0usize;
    let mut dmn_references = 0usize;
    let mut cmmn_references = 0usize;
    let mut event_references = 0usize;
    for reference in &composition.references {
        match reference.definition_type {
            DefinitionType::BpmnProcess => bpmn_references += 1,
            DefinitionType::DmnDecision => dmn_references += 1,
            DefinitionType::CmmnCase => cmmn_references += 1,
            DefinitionType::EventRegistry => event_references += 1,
        }
    }

    let lines = vec![
        format!("Key: {}", definition.key),
        format!("Version: {}", definition.version),
        format!("Pages: {}", definition.model.pages.len()),
        format!("Resolved references: {}", composition.references.len()),
        format!(
            "Breakdown: BPMN {bpmn_references}, DMN {dmn_references}, CMMN {cmmn_references}, Events {event_references}"
        ),
        format!("Resource: {}", definition.resource_name),
    ];
    render_summary_svg("App Definition", &definition.name, &lines)
}

fn render_summary_svg(kind: &str, title: &str, lines: &[String]) -> String {
    let width = 960usize;
    let header_height = 96usize;
    let line_height = 28usize;
    let footer_padding = 36usize;
    let height = header_height + (lines.len() * line_height) + footer_padding;

    let mut body = String::new();
    for (index, line) in lines.iter().enumerate() {
        let y = header_height + 14 + (index * line_height);
        body.push_str(&format!(
            r##"<text x="48" y="{y}" fill="#203040" font-family="Arial, Helvetica, sans-serif" font-size="18">{}</text>"##,
            escape_xml(line)
        ));
    }

    format!(
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">"##,
            r##"<rect width="{width}" height="{height}" rx="20" fill="#f7fafc"/>"##,
            r##"<rect x="24" y="24" width="{header_width}" height="56" rx="14" fill="#0f4c81"/>"##,
            r##"<text x="48" y="58" fill="#ffffff" font-family="Arial, Helvetica, sans-serif" font-size="26" font-weight="700">{kind}</text>"##,
            r##"<text x="48" y="110" fill="#102a43" font-family="Arial, Helvetica, sans-serif" font-size="30" font-weight="700">{title}</text>"##,
            r##"{body}"##,
            r##"</svg>"##
        ),
        width = width,
        height = height,
        header_width = width - 48,
        kind = escape_xml(kind),
        title = escape_xml(title),
        body = body,
    )
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub async fn run_server(
    engine: Arc<ProcessEngine>,
    listener: TcpListener,
) -> Result<(), Box<dyn std::error::Error>> {
    // Test/library helper: without_identity_seed + treat user id "admin" as REST admin
    // so existing integration tests that save_user(admin) keep write access to
    // privileged paths. Production entrypoints use run_platform_server / from_env.
    let config = config::RestConfig::default()
        .without_identity_seed()
        .with_test_admin_user();
    run_server_with_config(engine, listener, config).await
}

pub async fn run_server_with_config(
    engine: Arc<ProcessEngine>,
    listener: TcpListener,
    mut config: config::RestConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_seed_user_is_admin(&mut config);
    config.validate_for_startup()?;
    config.apply_identity_seed(engine.as_ref())?;

    let dmn_engine = match engine.get_config().dmn_engine.clone() {
        Some(engine) => engine,
        None => Arc::new(
            DmnEngine::new_in_memory()
                .map_err(|error| format!("failed to create fallback DMN engine: {error}"))?,
        ),
    };
    let cmmn_engine = match engine.get_config().cmmn_engine.clone() {
        Some(engine) => engine,
        None => Arc::new(
            CmmnEngine::new_in_memory()
                .map_err(|error| format!("failed to create fallback CMMN engine: {error}"))?,
        ),
    };
    let app_catalog = Arc::new(AppDefinitionCatalogAdapter {
        engine: Arc::clone(&engine),
        dmn_engine: Arc::clone(&dmn_engine),
        cmmn_engine: Arc::clone(&cmmn_engine),
        event_registry_service: FlowableEventRegistryService::new(Arc::clone(&engine)),
    });
    let app_engine = Arc::new(
        AppEngine::new_in_memory_with_catalog(app_catalog)
            .map_err(|error| format!("failed to create app engine: {error}"))?,
    );
    let directory_read_state = DirectoryReadState::internal();
    run_server_with_components(
        ServerComponents {
            engine,
            dmn_engine,
            cmmn_engine,
            app_engine,
            directory_read_state,
            management_state: None,
        },
        listener,
        config,
    )
    .await
}

pub async fn run_platform_server(
    platform: FlowablePlatform,
    listener: TcpListener,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = config::RestConfig::from_platform_configuration(platform.config());
    run_platform_server_with_config(platform, listener, config).await
}

pub async fn run_platform_server_with_config(
    platform: FlowablePlatform,
    listener: TcpListener,
    mut config: config::RestConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_seed_user_is_admin(&mut config);
    let directory_read_state = DirectoryReadState::from_platform(&platform);
    let management_state = Arc::new(routes::management::ManagementApiState {
        runtime_embedding_contract: platform.runtime_embedding_contract().clone(),
        enterprise_support_contracts: platform.enterprise_adapter_support_contracts().to_vec(),
        enterprise_support_statement: platform.enterprise_support_statement(),
        directory_support_contract: platform.directory_support_contract().clone(),
        operations_support_contract: platform.operations_support_contract().clone(),
        topology_certification_contract: platform.topology_certification_contract().clone(),
    });
    run_server_with_components(
        ServerComponents {
            engine: platform.process_engine(),
            dmn_engine: platform.dmn_engine(),
            cmmn_engine: platform.cmmn_engine(),
            app_engine: platform.app_engine(),
            directory_read_state,
            management_state: Some(management_state),
        },
        listener,
        config,
    )
    .await
}

fn ensure_seed_user_is_admin(config: &mut config::RestConfig) {
    if config.security.admin_seed.enabled
        && !config
            .security
            .auth
            .admin_users
            .iter()
            .any(|u| u == &config.security.admin_seed.user_id)
    {
        config
            .security
            .auth
            .admin_users
            .push(config.security.admin_seed.user_id.clone());
    }
}

struct ServerComponents {
    engine: Arc<ProcessEngine>,
    dmn_engine: Arc<DmnEngine>,
    cmmn_engine: Arc<CmmnEngine>,
    app_engine: Arc<AppEngine>,
    directory_read_state: Arc<DirectoryReadState>,
    management_state: Option<Arc<routes::management::ManagementApiState>>,
}

async fn run_server_with_components(
    components: ServerComponents,
    listener: TcpListener,
    config: config::RestConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    config.validate_for_startup()?;

    let ServerComponents {
        engine,
        dmn_engine,
        cmmn_engine,
        app_engine,
        directory_read_state,
        management_state,
    } = components;

    let form_service = FlowableFormService::new(Arc::clone(&engine));
    let form_repository: routes::forms::DynFormRepository = Arc::new(FormRepositoryAdapter {
        service: form_service.clone(),
        engine: Arc::clone(&engine),
    });
    let content_service: routes::content::DynContentService = Arc::new(ContentServiceAdapter {
        service: FlowableContentService::new(Arc::clone(&engine)),
        engine: Arc::clone(&engine),
    });
    let dmn_api = Arc::new(DmnApiAdapter::new(Arc::clone(&dmn_engine)));
    let dmn_repository: routes::dmn::DynDmnRepository = dmn_api.clone();
    let dmn_runtime: routes::dmn::DynDmnRuntime = dmn_api.clone();
    let dmn_history: routes::dmn::DynDmnHistory = dmn_api;
    let cmmn_api = Arc::new(CmmnApiAdapter::new(
        Arc::clone(&cmmn_engine),
        Arc::clone(&dmn_engine),
        form_service,
        // P114: the CMMN engine has no identity store, so candidateUser /
        // candidateOrAssigned group expansion is backed by the ProcessEngine
        // identity service (Java TaskQueryImpl.getGroupsForCandidateUser).
        Some(std::sync::Arc::new({
            let identity_engine = Arc::clone(&engine);
            move |user_id: &str| {
                identity_engine
                    .get_identity_service()
                    .get_groups_by_user(user_id)
                    .into_iter()
                    .map(|group| group.id)
                    .collect()
            }
        })),
    ));
    let cmmn_repository: routes::cmmn::DynCmmnRepository = cmmn_api.clone();
    let cmmn_runtime: routes::cmmn::DynCmmnRuntime = cmmn_api.clone();
    let cmmn_history: routes::cmmn::DynCmmnHistory = cmmn_api.clone();
    let cmmn_management: routes::cmmn::DynCmmnManagement = cmmn_api;
    let app_api = Arc::new(AppApiAdapter::new(Arc::clone(&app_engine)));
    let app_repository: routes::apps::DynAppRepository = app_api.clone();
    let app_runtime: routes::apps::DynAppRuntime = app_api;
    let rendering_api: routes::rendering::DynRenderingApi = Arc::new(RenderingApiAdapter::new(
        Arc::clone(&engine),
        Arc::clone(&dmn_engine),
        Arc::clone(&cmmn_engine),
        Arc::clone(&app_engine),
    ));

    let management_routes = management_state
        .clone()
        .map(routes::management::router)
        .unwrap_or_default();

    let api_routes = Router::new()
        .merge(routes::apps::router(app_repository, app_runtime))
        .merge(routes::rendering::router(rendering_api))
        .merge(routes::management::engine_router())
        .merge(management_routes)
        .merge(routes::forms::router(form_repository))
        .merge(routes::content::router(content_service.clone()))
        .merge(routes::cmmn::router_with_management(
            cmmn_repository,
            cmmn_runtime,
            cmmn_history,
            cmmn_management,
        ))
        .merge(routes::dmn::router(
            dmn_repository,
            dmn_runtime,
            dmn_history,
        ))
        .merge(routes::history::router())
        .merge(routes::external_worker::router())
        .route(
            "/event-registry-repository/deployments",
            get(routes::event_registry::list_deployments).post(routes::event_registry::deploy),
        )
        .route(
            "/event-registry-repository/deployments/:deployment_id",
            get(routes::event_registry::get_deployment)
                .delete(routes::event_registry::delete_deployment),
        )
        .route(
            "/event-registry-repository/deployments/:deployment_id/resources",
            get(routes::event_registry::list_deployment_resources),
        )
        .route(
            "/event-registry-repository/deployments/:deployment_id/resourcedata/*resource_name",
            get(routes::event_registry::get_deployment_resource_data),
        )
        .route(
            "/event-registry-repository/deployments/:deployment_id/resources/*resource_name",
            get(routes::event_registry::get_deployment_resource),
        )
        .route(
            "/event-registry-repository/channel-definitions",
            get(routes::event_registry::list_channel_definitions),
        )
        .route(
            "/event-registry-repository/channel-definitions/:channel_definition_id",
            get(routes::event_registry::get_channel_definition)
                .put(routes::event_registry::update_channel_definition),
        )
        .route(
            "/event-registry-repository/channel-definitions/:channel_definition_id/model",
            get(routes::event_registry::get_channel_definition_model),
        )
        .route(
            "/event-registry-repository/channel-definitions/:channel_definition_id/resourcedata",
            get(routes::event_registry::get_channel_definition_resource_data),
        )
        .route(
            "/event-registry-repository/event-definitions",
            get(routes::event_registry::list_event_definitions),
        )
        .route(
            "/event-registry-repository/event-definitions/:event_definition_id",
            get(routes::event_registry::get_event_definition)
                .put(routes::event_registry::update_event_definition),
        )
        .route(
            "/event-registry-repository/event-definitions/:event_definition_id/model",
            get(routes::event_registry::get_event_definition_model),
        )
        .route(
            "/event-registry-repository/event-definitions/:event_definition_id/resourcedata",
            get(routes::event_registry::get_event_definition_resource_data),
        )
        .route(
            "/event-registry-runtime/event-instances",
            post(routes::event_registry::publish_outbound_event),
        )
        .route(
            "/event-registry-runtime/inbound-event-instances",
            post(routes::event_registry::receive_inbound_event),
        )
        .route(
            "/event-registry-management/event-instance-deliveries",
            get(routes::event_registry::list_event_deliveries),
        )
        .route(
            "/event-registry-management/event-instance-deliveries/:delivery_id",
            get(routes::event_registry::get_event_delivery),
        )
        .route(
            "/event-registry-management/engine",
            get(routes::event_registry::get_engine_info),
        )
        .route(
            "/event-registry-management/event-deliveries/:delivery_id/retry",
            post(routes::event_registry::retry_event_delivery),
        )
        .route(
            "/event-registry-management/event-deliveries/:delivery_id",
            delete(routes::event_registry::delete_event_delivery),
        )
        .merge(routes::deployments::router())
        .merge(routes::models::router())
        .merge(routes::process_definitions::router())
        .merge(routes::process_instances::router(content_service.clone()))
        .merge(routes::adhoc::router())
        .merge(routes::signals::router())
        .merge(routes::messages::router())
        .merge(routes::tasks::router(content_service))
        .merge(routes::task_variables::router())
        .merge(routes::identity_links::router())
        .merge(routes::entity_links::router())
        .merge(routes::event_subscriptions::router())
        .merge(routes::batches::router())
        .merge(routes::idm::router())
        // /metrics previously sat outside auth; move under the authenticated API surface.
        .route("/metrics", get(routes::metrics::metrics));

    let api_routes = if config.security.auth.mode.is_enforced() {
        api_routes.layer(middleware::from_fn_with_state(
            Arc::new(security::RestSecurityState::from_auth_config(
                config.security.auth.clone(),
            )),
            security::auth_middleware,
        ))
    } else {
        api_routes
    };

    // Layers applied bottom-up on the request path: SetRequestId first, then
    // TraceLayer reads x-request-id into the span, then Propagate echoes it.
    let request_id_header = HeaderName::from_static("x-request-id");
    let app = Router::new()
        .route("/health", get(routes::health::health))
        .route("/ready", get(routes::health::ready))
        // Merged outside `api_routes` on purpose: the UI surface authenticates
        // with its own remember-me cookie scheme and must not sit behind the
        // engine API's Basic-auth middleware. It still lands inside the
        // `Extension(engine)` layer below, which its handlers require.
        //
        // It must also be merged *before* `api_routes`, and the order is load
        // bearing. `Router::layer` wraps a router's fallback as well as its
        // routes, so the Basic-auth layer on `api_routes` is what answers an
        // unknown URL with 401 rather than a bare 404. `Router::merge` takes the
        // fallback of whichever router is merged later, so merging a router with
        // no fallback of its own after `api_routes` would silently discard that
        // layered fallback and turn every unknown URL into a 404 — including the
        // deprecated `/service/**` prefix that
        // `rest_jmx_native_contract_test` pins at 401.
        // `ui_surface_wiring_test::unknown_paths_stay_behind_the_api_auth_layer`
        // guards this.
        // Stream B fills `admin`/`task` modules inside `ui_router()`.
        .merge(flowable_ui_rest::ui_router())
        .merge(api_routes)
        .layer(Extension(directory_read_state))
        .layer(Extension(dmn_engine))
        .layer(Extension(engine))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                let request_id = request
                    .headers()
                    .get("x-request-id")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("-");
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    uri = %request.uri(),
                    request_id = %request_id,
                )
            }),
        )
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(SetRequestIdLayer::new(
            request_id_header,
            MakeRequestUuid,
        ));

    // ConnectInfo is required so auth_middleware can key its per-IP failure
    // window on the real peer address (M2 brute-force lockout).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

// Primary REST surface for callers that need an owned handle around
// the engine and config (preferred over the raw router-only API).
pub struct FlowableRestApi {
    pub engine: Arc<ProcessEngine>,
    pub config: config::RestConfig,
}

impl FlowableRestApi {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        Self::new_with_config(engine, config::RestConfig::default())
    }

    pub fn new_with_config(engine: Arc<ProcessEngine>, config: config::RestConfig) -> Self {
        Self { engine, config }
    }

    pub fn authenticate_basic(&self, user: &str, pass: &str) -> Result<(), String> {
        if !self.config.security.auth.mode.is_enforced() {
            return Ok(());
        }

        if self
            .engine
            .get_identity_service()
            .check_password(user, pass)
        {
            Ok(())
        } else {
            Err("Unauthorized".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use flowable_cmmn_engine::{CmmnJob, CmmnJobFamily};
    use serde_json::Value;

    async fn spawn_server_with_cmmn_engine(
        test_name: &str,
        cmmn_engine: Arc<CmmnEngine>,
    ) -> (String, reqwest::Client) {
        let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
        engine
            .get_identity_service()
            .save_user(flowable_engine::identity::entities::User {
                id: "admin".to_string(),
                first_name: None,
                last_name: None,
                email: None,
                password: Some("test".to_string()),
                tenant_id: None,
            });

        let dmn_engine = Arc::new(DmnEngine::new_in_memory().expect("dmn engine"));
        let app_catalog = Arc::new(AppDefinitionCatalogAdapter {
            engine: Arc::clone(&engine),
            dmn_engine: Arc::clone(&dmn_engine),
            cmmn_engine: Arc::clone(&cmmn_engine),
            event_registry_service: FlowableEventRegistryService::new(Arc::clone(&engine)),
        });
        let app_engine =
            Arc::new(AppEngine::new_in_memory_with_catalog(app_catalog).expect("app engine"));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());

        tokio::spawn(async move {
            run_server_with_components(
                ServerComponents {
                    engine,
                    dmn_engine,
                    cmmn_engine,
                    app_engine,
                    directory_read_state: DirectoryReadState::internal(),
                    management_state: None,
                },
                listener,
                config::RestConfig::default()
                    .without_identity_seed()
                    .with_test_admin_user(),
            )
            .await
            .unwrap();
        });

        (base_url, reqwest::Client::new())
    }

    #[test]
    fn app_catalog_event_registry_resolution_honors_tenant_latest_and_default_fallback() {
        use flowable_app_engine::TenantResolutionPolicy;
        use flowable_engine::persistence::runtime_store::EventRegistryEventDefinition;

        let engine = Arc::new(ProcessEngine::new(
            "rest-app-event-catalog-tenant".to_string(),
        ));
        let event_registry_service = FlowableEventRegistryService::new(Arc::clone(&engine));
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        for (id, deployment_id, version, tenant_id, name) in [
            (
                "event-default-v2",
                "deployment-default",
                2,
                None,
                "Default v2",
            ),
            (
                "event-tenant-a-v1",
                "deployment-tenant-a",
                1,
                Some("tenant-a"),
                "Tenant A v1",
            ),
            (
                "event-tenant-b-v1",
                "deployment-tenant-b-v1",
                1,
                Some("tenant-b"),
                "Tenant B v1",
            ),
            (
                "event-tenant-b-v3",
                "deployment-tenant-b-v3",
                3,
                Some("tenant-b"),
                "Tenant B v3",
            ),
        ] {
            store.insert_event_registry_event_definition(
                EventRegistryEventDefinition {
                    id: id.to_string(),
                    deployment_id: deployment_id.to_string(),
                    key: "sharedOrderEvent".to_string(),
                    name: name.to_string(),
                    description: None,
                    category: None,
                    event_type: "shared.order".to_string(),
                    channel_key: "orders".to_string(),
                    resource_name: format!("{id}.event"),
                    version,
                    tenant_id: tenant_id.map(str::to_string),
                    parent_deployment_id: None,
                    payload: serde_json::json!([]),
                },
                &mut session,
            );
        }
        session.flush_and_commit().unwrap();

        let catalog = AppDefinitionCatalogAdapter {
            engine,
            dmn_engine: Arc::new(DmnEngine::new_in_memory().unwrap()),
            cmmn_engine: Arc::new(CmmnEngine::new_in_memory().unwrap()),
            event_registry_service,
        };

        let tenant_b = catalog
            .resolve_definition(
                DefinitionType::EventRegistry,
                "sharedOrderEvent",
                Some("tenant-b"),
            )
            .unwrap()
            .expect("tenant-b definition");
        assert_eq!(tenant_b.definition_id, "event-tenant-b-v3");
        assert_eq!(tenant_b.version, 3);
        assert_eq!(tenant_b.tenant_id.as_deref(), Some("tenant-b"));
        assert_eq!(tenant_b.deployment_id, "deployment-tenant-b-v3");

        let fallback = catalog
            .resolve_definition(
                DefinitionType::EventRegistry,
                "sharedOrderEvent",
                Some("tenant-c"),
            )
            .unwrap()
            .expect("tenantless default definition");
        assert_eq!(fallback.definition_id, "event-default-v2");
        assert_eq!(fallback.version, 2);
        assert_eq!(fallback.tenant_id, None);

        let strict = catalog
            .resolve_definition_with_policy(
                DefinitionType::EventRegistry,
                "sharedOrderEvent",
                Some("tenant-c"),
                TenantResolutionPolicy::Strict,
            )
            .unwrap();
        assert!(
            strict.is_none(),
            "strict tenant resolution must not use the default"
        );
    }

    #[tokio::test]
    async fn cmmn_management_job_paths_return_empty_lists_and_missing_404() {
        let cmmn_engine = Arc::new(CmmnEngine::new_in_memory().expect("cmmn engine"));
        let (base_url, client) =
            spawn_server_with_cmmn_engine("rest-cmmn-management-empty", cmmn_engine).await;

        for path in [
            "/cmmn-management/jobs",
            "/cmmn-management/timer-jobs",
            "/cmmn-management/deadletter-jobs",
            "/cmmn-management/history-jobs",
            "/cmmn-management/suspended-jobs",
        ] {
            let response = client
                .get(format!("{base_url}{path}?start=0&size=10"))
                .basic_auth("admin", Some("test"))
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success(), "{path}");
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["start"], 0, "{path}");
            assert_eq!(body["size"], 0, "{path}");
            assert_eq!(body["total"], 0, "{path}");
            assert_eq!(body["data"].as_array().unwrap().len(), 0, "{path}");
        }

        for path in [
            "/cmmn-management/jobs/missing-job",
            "/cmmn-management/timer-jobs/missing-job",
            "/cmmn-management/deadletter-jobs/missing-job",
            "/cmmn-management/history-jobs/missing-job",
            "/cmmn-management/suspended-jobs/missing-job",
            "/cmmn-management/suspended-jobs/missing-job/exception-stacktrace",
        ] {
            let response = client
                .get(format!("{base_url}{path}"))
                .basic_auth("admin", Some("test"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND, "{path}");
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["code"], "NOT_FOUND", "{path}");
        }
    }

    #[tokio::test]
    async fn cmmn_management_job_paths_return_real_persisted_jobs() {
        let cmmn_engine = Arc::new(CmmnEngine::new_in_memory().expect("cmmn engine"));
        let created_at = Utc.with_ymd_and_hms(2026, 4, 28, 9, 30, 0).unwrap();
        let due_date = Utc.with_ymd_and_hms(2026, 4, 28, 10, 0, 0).unwrap();
        let mut timer_job = CmmnJob::new("cmmn-timer-job-1", CmmnJobFamily::Timer);
        timer_job.scope_id = Some("case-instance-1".to_string());
        timer_job.sub_scope_id = Some("plan-item-1".to_string());
        timer_job.scope_definition_id = Some("case-definition-1".to_string());
        timer_job.element_id = Some("timer-plan-item".to_string());
        timer_job.tenant_id = Some("tenant-a".to_string());
        timer_job.created_at = created_at;
        timer_job.due_date = Some(due_date);
        timer_job.retries = 3;
        cmmn_engine
            .management_service()
            .insert_job(timer_job)
            .expect("insert timer job");

        let mut deadletter_job = CmmnJob::new("cmmn-deadletter-job-1", CmmnJobFamily::Deadletter);
        deadletter_job.scope_id = Some("case-instance-2".to_string());
        deadletter_job.element_id = Some("failed-plan-item".to_string());
        deadletter_job.created_at = created_at;
        deadletter_job.retries = 0;
        deadletter_job.exception_message = Some("CMMN worker failed".to_string());
        deadletter_job.exception_stacktrace = Some("cmmn stacktrace line 1\nline 2".to_string());
        cmmn_engine
            .management_service()
            .insert_job(deadletter_job)
            .expect("insert deadletter job");

        let (base_url, client) =
            spawn_server_with_cmmn_engine("rest-cmmn-management-persisted", cmmn_engine).await;

        let timer_jobs = client
            .get(format!(
                "{base_url}/cmmn-management/timer-jobs?start=0&size=10"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(timer_jobs.status().is_success());
        let timer_body: Value = timer_jobs.json().await.unwrap();
        assert_eq!(timer_body["total"], 1);
        assert_eq!(timer_body["data"][0]["id"], "cmmn-timer-job-1");
        assert_eq!(timer_body["data"][0]["jobType"], "timer");
        assert_eq!(timer_body["data"][0]["scopeType"], "cmmn");
        assert_eq!(timer_body["data"][0]["scopeId"], "case-instance-1");
        assert_eq!(timer_body["data"][0]["subScopeId"], "plan-item-1");
        assert_eq!(
            timer_body["data"][0]["scopeDefinitionId"],
            "case-definition-1"
        );
        assert_eq!(timer_body["data"][0]["elementId"], "timer-plan-item");
        assert_eq!(timer_body["data"][0]["tenantId"], "tenant-a");
        assert_eq!(timer_body["data"][0]["retries"], 3);
        assert_eq!(timer_body["data"][0]["createTime"], created_at.to_rfc3339());
        assert_eq!(timer_body["data"][0]["dueDate"], due_date.to_rfc3339());

        let timer_job = client
            .get(format!(
                "{base_url}/cmmn-management/timer-jobs/cmmn-timer-job-1"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(timer_job.status().is_success());
        assert_eq!(
            timer_job.json::<Value>().await.unwrap()["id"],
            "cmmn-timer-job-1"
        );

        let deadletter_jobs = client
            .get(format!(
                "{base_url}/cmmn-management/deadletter-jobs?start=0&size=10"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(deadletter_jobs.status().is_success());
        let deadletter_body: Value = deadletter_jobs.json().await.unwrap();
        assert_eq!(deadletter_body["total"], 1);
        assert_eq!(deadletter_body["data"][0]["id"], "cmmn-deadletter-job-1");
        assert_eq!(deadletter_body["data"][0]["jobType"], "deadletter");
        assert_eq!(
            deadletter_body["data"][0]["exceptionMessage"],
            "CMMN worker failed"
        );

        let stacktrace = client
            .get(format!(
                "{base_url}/cmmn-management/deadletter-jobs/cmmn-deadletter-job-1/exception-stacktrace"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(stacktrace.status().is_success());
        assert_eq!(
            stacktrace
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "text/plain"
        );
        assert_eq!(
            stacktrace.text().await.unwrap(),
            "cmmn stacktrace line 1\nline 2"
        );
    }
}
