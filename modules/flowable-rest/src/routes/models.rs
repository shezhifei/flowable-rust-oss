use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::Path,
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::repository::model::RepositoryModel;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const MODELS_PATH: &str = "/repository/models";
const MODEL_PATH: &str = "/repository/models/:model_id";
const MODEL_SOURCE_PATH: &str = "/repository/models/:model_id/source";
const MODEL_SOURCE_EXTRA_PATH: &str = "/repository/models/:model_id/source-extra";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{MODELS_PATH}"),
            get(list_models).post(create_model),
        )
        .route(
            &format!("{prefix}{MODEL_PATH}"),
            get(get_model).put(update_model).delete(delete_model),
        )
        .route(
            &format!("{prefix}{MODEL_SOURCE_PATH}"),
            get(get_model_source).put(update_model_source),
        )
        .route(
            &format!("{prefix}{MODEL_SOURCE_EXTRA_PATH}"),
            get(get_model_source_extra).put(update_model_source_extra),
        )
}

/// GET /repository/models — Java `ModelCollectionResource.java:93-143`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ModelListQuery {
    start: usize,
    size: Option<usize>,
    /// Java ModelCollectionResource.java:93
    id: Option<String>,
    key: Option<String>,
    name: Option<String>,
    /// Java ModelCollectionResource.java:108
    name_like: Option<String>,
    /// Java ModelCollectionResource.java:96
    category: Option<String>,
    /// Java ModelCollectionResource.java:99
    category_like: Option<String>,
    /// Java ModelCollectionResource.java:102
    category_not_equals: Option<String>,
    /// Java ModelCollectionResource.java:114
    version: Option<i32>,
    /// Java ModelCollectionResource.java:117-120
    latest_version: Option<bool>,
    deployment_id: Option<String>,
    /// Java ModelCollectionResource.java:126-131 — true: deploymentId not null;
    /// false: deploymentId is null.
    deployed: Option<bool>,
    tenant_id: Option<String>,
    /// Java ModelCollectionResource.java:137
    tenant_id_like: Option<String>,
    /// Java ModelCollectionResource.java:140-143
    without_tenant_id: Option<bool>,
}

impl ModelListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelResponse {
    id: String,
    name: Option<String>,
    key: String,
    category: Option<String>,
    version: i32,
    meta_info: Option<String>,
    deployment_id: Option<String>,
    tenant_id: Option<String>,
    create_time: Option<String>,
    last_update_time: Option<String>,
    url: String,
    source_url: String,
    source_extra_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelCreateRequest {
    name: Option<String>,
    key: Option<String>,
    category: Option<String>,
    version: Option<i32>,
    #[serde(rename = "metaInfo", alias = "meta_info")]
    meta_info: Option<String>,
    #[serde(rename = "deploymentId", alias = "deployment_id")]
    deployment_id: Option<String>,
    #[serde(rename = "tenantId", alias = "tenant_id")]
    tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelUpdateRequest {
    name: Option<String>,
    key: Option<String>,
    category: Option<String>,
    version: Option<i32>,
    #[serde(rename = "metaInfo", alias = "meta_info")]
    meta_info: Option<String>,
    #[serde(rename = "deploymentId", alias = "deployment_id")]
    deployment_id: Option<String>,
    #[serde(rename = "tenantId", alias = "tenant_id")]
    tenant_id: Option<String>,
}

fn to_model_response(model: RepositoryModel) -> ModelResponse {
    let create_time = millis_to_rfc3339(model.create_time);
    let last_update_time = millis_to_rfc3339(model.last_update_time);
    let id = model.id;
    ModelResponse {
        url: format!("/repository/models/{id}"),
        source_url: format!("/repository/models/{id}/source"),
        source_extra_url: format!("/repository/models/{id}/source-extra"),
        id,
        name: model.name,
        key: model.key,
        category: model.category,
        version: model.version,
        meta_info: model.meta_info,
        deployment_id: model.deployment_id,
        tenant_id: model.tenant_id,
        create_time,
        last_update_time,
    }
}

async fn create_model(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    body: Bytes,
) -> Result<(StatusCode, Json<ModelResponse>), ApiError> {
    let request: ModelCreateRequest = parse_json_body(&body)?;
    let key = required_non_blank(request.key, "key")?;
    let version = validate_version(request.version.unwrap_or(1))?;
    let model = RepositoryModel {
        id: String::new(),
        name: request.name,
        key,
        category: request.category,
        version,
        meta_info: request.meta_info,
        deployment_id: request.deployment_id,
        resource_name: None,
        process_definition_id: None,
        tenant_id: request.tenant_id,
        create_time: 0,
        last_update_time: 0,
        source_content_type: "application/octet-stream".to_string(),
        source_extra_content_type: "application/json".to_string(),
    };
    let model = engine
        .get_repository_service()
        .create_repository_model(model)?;
    Ok((StatusCode::CREATED, Json(to_model_response(model))))
}

async fn list_models(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ModelResponse>>, ApiError> {
    let query: ModelListQuery = parse_query(&uri)?;
    let mut models = engine.get_repository_service().get_repository_models()?;

    // P133: ModelCollectionResource.java:93-143
    if let Some(id) = query.id.as_deref() {
        models.retain(|model| model.id == id);
    }
    if let Some(key) = query.key.as_deref() {
        models.retain(|model| model.key == key);
    }
    if let Some(name) = query.name.as_deref() {
        models.retain(|model| model.name.as_deref() == Some(name));
    }
    if let Some(name_like) = query.name_like.as_deref() {
        models.retain(|model| {
            model
                .name
                .as_deref()
                .is_some_and(|name| sql_like_matches(name, name_like))
        });
    }
    if let Some(category) = query.category.as_deref() {
        models.retain(|model| model.category.as_deref() == Some(category));
    }
    if let Some(category_like) = query.category_like.as_deref() {
        models.retain(|model| {
            model
                .category
                .as_deref()
                .is_some_and(|category| sql_like_matches(category, category_like))
        });
    }
    if let Some(category_not_equals) = query.category_not_equals.as_deref() {
        models.retain(|model| model.category.as_deref() != Some(category_not_equals));
    }
    if let Some(version) = query.version {
        models.retain(|model| model.version == version);
    }
    if let Some(deployment_id) = query.deployment_id.as_deref() {
        models.retain(|model| model.deployment_id.as_deref() == Some(deployment_id));
    }
    if let Some(deployed) = query.deployed {
        // Java ModelCollectionResource.java:126-131
        if deployed {
            models.retain(|model| model.deployment_id.is_some());
        } else {
            models.retain(|model| model.deployment_id.is_none());
        }
    }
    if query.without_tenant_id == Some(true) {
        models.retain(|model| {
            model
                .tenant_id
                .as_deref()
                .is_none_or(|tenant_id| tenant_id.is_empty())
        });
    } else if let Some(tenant_id) = query.tenant_id.as_deref() {
        models.retain(|model| model.tenant_id.as_deref().unwrap_or("") == tenant_id);
    } else if let Some(tenant_id_like) = query.tenant_id_like.as_deref() {
        models.retain(|model| {
            model
                .tenant_id
                .as_deref()
                .is_some_and(|tenant_id| sql_like_matches(tenant_id, tenant_id_like))
        });
    }
    if query.latest_version == Some(true) {
        // Java ModelQueryImpl.latestVersion: highest version per key.
        models = retain_latest_model_versions(models);
    }

    let data = models.into_iter().map(to_model_response).collect();
    Ok(Json(query.paging().paginate(data)))
}

/// Keep the highest-version model per `key` (Java model latestVersion).
fn retain_latest_model_versions(models: Vec<RepositoryModel>) -> Vec<RepositoryModel> {
    use std::collections::HashMap;
    let mut best: HashMap<String, RepositoryModel> = HashMap::new();
    for model in models {
        match best.get(&model.key) {
            Some(existing) if existing.version >= model.version => {}
            _ => {
                best.insert(model.key.clone(), model);
            }
        }
    }
    let mut out: Vec<RepositoryModel> = best.into_values().collect();
    out.sort_by(|a, b| a.key.cmp(&b.key).then(a.id.cmp(&b.id)));
    out
}

fn sql_like_matches(value: &str, pattern: &str) -> bool {
    crate::routes::tasks::sql_like_matches(pattern, value)
}

async fn get_model(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<Json<ModelResponse>, ApiError> {
    let model = engine
        .get_repository_service()
        .get_repository_model(&model_id)?;
    Ok(Json(to_model_response(model)))
}

async fn update_model(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
    body: Bytes,
) -> Result<Json<ModelResponse>, ApiError> {
    let request: ModelUpdateRequest = parse_json_body(&body)?;
    let service = engine.get_repository_service();
    let mut model = service.get_repository_model(&model_id)?;

    if let Some(name) = request.name {
        model.name = Some(name);
    }
    if let Some(key) = request.key {
        model.key = required_non_blank(Some(key), "key")?;
    }
    if let Some(category) = request.category {
        model.category = Some(category);
    }
    if let Some(version) = request.version {
        model.version = validate_version(version)?;
    }
    if let Some(meta_info) = request.meta_info {
        model.meta_info = Some(meta_info);
    }
    if let Some(deployment_id) = request.deployment_id {
        model.deployment_id = Some(deployment_id);
    }
    if let Some(tenant_id) = request.tenant_id {
        model.tenant_id = Some(tenant_id);
    }

    let model = service.update_repository_model(model)?;
    Ok(Json(to_model_response(model)))
}

async fn delete_model(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .get_repository_service()
        .delete_repository_model(&model_id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_model_source(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<Response, ApiError> {
    let source = engine
        .get_repository_service()
        .get_repository_model_source(&model_id)?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, source.content_type)],
        source.bytes,
    )
        .into_response())
}

async fn update_model_source(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let content_type = request_content_type(&headers, "application/octet-stream")?;
    engine
        .get_repository_service()
        .update_repository_model_source(&model_id, content_type, body.to_vec())?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_model_source_extra(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
) -> Result<Response, ApiError> {
    let source_extra = engine
        .get_repository_service()
        .get_repository_model_source_extra(&model_id)?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, source_extra.content_type)],
        source_extra.bytes,
    )
        .into_response())
}

async fn update_model_source_extra(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(model_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let content_type = request_content_type(&headers, "application/json")?;
    engine
        .get_repository_service()
        .update_repository_model_source_extra(&model_id, content_type, body.to_vec())?;
    Ok(StatusCode::NO_CONTENT)
}

fn millis_to_rfc3339(value: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(value).map(|time| time.to_rfc3339())
}

fn parse_json_body<T>(body: &Bytes) -> Result<T, ApiError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(body).map_err(|error| ApiError::bad_request(error.to_string()))
}

fn request_content_type(
    headers: &HeaderMap,
    default_content_type: &str,
) -> Result<String, ApiError> {
    headers
        .get(header::CONTENT_TYPE)
        .map(|value| {
            value
                .to_str()
                .map(|value| value.to_string())
                .map_err(|error| ApiError::bad_request(error.to_string()))
        })
        .unwrap_or_else(|| Ok(default_content_type.to_string()))
}

fn required_non_blank(value: Option<String>, field_name: &str) -> Result<String, ApiError> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(ApiError::bad_request(format!("{field_name} is required"))),
    }
}

fn validate_version(version: i32) -> Result<i32, ApiError> {
    if version < 1 {
        return Err(ApiError::bad_request("version must be greater than zero"));
    }
    Ok(version)
}
