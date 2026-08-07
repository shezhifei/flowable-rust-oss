use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use axum::extract::Query as AxumQuery;
use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, LocalResult, TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::{BatchEntity, BatchPartEntity};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type EngineState = Extension<Arc<ProcessEngine>>;

const BATCHES_PATH: &str = "/management/batches";
const BATCH_PATH: &str = "/management/batches/:batch_id";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{BATCHES_PATH}"),
            post(create_batch).get(list_batches),
        )
        .route(
            &format!("{prefix}{BATCH_PATH}"),
            get(get_batch).delete(delete_batch),
        )
        .route(
            &format!("{prefix}{BATCH_PATH}/batch-document"),
            get(get_batch_document),
        )
        .route(
            &format!("{prefix}{BATCH_PATH}/batch-parts"),
            get(list_batch_parts),
        )
        .route(
            &format!("{prefix}/management/batch-parts/:batch_part_id"),
            get(get_batch_part),
        )
        .route(
            &format!("{prefix}/management/batch-parts/:batch_part_id/batch-part-document"),
            get(get_batch_part_document),
        )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchResponse {
    pub id: String,
    pub url: String,
    pub batch_type: String,
    pub search_key: Option<String>,
    pub search_key2: Option<String>,
    pub status: String,
    pub total_items: i64,
    pub items_processed: i64,
    #[serde(serialize_with = "serialize_batch_timestamp")]
    pub create_time: u64,
    #[serde(serialize_with = "serialize_option_batch_timestamp")]
    pub complete_time: Option<u64>,
    #[serde(serialize_with = "serialize_option_batch_timestamp")]
    pub end_time: Option<u64>,
    pub tenant_id: Option<String>,
}

impl From<BatchEntity> for BatchResponse {
    fn from(b: BatchEntity) -> Self {
        Self {
            url: format!("/management/batches/{}", b.id),
            id: b.id,
            batch_type: b.batch_type,
            search_key: b.search_key,
            search_key2: b.search_key2,
            status: b.status,
            total_items: b.total_items,
            items_processed: b.items_processed,
            create_time: b.create_time,
            complete_time: b.end_time,
            end_time: b.end_time,
            tenant_id: b.tenant_id,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct BatchQueryParams {
    pub start: usize,
    pub size: Option<usize>,
    pub id: Option<String>,
    pub batch_type: Option<String>,
    pub search_key: Option<String>,
    pub search_key2: Option<String>,
    pub create_time_before: Option<String>,
    pub create_time_after: Option<String>,
    pub complete_time_before: Option<String>,
    pub complete_time_after: Option<String>,
    pub status: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub without_tenant_id: Option<bool>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

impl BatchQueryParams {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchPartResponse {
    pub id: String,
    pub url: String,
    pub batch_id: String,
    pub batch_url: String,
    pub batch_type: String,
    pub search_key: Option<String>,
    pub search_key2: Option<String>,
    pub scope_id: Option<String>,
    pub sub_scope_id: Option<String>,
    pub scope_type: Option<String>,
    #[serde(serialize_with = "serialize_batch_timestamp")]
    pub create_time: u64,
    #[serde(serialize_with = "serialize_option_batch_timestamp")]
    pub complete_time: Option<u64>,
    pub status: String,
    pub tenant_id: Option<String>,
}

impl From<BatchPartEntity> for BatchPartResponse {
    fn from(part: BatchPartEntity) -> Self {
        Self {
            url: format!("/management/batch-parts/{}", part.id),
            batch_url: format!("/management/batches/{}", part.batch_id),
            id: part.id,
            batch_id: part.batch_id,
            batch_type: part.batch_type,
            search_key: part.search_key,
            search_key2: part.search_key2,
            scope_id: part.scope_id,
            sub_scope_id: part.sub_scope_id,
            scope_type: part.scope_type,
            create_time: part.create_time,
            complete_time: part.complete_time,
            status: part.status,
            tenant_id: part.tenant_id,
        }
    }
}

#[derive(Deserialize)]
pub struct BatchPartQueryParams {
    pub status: Option<String>,
}

pub async fn list_batches(
    engine: EngineState,
    uri: Uri,
) -> Result<Json<PagedResponse<BatchResponse>>, ApiError> {
    let params: BatchQueryParams = parse_query(&uri)?;
    let service = engine.0.get_batch_service();
    let mut query = service.create_batch_query();
    if let Some(batch_type) = &params.batch_type {
        query = query.batch_type(batch_type.clone());
    }
    if let Some(status) = &params.status {
        query = query.status(status.clone());
    }
    if let Some(tenant_id) = &params.tenant_id {
        query = query.tenant_id(tenant_id.clone());
    }
    if let Some(tenant_id_like) = &params.tenant_id_like {
        query = query.tenant_id_like(tenant_id_like.clone());
    }
    if params.without_tenant_id.unwrap_or(false) {
        query = query.without_tenant_id();
    }
    let mut batches = query
        .list()
        .map_err(|e| ApiError::InternalServerError(format!("Batch query failed: {}", e)))?;
    filter_and_sort_batches(&mut batches, &params)?;
    Ok(Json(params.paging().paginate(
        batches.into_iter().map(BatchResponse::from).collect(),
    )))
}

pub async fn get_batch(
    engine: EngineState,
    Path(batch_id): Path<String>,
) -> Result<Json<BatchResponse>, ApiError> {
    let batch = engine.0.get_batch_service().find_batch_by_id(&batch_id);
    match batch {
        Some(b) => Ok(Json(BatchResponse::from(b))),
        None => Err(ApiError::NotFound(format!(
            "Batch '{}' not found",
            batch_id
        ))),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateBatchRequest {
    pub id: String,
    pub batch_type: String,
    pub search_key: Option<String>,
    pub search_key2: Option<String>,
    pub status: String,
    pub total_items: i64,
    pub items_processed: i64,
    pub create_time: u64,
    pub end_time: Option<u64>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    pub batch_document_json: Option<String>,
}

pub async fn create_batch(
    engine: EngineState,
    Json(req): Json<CreateBatchRequest>,
) -> Result<Json<BatchResponse>, ApiError> {
    let batch = BatchEntity {
        id: req.id.clone(),
        batch_type: req.batch_type.clone(),
        search_key: req.search_key.clone(),
        search_key2: req.search_key2.clone(),
        status: req.status.clone(),
        total_items: req.total_items,
        items_processed: req.items_processed,
        create_time: req.create_time,
        end_time: req.end_time,
        tenant_id: req.tenant_id.clone(),
        batch_document_json: req.batch_document_json.clone(),
    };
    engine.0.get_batch_service().create_batch(batch.clone());
    Ok(Json(BatchResponse::from(batch)))
}

pub async fn delete_batch(
    engine: EngineState,
    Path(batch_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine.0.get_batch_service().delete_batch(&batch_id);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_batch_document(
    engine: EngineState,
    Path(batch_id): Path<String>,
) -> Result<Response, ApiError> {
    let batch = engine
        .0
        .get_batch_service()
        .find_batch_by_id(&batch_id)
        .ok_or_else(|| ApiError::NotFound(format!("Batch '{}' not found", batch_id)))?;
    json_document_response(
        batch.batch_document_json,
        format!(
            "Batch with id '{}' does not have a batch document.",
            batch.id
        ),
    )
}

pub async fn list_batch_parts(
    engine: EngineState,
    Path(batch_id): Path<String>,
    params: AxumQuery<BatchPartQueryParams>,
) -> Result<Json<Vec<BatchPartResponse>>, ApiError> {
    let service = engine.0.get_batch_service();
    if service.find_batch_by_id(&batch_id).is_none() {
        return Err(ApiError::NotFound(format!(
            "No batch found for id {}",
            batch_id
        )));
    }

    let mut batch_parts = if let Some(status) = &params.status {
        service.find_batch_parts_by_batch_id_and_status(&batch_id, status)
    } else {
        service.find_batch_parts_by_batch_id(&batch_id)
    };
    batch_parts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(Json(
        batch_parts
            .into_iter()
            .map(BatchPartResponse::from)
            .collect(),
    ))
}

pub async fn get_batch_part(
    engine: EngineState,
    Path(batch_part_id): Path<String>,
) -> Result<Json<BatchPartResponse>, ApiError> {
    engine
        .0
        .get_batch_service()
        .find_batch_part_by_id(&batch_part_id)
        .map(BatchPartResponse::from)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("Batch part '{}' not found", batch_part_id)))
}

pub async fn get_batch_part_document(
    engine: EngineState,
    Path(batch_part_id): Path<String>,
) -> Result<Response, ApiError> {
    let batch_part = engine
        .0
        .get_batch_service()
        .find_batch_part_by_id(&batch_part_id)
        .ok_or_else(|| ApiError::NotFound(format!("Batch part '{}' not found", batch_part_id)))?;
    json_document_response(
        batch_part.batch_part_document_json,
        format!(
            "Batch part with id '{}' does not have a batch part document.",
            batch_part.id
        ),
    )
}

fn json_document_response(
    document: Option<String>,
    not_found_message: String,
) -> Result<Response, ApiError> {
    document
        .filter(|value| !value.is_empty())
        .map(|value| ([(header::CONTENT_TYPE, "application/json")], value).into_response())
        .ok_or(ApiError::NotFound(not_found_message))
}

fn filter_and_sort_batches(
    batches: &mut Vec<BatchEntity>,
    query: &BatchQueryParams,
) -> Result<(), ApiError> {
    let create_time_before = query
        .create_time_before
        .as_deref()
        .map(|value| parse_batch_date_millis("createTimeBefore", value))
        .transpose()?;
    let create_time_after = query
        .create_time_after
        .as_deref()
        .map(|value| parse_batch_date_millis("createTimeAfter", value))
        .transpose()?;
    let complete_time_before = query
        .complete_time_before
        .as_deref()
        .map(|value| parse_batch_date_millis("completeTimeBefore", value))
        .transpose()?;
    let complete_time_after = query
        .complete_time_after
        .as_deref()
        .map(|value| parse_batch_date_millis("completeTimeAfter", value))
        .transpose()?;

    batches.retain(|batch| {
        query.id.as_deref().is_none_or(|id| batch.id == id)
            && query
                .batch_type
                .as_deref()
                .is_none_or(|batch_type| batch.batch_type == batch_type)
            && query
                .search_key
                .as_deref()
                .is_none_or(|search_key| batch.search_key.as_deref() == Some(search_key))
            && query
                .search_key2
                .as_deref()
                .is_none_or(|search_key2| batch.search_key2.as_deref() == Some(search_key2))
            && query
                .status
                .as_deref()
                .is_none_or(|status| batch.status == status)
            && create_time_before.is_none_or(|limit| (batch.create_time as i64) < limit)
            && create_time_after.is_none_or(|limit| (batch.create_time as i64) > limit)
            && complete_time_before.is_none_or(|limit| {
                batch
                    .end_time
                    .is_some_and(|end_time| (end_time as i64) < limit)
            })
            && complete_time_after.is_none_or(|limit| {
                batch
                    .end_time
                    .is_some_and(|end_time| (end_time as i64) > limit)
            })
            && query
                .tenant_id
                .as_deref()
                .is_none_or(|tenant_id| batch.tenant_id.as_deref() == Some(tenant_id))
            && query
                .tenant_id_like
                .as_deref()
                .is_none_or(|tenant_id_like| {
                    batch
                        .tenant_id
                        .as_deref()
                        .is_some_and(|tenant_id| tenant_id.contains(tenant_id_like))
                })
            && (!query.without_tenant_id.unwrap_or(false)
                || batch.tenant_id.as_deref().is_none_or(str::is_empty))
    });

    sort_batches(batches, query)
}

fn sort_batches(batches: &mut [BatchEntity], query: &BatchQueryParams) -> Result<(), ApiError> {
    let sort = query.sort.as_deref().unwrap_or("id");
    batches.sort_by(|left, right| {
        let ordering = match sort {
            "id" => left.id.cmp(&right.id),
            "batchType" => left.batch_type.cmp(&right.batch_type),
            "searchKey" => left.search_key.cmp(&right.search_key),
            "searchKey2" => left.search_key2.cmp(&right.search_key2),
            "createTime" => left.create_time.cmp(&right.create_time),
            "completeTime" => left.end_time.cmp(&right.end_time),
            "status" => left.status.cmp(&right.status),
            "tenantId" => left.tenant_id.cmp(&right.tenant_id),
            _ => std::cmp::Ordering::Equal,
        };
        if matches!(query.order.as_deref(), Some("desc")) {
            ordering.reverse()
        } else {
            ordering
        }
    });

    match sort {
        "id" | "batchType" | "searchKey" | "searchKey2" | "createTime" | "completeTime"
        | "status" | "tenantId" => Ok(()),
        other => Err(ApiError::bad_request(format!(
            "Unsupported sort property '{other}' for management batches. Supported sort properties: id, batchType, searchKey, searchKey2, createTime, completeTime, status, tenantId"
        ))),
    }?;

    match query.order.as_deref() {
        None | Some("asc") | Some("desc") => Ok(()),
        Some(order) => Err(ApiError::bad_request(format!(
            "Unsupported order '{order}' for management batches. Supported orders: asc, desc"
        ))),
    }
}

fn serialize_batch_timestamp<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let value =
        i64::try_from(*value).map_err(|_| serde::ser::Error::custom("invalid batch timestamp"))?;
    let dt = match Utc.timestamp_millis_opt(value) {
        LocalResult::Single(dt) => dt,
        _ => return Err(serde::ser::Error::custom("invalid batch timestamp")),
    };
    serializer.serialize_str(&dt.to_rfc3339())
}

fn serialize_option_batch_timestamp<S>(
    value: &Option<u64>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(value) => serialize_batch_timestamp(value, serializer),
        None => serializer.serialize_none(),
    }
}

fn parse_batch_date_millis(field: &str, value: &str) -> Result<i64, ApiError> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.timestamp_millis())
        .map_err(|_| ApiError::bad_request(format!("Invalid date-time value for '{field}'")))
}
