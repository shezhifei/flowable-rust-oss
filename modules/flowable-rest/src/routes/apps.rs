use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::Path,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde::ser::{Error as _, SerializeStruct};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub type DynAppRepository = Arc<dyn AppRepositoryApi>;
pub type DynAppRuntime = Arc<dyn AppRuntimeApi>;

pub trait AppRepositoryApi: Send + Sync {
    fn deploy_applications(
        &self,
        command: AppDeploymentCommand,
    ) -> Result<AppDeploymentRecord, ApiError>;
    fn list_app_deployments(
        &self,
        query: AppDeploymentQuery,
    ) -> Result<PagedResponse<AppDeploymentRecord>, ApiError>;
    fn get_app_deployment(&self, deployment_id: &str) -> Result<AppDeploymentRecord, ApiError>;
    fn delete_app_deployment(&self, deployment_id: &str) -> Result<(), ApiError> {
        let _ = deployment_id;
        Err(ApiError::NotFound(
            "App deployment was not found".to_string(),
        ))
    }
    fn list_app_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<AppDeploymentResourceRecord>, ApiError> {
        let deployment = self.get_app_deployment(deployment_id)?;
        Ok(deployment
            .resource_names
            .into_iter()
            .map(|resource_name| AppDeploymentResourceRecord {
                deployment_id: deployment_id.to_string(),
                resource_type: app_resource_type_for_name(&resource_name).to_string(),
                content_type: app_content_type_for_name(&resource_name).to_string(),
                resource_name,
                bytes: Vec::new(),
            })
            .collect())
    }
    fn get_app_deployment_resource(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<AppDeploymentResourceRecord, ApiError> {
        let _ = (deployment_id, resource_name);
        Err(ApiError::NotFound(
            "App deployment resource was not found".to_string(),
        ))
    }
    fn list_app_definitions(
        &self,
        query: AppDefinitionQuery,
    ) -> Result<PagedResponse<AppDefinitionRecord>, ApiError>;
    fn get_app_definition(&self, app_definition_id: &str) -> Result<AppDefinitionRecord, ApiError>;
    fn get_app_definition_resource_data(
        &self,
        app_definition_id: &str,
    ) -> Result<AppDeploymentResourceRecord, ApiError> {
        let definition = self.get_app_definition(app_definition_id)?;
        self.get_app_deployment_resource(&definition.deployment_id, &definition.resource_name)
    }
    fn get_app_definition_model(&self, app_definition_id: &str) -> Result<Value, ApiError> {
        serde_json::to_value(self.get_app_definition(app_definition_id)?)
            .map_err(|err| ApiError::InternalServerError(err.to_string()))
    }
}

pub trait AppRuntimeApi: Send + Sync {
    fn list_app_compositions(
        &self,
        query: AppCompositionQuery,
    ) -> Result<PagedResponse<AppCompositionRecord>, ApiError>;
    fn get_app_composition(
        &self,
        app_definition_id: &str,
        filter: AppCompositionFilter,
    ) -> Result<AppCompositionRecord, ApiError>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfoRecord {
    pub name: String,
    pub version: String,
    pub resource_url: Option<String>,
    pub exception: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppDeploymentCommand {
    pub name: String,
    pub category: Option<String>,
    pub tenant_id: Option<String>,
    pub resources: Vec<AppDeploymentResourcePayload>,
}

#[derive(Debug, Clone)]
pub struct AppDeploymentResourcePayload {
    pub resource_name: String,
    pub resource: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AppDeploymentRecord {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub deployed_at: i64,
    pub resource_names: Vec<String>,
    pub tenant_id: Option<String>,
}

impl Serialize for AppDeploymentRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let deployment_time = rfc3339_millis(self.deployed_at).ok_or_else(|| {
            S::Error::custom(format!(
                "App deployment '{}' has invalid deployment millis '{}'",
                self.id, self.deployed_at
            ))
        })?;
        let mut state = serializer.serialize_struct("AppDeploymentRecord", 8)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("deploymentTime", &deployment_time)?;
        state.serialize_field("category", &self.category)?;
        state.serialize_field("url", &app_deployment_url(&self.id))?;
        state.serialize_field("tenantId", &self.tenant_id)?;
        state.serialize_field("deployedAt", &self.deployed_at)?;
        state.serialize_field("resourceNames", &self.resource_names)?;
        state.end()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDeploymentResourceRecord {
    pub deployment_id: String,
    pub resource_name: String,
    pub resource_type: String,
    pub content_type: String,
    #[serde(skip_serializing)]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct AppDeploymentQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub category: Option<String>,
    pub category_not_equals: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub without_tenant_id: bool,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AppDefinitionQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub key: Option<String>,
    pub key_like: Option<String>,
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub category: Option<String>,
    pub category_like: Option<String>,
    pub category_not_equals: Option<String>,
    pub deployment_id: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub without_tenant_id: bool,
    pub resource_name: Option<String>,
    pub resource_name_like: Option<String>,
    pub version: Option<i32>,
    pub version_greater_than: Option<i32>,
    pub version_greater_than_or_equals: Option<i32>,
    pub version_lower_than: Option<i32>,
    pub version_lower_than_or_equals: Option<i32>,
    pub latest: bool,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppDefinitionRecord {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub version: i32,
    pub deployment_id: String,
    pub resource_name: String,
    pub tenant_id: Option<String>,
}

impl Serialize for AppDefinitionRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("AppDefinitionRecord", 10)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("url", &app_definition_url(&self.id))?;
        state.serialize_field("category", &self.category)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("key", &self.key)?;
        state.serialize_field("description", &self.description)?;
        state.serialize_field("version", &self.version)?;
        state.serialize_field("resourceName", &self.resource_name)?;
        state.serialize_field("deploymentId", &self.deployment_id)?;
        state.serialize_field("tenantId", &self.tenant_id)?;
        state.end()
    }
}

#[derive(Debug, Clone, Default)]
pub struct AppCompositionQuery {
    pub paging: PagingQuery,
    pub app_definition_id: Option<String>,
    pub app_definition_key: Option<String>,
    pub tenant_id: Option<String>,
    pub definition_type: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AppCompositionFilter {
    pub definition_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCompositionRecord {
    pub app_definition_id: String,
    pub app_definition_key: String,
    pub app_definition_name: String,
    pub app_definition_version: i32,
    pub deployment_id: String,
    pub tenant_id: Option<String>,
    pub references: Vec<AppResolvedReferenceRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppResolvedReferenceRecord {
    pub page_id: String,
    pub page_name: Option<String>,
    pub reference_id: String,
    pub reference_name: Option<String>,
    pub definition_type: String,
    pub resolved_definition_id: String,
    pub resolved_definition_key: String,
    pub resolved_definition_name: String,
    pub resolved_definition_version: i32,
    pub resolved_tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppDeploymentRequest {
    name: String,
    category: Option<String>,
    #[serde(default)]
    resources: Vec<AppDeploymentResourceRequest>,
    tenant_id: Option<String>,
    resource_name: Option<String>,
    resource: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppDeploymentResourceRequest {
    resource_name: String,
    resource: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct AppDeploymentQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    category: Option<String>,
    category_not_equals: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: Option<bool>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct AppDefinitionQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    key: Option<String>,
    key_like: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    category: Option<String>,
    category_like: Option<String>,
    category_not_equals: Option<String>,
    deployment_id: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: Option<bool>,
    resource_name: Option<String>,
    resource_name_like: Option<String>,
    version: Option<i32>,
    version_greater_than: Option<i32>,
    version_greater_than_or_equals: Option<i32>,
    version_lower_than: Option<i32>,
    version_lower_than_or_equals: Option<i32>,
    latest: Option<bool>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct AppCompositionQueryParams {
    start: usize,
    size: Option<usize>,
    app_definition_id: Option<String>,
    app_definition_key: Option<String>,
    tenant_id: Option<String>,
    definition_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct AppCompositionFilterParams {
    definition_type: Option<String>,
}

impl AppDeploymentRequest {
    fn into_command(self) -> Result<AppDeploymentCommand, ApiError> {
        let resources = if self.resources.is_empty() {
            match (self.resource_name, self.resource) {
                (Some(resource_name), Some(resource)) => {
                    vec![AppDeploymentResourcePayload {
                        resource_name,
                        resource: resource.into_bytes(),
                    }]
                }
                _ => {
                    return Err(ApiError::bad_request(
                        "App deployment requires at least one resource",
                    ));
                }
            }
        } else {
            self.resources
                .into_iter()
                .map(|resource| AppDeploymentResourcePayload {
                    resource_name: resource.resource_name,
                    resource: resource.resource.into_bytes(),
                })
                .collect()
        };

        Ok(AppDeploymentCommand {
            name: self.name,
            category: self.category,
            tenant_id: self.tenant_id,
            resources,
        })
    }
}

impl From<AppDeploymentQueryParams> for AppDeploymentQuery {
    fn from(value: AppDeploymentQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            name: value.name,
            name_like: value.name_like,
            category: value.category,
            category_not_equals: value.category_not_equals,
            tenant_id: value.tenant_id,
            tenant_id_like: value.tenant_id_like,
            without_tenant_id: value.without_tenant_id.unwrap_or(false),
            sort: value.sort,
            order: value.order,
        }
    }
}

impl From<AppDefinitionQueryParams> for AppDefinitionQuery {
    fn from(value: AppDefinitionQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            key: value.key,
            key_like: value.key_like,
            name: value.name,
            name_like: value.name_like,
            category: value.category,
            category_like: value.category_like,
            category_not_equals: value.category_not_equals,
            deployment_id: value.deployment_id,
            tenant_id: value.tenant_id,
            tenant_id_like: value.tenant_id_like,
            without_tenant_id: value.without_tenant_id.unwrap_or(false),
            resource_name: value.resource_name,
            resource_name_like: value.resource_name_like,
            version: value.version,
            version_greater_than: value.version_greater_than,
            version_greater_than_or_equals: value.version_greater_than_or_equals,
            version_lower_than: value.version_lower_than,
            version_lower_than_or_equals: value.version_lower_than_or_equals,
            latest: value.latest.unwrap_or(false),
            sort: value.sort,
            order: value.order,
        }
    }
}

impl From<AppCompositionQueryParams> for AppCompositionQuery {
    fn from(value: AppCompositionQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            app_definition_id: value.app_definition_id,
            app_definition_key: value.app_definition_key,
            tenant_id: value.tenant_id,
            definition_type: value.definition_type,
        }
    }
}

impl From<AppCompositionFilterParams> for AppCompositionFilter {
    fn from(value: AppCompositionFilterParams) -> Self {
        Self {
            definition_type: value.definition_type,
        }
    }
}

pub async fn deploy(
    Extension(repository): Extension<DynAppRepository>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let request = if is_multipart_request(&headers) {
        parse_multipart_app_deployment(&headers, &body)?
    } else {
        serde_json::from_slice::<AppDeploymentRequest>(&body)
            .map_err(|err| ApiError::bad_request(err.to_string()))?
    };
    let deployment = repository.deploy_applications(request.into_command()?)?;
    Ok((StatusCode::CREATED, Json(deployment)))
}

pub async fn list_app_deployments(
    Extension(repository): Extension<DynAppRepository>,
    uri: Uri,
) -> Result<Json<PagedResponse<AppDeploymentRecord>>, ApiError> {
    let query = parse_query::<AppDeploymentQueryParams>(&uri)?;
    validate_sort_order(
        query.sort.as_deref(),
        query.order.as_deref(),
        &["id", "name", "deployTime", "tenantId"],
    )?;
    Ok(Json(repository.list_app_deployments(query.into())?))
}

pub async fn get_app_deployment(
    Extension(repository): Extension<DynAppRepository>,
    Path(deployment_id): Path<String>,
) -> Result<Json<AppDeploymentRecord>, ApiError> {
    Ok(Json(repository.get_app_deployment(&deployment_id)?))
}

pub async fn delete_app_deployment(
    Extension(repository): Extension<DynAppRepository>,
    Path(deployment_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    repository.delete_app_deployment(&deployment_id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_app_deployment_resources(
    Extension(repository): Extension<DynAppRepository>,
    Path(deployment_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let resources = repository.list_app_deployment_resources(&deployment_id)?;
    Ok(Json(serde_json::json!(
        resources
            .iter()
            .map(|resource| app_resource_response(&deployment_id, resource))
            .collect::<Vec<_>>()
    )))
}

pub async fn get_app_deployment_resource(
    Extension(repository): Extension<DynAppRepository>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let resource = repository.get_app_deployment_resource(&deployment_id, &resource_name)?;
    Ok(Json(app_resource_response(&deployment_id, &resource)))
}

pub async fn get_app_deployment_resource_data(
    Extension(repository): Extension<DynAppRepository>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let resource = repository.get_app_deployment_resource(&deployment_id, &resource_name)?;
    Ok(binary_resource_response(resource))
}

pub async fn list_app_definitions(
    Extension(repository): Extension<DynAppRepository>,
    uri: Uri,
) -> Result<Json<PagedResponse<AppDefinitionRecord>>, ApiError> {
    let query = parse_query::<AppDefinitionQueryParams>(&uri)?;
    validate_sort_order(
        query.sort.as_deref(),
        query.order.as_deref(),
        &[
            "id",
            "key",
            "category",
            "name",
            "version",
            "deploymentId",
            "tenantId",
        ],
    )?;
    Ok(Json(repository.list_app_definitions(query.into())?))
}

pub async fn get_app_definition(
    Extension(repository): Extension<DynAppRepository>,
    Path(app_definition_id): Path<String>,
) -> Result<Json<AppDefinitionRecord>, ApiError> {
    Ok(Json(repository.get_app_definition(&app_definition_id)?))
}

pub async fn get_app_definition_model(
    Extension(repository): Extension<DynAppRepository>,
    Path(app_definition_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        repository.get_app_definition_model(&app_definition_id)?,
    ))
}

pub async fn get_app_definition_resource_data(
    Extension(repository): Extension<DynAppRepository>,
    Path(app_definition_id): Path<String>,
) -> Result<Response, ApiError> {
    let resource = repository.get_app_definition_resource_data(&app_definition_id)?;
    Ok(binary_resource_response(resource))
}

pub async fn list_app_compositions(
    Extension(runtime): Extension<DynAppRuntime>,
    uri: Uri,
) -> Result<Json<PagedResponse<AppCompositionRecord>>, ApiError> {
    let query = parse_query::<AppCompositionQueryParams>(&uri)?;
    Ok(Json(runtime.list_app_compositions(query.into())?))
}

pub async fn get_app_composition(
    Extension(runtime): Extension<DynAppRuntime>,
    Path(app_definition_id): Path<String>,
    uri: Uri,
) -> Result<Json<AppCompositionRecord>, ApiError> {
    let filter = parse_query::<AppCompositionFilterParams>(&uri)?;
    Ok(Json(
        runtime.get_app_composition(&app_definition_id, filter.into())?,
    ))
}

pub async fn get_engine_info() -> Json<EngineInfoRecord> {
    Json(EngineInfoRecord {
        name: "flowable-app-engine".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        resource_url: None,
        exception: None,
    })
}

fn app_deployment_url(deployment_id: &str) -> String {
    format!("/app-repository/deployments/{deployment_id}")
}

fn app_definition_url(app_definition_id: &str) -> String {
    format!("/app-repository/app-definitions/{app_definition_id}")
}

fn rfc3339_millis(epoch_millis: i64) -> Option<String> {
    Utc.timestamp_millis_opt(epoch_millis)
        .single()
        .map(|date_time| date_time.to_rfc3339_opts(SecondsFormat::Millis, false))
}

fn is_multipart_request(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("multipart/form-data"))
}

fn parse_multipart_app_deployment(
    headers: &axum::http::HeaderMap,
    body: &Bytes,
) -> Result<AppDeploymentRequest, ApiError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("Missing Content-Type header"))?;
    let boundary = multipart_boundary(content_type)
        .ok_or_else(|| ApiError::bad_request("Expected multipart/form-data request"))?;
    let parts = parse_multipart_parts(body.as_ref(), &boundary)?;

    let mut request = AppDeploymentRequest {
        name: String::new(),
        category: None,
        resources: Vec::new(),
        tenant_id: None,
        resource_name: None,
        resource: None,
    };
    let mut resource_payloads = Vec::new();

    for part in parts {
        match part.name.as_deref() {
            Some("name") => {
                request.name = string_part(part.body, "name")?;
            }
            Some("category") => {
                request.category = Some(string_part(part.body, "category")?);
            }
            Some("tenantId") => {
                request.tenant_id = Some(string_part(part.body, "tenantId")?);
            }
            Some("resourceName") => {
                request.resource_name = Some(string_part(part.body, "resourceName")?);
            }
            Some("resource") => {
                request.resource = Some(string_part(part.body, "resource")?);
            }
            Some("resources") => {
                let resource: AppDeploymentResourceRequest = serde_json::from_slice(&part.body)
                    .map_err(|err| {
                        ApiError::bad_request(format!("Invalid resource part: {err}"))
                    })?;
                resource_payloads.push(resource);
            }
            Some(_) | None => {}
        }
    }

    request.resources = resource_payloads;
    if request.name.trim().is_empty() {
        return Err(ApiError::bad_request("App deployment name is required"));
    }
    Ok(request)
}

fn string_part(body: Vec<u8>, field: &str) -> Result<String, ApiError> {
    String::from_utf8(body)
        .map(|value| value.trim().to_string())
        .map_err(|err| ApiError::bad_request(format!("Invalid {field} field: {err}")))
}

fn multipart_boundary(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("boundary="))
        .map(|boundary| boundary.trim_matches('"').to_string())
        .filter(|boundary| !boundary.is_empty())
}

struct MultipartPart {
    name: Option<String>,
    body: Vec<u8>,
}

fn parse_multipart_parts(body: &[u8], boundary: &str) -> Result<Vec<MultipartPart>, ApiError> {
    let delimiter = format!("--{boundary}").into_bytes();
    let mut positions = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = find_subslice(&body[cursor..], &delimiter) {
        let position = cursor + offset;
        positions.push(position);
        cursor = position + delimiter.len();
    }

    let mut parts = Vec::new();
    for pair in positions.windows(2) {
        let mut start = pair[0] + delimiter.len();
        if body.get(start..start + 2) == Some(b"--") {
            break;
        }
        if body.get(start..start + 2) == Some(b"\r\n") {
            start += 2;
        }

        let mut end = pair[1];
        if end >= 2 && body.get(end - 2..end) == Some(b"\r\n") {
            end -= 2;
        }
        if start >= end {
            continue;
        }

        let part = &body[start..end];
        let header_end = find_subslice(part, b"\r\n\r\n")
            .ok_or_else(|| ApiError::bad_request("Malformed multipart part"))?;
        let headers = std::str::from_utf8(&part[..header_end])
            .map_err(|err| ApiError::bad_request(format!("Invalid multipart headers: {err}")))?;
        let content_disposition = headers
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .ok_or_else(|| ApiError::bad_request("Multipart part missing Content-Disposition"))?;
        parts.push(MultipartPart {
            name: disposition_param(content_disposition, "name"),
            body: part[header_end + 4..].to_vec(),
        });
    }

    Ok(parts)
}

fn disposition_param(content_disposition: &str, name: &str) -> Option<String> {
    content_disposition
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
        .map(|value| value.trim_matches('"').to_string())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn app_resource_response(deployment_id: &str, resource: &AppDeploymentResourceRecord) -> Value {
    serde_json::json!({
        "id": resource.resource_name,
        "url": format!(
            "/app-repository/deployments/{deployment_id}/resources/{}",
            resource.resource_name
        ),
        "contentUrl": format!(
            "/app-repository/deployments/{deployment_id}/resourcedata/{}",
            resource.resource_name
        ),
        "mediaType": resource.content_type,
        "type": app_resource_type(resource),
    })
}

fn app_resource_type(resource: &AppDeploymentResourceRecord) -> &str {
    app_resource_type_for_name(&resource.resource_name)
}

fn app_resource_type_for_name(resource_name: &str) -> &str {
    if resource_name.ends_with(".app") {
        "appDefinition"
    } else {
        "resource"
    }
}

fn app_content_type_for_name(resource_name: &str) -> &str {
    let lower_name = resource_name.to_ascii_lowercase();
    if lower_name.ends_with(".json") || lower_name.ends_with(".app") {
        "application/json"
    } else if lower_name.ends_with(".xml") {
        "application/xml"
    } else if lower_name.ends_with(".svg") {
        "image/svg+xml"
    } else if lower_name.ends_with(".png") {
        "image/png"
    } else if lower_name.ends_with(".jpg") || lower_name.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower_name.ends_with(".gif") {
        "image/gif"
    } else if lower_name.ends_with(".txt") {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

fn binary_resource_response(resource: AppDeploymentResourceRecord) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, resource.content_type)],
        resource.bytes,
    )
        .into_response()
}

fn validate_sort_order(
    sort: Option<&str>,
    order: Option<&str>,
    allowed_sorts: &[&str],
) -> Result<(), ApiError> {
    if let Some(sort) = sort
        && !allowed_sorts.contains(&sort)
    {
        return Err(ApiError::bad_request(format!(
            "Invalid sort parameter '{sort}'"
        )));
    }

    if let Some(order) = order
        && !matches!(order, "asc" | "desc")
    {
        return Err(ApiError::bad_request(format!(
            "Invalid order parameter '{order}'"
        )));
    }

    Ok(())
}

pub fn router(repository: DynAppRepository, runtime: DynAppRuntime) -> Router {
    Router::new()
        .route("/app-management/engine", get(get_engine_info))
        .route(
            "/app-repository/deployments",
            post(deploy).get(list_app_deployments),
        )
        .route(
            "/app-repository/deployments/:deployment_id",
            get(get_app_deployment).delete(delete_app_deployment),
        )
        .route(
            "/app-repository/deployments/:deployment_id/resources",
            get(list_app_deployment_resources),
        )
        .route(
            "/app-repository/deployments/:deployment_id/resourcedata/*resource_name",
            get(get_app_deployment_resource_data),
        )
        .route(
            "/app-repository/deployments/:deployment_id/resources/*resource_name",
            get(get_app_deployment_resource),
        )
        .route("/app-repository/app-definitions", get(list_app_definitions))
        .route(
            "/app-repository/app-definitions/:app_definition_id/model",
            get(get_app_definition_model),
        )
        .route(
            "/app-repository/app-definitions/:app_definition_id/resourcedata",
            get(get_app_definition_resource_data),
        )
        .route(
            "/app-repository/app-definitions/:app_definition_id",
            get(get_app_definition),
        )
        .route("/app-runtime/compositions", get(list_app_compositions))
        .route(
            "/app-runtime/compositions/:app_definition_id",
            get(get_app_composition),
        )
        .layer(Extension(repository))
        .layer(Extension(runtime))
}
