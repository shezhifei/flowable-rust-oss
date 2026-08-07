//! Task attachment create/delete as a single engine command.
//!
//! Java parity:
//! - `CreateAttachmentCmd` / `DeleteAttachmentCmd` (flowable-engine/.../cmd/)
//! - History event via `AbstractHistoryManager.createAttachmentComment`
//!   (`ACTION_ADD_ATTACHMENT` / `ACTION_DELETE_ATTACHMENT`)
//!
//! Transaction design: content-item metadata + optional binary payload (DB blob)
//! + historic task event are written on the command session and commit/roll back
//! together. Physical FS content storage used by the general Content Service API
//! is intentionally not used on this path so binary bytes stay session-backed
//! (Java stores them as `ByteArrayEntity` in the same command).

use crate::models::ContentItem;
use crate::repository;
use flowable_engine::error::FlowableError;
use flowable_engine::history::historic_entities::HistoricTaskEvent;
use flowable_engine::interceptor::command::Command;
use flowable_engine::interceptor::command_context::CommandContext;
use flowable_engine::persistence::StorageError;
use uuid::Uuid;

/// Injectable failure type for contract tests proving mid-command rollback.
/// Java has no equivalent; used only to force an error after content is staged
/// but before the task event is written — both roll back with the session.
pub const FORCE_FAIL_ATTACHMENT_TYPE: &str = "application/x-flowable-force-fail";

/// Input for creating a task attachment (link or binary).
#[derive(Clone, Debug)]
pub struct CreateTaskAttachmentInput {
    pub task_id: String,
    pub name: String,
    pub description: Option<String>,
    /// Java attachment `type` (arbitrary string or media type).
    pub attachment_type: Option<String>,
    /// External link URL (JSON create path). Mutually exclusive with content
    /// for typical Java use; when set, response uses `externalUrl`.
    pub external_url: Option<String>,
    /// Binary payload (multipart file bytes or JSON `content` extension).
    pub content: Option<Vec<u8>>,
    pub user_id: Option<String>,
    /// When None, taken from the runtime task's process instance id.
    pub process_instance_id: Option<String>,
}

/// Create task attachment + AddAttachment event in one session.
///
/// Java: `CreateAttachmentCmd` (verify active task → insert AttachmentEntity →
/// optional ByteArrayEntity → createAttachmentComment(true)).
pub struct CreateTaskAttachmentCmd {
    input: CreateTaskAttachmentInput,
}

impl CreateTaskAttachmentCmd {
    pub fn new(input: CreateTaskAttachmentInput) -> Self {
        Self { input }
    }
}

impl Command<ContentItem> for CreateTaskAttachmentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ContentItem, FlowableError> {
        let input = &self.input;

        // REST validates name; engine guard mirrors createAttachment contract.
        if input.name.trim().is_empty() {
            return Err(FlowableError::BadRequest(
                "Attachment name is required.".to_string(),
            ));
        }

        // Java CreateAttachmentCmd.verifyTaskParameters: runtime task + not suspended.
        let (store, session) = command_context.store_and_session();
        let task = store.find_task(&input.task_id, session).ok_or_else(|| {
            FlowableError::NotFound(format!("Cannot find task with id {}", input.task_id))
        })?;
        if task.is_suspended() {
            return Err(FlowableError::ExecutionError(format!(
                "It is not allowed to add an attachment to a suspended task '{}'",
                input.task_id
            )));
        }

        let process_instance_id = input.process_instance_id.clone().or_else(|| {
            if task.process_instance_id.is_empty() {
                None
            } else {
                Some(task.process_instance_id.clone())
            }
        });
        // P1 tenant fix: attachment ownership inherits the task's tenant
        // (normalized: empty string means tenantless).
        let tenant_id = task.tenant_id.clone().filter(|tenant| !tenant.is_empty());

        let now = store.time_source().now().timestamp_millis();
        let event_time = store.time_source().now();
        let content_item_id = format!("content-item:{}", Uuid::new_v4());
        let payload = input.content.as_deref();
        let content_size = payload.map(|p| p.len()).unwrap_or(0);

        let item = ContentItem {
            id: content_item_id,
            name: input.name.clone(),
            mime_type: input.attachment_type.clone(),
            description: input.description.clone(),
            attachment_type: input.attachment_type.clone(),
            external_url: input.external_url.clone(),
            content: None,
            content_size,
            task_id: Some(input.task_id.clone()),
            process_instance_id: process_instance_id.clone(),
            scope_type: Some("bpmn".to_string()),
            scope_id: process_instance_id,
            field: None,
            // Inherited from the task context (P1 tenant fix).
            tenant_id,
            created_by: input.user_id.clone(),
            created_at: now,
            updated_at: now,
            // Session-backed blob path — not FS storage (see module docs).
            storage_id: None,
            storage_backend: None,
            version: Some(1),
            expires_at: None,
        };

        // Re-borrow after building item (same pattern as CreateTaskCommentCmd).
        let (store, session) = command_context.store_and_session();
        repository::insert_content_item_in_session(session, &item, payload)
            .map_err(map_storage_error)?;

        // Injectable mid-command failure: content staged in session but not
        // committed; returning Err rolls back content + any later event write.
        if input.attachment_type.as_deref() == Some(FORCE_FAIL_ATTACHMENT_TYPE) {
            return Err(FlowableError::BadRequest(
                "Unsupported attachment type".to_string(),
            ));
        }

        // Java AbstractHistoryManager.createAttachmentComment(..., create=true)
        // → CommentEntity TYPE_EVENT action AddAttachment, message = name.
        let event = HistoricTaskEvent {
            id: Uuid::new_v4().to_string(),
            task_id: input.task_id.clone(),
            action: "AddAttachment".to_string(),
            message: vec![input.name.clone()],
            user_id: input.user_id.clone(),
            time: event_time,
        };
        store.insert_historic_task_event(event, session);

        Ok(item)
    }
}

/// Delete task attachment + DeleteAttachment event in one session.
///
/// Java: `DeleteAttachmentCmd` (delete attachment → delete byte array →
/// createAttachmentComment(false)). Suspension is not checked on delete in
/// Java; runtime task existence is enforced by the REST layer.
pub struct DeleteTaskAttachmentCmd {
    task_id: String,
    attachment_id: String,
    user_id: Option<String>,
}

impl DeleteTaskAttachmentCmd {
    pub fn new(task_id: String, attachment_id: String, user_id: Option<String>) -> Self {
        Self {
            task_id,
            attachment_id,
            user_id,
        }
    }
}

impl Command<ContentItem> for DeleteTaskAttachmentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ContentItem, FlowableError> {
        // Runtime task must exist (REST also guards; keep for direct service use).
        let (store, session) = command_context.store_and_session();
        let _task = store.find_task(&self.task_id, session).ok_or_else(|| {
            FlowableError::NotFound(format!("Cannot find task with id {}", self.task_id))
        })?;

        let item = repository::find_content_item_in_session(session, &self.attachment_id)
            .map_err(map_storage_error)?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Task '{}' does not have an attachment with id '{}'.",
                    self.task_id, self.attachment_id
                ))
            })?;

        if item.task_id.as_deref() != Some(self.task_id.as_str()) {
            return Err(FlowableError::NotFound(format!(
                "Task '{}' does not have an attachment with id '{}'.",
                self.task_id, self.attachment_id
            )));
        }

        let attachment_name = item.name.clone();
        let deleted = repository::delete_content_item_in_session(session, &self.attachment_id)
            .map_err(map_storage_error)?;
        if !deleted {
            return Err(FlowableError::NotFound(format!(
                "Task '{}' does not have an attachment with id '{}'.",
                self.task_id, self.attachment_id
            )));
        }

        // Java createAttachmentComment(..., create=false) → DeleteAttachment.
        let event = HistoricTaskEvent {
            id: Uuid::new_v4().to_string(),
            task_id: self.task_id.clone(),
            action: "DeleteAttachment".to_string(),
            message: vec![attachment_name],
            user_id: self.user_id.clone(),
            time: store.time_source().now(),
        };
        store.insert_historic_task_event(event, session);

        Ok(item)
    }
}

/// Load attachment for a task (no runtime-task requirement — historic list/get).
pub struct GetTaskAttachmentCmd {
    task_id: String,
    attachment_id: String,
}

impl GetTaskAttachmentCmd {
    pub fn new(task_id: String, attachment_id: String) -> Self {
        Self {
            task_id,
            attachment_id,
        }
    }
}

impl Command<ContentItem> for GetTaskAttachmentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ContentItem, FlowableError> {
        let (store, session) = command_context.store_and_session();
        // Historic visibility: only require historic task exists is done at REST;
        // here we resolve the attachment by id + task scope.
        let _ = store; // store unused beyond session
        let item = repository::find_content_item_in_session(session, &self.attachment_id)
            .map_err(map_storage_error)?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Task '{}' does not have an attachment with id '{}'.",
                    self.task_id, self.attachment_id
                ))
            })?;
        if item.task_id.as_deref() != Some(self.task_id.as_str()) {
            return Err(FlowableError::NotFound(format!(
                "Task '{}' does not have an attachment with id '{}'.",
                self.task_id, self.attachment_id
            )));
        }
        Ok(item)
    }
}

/// List attachments for a task id (historic-safe).
pub struct ListTaskAttachmentsCmd {
    task_id: String,
}

impl ListTaskAttachmentsCmd {
    pub fn new(task_id: String) -> Self {
        Self { task_id }
    }
}

impl Command<Vec<ContentItem>> for ListTaskAttachmentsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<ContentItem>, FlowableError> {
        let (_store, session) = command_context.store_and_session();
        repository::find_content_items_by_task_id_in_session(session, &self.task_id)
            .map_err(map_storage_error)
    }
}

/// Load binary content for an attachment (session-backed blob first).
pub struct GetTaskAttachmentContentCmd {
    task_id: String,
    attachment_id: String,
}

impl GetTaskAttachmentContentCmd {
    pub fn new(task_id: String, attachment_id: String) -> Self {
        Self {
            task_id,
            attachment_id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TaskAttachmentContent {
    pub item: ContentItem,
    pub bytes: Vec<u8>,
}

impl Command<TaskAttachmentContent> for GetTaskAttachmentContentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<TaskAttachmentContent, FlowableError> {
        let (_store, session) = command_context.store_and_session();
        let item = repository::find_content_item_in_session(session, &self.attachment_id)
            .map_err(map_storage_error)?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Task '{}' does not have an attachment with id '{}'.",
                    self.task_id, self.attachment_id
                ))
            })?;
        if item.task_id.as_deref() != Some(self.task_id.as_str()) {
            return Err(FlowableError::NotFound(format!(
                "Task '{}' does not have an attachment with id '{}'.",
                self.task_id, self.attachment_id
            )));
        }

        // Java GetAttachmentContentCmd: contentId null → no stream → REST 404.
        let bytes = repository::find_content_item_payload_in_session(session, &self.attachment_id)
            .map_err(map_storage_error)?
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Attachment with id '{}' does not have content associated with it.",
                    self.attachment_id
                ))
            })?;

        Ok(TaskAttachmentContent { item, bytes })
    }
}

fn map_storage_error(error: StorageError) -> FlowableError {
    FlowableError::ExecutionError(format!("Failed to persist attachment data: {error}"))
}
