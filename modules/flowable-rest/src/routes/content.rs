use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::Path,
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use flowable_content_service::ContentObjectStorageMetadata;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub type DynContentService = Arc<dyn ContentServiceApi>;

const CONTENT_ITEMS_PATH: &str = "/content-service/content-items";
const CONTENT_ITEM_PATH: &str = "/content-service/content-items/:content_item_id";
const CONTENT_ITEM_DATA_PATH: &str = "/content-service/content-items/:content_item_id/data";
const CONTENT_ITEM_OBJECT_PATH: &str = "/content-service/content-items/:content_item_id/object";
const CONTENT_ITEM_OBJECT_DATA_PATH: &str =
    "/content-service/content-items/:content_item_id/object/data";
const STORAGE_STATUS_PATH: &str = "/content-service/storage/status";

pub trait ContentServiceApi: Send + Sync {
    /// Create a content item. `authenticated_user_id` is the trusted request
    /// principal (HTTP Basic auth); implementations derive tenant ownership
    /// from it — the request body cannot carry a tenant.
    fn create_content_item(
        &self,
        command: ContentItemCreateCommand,
        authenticated_user_id: Option<&str>,
    ) -> Result<ContentItemRecord, ApiError>;
    fn list_content_items(
        &self,
        query: ContentItemQuery,
    ) -> Result<PagedResponse<ContentItemRecord>, ApiError>;
    fn get_content_item(&self, content_item_id: &str) -> Result<ContentItemRecord, ApiError>;
    fn get_content_item_data(
        &self,
        content_item_id: &str,
    ) -> Result<ContentItemDataRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "Content item data for '{}' was not found",
            content_item_id
        )))
    }
    fn delete_content_item(&self, content_item_id: &str) -> Result<(), ApiError>;
    /// M41: Get content object storage metadata.
    fn get_content_item_object_metadata(
        &self,
        content_item_id: &str,
    ) -> Result<ContentObjectStorageMetadata, ApiError>;
    /// M41: Get content object raw data (binary).
    fn get_content_item_object_data(
        &self,
        content_item_id: &str,
    ) -> Result<ContentItemDataRecord, ApiError> {
        self.get_content_item_data(content_item_id)
    }
    /// M41: Get storage backend status.
    fn get_storage_status(&self) -> Result<Value, ApiError>;

    // --- Task attachment (Java CreateAttachmentCmd / DeleteAttachmentCmd) ---
    // Defaults keep Content Service mock tests compiling; production adapter overrides.

    /// Atomic create: content item + AddAttachment event in one engine session.
    #[allow(clippy::too_many_arguments)]
    fn create_task_attachment(
        &self,
        _task_id: String,
        _name: String,
        _description: Option<String>,
        _attachment_type: Option<String>,
        _external_url: Option<String>,
        _content: Option<Vec<u8>>,
        _user_id: Option<String>,
        _process_instance_id: Option<String>,
    ) -> Result<TaskAttachmentRecord, ApiError> {
        Err(ApiError::InternalServerError(
            "Task attachment API is not implemented by this content service".to_string(),
        ))
    }

    fn list_task_attachments(&self, _task_id: &str) -> Result<Vec<TaskAttachmentRecord>, ApiError> {
        Ok(Vec::new())
    }

    fn get_task_attachment(
        &self,
        task_id: &str,
        attachment_id: &str,
    ) -> Result<TaskAttachmentRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "Task '{}' does not have an attachment with id '{}'.",
            task_id, attachment_id
        )))
    }

    fn get_task_attachment_content(
        &self,
        task_id: &str,
        attachment_id: &str,
    ) -> Result<TaskAttachmentContentRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "Task '{}' does not have an attachment with id '{}'.",
            task_id, attachment_id
        )))
    }

    fn delete_task_attachment(
        &self,
        task_id: &str,
        attachment_id: &str,
        _user_id: Option<&str>,
    ) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "Task '{}' does not have an attachment with id '{}'.",
            task_id, attachment_id
        )))
    }

    // --- Process attachment (Java processInstanceId createAttachment variants) ---
    // REST collection is a Rust extension; Java has TaskService APIs only.

    #[allow(clippy::too_many_arguments)]
    fn create_process_attachment(
        &self,
        _process_instance_id: String,
        _task_id: Option<String>,
        _name: String,
        _description: Option<String>,
        _attachment_type: Option<String>,
        _external_url: Option<String>,
        _content: Option<Vec<u8>>,
        _user_id: Option<String>,
    ) -> Result<TaskAttachmentRecord, ApiError> {
        Err(ApiError::InternalServerError(
            "Process attachment API is not implemented by this content service".to_string(),
        ))
    }

    fn list_process_attachments(
        &self,
        _process_instance_id: &str,
    ) -> Result<Vec<TaskAttachmentRecord>, ApiError> {
        Ok(Vec::new())
    }

    fn get_process_attachment(
        &self,
        process_instance_id: &str,
        attachment_id: &str,
    ) -> Result<TaskAttachmentRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "Process instance '{}' does not have an attachment with id '{}'.",
            process_instance_id, attachment_id
        )))
    }

    fn get_process_attachment_content(
        &self,
        process_instance_id: &str,
        attachment_id: &str,
    ) -> Result<TaskAttachmentContentRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "Process instance '{}' does not have an attachment with id '{}'.",
            process_instance_id, attachment_id
        )))
    }

    fn delete_process_attachment(
        &self,
        process_instance_id: &str,
        attachment_id: &str,
        _user_id: Option<&str>,
    ) -> Result<(), ApiError> {
        Err(ApiError::NotFound(format!(
            "Process instance '{}' does not have an attachment with id '{}'.",
            process_instance_id, attachment_id
        )))
    }
}

/// Attachment-oriented view of a content item (Java AttachmentResponse fields).
#[derive(Debug, Clone)]
pub struct TaskAttachmentRecord {
    pub id: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub description: Option<String>,
    pub attachment_type: Option<String>,
    pub external_url: Option<String>,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub user_id: Option<String>,
    pub content_size: usize,
    pub created: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TaskAttachmentContentRecord {
    pub bytes: Vec<u8>,
    pub mime_type: Option<String>,
    pub attachment_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContentItemCreateCommand {
    pub name: String,
    pub mime_type: Option<String>,
    pub description: Option<String>,
    pub attachment_type: Option<String>,
    pub external_url: Option<String>,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub content: Option<String>,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct ContentItemQuery {
    pub paging: PagingQuery,
    pub name: Option<String>,
    pub mime_type: Option<String>,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub sort: Option<ContentItemSort>,
    pub order: Option<SortOrder>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentItemSort {
    Created,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentItemRecord {
    pub id: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub description: Option<String>,
    pub attachment_type: Option<String>,
    pub external_url: Option<String>,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub created: i64,
    pub modified: i64,
    pub content_size: usize,
}

#[derive(Debug, Clone)]
pub struct ContentItemDataRecord {
    pub mime_type: Option<String>,
    pub content: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContentItemCreateRequest {
    name: String,
    mime_type: Option<String>,
    description: Option<String>,
    attachment_type: Option<String>,
    external_url: Option<String>,
    task_id: Option<String>,
    process_instance_id: Option<String>,
    scope_type: Option<String>,
    scope_id: Option<String>,
    content: String,
    #[serde(default)]
    expires_in_seconds: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct ContentItemQueryParams {
    start: usize,
    size: Option<usize>,
    name: Option<String>,
    mime_type: Option<String>,
    task_id: Option<String>,
    process_instance_id: Option<String>,
    scope_type: Option<String>,
    scope_id: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

impl From<ContentItemCreateRequest> for ContentItemCreateCommand {
    fn from(value: ContentItemCreateRequest) -> Self {
        Self {
            name: value.name,
            mime_type: value.mime_type,
            description: value.description,
            attachment_type: value.attachment_type,
            external_url: value.external_url,
            task_id: value.task_id,
            process_instance_id: value.process_instance_id,
            scope_type: value.scope_type,
            scope_id: value.scope_id,
            content: Some(value.content),
            expires_in_seconds: value.expires_in_seconds,
        }
    }
}

impl TryFrom<ContentItemQueryParams> for ContentItemQuery {
    type Error = ApiError;

    fn try_from(value: ContentItemQueryParams) -> Result<Self, Self::Error> {
        let sort = match value.sort.as_deref() {
            None => None,
            Some("created" | "createdDate") => Some(ContentItemSort::Created),
            Some(other) => {
                return Err(ApiError::bad_request(format!(
                    "Unsupported content item sort property '{other}'. Supported sort properties: created"
                )));
            }
        };
        let order = match value.order.as_deref() {
            None => None,
            Some("asc") => Some(SortOrder::Asc),
            Some("desc") => Some(SortOrder::Desc),
            Some(other) => {
                return Err(ApiError::bad_request(format!(
                    "Unsupported content item sort order '{other}'. Supported orders: asc, desc"
                )));
            }
        };

        Ok(Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            name: value.name,
            mime_type: value.mime_type,
            task_id: value.task_id,
            process_instance_id: value.process_instance_id,
            scope_type: value.scope_type,
            scope_id: value.scope_id,
            sort,
            order,
        })
    }
}

pub fn router(service: DynContentService) -> Router {
    router_with_prefix("", service)
}

fn router_with_prefix(prefix: &str, service: DynContentService) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{CONTENT_ITEMS_PATH}"),
            post(create).get(list_content_items),
        )
        .route(
            &format!("{prefix}{CONTENT_ITEM_PATH}"),
            get(get_content_item).delete(delete_content_item),
        )
        .route(
            &format!("{prefix}{CONTENT_ITEM_DATA_PATH}"),
            get(get_content_item_data),
        )
        .route(
            &format!("{prefix}{CONTENT_ITEM_OBJECT_PATH}"),
            get(get_content_item_object_metadata),
        )
        .route(
            &format!("{prefix}{CONTENT_ITEM_OBJECT_DATA_PATH}"),
            get(get_content_item_object_data),
        )
        .route(
            &format!("{prefix}{STORAGE_STATUS_PATH}"),
            get(get_storage_status),
        )
        .layer(Extension(service))
}

pub async fn create(
    Extension(service): Extension<DynContentService>,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let payload: ContentItemCreateRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let authenticated_user_id = user_id_from_basic_auth(&headers);
    let item = service.create_content_item(payload.into(), authenticated_user_id.as_deref())?;
    Ok((StatusCode::CREATED, Json(item)))
}

/// Extracts the request principal from an HTTP Basic `Authorization` header.
/// Mirrors `routes::tasks::user_id_from_basic_auth`: the REST layer
/// authenticates via Basic auth, so the username is the trusted principal.
fn user_id_from_basic_auth(headers: &HeaderMap) -> Option<String> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())?;
    let encoded = auth_header.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let user_id = decoded.split_once(':')?.0.trim();
    if user_id.is_empty() {
        None
    } else {
        Some(user_id.to_string())
    }
}

pub async fn list_content_items(
    Extension(service): Extension<DynContentService>,
    uri: Uri,
) -> Result<Json<PagedResponse<ContentItemRecord>>, ApiError> {
    let query: ContentItemQueryParams = parse_query(&uri)?;
    Ok(Json(service.list_content_items(query.try_into()?)?))
}

pub async fn get_content_item(
    Extension(service): Extension<DynContentService>,
    Path(content_item_id): Path<String>,
) -> Result<Json<ContentItemRecord>, ApiError> {
    Ok(Json(service.get_content_item(&content_item_id)?))
}

fn parse_range(range_header: &str, content_len: usize) -> Option<(usize, usize)> {
    if !range_header.starts_with("bytes=") {
        return None;
    }
    let range_str = &range_header[6..];
    let parts: Vec<&str> = range_str.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
    let start_str = parts[0].trim();
    let end_str = parts[1].trim();

    if start_str.is_empty() && end_str.is_empty() {
        return None;
    }

    if start_str.is_empty() {
        let end_val = end_str.parse::<usize>().ok()?;
        if content_len == 0 {
            // Suffix range on an empty body is unsatisfiable; return None
            // instead of underflowing `content_len - 1` (0usize - 1 wraps).
            return None;
        }
        if end_val >= content_len {
            Some((0, content_len - 1))
        } else {
            Some((content_len - end_val, content_len - 1))
        }
    } else if end_str.is_empty() {
        let start_val = start_str.parse::<usize>().ok()?;
        if start_val >= content_len {
            None
        } else {
            Some((start_val, content_len - 1))
        }
    } else {
        let start_val = start_str.parse::<usize>().ok()?;
        let mut end_val = end_str.parse::<usize>().ok()?;
        if start_val >= content_len {
            return None;
        }
        if end_val >= content_len {
            end_val = content_len - 1;
        }
        if start_val > end_val {
            return None;
        }
        Some((start_val, end_val))
    }
}

pub async fn get_content_item_data(
    Extension(service): Extension<DynContentService>,
    headers: axum::http::HeaderMap,
    Path(content_item_id): Path<String>,
) -> Result<Response, ApiError> {
    let record = service.get_content_item_data(&content_item_id)?;
    let total_len = record.content.len();
    let content_type = record
        .mime_type
        .unwrap_or_else(|| "application/octet-stream".to_string());

    if let Some(range_header) = headers.get(header::RANGE).and_then(|h| h.to_str().ok()) {
        if let Some((start, end)) = parse_range(range_header, total_len) {
            let slice = &record.content[start..=end];
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {}-{}/{}", start, end, total_len),
                )
                .header(header::CONTENT_LENGTH, slice.len())
                .body(Body::from(slice.to_vec()))
                .map_err(|err| ApiError::InternalServerError(err.to_string()))
        } else {
            Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{}", total_len))
                .body(Body::empty())
                .map_err(|err| ApiError::InternalServerError(err.to_string()))
        }
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_LENGTH, total_len)
            .body(Body::from(record.content))
            .map_err(|err| ApiError::InternalServerError(err.to_string()))
    }
}

pub async fn delete_content_item(
    Extension(service): Extension<DynContentService>,
    Path(content_item_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    service.delete_content_item(&content_item_id)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── M41: Breadth endpoints ────────────────────────────────────────────

/// GET /content-service/content-items/{id}/object
pub async fn get_content_item_object_metadata(
    Extension(service): Extension<DynContentService>,
    Path(content_item_id): Path<String>,
) -> Result<Json<ContentObjectStorageMetadata>, ApiError> {
    Ok(Json(
        service.get_content_item_object_metadata(&content_item_id)?,
    ))
}

/// GET /content-service/content-items/{id}/object/data
pub async fn get_content_item_object_data(
    Extension(service): Extension<DynContentService>,
    headers: axum::http::HeaderMap,
    Path(content_item_id): Path<String>,
) -> Result<Response, ApiError> {
    let record = service.get_content_item_object_data(&content_item_id)?;
    let total_len = record.content.len();
    let content_type = record
        .mime_type
        .unwrap_or_else(|| "application/octet-stream".to_string());

    if let Some(range_header) = headers.get(header::RANGE).and_then(|h| h.to_str().ok()) {
        if let Some((start, end)) = parse_range(range_header, total_len) {
            let slice = &record.content[start..=end];
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {}-{}/{}", start, end, total_len),
                )
                .header(header::CONTENT_LENGTH, slice.len())
                .body(Body::from(slice.to_vec()))
                .map_err(|err| ApiError::InternalServerError(err.to_string()))
        } else {
            Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{}", total_len))
                .body(Body::empty())
                .map_err(|err| ApiError::InternalServerError(err.to_string()))
        }
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_LENGTH, total_len)
            .body(Body::from(record.content))
            .map_err(|err| ApiError::InternalServerError(err.to_string()))
    }
}

/// GET /content-service/storage/status
pub async fn get_storage_status(
    Extension(service): Extension<DynContentService>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(service.get_storage_status()?))
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn suffix_range_on_empty_content_is_unsatisfiable_not_underflow() {
        assert_eq!(parse_range("bytes=-5", 0), None);
    }

    #[test]
    fn suffix_range_clamps_to_content_length() {
        assert_eq!(parse_range("bytes=-5", 10), Some((5, 9)));
        assert_eq!(parse_range("bytes=-99", 10), Some((0, 9)));
        assert_eq!(parse_range("bytes=-10", 10), Some((0, 9)));
    }

    #[test]
    fn open_ranges_are_unsatisfiable_on_empty_content() {
        assert_eq!(parse_range("bytes=0-", 0), None);
        assert_eq!(parse_range("bytes=0-5", 0), None);
        assert_eq!(parse_range("bytes=-", 0), None);
    }

    #[test]
    fn prefix_and_bounded_ranges_still_parse() {
        assert_eq!(parse_range("bytes=0-4", 10), Some((0, 4)));
        assert_eq!(parse_range("bytes=0-99", 10), Some((0, 9)));
        assert_eq!(parse_range("bytes=0-", 10), Some((0, 9)));
    }

    #[test]
    fn malformed_ranges_are_rejected() {
        assert_eq!(parse_range("bytes=abc", 10), None);
        assert_eq!(parse_range("items=0-5", 10), None);
        assert_eq!(parse_range("bytes=5-2", 10), None);
    }
}
