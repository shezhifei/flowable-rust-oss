use crate::models::{
    ContentItem, ContentItemData, ContentObject, ContentObjectStorageMetadata,
    CreateContentItemRequest,
};
use crate::query::ContentItemQuery;
use crate::repository;
use crate::storage::{ContentStorage, LocalFileSystemStorage, LocalFileSystemStorageConfig};
use crate::process_attachment::{
    CreateProcessAttachmentCmd, CreateProcessAttachmentInput, DeleteProcessAttachmentCmd,
    GetProcessAttachmentCmd, GetProcessAttachmentContentCmd, ListProcessAttachmentsCmd,
};
use crate::task_attachment::{
    CreateTaskAttachmentCmd, CreateTaskAttachmentInput, DeleteTaskAttachmentCmd,
    GetTaskAttachmentCmd, GetTaskAttachmentContentCmd, ListTaskAttachmentsCmd,
    TaskAttachmentContent,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use flowable_engine::persistence::{DbParams, StorageError};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct FlowableContentService {
    engine: Arc<ProcessEngine>,
    storage: Arc<dyn ContentStorage>,
}

impl FlowableContentService {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        let storage = Arc::new(LocalFileSystemStorage::new(LocalFileSystemStorageConfig {
            root_dir: PathBuf::from("./flowable-content-storage"),
        }));
        Self::with_storage(engine, storage)
    }

    pub fn with_storage(engine: Arc<ProcessEngine>, storage: Arc<dyn ContentStorage>) -> Self {
        repository::ensure_schema(&engine.get_runtime_store());
        Self { engine, storage }
    }

    pub fn create_content_item(
        &self,
        request: CreateContentItemRequest,
    ) -> Result<ContentItem, FlowableError> {
        // Public tenantless entry point; tenant-scoped creation must go through
        // the trusted `create_content_item_for_tenant` (the tenant comes from an
        // authenticated context, never from the request payload).
        self.create_content_item_for_tenant(request, None)
    }

    /// Create a content item owned by `tenant_id`.
    ///
    /// The tenant must originate from a trusted source (authenticated request
    /// context, engine-internal caller) — it is deliberately not part of
    /// [`CreateContentItemRequest`] so ordinary clients cannot forge ownership.
    /// Tenant-scoped form submits can only claim same-tenant content, so this
    /// is the supported pre-upload path for tenant users.
    pub fn create_content_item_for_tenant(
        &self,
        request: CreateContentItemRequest,
        tenant_id: Option<&str>,
    ) -> Result<ContentItem, FlowableError> {
        if request.name.trim().is_empty() {
            return Err(FlowableError::ExecutionError(
                "Content item name is required".to_string(),
            ));
        }

        let store = self.engine.get_runtime_store();
        let now = store.time_source().now().timestamp_millis();
        let payload = request.content.map(|value| value.into_bytes());
        let content_item_id = format!("content-item:{}", Uuid::new_v4());

        let expires_at = request
            .expires_in_seconds
            .map(|secs| now + (secs as i64) * 1000);

        let mut matching_items = Vec::new();
        if let Some(task_id) = request.task_id.as_ref() {
            let mut params = DbParams::new();
            params.push(task_id.as_str());
            params.push(request.name.as_str());
            matching_items.extend(repository::find_content_items_by_filter(
                &store,
                "task_id = ? AND name = ?",
                params,
            ));
        }
        if let Some(proc_id) = request.process_instance_id.as_ref() {
            let mut params = DbParams::new();
            params.push(proc_id.as_str());
            params.push(request.name.as_str());
            matching_items.extend(repository::find_content_items_by_filter(
                &store,
                "process_instance_id = ? AND name = ?",
                params,
            ));
        }
        if let (Some(scope_id), Some(scope_type)) =
            (request.scope_id.as_ref(), request.scope_type.as_ref())
        {
            let mut params = DbParams::new();
            params.push(scope_id.as_str());
            params.push(scope_type.as_str());
            params.push(request.name.as_str());
            matching_items.extend(repository::find_content_items_by_filter(
                &store,
                "scope_id = ? AND scope_type = ? AND name = ?",
                params,
            ));
        }

        let max_version = matching_items
            .iter()
            .filter_map(|item| item.version)
            .max()
            .unwrap_or(0);
        let new_version = max_version + 1;

        let (storage_id, storage_backend) = if let Some(ref data) = payload {
            let object = ContentObject {
                id: Uuid::new_v4().to_string(),
                content_item_id: content_item_id.clone(),
                data: data.clone(),
                mime_type: request
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                file_name: Some(request.name.clone()),
                size: data.len() as u64,
            };
            let metadata = self.storage.store(&object)?;
            (Some(metadata.storage_id), Some(metadata.storage_backend))
        } else {
            (None, None)
        };

        let item = ContentItem {
            id: content_item_id,
            name: request.name,
            mime_type: request.mime_type,
            description: request.description,
            attachment_type: request.attachment_type,
            external_url: request.external_url,
            content_size: payload.as_ref().map(|value| value.len()).unwrap_or(0),
            content: None,
            task_id: request.task_id,
            process_instance_id: request.process_instance_id,
            scope_type: request.scope_type,
            scope_id: request.scope_id,
            field: None,
            tenant_id: tenant_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            created_by: request.created_by,
            created_at: now,
            updated_at: now,
            storage_id,
            storage_backend,
            version: Some(new_version),
            expires_at,
        };

        repository::insert_content_item(&store, item.clone(), None).map_err(map_storage_error)?;
        Ok(item)
    }

    pub fn cleanup_expired_items(&self) -> Result<usize, FlowableError> {
        let store = self.engine.get_runtime_store();
        let now = store.time_source().now().timestamp_millis();
        let expired_items = repository::find_expired_content_items(&store, now);
        let count = expired_items.len();

        for item in &expired_items {
            if let Some(ref storage_id) = item.storage_id {
                let _ = self.storage.delete(storage_id);
            }
            repository::delete_content_item(&store, &item.id);
        }

        Ok(count)
    }

    pub fn create_content_item_query(&self) -> ContentItemQuery {
        ContentItemQuery::new(Arc::clone(&self.engine))
    }

    pub fn get_content_item(&self, content_item_id: &str) -> Result<ContentItem, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::find_content_item(&store, content_item_id).ok_or_else(|| {
            FlowableError::NotFound(format!("Content item '{}' was not found", content_item_id))
        })
    }

    pub fn get_content_item_data(
        &self,
        content_item_id: &str,
    ) -> Result<ContentItemData, FlowableError> {
        let store = self.engine.get_runtime_store();
        let item = repository::find_content_item(&store, content_item_id).ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Content item data for '{}' was not found",
                content_item_id
            ))
        })?;

        if let Some(ref storage_id) = item.storage_id {
            let content = self.storage.retrieve(storage_id).map_err(|e| match e {
                FlowableError::NotFound(message) => FlowableError::NotFound(format!(
                    "Content item data for '{}' was not found: {}",
                    content_item_id, message
                )),
                other => FlowableError::ExecutionError(format!(
                    "Failed to retrieve content for '{}': {other}",
                    content_item_id
                )),
            })?;
            return Ok(ContentItemData {
                content_item_id: item.id,
                mime_type: item.mime_type,
                content_size: content.len(),
                content,
            });
        }

        // Task-attachment path stores bytes in the session-backed blob table
        // (Java ByteArrayEntity parity) instead of FS ContentStorage.
        let mut session = store.create_session().map_err(|e| {
            FlowableError::ExecutionError(format!("Failed to open session: {e}"))
        })?;
        if let Some(content) =
            repository::find_content_item_payload_in_session(&mut session, content_item_id)
                .map_err(map_storage_error)?
        {
            return Ok(ContentItemData {
                content_item_id: item.id,
                mime_type: item.mime_type,
                content_size: content.len(),
                content,
            });
        }

        Err(FlowableError::NotFound(format!(
            "No storage object associated with content item '{}'",
            content_item_id
        )))
    }

    // -----------------------------------------------------------------------
    // Task attachment API (Java CreateAttachmentCmd / DeleteAttachmentCmd)
    // -----------------------------------------------------------------------

    /// Create a task attachment (link or binary) and AddAttachment event
    /// atomically in a single engine command session.
    pub fn create_task_attachment(
        &self,
        input: CreateTaskAttachmentInput,
    ) -> Result<ContentItem, FlowableError> {
        let cmd = CreateTaskAttachmentCmd::new(input);
        self.engine.get_command_executor().execute(&cmd)
    }

    /// Delete a task attachment and write DeleteAttachment event atomically.
    pub fn delete_task_attachment(
        &self,
        task_id: &str,
        attachment_id: &str,
        user_id: Option<&str>,
    ) -> Result<ContentItem, FlowableError> {
        // Capture FS storage_id (if any, from Content Service extension path)
        // before the command deletes the DB row; best-effort FS cleanup after
        // successful commit. Session-backed blobs are removed inside the command.
        let preexisting = self.get_content_item(attachment_id).ok();
        let cmd = DeleteTaskAttachmentCmd::new(
            task_id.to_string(),
            attachment_id.to_string(),
            user_id.map(str::to_string),
        );
        let item = self.engine.get_command_executor().execute(&cmd)?;
        if let Some(storage_id) = preexisting
            .as_ref()
            .and_then(|i| i.storage_id.as_deref())
        {
            let _ = self.storage.delete(storage_id);
        }
        Ok(item)
    }

    /// List attachments for a task (does not require a runtime task).
    pub fn list_task_attachments(&self, task_id: &str) -> Result<Vec<ContentItem>, FlowableError> {
        let cmd = ListTaskAttachmentsCmd::new(task_id.to_string());
        self.engine.get_command_executor().execute(&cmd)
    }

    /// Get one attachment scoped to a task (does not require a runtime task).
    pub fn get_task_attachment(
        &self,
        task_id: &str,
        attachment_id: &str,
    ) -> Result<ContentItem, FlowableError> {
        let cmd = GetTaskAttachmentCmd::new(task_id.to_string(), attachment_id.to_string());
        self.engine.get_command_executor().execute(&cmd)
    }

    /// Get binary content for a task attachment (404 when link-only / no bytes).
    pub fn get_task_attachment_content(
        &self,
        task_id: &str,
        attachment_id: &str,
    ) -> Result<TaskAttachmentContent, FlowableError> {
        let cmd =
            GetTaskAttachmentContentCmd::new(task_id.to_string(), attachment_id.to_string());
        let result = self.engine.get_command_executor().execute(&cmd);
        if result.is_ok() {
            return result;
        }
        // Fallback: attachment created via Content Service FS path (extension).
        let item = self.get_task_attachment(task_id, attachment_id)?;
        if let Some(ref storage_id) = item.storage_id {
            let content = self.storage.retrieve(storage_id)?;
            return Ok(TaskAttachmentContent {
                item,
                bytes: content,
            });
        }
        result
    }

    // -----------------------------------------------------------------------
    // Process attachment API (Java processInstanceId createAttachment variants)
    // -----------------------------------------------------------------------

    /// Create a process-instance attachment (optionally also task-scoped) and
    /// lifecycle history atomically in a single engine command session.
    pub fn create_process_attachment(
        &self,
        input: CreateProcessAttachmentInput,
    ) -> Result<ContentItem, FlowableError> {
        let cmd = CreateProcessAttachmentCmd::new(input);
        self.engine.get_command_executor().execute(&cmd)
    }

    /// Delete a process-scoped attachment and write lifecycle history atomically.
    pub fn delete_process_attachment(
        &self,
        process_instance_id: &str,
        attachment_id: &str,
        user_id: Option<&str>,
    ) -> Result<ContentItem, FlowableError> {
        let preexisting = self.get_content_item(attachment_id).ok();
        let cmd = DeleteProcessAttachmentCmd::new(
            process_instance_id.to_string(),
            attachment_id.to_string(),
            user_id.map(str::to_string),
        );
        let item = self.engine.get_command_executor().execute(&cmd)?;
        if let Some(storage_id) = preexisting
            .as_ref()
            .and_then(|i| i.storage_id.as_deref())
        {
            let _ = self.storage.delete(storage_id);
        }
        Ok(item)
    }

    /// List attachments for a process instance (does not require a runtime PI for list itself;
    /// history must be enabled — Java AttachmentEntityManager check).
    pub fn list_process_attachments(
        &self,
        process_instance_id: &str,
    ) -> Result<Vec<ContentItem>, FlowableError> {
        let cmd = ListProcessAttachmentsCmd::new(process_instance_id.to_string());
        self.engine.get_command_executor().execute(&cmd)
    }

    /// Get one attachment scoped to a process instance.
    pub fn get_process_attachment(
        &self,
        process_instance_id: &str,
        attachment_id: &str,
    ) -> Result<ContentItem, FlowableError> {
        let cmd = GetProcessAttachmentCmd::new(
            process_instance_id.to_string(),
            attachment_id.to_string(),
        );
        self.engine.get_command_executor().execute(&cmd)
    }

    /// Get binary content for a process attachment (404 when link-only / no bytes).
    pub fn get_process_attachment_content(
        &self,
        process_instance_id: &str,
        attachment_id: &str,
    ) -> Result<TaskAttachmentContent, FlowableError> {
        let cmd = GetProcessAttachmentContentCmd::new(
            process_instance_id.to_string(),
            attachment_id.to_string(),
        );
        let result = self.engine.get_command_executor().execute(&cmd);
        if result.is_ok() {
            return result;
        }
        let item = self.get_process_attachment(process_instance_id, attachment_id)?;
        if let Some(ref storage_id) = item.storage_id {
            let content = self.storage.retrieve(storage_id)?;
            return Ok(TaskAttachmentContent {
                item,
                bytes: content,
            });
        }
        result
    }

    pub fn delete_content_item(&self, content_item_id: &str) -> Result<(), FlowableError> {
        let store = self.engine.get_runtime_store();

        if let Some(item) = repository::find_content_item(&store, content_item_id)
            && let Some(ref storage_id) = item.storage_id
        {
            let _ = self.storage.delete(storage_id);
        }

        if repository::delete_content_item(&store, content_item_id) {
            Ok(())
        } else {
            Err(FlowableError::NotFound(format!(
                "Content item '{}' was not found",
                content_item_id
            )))
        }
    }

    pub fn delete_content_items_by_process_instance_id(
        &self,
        process_instance_id: &str,
    ) -> Result<usize, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut params = DbParams::new();
        params.push(process_instance_id);
        self.delete_storage_for_items_by_filter(&store, "process_instance_id = ?", params);
        Ok(repository::delete_content_items_by_process_instance_id(
            &store,
            process_instance_id,
        ))
    }

    pub fn delete_content_items_by_task_id(&self, task_id: &str) -> Result<usize, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut params = DbParams::new();
        params.push(task_id);
        self.delete_storage_for_items_by_filter(&store, "task_id = ?", params);
        Ok(repository::delete_content_items_by_task_id(&store, task_id))
    }

    pub fn delete_content_items_by_scope_id_and_scope_type(
        &self,
        scope_id: &str,
        scope_type: &str,
    ) -> Result<usize, FlowableError> {
        let store = self.engine.get_runtime_store();
        let mut params = DbParams::new();
        params.push(scope_id);
        params.push(scope_type);
        self.delete_storage_for_items_by_filter(&store, "scope_id = ? AND scope_type = ?", params);
        Ok(repository::delete_content_items_by_scope_id_and_scope_type(
            &store, scope_id, scope_type,
        ))
    }

    pub fn get_content_item_object_metadata(
        &self,
        content_item_id: &str,
    ) -> Result<ContentObjectStorageMetadata, FlowableError> {
        let store = self.engine.get_runtime_store();
        let item = repository::find_content_item(&store, content_item_id).ok_or_else(|| {
            FlowableError::NotFound(format!("Content item '{}' was not found", content_item_id))
        })?;

        match item.storage_id {
            Some(ref storage_id) => self.storage.get_metadata(storage_id),
            None => Err(FlowableError::NotFound(format!(
                "No storage object associated with content item '{}'",
                content_item_id
            ))),
        }
    }

    pub fn get_storage_status(&self) -> serde_json::Value {
        serde_json::json!({
            "backend": self.storage.backend_name(),
            "status": "ok"
        })
    }

    fn delete_storage_for_items_by_filter(
        &self,
        store: &flowable_engine::persistence::runtime_store::RuntimeStore,
        predicate: &str,
        params: DbParams,
    ) {
        let items = repository::find_content_items_by_filter(store, predicate, params);
        for item in &items {
            if let Some(ref storage_id) = item.storage_id {
                let _ = self.storage.delete(storage_id);
            }
        }
    }
}

fn map_storage_error(error: StorageError) -> FlowableError {
    FlowableError::ExecutionError(format!("Failed to persist content item data: {error}"))
}
