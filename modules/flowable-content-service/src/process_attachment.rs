//! Process-instance attachment create/list/get/content/delete as engine commands.
//!
//! Java parity:
//! - `TaskService.createAttachment(type, taskId, processInstanceId, ...)`
//! - `GetProcessInstanceAttachmentsCmd` / `CreateAttachmentCmd` /
//!   `DeleteAttachmentCmd` (processInstanceId variants)
//! - History via `AbstractHistoryManager.createAttachmentComment`
//! - `AttachmentEntityManagerImpl.checkHistoryEnabled`
//!
//! Physical storage reuses the content-item metadata + session-backed blob
//! tables already used by task attachments (no duplicate attachment table).
//! Pure process attachments record a process-associated historic comment with
//! `AddAttachment` / `DeleteAttachment` action rather than fabricating a task id.

use crate::models::ContentItem;
use crate::repository;
use crate::task_attachment::{FORCE_FAIL_ATTACHMENT_TYPE, TaskAttachmentContent};
use flowable_engine::error::FlowableError;
use flowable_engine::history::historic_entities::{HistoricComment, HistoricTaskEvent};
use flowable_engine::interceptor::command::Command;
use flowable_engine::interceptor::command_context::CommandContext;
use flowable_engine::persistence::StorageError;
use uuid::Uuid;

/// Input for creating a process-scoped (or task+process) attachment.
///
/// Java `createAttachment`: task id and process instance id are independently
/// optional at the API level, but process-scoped service methods always supply
/// `process_instance_id`. At least one scope id is required.
#[derive(Clone, Debug)]
pub struct CreateProcessAttachmentInput {
    pub process_instance_id: String,
    /// When set, the task must belong to `process_instance_id`.
    pub task_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub attachment_type: Option<String>,
    pub external_url: Option<String>,
    pub content: Option<Vec<u8>>,
    pub user_id: Option<String>,
}

/// Create a process-instance attachment (+ optional task scope) in one session.
pub struct CreateProcessAttachmentCmd {
    input: CreateProcessAttachmentInput,
}

impl CreateProcessAttachmentCmd {
    pub fn new(input: CreateProcessAttachmentInput) -> Self {
        Self { input }
    }
}

impl Command<ContentItem> for CreateProcessAttachmentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ContentItem, FlowableError> {
        ensure_history_enabled(command_context)?;

        let input = &self.input;
        if input.name.trim().is_empty() {
            return Err(FlowableError::BadRequest(
                "Attachment name is required.".to_string(),
            ));
        }
        if input.process_instance_id.trim().is_empty()
            && input.task_id.as_ref().is_none_or(|t| t.trim().is_empty())
        {
            return Err(FlowableError::BadRequest(
                "Attachment requires a task id or process instance id.".to_string(),
            ));
        }

        // Java CreateAttachmentCmd.verifyExecutionParameters when processInstanceId set.
        let (store, session) = command_context.store_and_session();
        let process_instance = store
            .find_process_instance(&input.process_instance_id, session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Process instance {} doesn't exist",
                    input.process_instance_id
                ))
            })?;
        if process_instance.is_suspended {
            return Err(FlowableError::ExecutionError(format!(
                "It is not allowed to add an attachment to a suspended process instance '{}'",
                input.process_instance_id
            )));
        }
        // P1 tenant fix: attachment ownership inherits the process tenant
        // (normalized: empty string means tenantless).
        let tenant_id = process_instance
            .tenant_id
            .clone()
            .filter(|tenant| !tenant.is_empty());

        // Optional task: must exist, not suspended, and belong to the process.
        let task_id = if let Some(task_id) = input.task_id.as_ref() {
            let task = store.find_task(task_id, session).ok_or_else(|| {
                FlowableError::NotFound(format!("Cannot find task with id {task_id}"))
            })?;
            if task.is_suspended() {
                return Err(FlowableError::ExecutionError(format!(
                    "It is not allowed to add an attachment to a suspended task '{task_id}'"
                )));
            }
            // A task-scoped process attachment requires the task to actually run
            // inside the target process instance; standalone tasks (empty
            // process_instance_id) must not be combined with a process scope.
            if task.process_instance_id.is_empty()
                || task.process_instance_id != input.process_instance_id
            {
                return Err(FlowableError::BadRequest(format!(
                    "Task '{task_id}' does not belong to process instance '{}'",
                    input.process_instance_id
                )));
            }
            let task_tenant = task.tenant_id.clone().filter(|tenant| !tenant.is_empty());
            if task_tenant != tenant_id {
                return Err(FlowableError::BadRequest(format!(
                    "Task '{task_id}' belongs to a different tenant than process instance '{}'",
                    input.process_instance_id
                )));
            }
            Some(task_id.clone())
        } else {
            None
        };

        let now = store.time_source().now().timestamp_millis();
        let event_time = store.time_source().now();
        let content_item_id = format!("content-item:{}", Uuid::new_v4());
        let payload = input.content.as_deref();
        let content_size = payload.map(|p| p.len()).unwrap_or(0);
        let process_instance_id = Some(input.process_instance_id.clone());

        let item = ContentItem {
            id: content_item_id,
            name: input.name.clone(),
            mime_type: input.attachment_type.clone(),
            description: input.description.clone(),
            attachment_type: input.attachment_type.clone(),
            external_url: input.external_url.clone(),
            content: None,
            content_size,
            task_id: task_id.clone(),
            process_instance_id: process_instance_id.clone(),
            scope_type: Some("bpmn".to_string()),
            scope_id: process_instance_id.clone(),
            field: None,
            // Inherited from the process instance (P1 tenant fix).
            tenant_id,
            created_by: input.user_id.clone(),
            created_at: now,
            updated_at: now,
            storage_id: None,
            storage_backend: None,
            version: Some(1),
            expires_at: None,
        };

        let (store, session) = command_context.store_and_session();
        repository::insert_content_item_in_session(session, &item, payload)
            .map_err(map_storage_error)?;

        if input.attachment_type.as_deref() == Some(FORCE_FAIL_ATTACHMENT_TYPE) {
            return Err(FlowableError::BadRequest(
                "Unsupported attachment type".to_string(),
            ));
        }

        // History: task lifecycle when task exists; pure process → process comment.
        if let Some(ref tid) = task_id {
            let event = HistoricTaskEvent {
                id: Uuid::new_v4().to_string(),
                task_id: tid.clone(),
                action: "AddAttachment".to_string(),
                message: vec![input.name.clone()],
                user_id: input.user_id.clone(),
                time: event_time,
            };
            store.insert_historic_task_event(event, session);
        } else {
            let comment = HistoricComment {
                id: Uuid::new_v4().to_string(),
                task_id: None,
                process_instance_id: process_instance_id.clone(),
                message: input.name.clone(),
                author: input.user_id.clone(),
                time: event_time,
                action: Some("AddAttachment".to_string()),
                // Java identity-link/attachment audit rows are TYPE_EVENT comments.
                comment_type: Some(HistoricComment::TYPE_EVENT.to_string()),
            };
            store.insert_historic_comment(comment, session);
        }

        Ok(item)
    }
}

/// Delete a process-scoped attachment (+ payload + history) in one session.
pub struct DeleteProcessAttachmentCmd {
    process_instance_id: String,
    attachment_id: String,
    user_id: Option<String>,
}

impl DeleteProcessAttachmentCmd {
    pub fn new(
        process_instance_id: String,
        attachment_id: String,
        user_id: Option<String>,
    ) -> Self {
        Self {
            process_instance_id,
            attachment_id,
            user_id,
        }
    }
}

impl Command<ContentItem> for DeleteProcessAttachmentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ContentItem, FlowableError> {
        ensure_history_enabled(command_context)?;

        let (store, session) = command_context.store_and_session();
        // Runtime process required for mutation (mirrors create validation).
        let _process_instance = store
            .find_process_instance(&self.process_instance_id, session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "Process instance {} doesn't exist",
                    self.process_instance_id
                ))
            })?;

        let item = repository::find_content_item_in_session(session, &self.attachment_id)
            .map_err(map_storage_error)?
            .ok_or_else(|| not_found_for_process(&self.process_instance_id, &self.attachment_id))?;

        if item.process_instance_id.as_deref() != Some(self.process_instance_id.as_str()) {
            return Err(not_found_for_process(
                &self.process_instance_id,
                &self.attachment_id,
            ));
        }

        let attachment_name = item.name.clone();
        let task_id = item.task_id.clone();
        let deleted = repository::delete_content_item_in_session(session, &self.attachment_id)
            .map_err(map_storage_error)?;
        if !deleted {
            return Err(not_found_for_process(
                &self.process_instance_id,
                &self.attachment_id,
            ));
        }

        if let Some(tid) = task_id {
            // Prefer task event when attachment had a task (and runtime task may
            // still exist — Java only comments when taskId was set on attachment).
            let event = HistoricTaskEvent {
                id: Uuid::new_v4().to_string(),
                task_id: tid,
                action: "DeleteAttachment".to_string(),
                message: vec![attachment_name],
                user_id: self.user_id.clone(),
                time: store.time_source().now(),
            };
            store.insert_historic_task_event(event, session);
        } else {
            let comment = HistoricComment {
                id: Uuid::new_v4().to_string(),
                task_id: None,
                process_instance_id: Some(self.process_instance_id.clone()),
                message: attachment_name,
                author: self.user_id.clone(),
                time: store.time_source().now(),
                action: Some("DeleteAttachment".to_string()),
                comment_type: Some(HistoricComment::TYPE_EVENT.to_string()),
            };
            store.insert_historic_comment(comment, session);
        }

        Ok(item)
    }
}

/// Load one attachment scoped to a process instance (historic-safe for metadata).
pub struct GetProcessAttachmentCmd {
    process_instance_id: String,
    attachment_id: String,
}

impl GetProcessAttachmentCmd {
    pub fn new(process_instance_id: String, attachment_id: String) -> Self {
        Self {
            process_instance_id,
            attachment_id,
        }
    }
}

impl Command<ContentItem> for GetProcessAttachmentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ContentItem, FlowableError> {
        ensure_history_enabled(command_context)?;
        let (_store, session) = command_context.store_and_session();
        let item = repository::find_content_item_in_session(session, &self.attachment_id)
            .map_err(map_storage_error)?
            .ok_or_else(|| not_found_for_process(&self.process_instance_id, &self.attachment_id))?;
        if item.process_instance_id.as_deref() != Some(self.process_instance_id.as_str()) {
            return Err(not_found_for_process(
                &self.process_instance_id,
                &self.attachment_id,
            ));
        }
        Ok(item)
    }
}

/// List attachments for a process instance id (historic-safe).
pub struct ListProcessAttachmentsCmd {
    process_instance_id: String,
}

impl ListProcessAttachmentsCmd {
    pub fn new(process_instance_id: String) -> Self {
        Self { process_instance_id }
    }
}

impl Command<Vec<ContentItem>> for ListProcessAttachmentsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<ContentItem>, FlowableError> {
        ensure_history_enabled(command_context)?;
        let (_store, session) = command_context.store_and_session();
        repository::find_content_items_by_process_instance_id_in_session(
            session,
            &self.process_instance_id,
        )
        .map_err(map_storage_error)
    }
}

/// Load binary content for a process-scoped attachment.
pub struct GetProcessAttachmentContentCmd {
    process_instance_id: String,
    attachment_id: String,
}

impl GetProcessAttachmentContentCmd {
    pub fn new(process_instance_id: String, attachment_id: String) -> Self {
        Self {
            process_instance_id,
            attachment_id,
        }
    }
}

impl Command<TaskAttachmentContent> for GetProcessAttachmentContentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<TaskAttachmentContent, FlowableError> {
        ensure_history_enabled(command_context)?;
        let (_store, session) = command_context.store_and_session();
        let item = repository::find_content_item_in_session(session, &self.attachment_id)
            .map_err(map_storage_error)?
            .ok_or_else(|| not_found_for_process(&self.process_instance_id, &self.attachment_id))?;
        if item.process_instance_id.as_deref() != Some(self.process_instance_id.as_str()) {
            return Err(not_found_for_process(
                &self.process_instance_id,
                &self.attachment_id,
            ));
        }

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

fn ensure_history_enabled(command_context: &mut CommandContext) -> Result<(), FlowableError> {
    // Java AttachmentEntityManagerImpl.checkHistoryEnabled
    if !command_context.history_manager().is_history_enabled() {
        return Err(FlowableError::ExecutionError(
            "In order to use attachments, history should be enabled".to_string(),
        ));
    }
    Ok(())
}

fn not_found_for_process(process_instance_id: &str, attachment_id: &str) -> FlowableError {
    FlowableError::NotFound(format!(
        "Process instance '{}' does not have an attachment with id '{}'.",
        process_instance_id, attachment_id
    ))
}

fn map_storage_error(error: StorageError) -> FlowableError {
    FlowableError::ExecutionError(format!("Failed to persist attachment data: {error}"))
}
