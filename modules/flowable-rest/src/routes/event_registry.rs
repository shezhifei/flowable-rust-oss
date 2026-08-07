use crate::common::{PagedResponse, parse_query};
use crate::error::ApiError;
use axum::{
    Extension, Json,
    extract::Path,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_event_registry_service::{
    ChannelDefinitionUpdateRequest, EventDefinitionUpdateRequest, EventDirection,
    EventInstanceDelivery, EventInstanceRequest, EventInstanceStatus, EventRegistryDeployment,
    EventRegistryDeploymentRequest as ServiceDeploymentRequest,
    EventRegistryDeploymentResource as ServiceDeploymentResource, EventRegistryEngineInfo,
    FlowableEventRegistryService, InboundEventRequest, InboundRawEvent, OutboundEventRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventRegistryDeploymentRequest {
    pub name: String,
    pub category: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub resources: Vec<EventRegistryDeploymentResource>,
    pub resource_name: Option<String>,
    pub resource: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventRegistryDeploymentResource {
    pub resource_name: String,
    pub resource: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DeploymentQueryParams {
    start: usize,
    size: Option<usize>,
    name: Option<String>,
    #[serde(rename = "nameLike")]
    name_like: Option<String>,
    category: Option<String>,
    #[serde(rename = "categoryNotEquals")]
    category_not_equals: Option<String>,
    #[serde(rename = "parentDeploymentId")]
    parent_deployment_id: Option<String>,
    #[serde(rename = "parentDeploymentIdLike")]
    parent_deployment_id_like: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    #[serde(rename = "withoutTenantId")]
    without_tenant_id: Option<bool>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ChannelDefinitionQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    key: Option<String>,
    #[serde(rename = "keyLike")]
    key_like: Option<String>,
    #[serde(rename = "keyLikeIgnoreCase")]
    key_like_ignore_case: Option<String>,
    name: Option<String>,
    #[serde(rename = "nameLike")]
    name_like: Option<String>,
    #[serde(rename = "nameLikeIgnoreCase")]
    name_like_ignore_case: Option<String>,
    category: Option<String>,
    #[serde(rename = "categoryLike")]
    category_like: Option<String>,
    #[serde(rename = "categoryNotEquals")]
    category_not_equals: Option<String>,
    #[serde(rename = "deploymentId")]
    deployment_id: Option<String>,
    #[serde(rename = "parentDeploymentId")]
    parent_deployment_id: Option<String>,
    #[serde(rename = "channelType")]
    channel_type: Option<String>,
    #[serde(rename = "resourceName")]
    resource_name: Option<String>,
    #[serde(rename = "resourceNameLike")]
    resource_name_like: Option<String>,
    #[serde(rename = "onlyInbound")]
    only_inbound: Option<bool>,
    #[serde(rename = "onlyOutbound")]
    only_outbound: Option<bool>,
    implementation: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    version: Option<i32>,
    latest: Option<bool>,
    /// P133: ChannelDefinitionCollectionResource.java:126-133
    #[serde(rename = "createTime")]
    create_time: Option<String>,
    #[serde(rename = "createTimeAfter")]
    create_time_after: Option<String>,
    #[serde(rename = "createTimeBefore")]
    create_time_before: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EventDefinitionQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    key: Option<String>,
    #[serde(rename = "keyLike")]
    key_like: Option<String>,
    #[serde(rename = "keyLikeIgnoreCase")]
    key_like_ignore_case: Option<String>,
    name: Option<String>,
    #[serde(rename = "nameLike")]
    name_like: Option<String>,
    #[serde(rename = "nameLikeIgnoreCase")]
    name_like_ignore_case: Option<String>,
    category: Option<String>,
    #[serde(rename = "categoryLike")]
    category_like: Option<String>,
    #[serde(rename = "categoryNotEquals")]
    category_not_equals: Option<String>,
    #[serde(rename = "deploymentId")]
    deployment_id: Option<String>,
    #[serde(rename = "parentDeploymentId")]
    parent_deployment_id: Option<String>,
    #[serde(rename = "eventType")]
    event_type: Option<String>,
    #[serde(rename = "channelKey")]
    channel_key: Option<String>,
    #[serde(rename = "resourceName")]
    resource_name: Option<String>,
    #[serde(rename = "resourceNameLike")]
    resource_name_like: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    version: Option<i32>,
    latest: Option<bool>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DeliveryQueryParams {
    start: usize,
    size: Option<usize>,
    direction: Option<String>,
    status: Option<String>,
    #[serde(rename = "eventType")]
    event_type: Option<String>,
    #[serde(rename = "channelKey")]
    channel_key: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    #[serde(rename = "withoutTenantId")]
    without_tenant_id: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentResponse {
    id: String,
    name: String,
    deployed_at: i64,
    category: Option<String>,
    parent_deployment_id: Option<String>,
    tenant_id: Option<String>,
    resource_names: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentResourceResponse {
    id: String,
    url: String,
    content_url: String,
    media_type: String,
    #[serde(rename = "type")]
    resource_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDefinitionResponse {
    id: String,
    url: String,
    deployment_id: String,
    deployment_url: String,
    key: String,
    name: String,
    description: Option<String>,
    category: Option<String>,
    channel_type: String,
    #[serde(rename = "type")]
    channel_kind: String,
    implementation: Option<String>,
    resource_name: String,
    resource: String,
    version: i32,
    tenant_id: Option<String>,
    parent_deployment_id: Option<String>,
    configuration: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDefinitionResponse {
    id: String,
    url: String,
    deployment_id: String,
    deployment_url: String,
    key: String,
    name: String,
    description: Option<String>,
    category: Option<String>,
    event_type: String,
    channel_key: String,
    resource_name: String,
    resource: String,
    version: i32,
    tenant_id: Option<String>,
    parent_deployment_id: Option<String>,
    payload: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventInstanceDeliveryResponse {
    id: String,
    event_definition_id: String,
    event_definition_key: String,
    event_type: String,
    channel_key: String,
    direction: EventDirection,
    status: EventInstanceStatus,
    status_history: Vec<EventInstanceStatus>,
    last_error: Option<String>,
    retry_count: u32,
    last_retry_at: Option<i64>,
    last_failure_at: Option<i64>,
    next_retry_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dispatch_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tenant_id: Option<String>,
    payload: Value,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InboundEventRequestBody {
    /// Compatibility path: event-type based receive.
    #[serde(default)]
    event_type: Option<String>,
    /// Channel pipeline path: process by channel key through ADR-6 stages.
    #[serde(default)]
    channel_key: Option<String>,
    event_payload: Value,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventInstanceRequestBody {
    event_definition_id: Option<String>,
    event_definition_key: Option<String>,
    channel_definition_id: Option<String>,
    channel_definition_key: Option<String>,
    event_payload: Value,
    tenant_id: Option<String>,
}

impl EventRegistryDeploymentRequest {
    fn into_service_request(self) -> Result<ServiceDeploymentRequest, ApiError> {
        let resources = if self.resources.is_empty() {
            match (self.resource_name, self.resource) {
                (Some(resource_name), Some(resource)) => {
                    vec![ServiceDeploymentResource {
                        resource_name,
                        resource,
                    }]
                }
                _ => {
                    return Err(ApiError::bad_request(
                        "Event Registry deployment requires at least one resource",
                    ));
                }
            }
        } else {
            self.resources
                .into_iter()
                .map(|resource| ServiceDeploymentResource {
                    resource_name: resource.resource_name,
                    resource: resource.resource,
                })
                .collect()
        };

        Ok(ServiceDeploymentRequest {
            name: self.name,
            category: self.category,
            parent_deployment_id: self.parent_deployment_id,
            tenant_id: self.tenant_id,
            resources,
        })
    }
}

impl From<EventRegistryDeployment> for DeploymentResponse {
    fn from(value: EventRegistryDeployment) -> Self {
        Self {
            id: value.id,
            name: value.name,
            deployed_at: value.deployed_at,
            category: value.category,
            parent_deployment_id: value.parent_deployment_id,
            tenant_id: value.tenant_id,
            resource_names: value.resource_names,
        }
    }
}

impl From<flowable_event_registry_service::ChannelDefinition> for ChannelDefinitionResponse {
    fn from(value: flowable_event_registry_service::ChannelDefinition) -> Self {
        let url = format!(
            "/event-registry-repository/channel-definitions/{}",
            value.id
        );
        let deployment_url = format!(
            "/event-registry-repository/deployments/{}",
            value.deployment_id
        );
        let resource = format!(
            "/event-registry-repository/deployments/{}/resources/{}",
            value.deployment_id, value.resource_name
        );
        let implementation = value
            .configuration
            .get("type")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned);
        Self {
            id: value.id,
            url,
            deployment_id: value.deployment_id,
            deployment_url,
            key: value.key,
            name: value.name,
            description: value.description,
            category: value.category,
            channel_type: value.channel_type.clone(),
            channel_kind: value.channel_type,
            implementation,
            resource_name: value.resource_name,
            resource,
            version: value.version,
            tenant_id: value.tenant_id,
            parent_deployment_id: value.parent_deployment_id,
            configuration: value.configuration,
        }
    }
}

impl From<flowable_event_registry_service::EventDefinition> for EventDefinitionResponse {
    fn from(value: flowable_event_registry_service::EventDefinition) -> Self {
        let url = format!("/event-registry-repository/event-definitions/{}", value.id);
        let deployment_url = format!(
            "/event-registry-repository/deployments/{}",
            value.deployment_id
        );
        let resource = format!(
            "/event-registry-repository/deployments/{}/resources/{}",
            value.deployment_id, value.resource_name
        );
        Self {
            id: value.id,
            url,
            deployment_id: value.deployment_id,
            deployment_url,
            key: value.key,
            name: value.name,
            description: value.description,
            category: value.category,
            event_type: value.event_type,
            channel_key: value.channel_key,
            resource_name: value.resource_name,
            resource,
            version: value.version,
            tenant_id: value.tenant_id,
            parent_deployment_id: value.parent_deployment_id,
            payload: value.payload,
        }
    }
}

impl From<EventInstanceDelivery> for EventInstanceDeliveryResponse {
    fn from(value: EventInstanceDelivery) -> Self {
        Self {
            id: value.id,
            event_definition_id: value.event_definition_id,
            event_definition_key: value.event_definition_key,
            event_type: value.event_type,
            channel_key: value.channel_key,
            direction: value.direction,
            status: value.status,
            status_history: value.status_history,
            last_error: value.last_error,
            retry_count: value.retry_count,
            last_retry_at: value.last_retry_at,
            last_failure_at: value.last_failure_at,
            next_retry_at: value.next_retry_at,
            dispatch_token: value.dispatch_token,
            tenant_id: value.tenant_id,
            payload: value.payload,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

fn service(engine: Arc<ProcessEngine>) -> FlowableEventRegistryService {
    FlowableEventRegistryService::new(engine)
}

fn parse_direction(value: &str) -> Result<EventDirection, ApiError> {
    if value.eq_ignore_ascii_case("inbound") {
        Ok(EventDirection::Inbound)
    } else if value.eq_ignore_ascii_case("outbound") {
        Ok(EventDirection::Outbound)
    } else {
        Err(ApiError::bad_request(format!(
            "Unsupported event delivery direction '{value}'"
        )))
    }
}

/// P133: Java `RequestUtil.getDate` → epoch millis for channel createTime filters.
fn parse_optional_epoch_millis(
    param: &str,
    value: Option<&str>,
) -> Result<Option<i64>, ApiError> {
    let Some(raw) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if let Ok(millis) = raw.parse::<i64>() {
        return Ok(Some(millis));
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.timestamp_millis())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
                .map(|ndt| ndt.and_utc().timestamp_millis())
        })
        .map_err(|_| {
            ApiError::bad_request(format!(
                "Invalid date value for parameter '{param}': '{raw}'"
            ))
        })?;
    Ok(Some(parsed))
}

fn parse_status(value: &str) -> Result<EventInstanceStatus, ApiError> {
    if value.eq_ignore_ascii_case("CREATED") {
        Ok(EventInstanceStatus::Created)
    } else if value.eq_ignore_ascii_case("RECEIVED") {
        Ok(EventInstanceStatus::Received)
    } else if value.eq_ignore_ascii_case("PROCESSED") {
        Ok(EventInstanceStatus::Processed)
    } else if value.eq_ignore_ascii_case("PUBLISHED") {
        Ok(EventInstanceStatus::Published)
    } else if value.eq_ignore_ascii_case("FAILED") {
        Ok(EventInstanceStatus::Failed)
    } else {
        Err(ApiError::bad_request(format!(
            "Unsupported event delivery status '{value}'"
        )))
    }
}

fn descending_order(order: Option<&str>) -> Result<bool, ApiError> {
    match order.unwrap_or("asc") {
        value if value.eq_ignore_ascii_case("asc") => Ok(false),
        value if value.eq_ignore_ascii_case("desc") => Ok(true),
        value => Err(ApiError::bad_request(format!(
            "Unsupported event registry query order '{value}'"
        ))),
    }
}

fn supported_definition_sort(sort: Option<&str>) -> Result<Option<String>, ApiError> {
    match sort {
        None => Ok(None),
        Some("id" | "key" | "name" | "category" | "deploymentId" | "tenantId" | "version") => {
            Ok(sort.map(ToOwned::to_owned))
        }
        Some(value) => Err(ApiError::bad_request(format!(
            "Unsupported event registry definition sort '{value}'"
        ))),
    }
}

fn supported_deployment_sort(sort: Option<&str>) -> Result<Option<String>, ApiError> {
    match sort {
        None => Ok(None),
        Some("id" | "name" | "deployTime" | "tenantId") => Ok(sort.map(ToOwned::to_owned)),
        Some(value) => Err(ApiError::bad_request(format!(
            "Unsupported event registry deployment sort '{value}'"
        ))),
    }
}

pub async fn deploy(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: String,
) -> Result<Json<DeploymentResponse>, ApiError> {
    let payload: EventRegistryDeploymentRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let deployment = service(engine).deploy(payload.into_service_request()?)?;
    Ok(Json(deployment.into()))
}

pub async fn list_deployments(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<DeploymentResponse>>, ApiError> {
    let params: DeploymentQueryParams = parse_query(&uri)?;
    let mut query = service(engine).create_deployment_query();
    if let Some(name) = params.name {
        query = query.name(name);
    }
    if let Some(name_like) = params.name_like {
        query = query.name_like(name_like);
    }
    if let Some(category) = params.category {
        query = query.category(category);
    }
    if let Some(category_not_equals) = params.category_not_equals {
        query = query.category_not_equals(category_not_equals);
    }
    if let Some(parent_deployment_id) = params.parent_deployment_id {
        query = query.parent_deployment_id(parent_deployment_id);
    }
    if let Some(parent_deployment_id_like) = params.parent_deployment_id_like {
        query = query.parent_deployment_id_like(parent_deployment_id_like);
    }
    if let Some(tenant_id) = params.tenant_id {
        query = query.tenant_id(tenant_id);
    }
    if let Some(tenant_id_like) = params.tenant_id_like {
        query = query.tenant_id_like(tenant_id_like);
    }
    if params.without_tenant_id.unwrap_or(false) {
        query = query.without_tenant_id();
    }
    if let Some(sort) = supported_deployment_sort(params.sort.as_deref())? {
        query = query.order_by(sort, descending_order(params.order.as_deref())?);
    } else {
        let _ = descending_order(params.order.as_deref())?;
    }
    if let Some(size) = params.size {
        query = query.page(params.start, size);
    } else if params.start > 0 {
        query = query.page(params.start, usize::MAX);
    }
    let page = query.list_page()?;
    Ok(Json(PagedResponse {
        start: page.start,
        size: page.size,
        total: page.total,
        data: page.data.into_iter().map(Into::into).collect(),
        sort: None,
        order: None,
    }))
}

pub async fn get_deployment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(deployment_id): Path<String>,
) -> Result<Json<DeploymentResponse>, ApiError> {
    Ok(Json(service(engine).get_deployment(&deployment_id)?.into()))
}

pub async fn delete_deployment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(deployment_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service(engine).delete_deployment(&deployment_id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_deployment_resources(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(deployment_id): Path<String>,
) -> Result<Json<Vec<DeploymentResourceResponse>>, ApiError> {
    let resources = service(engine).get_deployment_resources(&deployment_id)?;
    Ok(Json(
        resources
            .iter()
            .map(|resource| deployment_resource_response(&deployment_id, resource))
            .collect(),
    ))
}

pub async fn get_deployment_resource(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<Json<DeploymentResourceResponse>, ApiError> {
    let resource = service(engine).get_deployment_resource(&deployment_id, &resource_name)?;
    Ok(Json(deployment_resource_response(
        &deployment_id,
        &resource,
    )))
}

pub async fn get_deployment_resource_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let resource = service(engine).get_deployment_resource(&deployment_id, &resource_name)?;
    Ok(binary_resource_response(resource))
}

pub async fn list_channel_definitions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ChannelDefinitionResponse>>, ApiError> {
    let params: ChannelDefinitionQueryParams = parse_query(&uri)?;
    if params.only_inbound.unwrap_or(false) && params.only_outbound.unwrap_or(false) {
        return Err(ApiError::bad_request(
            "onlyInbound and onlyOutbound are mutually exclusive",
        ));
    }
    let mut query = service(engine).create_channel_definition_query();
    if let Some(id) = params.id {
        query = query.id(id);
    }
    if let Some(key) = params.key {
        query = query.key(key);
    }
    if let Some(key_like) = params.key_like {
        query = query.key_like(key_like);
    }
    if let Some(key_like_ignore_case) = params.key_like_ignore_case {
        query = query.key_like_ignore_case(key_like_ignore_case);
    }
    if let Some(name) = params.name {
        query = query.name(name);
    }
    if let Some(name_like) = params.name_like {
        query = query.name_like(name_like);
    }
    if let Some(name_like_ignore_case) = params.name_like_ignore_case {
        query = query.name_like_ignore_case(name_like_ignore_case);
    }
    if let Some(category) = params.category {
        query = query.category(category);
    }
    if let Some(category_like) = params.category_like {
        query = query.category_like(category_like);
    }
    if let Some(category_not_equals) = params.category_not_equals {
        query = query.category_not_equals(category_not_equals);
    }
    if let Some(deployment_id) = params.deployment_id {
        query = query.deployment_id(deployment_id);
    }
    if let Some(parent_deployment_id) = params.parent_deployment_id {
        query = query.parent_deployment_id(parent_deployment_id);
    }
    if let Some(channel_type) = params.channel_type {
        query = query.channel_type(channel_type);
    }
    if params.only_inbound.unwrap_or(false) {
        query = query.channel_type("inbound");
    }
    if params.only_outbound.unwrap_or(false) {
        query = query.channel_type("outbound");
    }
    if let Some(resource_name) = params.resource_name {
        query = query.resource_name(resource_name);
    }
    if let Some(resource_name_like) = params.resource_name_like {
        query = query.resource_name_like(resource_name_like);
    }
    if let Some(implementation) = params.implementation {
        query = query.implementation(implementation);
    }
    if let Some(tenant_id) = params.tenant_id {
        query = query.tenant_id(tenant_id);
    }
    if let Some(tenant_id_like) = params.tenant_id_like {
        query = query.tenant_id_like(tenant_id_like);
    }
    if let Some(version) = params.version {
        query = query.version(version);
    }
    if params.latest.unwrap_or(false) {
        query = query.latest();
    }
    // P133: createTime / createTimeAfter / createTimeBefore
    // (ChannelDefinitionCollectionResource.java:126-133)
    if let Some(create_time) = parse_optional_epoch_millis("createTime", params.create_time.as_deref())? {
        query = query.create_time(create_time);
    }
    if let Some(create_time_after) =
        parse_optional_epoch_millis("createTimeAfter", params.create_time_after.as_deref())?
    {
        query = query.create_time_after(create_time_after);
    }
    if let Some(create_time_before) =
        parse_optional_epoch_millis("createTimeBefore", params.create_time_before.as_deref())?
    {
        query = query.create_time_before(create_time_before);
    }
    if let Some(sort) = supported_definition_sort(params.sort.as_deref())? {
        query = query.order_by(sort, descending_order(params.order.as_deref())?);
    } else {
        let _ = descending_order(params.order.as_deref())?;
    }
    if let Some(size) = params.size {
        query = query.page(params.start, size);
    } else if params.start > 0 {
        query = query.page(params.start, usize::MAX);
    }
    let page = query.list_page()?;
    Ok(Json(PagedResponse {
        start: page.start,
        size: page.size,
        total: page.total,
        data: page.data.into_iter().map(Into::into).collect(),
        sort: None,
        order: None,
    }))
}

pub async fn get_channel_definition(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(channel_definition_id): Path<String>,
) -> Result<Json<ChannelDefinitionResponse>, ApiError> {
    let definition = service(engine).get_channel_definition(&channel_definition_id)?;
    Ok(Json(definition.into()))
}

pub async fn get_channel_definition_model(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(channel_definition_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let definition = service(engine).get_channel_definition(&channel_definition_id)?;
    Ok(Json(serde_json::json!({
        "id": definition.id,
        "key": definition.key,
        "name": definition.name,
        "description": definition.description,
        "channelType": definition.channel_type,
        "resourceName": definition.resource_name,
        "configuration": definition.configuration,
    })))
}

pub async fn get_channel_definition_resource_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(channel_definition_id): Path<String>,
) -> Result<Response, ApiError> {
    let resource = service(engine).get_channel_definition_resource_data(&channel_definition_id)?;
    Ok(binary_resource_response(resource))
}

pub async fn list_event_definitions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<EventDefinitionResponse>>, ApiError> {
    let params: EventDefinitionQueryParams = parse_query(&uri)?;
    let mut query = service(engine).create_event_definition_query();
    if let Some(id) = params.id {
        query = query.id(id);
    }
    if let Some(key) = params.key {
        query = query.key(key);
    }
    if let Some(key_like) = params.key_like {
        query = query.key_like(key_like);
    }
    if let Some(key_like_ignore_case) = params.key_like_ignore_case {
        query = query.key_like_ignore_case(key_like_ignore_case);
    }
    if let Some(name) = params.name {
        query = query.name(name);
    }
    if let Some(name_like) = params.name_like {
        query = query.name_like(name_like);
    }
    if let Some(name_like_ignore_case) = params.name_like_ignore_case {
        query = query.name_like_ignore_case(name_like_ignore_case);
    }
    if let Some(category) = params.category {
        query = query.category(category);
    }
    if let Some(category_like) = params.category_like {
        query = query.category_like(category_like);
    }
    if let Some(category_not_equals) = params.category_not_equals {
        query = query.category_not_equals(category_not_equals);
    }
    if let Some(deployment_id) = params.deployment_id {
        query = query.deployment_id(deployment_id);
    }
    if let Some(parent_deployment_id) = params.parent_deployment_id {
        query = query.parent_deployment_id(parent_deployment_id);
    }
    if let Some(event_type) = params.event_type {
        query = query.event_type(event_type);
    }
    if let Some(channel_key) = params.channel_key {
        query = query.channel_key(channel_key);
    }
    if let Some(resource_name) = params.resource_name {
        query = query.resource_name(resource_name);
    }
    if let Some(resource_name_like) = params.resource_name_like {
        query = query.resource_name_like(resource_name_like);
    }
    if let Some(tenant_id) = params.tenant_id {
        query = query.tenant_id(tenant_id);
    }
    if let Some(tenant_id_like) = params.tenant_id_like {
        query = query.tenant_id_like(tenant_id_like);
    }
    if let Some(version) = params.version {
        query = query.version(version);
    }
    if params.latest.unwrap_or(false) {
        query = query.latest();
    }
    if let Some(sort) = supported_definition_sort(params.sort.as_deref())? {
        query = query.order_by(sort, descending_order(params.order.as_deref())?);
    } else {
        let _ = descending_order(params.order.as_deref())?;
    }
    if let Some(size) = params.size {
        query = query.page(params.start, size);
    } else if params.start > 0 {
        query = query.page(params.start, usize::MAX);
    }
    let page = query.list_page()?;
    Ok(Json(PagedResponse {
        start: page.start,
        size: page.size,
        total: page.total,
        data: page.data.into_iter().map(Into::into).collect(),
        sort: None,
        order: None,
    }))
}

pub async fn get_event_definition(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(event_definition_id): Path<String>,
) -> Result<Json<EventDefinitionResponse>, ApiError> {
    let definition = service(engine).get_event_definition(&event_definition_id)?;
    Ok(Json(definition.into()))
}

pub async fn get_event_definition_model(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(event_definition_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let definition = service(engine).get_event_definition(&event_definition_id)?;
    Ok(Json(serde_json::json!({
        "id": definition.id,
        "key": definition.key,
        "name": definition.name,
        "description": definition.description,
        "category": definition.category,
        "eventType": definition.event_type,
        "channelKey": definition.channel_key,
        "resourceName": definition.resource_name,
        "version": definition.version,
        "tenantId": definition.tenant_id,
        "parentDeploymentId": definition.parent_deployment_id,
        "payload": definition.payload,
    })))
}

pub async fn get_event_definition_resource_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(event_definition_id): Path<String>,
) -> Result<Response, ApiError> {
    let resource = service(engine).get_event_definition_resource_data(&event_definition_id)?;
    Ok(binary_resource_response(resource))
}

pub async fn receive_inbound_event(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(request): Json<InboundEventRequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let service = service(engine);
    let delivery = if let Some(channel_key) = request.channel_key.filter(|value| !value.is_empty()) {
        // Prefer channel-keyed raw processing when channel semantics are present.
        service.process_inbound_channel_event(InboundRawEvent {
            channel_key,
            body: request.event_payload,
            headers: request.headers,
            tenant_hint: request.tenant_id,
        })?
    } else if let Some(event_type) = request.event_type.filter(|value| !value.is_empty()) {
        // Explicit compatibility endpoint behavior for event-type receive.
        service.receive_inbound_event(InboundEventRequest {
            event_type,
            event_payload: request.event_payload,
            tenant_id: request.tenant_id,
        })?
    } else {
        return Err(ApiError::bad_request(
            "Either channelKey or eventType is required for inbound event processing.",
        ));
    };
    Ok((
        StatusCode::CREATED,
        Json(EventInstanceDeliveryResponse::from(delivery)),
    ))
}

pub async fn publish_outbound_event(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Json(request): Json<EventInstanceRequestBody>,
) -> Result<Response, ApiError> {
    let event_definition_key = request.event_definition_key.clone();
    if request.event_definition_id.is_some()
        || request.channel_definition_id.is_some()
        || request.channel_definition_key.is_some()
    {
        service(engine).receive_event_instance(EventInstanceRequest {
            event_definition_id: request.event_definition_id,
            event_definition_key: request.event_definition_key,
            channel_definition_id: request.channel_definition_id,
            channel_definition_key: request.channel_definition_key,
            event_payload: request.event_payload,
            tenant_id: request.tenant_id,
        })?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    let event_definition_key = event_definition_key.ok_or_else(|| {
        ApiError::bad_request("Either eventDefinitionId or eventDefinitionKey is required.")
    })?;
    let delivery = service(engine).publish_outbound_event(OutboundEventRequest {
        event_definition_key,
        event_payload: request.event_payload,
        tenant_id: request.tenant_id,
    })?;
    Ok((
        StatusCode::CREATED,
        Json(EventInstanceDeliveryResponse::from(delivery)),
    )
        .into_response())
}

pub async fn list_event_deliveries(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<EventInstanceDeliveryResponse>>, ApiError> {
    let params: DeliveryQueryParams = parse_query(&uri)?;
    let mut query = service(engine).create_event_instance_delivery_query();
    if let Some(direction) = params.direction {
        query = query.direction(parse_direction(&direction)?);
    }
    if let Some(status) = params.status {
        query = query.status(parse_status(&status)?);
    }
    if let Some(event_type) = params.event_type {
        query = query.event_type(event_type);
    }
    if let Some(channel_key) = params.channel_key {
        query = query.channel_key(channel_key);
    }
    if let Some(tenant_id) = params.tenant_id {
        query = query.tenant_id(tenant_id);
    }
    if let Some(tenant_id_like) = params.tenant_id_like {
        query = query.tenant_id_like(tenant_id_like);
    }
    if params.without_tenant_id.unwrap_or(false) {
        query = query.without_tenant_id(true);
    }
    if let Some(size) = params.size {
        query = query.page(params.start, size);
    } else if params.start > 0 {
        query = query.page(params.start, usize::MAX);
    }
    let page = query.list_page()?;
    Ok(Json(PagedResponse {
        start: page.start,
        size: page.size,
        total: page.total,
        data: page.data.into_iter().map(Into::into).collect(),
        sort: None,
        order: None,
    }))
}

pub async fn get_event_delivery(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(delivery_id): Path<String>,
) -> Result<Json<EventInstanceDeliveryResponse>, ApiError> {
    let delivery = service(engine).get_event_instance_delivery(&delivery_id)?;
    Ok(Json(delivery.into()))
}

pub async fn get_engine_info(
    Extension(engine): Extension<Arc<ProcessEngine>>,
) -> Result<Json<EventRegistryEngineInfo>, ApiError> {
    Ok(Json(service(engine).get_engine_info()))
}

fn deployment_resource_response(
    deployment_id: &str,
    resource: &flowable_event_registry_service::EventRegistryResourceData,
) -> DeploymentResourceResponse {
    let content_url = format!(
        "/event-registry-repository/deployments/{deployment_id}/resourcedata/{}",
        resource.resource_name
    );
    let url = format!(
        "/event-registry-repository/deployments/{deployment_id}/resources/{}",
        resource.resource_name
    );
    DeploymentResourceResponse {
        id: resource.resource_name.clone(),
        url,
        content_url,
        media_type: resource.content_type.clone(),
        resource_type: resource.resource_type.clone(),
    }
}

fn binary_resource_response(
    resource: flowable_event_registry_service::EventRegistryResourceData,
) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, resource.content_type)],
        resource.bytes,
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateChannelDefinitionRequestBody {
    pub name: Option<String>,
    pub configuration: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateEventDefinitionRequestBody {
    pub name: Option<String>,
    pub payload: Option<Value>,
}

pub async fn update_channel_definition(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(channel_definition_id): Path<String>,
    Json(body): Json<UpdateChannelDefinitionRequestBody>,
) -> Result<Json<ChannelDefinitionResponse>, ApiError> {
    let definition = service(engine).update_channel_definition(
        &channel_definition_id,
        ChannelDefinitionUpdateRequest {
            name: body.name,
            configuration: body.configuration,
        },
    )?;
    Ok(Json(definition.into()))
}

pub async fn update_event_definition(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(event_definition_id): Path<String>,
    Json(body): Json<UpdateEventDefinitionRequestBody>,
) -> Result<Json<EventDefinitionResponse>, ApiError> {
    let definition = service(engine).update_event_definition(
        &event_definition_id,
        EventDefinitionUpdateRequest {
            name: body.name,
            payload: body.payload,
        },
    )?;
    Ok(Json(definition.into()))
}

pub async fn retry_event_delivery(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(delivery_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let delivery = service(engine).retry_event_delivery(&delivery_id)?;
    Ok((
        StatusCode::OK,
        Json(EventInstanceDeliveryResponse::from(delivery)),
    ))
}

pub async fn delete_event_delivery(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(delivery_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    service(engine).delete_event_delivery(&delivery_id)?;
    Ok(StatusCode::NO_CONTENT)
}
