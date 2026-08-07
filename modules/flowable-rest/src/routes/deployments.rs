use crate::common::{PagedResponse, PagingQuery, absolute_url, parse_query};
use crate::error::ApiError;
use crate::routes::tasks::sql_like_matches;
use axum::{
    Extension, Json, Router,
    extract::{FromRequest, Multipart, Path, Query, Request},
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::repository::deployment::Deployment;
use flowable_engine::repository::deployment_builder::DeploymentBuilder;
use flowable_engine::repository::deployment_resource::DeploymentResource;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct DeployRequest {
    pub name: String,
    #[serde(rename = "resourceName")]
    pub resource_name: String,
    pub resource: String,
}

/// Query parameters of Java `DeploymentCollectionResource.uploadDeployment`
/// (DeploymentCollectionResource.java:164-166). Unknown query parameters are
/// ignored, mirroring Java's named `@RequestParam` binding.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DeployQuery {
    #[serde(rename = "deploymentKey")]
    deployment_key: Option<String>,
    #[serde(rename = "deploymentName")]
    deployment_name: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
}

#[derive(Default, Deserialize)]
pub struct DeleteDeploymentQuery {
    #[serde(default)]
    pub cascade: bool,
}

/// Query parameters of Java `DeploymentCollectionResource.getDeployments`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeploymentListQuery {
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

const DEPLOYMENTS_PATH: &str = "/repository/deployments";
const DEPLOYMENT_PATH: &str = "/repository/deployments/:deployment_id";
const DEPLOYMENT_RESOURCES_PATH: &str = "/repository/deployments/:deployment_id/resources";
const DEPLOYMENT_RESOURCE_DATA_PATH: &str =
    "/repository/deployments/:deployment_id/resourcedata/*resource_name";
const DEPLOYMENT_RESOURCE_PATH: &str =
    "/repository/deployments/:deployment_id/resources/*resource_name";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{DEPLOYMENTS_PATH}"),
            get(list_deployments).post(deploy),
        )
        .route(
            &format!("{prefix}{DEPLOYMENT_PATH}"),
            get(get_deployment).delete(delete_deployment),
        )
        .route(
            &format!("{prefix}{DEPLOYMENT_RESOURCES_PATH}"),
            get(list_deployment_resources),
        )
        .route(
            &format!("{prefix}{DEPLOYMENT_RESOURCE_DATA_PATH}"),
            get(get_deployment_resource_data),
        )
        .route(
            &format!("{prefix}{DEPLOYMENT_RESOURCE_PATH}"),
            get(get_deployment_resource),
        )
}

fn deployment_response(deployment: Deployment) -> Value {
    json!({
        "id": deployment.id,
        "name": deployment.name,
        "deploymentTime": deployment.deployment_time.map(|t| t.to_rfc3339()),
        "category": deployment.category,
        "key": deployment.key,
        "tenantId": deployment.tenant_id,
        "parentDeploymentId": deployment.parent_deployment_id,
        "derivedFrom": deployment.derived_from,
        "derivedFromRoot": deployment.derived_from_root,
        "engineVersion": deployment.engine_version,
    })
}

fn resource_response(deployment_id: &str, resource: &DeploymentResource) -> Value {
    let path = format!(
        "/repository/deployments/{deployment_id}/resourcedata/{}",
        resource.resource_name
    );
    json!({
        "id": resource.resource_name,
        "url": absolute_url("", &path),
        "contentUrl": absolute_url("", &path),
        "mediaType": resource.content_type,
        "type": resource.resource_type,
    })
}

/// Java `DeploymentCollectionResource.getDeployments`: list deployments with
/// optional filters, defaulting the sort property to `id`.
pub async fn list_deployments(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<Value>>, ApiError> {
    let query: DeploymentListQuery = parse_query(&uri)?;
    let mut deployments = engine.get_repository_service().get_deployments()?;

    if let Some(name) = &query.name {
        deployments.retain(|d| d.name.as_deref() == Some(name.as_str()));
    }
    if let Some(name_like) = &query.name_like {
        deployments.retain(|d| {
            d.name
                .as_deref()
                .is_some_and(|v| sql_like_matches(name_like, v))
        });
    }
    if let Some(category) = &query.category {
        deployments.retain(|d| d.category.as_deref() == Some(category.as_str()));
    }
    if let Some(category_not_equals) = &query.category_not_equals {
        deployments.retain(|d| d.category.as_deref() != Some(category_not_equals.as_str()));
    }
    if let Some(parent_deployment_id) = &query.parent_deployment_id {
        deployments
            .retain(|d| d.parent_deployment_id.as_deref() == Some(parent_deployment_id.as_str()));
    }
    if let Some(parent_deployment_id_like) = &query.parent_deployment_id_like {
        deployments.retain(|d| {
            d.parent_deployment_id
                .as_deref()
                .is_some_and(|v| sql_like_matches(parent_deployment_id_like, v))
        });
    }
    if let Some(tenant_id) = &query.tenant_id {
        deployments.retain(|d| d.tenant_id.as_deref() == Some(tenant_id.as_str()));
    }
    if let Some(tenant_id_like) = &query.tenant_id_like {
        deployments.retain(|d| {
            d.tenant_id
                .as_deref()
                .is_some_and(|v| sql_like_matches(tenant_id_like, v))
        });
    }
    if query.without_tenant_id == Some(true) {
        // Java parity: only `withoutTenantId=true` activates the filter.
        deployments.retain(|d| d.tenant_id.as_deref().unwrap_or("").is_empty());
    }

    // Java `DeploymentsPaginateList` allowed sort properties.
    let sort = query.sort.clone().unwrap_or_else(|| "id".to_string());
    match sort.as_str() {
        "id" => deployments.sort_by(|l, r| l.id.cmp(&r.id)),
        "name" => deployments.sort_by(|l, r| l.name.cmp(&r.name).then(l.id.cmp(&r.id))),
        "deployTime" => deployments.sort_by(|l, r| {
            l.deployment_time
                .cmp(&r.deployment_time)
                .then(l.id.cmp(&r.id))
        }),
        "tenantId" => {
            deployments.sort_by(|l, r| l.tenant_id.cmp(&r.tenant_id).then(l.id.cmp(&r.id)))
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "Unsupported deployment sort field '{other}'"
            )));
        }
    }
    match query.order.as_deref() {
        None | Some("asc") => {}
        Some("desc") => deployments.reverse(),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported deployment sort order '{other}'"
            )));
        }
    }

    let paging = PagingQuery {
        start: query.start,
        size: query.size,
    };
    let mut response = paging.paginate(deployments.into_iter().map(deployment_response).collect());
    response.sort = Some(sort);
    response.order = Some(query.order.clone().unwrap_or_else(|| "asc".to_string()));
    Ok(Json(response))
}

/// Java `DeploymentCollectionResource.uploadDeployment`
/// (DeploymentCollectionResource.java:162-247): the multipart/form-data path
/// is the Java contract, plus a JSON path kept as a Rust superset so existing
/// JSON clients keep working. Both paths return 201 with the full
/// `deployment_response` shape; non-multipart non-JSON requests are a 400 with
/// the Java message.
pub async fn deploy(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    request: Request,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let query: DeployQuery = parse_query(request.uri())?;
    if is_multipart(&request) {
        let multipart = Multipart::from_request(request, &())
            .await
            .map_err(multipart_error)?;
        let form = parse_upload_deployment_form(multipart).await?;
        let builder = deployment_builder_from_upload(form, &query)?;
        let deployment = engine.get_repository_service().deploy(builder)?;
        return Ok((StatusCode::CREATED, Json(deployment_response(deployment))));
    }

    // JSON superset path. Java only accepts multipart here; the Rust JSON
    // extension keeps the pre-existing request shape (`DeployRequest`) and is
    // aligned with the multipart path on status code and response fields.
    if !is_json_content_type(&request) {
        // Java: `FlowableIllegalArgumentException("Multipart request is required")`
        // (DeploymentCollectionResource.java:169-171).
        return Err(ApiError::bad_request("Multipart request is required"));
    }
    let body = request_body_string(request).await?;
    let payload: DeployRequest =
        serde_json::from_str(&body).map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name(payload.name.clone())
        .add_string(payload.resource_name, payload.resource);

    let deployment = engine.get_repository_service().deploy(builder)?;

    Ok((StatusCode::CREATED, Json(deployment_response(deployment))))
}

fn is_multipart(request: &Request) -> bool {
    request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().starts_with("multipart/form-data"))
        .unwrap_or(false)
}

fn is_json_content_type(request: &Request) -> bool {
    request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().starts_with("application/json"))
        .unwrap_or(false)
}

async fn request_body_string(request: Request) -> Result<String, ApiError> {
    let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    String::from_utf8(bytes.to_vec()).map_err(|err| ApiError::bad_request(err.to_string()))
}

fn multipart_error(err: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request(err.to_string())
}

/// Multipart form fields of Java `uploadDeployment`: the first uploaded file
/// is the deployment resource (additional files are ignored); a `tenantId`
/// form field is picked up like Spring's `@RequestParam`, which binds
/// multipart parts (documented at DeploymentCollectionResource.java:152).
#[derive(Default)]
struct UploadDeploymentForm {
    tenant_id: Option<String>,
    field_name: Option<String>,
    original_name: Option<String>,
    file_bytes: Option<Vec<u8>>,
}

async fn parse_upload_deployment_form(
    mut multipart: Multipart,
) -> Result<UploadDeploymentForm, ApiError> {
    let mut form = UploadDeploymentForm::default();
    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        if field.file_name().is_some() {
            // Java: use the first file in the request, ignore possible others.
            if form.file_bytes.is_none() {
                form.field_name = field.name().map(str::to_string);
                form.original_name = field.file_name().map(str::to_string);
                form.file_bytes = Some(field.bytes().await.map_err(multipart_error)?.to_vec());
            }
            continue;
        }
        let field_name = field.name().unwrap_or_default().to_string();
        let text = field.text().await.map_err(multipart_error)?;
        if field_name.eq_ignore_ascii_case("tenantId") {
            form.tenant_id = Some(text);
        }
    }
    Ok(form)
}

/// Java `DeploymentCollectionResource.uploadDeployment` file-name handling and
/// resource dispatch (DeploymentCollectionResource.java:190-210): the original
/// filename is validated against the supported suffixes, falling back to the
/// multipart field name; `.bpmn20.xml`/`.bpmn` files are added as a single
/// resource and `.bar`/`.zip` archives are expanded. The
/// `deploymentKey`/`deploymentName`/`tenantId` query parameters pass through
/// per Java (DeploymentCollectionResource.java:212-231).
fn deployment_builder_from_upload(
    form: UploadDeploymentForm,
    query: &DeployQuery,
) -> Result<DeploymentBuilder, ApiError> {
    // Java: `multipartRequest.getFileMap().size() == 0`
    // (DeploymentCollectionResource.java:182-184).
    let file_bytes = form
        .file_bytes
        .ok_or_else(|| ApiError::bad_request("Multipart request with file content is required"))?;

    // Java: fall back to `file.getName()` (the field name) when the original
    // filename is empty or has no supported suffix.
    let mut file_name = form.original_name.clone().unwrap_or_default();
    if file_name.is_empty()
        || !(file_name.ends_with(".bpmn20.xml")
            || file_name.ends_with(".bpmn")
            || file_name.to_ascii_lowercase().ends_with(".bar")
            || file_name.to_ascii_lowercase().ends_with(".zip"))
    {
        file_name = form.field_name.clone().unwrap_or_default();
    }

    let mut builder = DeploymentBuilder::new();
    if file_name.ends_with(".bpmn20.xml") || file_name.ends_with(".bpmn") {
        builder = builder.add_bytes(file_name.clone(), file_bytes);
    } else if file_name.to_ascii_lowercase().ends_with(".bar")
        || file_name.to_ascii_lowercase().ends_with(".zip")
    {
        // Java `DeploymentBuilderImpl.addZipInputStream`
        // (DeploymentBuilderImpl.java:116-134): every non-directory zip entry
        // becomes a deployment resource, so a zip of several BPMN files
        // registers several process definitions.
        builder = add_zip_entries(builder, &file_bytes)?;
    } else {
        // Java (DeploymentCollectionResource.java:208-210).
        return Err(ApiError::bad_request(
            "File must be of type .bpmn20.xml, .bpmn, .bar or .zip",
        ));
    }

    // Deployment name: the `deploymentName` query parameter wins, otherwise the
    // resource name without its extension (Java `fileName.split("\\.")[0]`,
    // DeploymentCollectionResource.java:212-223).
    let name = match query.deployment_name.as_deref() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => file_name
            .split('.')
            .next()
            .filter(|segment| !segment.is_empty())
            .unwrap_or(&file_name)
            .to_string(),
    };
    builder = builder.name(name);

    // Java: `deploymentKey` applies when present and non-empty
    // (DeploymentCollectionResource.java:225-227).
    if let Some(key) = query.deployment_key.as_deref().filter(|key| !key.is_empty()) {
        builder = builder.key(key.to_string());
    }

    // Java: `tenantId` applies whenever non-null. The query parameter and the
    // multipart `tenantId` form field are both resolved like Spring's
    // `@RequestParam` (DeploymentCollectionResource.java:229-231); the query
    // parameter wins when both are present.
    if let Some(tenant_id) = query.tenant_id.as_deref().or(form.tenant_id.as_deref()) {
        builder = builder.tenant_id(tenant_id.to_string());
    }

    Ok(builder)
}

fn add_zip_entries(
    builder: DeploymentBuilder,
    bytes: &[u8],
) -> Result<DeploymentBuilder, ApiError> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|error| ApiError::bad_request(format!("problem reading zip input stream: {error}")))?;
    let mut builder = builder;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            ApiError::bad_request(format!("problem reading zip input stream: {error}"))
        })?;
        if entry.is_dir() {
            continue;
        }
        let entry_name = entry.name().to_string();
        let mut contents = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut contents).map_err(|error| {
            ApiError::bad_request(format!("problem reading zip input stream: {error}"))
        })?;
        builder = builder.add_bytes(entry_name, contents);
    }
    Ok(builder)
}

pub async fn get_deployment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(deployment_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let deployment = engine
        .get_repository_service()
        .get_deployment(&deployment_id)?;

    Ok(Json(deployment_response(deployment)))
}

pub async fn delete_deployment(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(deployment_id): Path<String>,
    Query(query): Query<DeleteDeploymentQuery>,
) -> Result<StatusCode, ApiError> {
    engine
        .get_repository_service()
        .delete_deployment_with_cascade(&deployment_id, query.cascade)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_deployment_resources(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(deployment_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let resources = engine
        .get_repository_service()
        .get_deployment_resources(&deployment_id)?;
    let response = resources
        .iter()
        .map(|resource| resource_response(&deployment_id, resource))
        .collect::<Vec<_>>();

    Ok(Json(json!(response)))
}

pub async fn get_deployment_resource(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let resource = engine
        .get_repository_service()
        .get_deployment_resource(&deployment_id, &resource_name)?;

    Ok(Json(resource_response(&deployment_id, &resource)))
}

pub async fn get_deployment_resource_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let resource = engine
        .get_repository_service()
        .get_deployment_resource(&deployment_id, &resource_name)?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, resource.content_type)],
        resource.bytes,
    )
        .into_response())
}
