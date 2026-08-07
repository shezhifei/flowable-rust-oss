//! Shared attachment REST helpers for task and process-instance routes.
//!
//! Java exposes AttachmentResponse via RestResponseFactory for task attachments.
//! Process-instance collection endpoints are a Rust extension (Java TaskService
//! has processInstanceId APIs but no BPMN REST collection equivalent).

use crate::error::ApiError;
use axum::{
    body::Bytes,
    extract::{FromRequest, Multipart, Request},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::content::TaskAttachmentRecord;

// --- P142c resource limits -------------------------------------------------
// axum `DefaultBodyLimit` does not apply to Multipart extractors. Fixed consts
// (not config) so P142a can own config.rs without coupling.

/// Single file part cap (64 MiB).
const MAX_MULTIPART_FILE_BYTES: usize = 64 * 1024 * 1024;
/// Cumulative bytes across all parts of one multipart request (256 MiB).
const MAX_MULTIPART_REQUEST_BYTES: usize = 256 * 1024 * 1024;
/// Text form fields (name/description/type) stay small; still stream-counted.
const MAX_MULTIPART_TEXT_FIELD_BYTES: usize = 1024 * 1024;

/// Java `AttachmentRequest` (+ optional Rust `content` extension for JSON binary).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateAttachmentRequest {
    /// Required by Java (`Attachment name is required.`); Option so missing → 400.
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub attachment_type: Option<String>,
    /// Java field `externalUrl` for link attachments.
    pub external_url: Option<String>,
    /// Rust extension: JSON body may carry string content without multipart.
    pub content: Option<String>,
}

/// Java `AttachmentResponse` fields (RestResponseFactory.createAttachmentResponse).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentResponse {
    pub id: String,
    pub url: String,
    pub name: String,
    pub user_id: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub attachment_type: Option<String>,
    pub task_url: Option<String>,
    pub process_instance_url: Option<String>,
    pub external_url: Option<String>,
    pub content_url: Option<String>,
    pub time: Option<String>,
}

pub(crate) struct ParsedAttachmentCreate {
    pub name: String,
    pub description: Option<String>,
    pub attachment_type: Option<String>,
    pub external_url: Option<String>,
    pub content: Option<Vec<u8>>,
}

pub(crate) fn is_multipart_request(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .to_ascii_lowercase()
                .starts_with("multipart/form-data")
        })
}

pub(crate) async fn parse_json_attachment(
    request: Request,
) -> Result<ParsedAttachmentCreate, ApiError> {
    let body = Bytes::from_request(request, &())
        .await
        .map_err(|err| ApiError::bad_request(format!("Failed to read request body: {err}")))?;
    let attachment_request: CreateAttachmentRequest =
        serde_json::from_slice(&body).map_err(|err| {
            ApiError::bad_request(format!(
                "Failed to serialize to a AttachmentRequest instance: {err}"
            ))
        })?;
    let name = attachment_request
        .name
        .filter(|n| !n.is_empty())
        .ok_or_else(|| ApiError::bad_request("Attachment name is required."))?;
    Ok(ParsedAttachmentCreate {
        name,
        description: attachment_request.description,
        attachment_type: attachment_request.attachment_type,
        external_url: attachment_request.external_url,
        // Rust extension: JSON string content → binary bytes (UTF-8).
        content: attachment_request.content.map(|c| c.into_bytes()),
    })
}

/// Stream a multipart field with per-field and request-total caps.
async fn read_multipart_field_limited(
    mut field: axum::extract::multipart::Field<'_>,
    per_field_limit: usize,
    request_total: &mut usize,
    request_limit: usize,
) -> Result<Vec<u8>, ApiError> {
    let mut buf = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|err| {
        ApiError::bad_request(format!("Failed to read multipart field: {err}"))
    })? {
        let n = chunk.len();
        if buf.len().saturating_add(n) > per_field_limit {
            return Err(ApiError::payload_too_large(format!(
                "multipart field exceeds limit of {per_field_limit} bytes"
            )));
        }
        if request_total.saturating_add(n) > request_limit {
            return Err(ApiError::payload_too_large(format!(
                "multipart request exceeds limit of {request_limit} bytes"
            )));
        }
        *request_total += n;
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

pub(crate) async fn parse_multipart_attachment(
    request: Request,
) -> Result<ParsedAttachmentCreate, ApiError> {
    // Java `createBinaryAttachment`: form fields name/description/type + first file.
    let mut multipart = Multipart::from_request(request, &())
        .await
        .map_err(|err| ApiError::bad_request(format!("Invalid multipart request: {err}")))?;

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut attachment_type: Option<String> = None;
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut request_total = 0usize;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| ApiError::bad_request(format!("Invalid multipart field: {err}")))?
    {
        let field_name = field.name().unwrap_or("").to_ascii_lowercase();
        let file_name = field.file_name().map(|s| s.to_string());
        let is_file_part = field_name == "file" || file_name.is_some();
        let per_field_limit = if is_file_part {
            MAX_MULTIPART_FILE_BYTES
        } else {
            MAX_MULTIPART_TEXT_FIELD_BYTES
        };
        let data = read_multipart_field_limited(
            field,
            per_field_limit,
            &mut request_total,
            MAX_MULTIPART_REQUEST_BYTES,
        )
        .await?;

        match field_name.as_str() {
            "name" => {
                name = Some(
                    String::from_utf8(data)
                        .map_err(|_| ApiError::bad_request("Attachment name must be UTF-8"))?,
                );
            }
            "description" => {
                description = Some(String::from_utf8(data).map_err(|_| {
                    ApiError::bad_request("Attachment description must be UTF-8")
                })?);
            }
            "type" => {
                attachment_type = Some(
                    String::from_utf8(data)
                        .map_err(|_| ApiError::bad_request("Attachment type must be UTF-8"))?,
                );
            }
            _ => {
                // File part: named "file" or any part that includes a filename.
                if file_bytes.is_none() && is_file_part {
                    file_bytes = Some(data);
                }
            }
        }
    }

    let name = name
        .filter(|n| !n.is_empty())
        .ok_or_else(|| ApiError::bad_request("Attachment name is required."))?;
    let content =
        file_bytes.ok_or_else(|| ApiError::bad_request("Attachment content is required."))?;

    Ok(ParsedAttachmentCreate {
        name,
        description,
        attachment_type,
        external_url: None,
        content: Some(content),
    })
}

/// Java: try MediaType.valueOf(type); on failure use application/octet-stream.
pub(crate) fn content_type_for_attachment(attachment_type: Option<&str>) -> String {
    match attachment_type {
        Some(t) if is_plausible_media_type(t) => t.to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

fn is_plausible_media_type(value: &str) -> bool {
    // Minimal check mirroring MediaType.valueOf success: type/subtype form.
    let mut parts = value.splitn(2, '/');
    match (parts.next(), parts.next()) {
        (Some(t), Some(s)) => {
            !t.is_empty() && !s.is_empty() && !t.contains(' ') && !s.contains(' ')
        }
        _ => false,
    }
}

/// Map Content Service attachment record → Java AttachmentResponse shape.
///
/// `collection_url` is the REST collection path used for `url` / `contentUrl`
/// (e.g. `/runtime/tasks/{id}/attachments/{attachmentId}` or
/// `/runtime/process-instances/{id}/attachments/{attachmentId}`).
pub(crate) fn attachment_response_from_record(
    collection_item_url: String,
    task_id: Option<&str>,
    item: TaskAttachmentRecord,
) -> AttachmentResponse {
    let (external_url, content_url) = if item.external_url.is_some() {
        (item.external_url, None)
    } else {
        (None, Some(format!("{collection_item_url}/content")))
    };
    let time = item.created.map(|millis| {
        DateTime::<Utc>::from_timestamp_millis(millis)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| millis.to_string())
    });
    AttachmentResponse {
        id: item.id,
        url: collection_item_url,
        name: item.name,
        user_id: item.user_id,
        description: item.description,
        attachment_type: item.attachment_type.or(item.mime_type),
        task_url: task_id.map(|id| format!("/runtime/tasks/{id}")),
        // Java RestUrls.URL_PROCESS_INSTANCE → /runtime/process-instances/{id}
        process_instance_url: item
            .process_instance_id
            .map(|id| format!("/runtime/process-instances/{id}")),
        external_url,
        content_url,
        time,
    }
}

/// Task-scoped convenience: same shape Java uses under `/runtime/tasks/.../attachments`.
pub(crate) fn task_attachment_response_from_record(
    task_id: &str,
    item: TaskAttachmentRecord,
) -> AttachmentResponse {
    let url = format!("/runtime/tasks/{task_id}/attachments/{}", item.id);
    attachment_response_from_record(url, Some(task_id), item)
}

/// Process-scoped convenience for the Rust process-attachment REST extension.
pub(crate) fn process_attachment_response_from_record(
    process_instance_id: &str,
    item: TaskAttachmentRecord,
) -> AttachmentResponse {
    let task_id = item.task_id.clone();
    let url = format!(
        "/runtime/process-instances/{process_instance_id}/attachments/{}",
        item.id
    );
    attachment_response_from_record(url, task_id.as_deref(), item)
}
