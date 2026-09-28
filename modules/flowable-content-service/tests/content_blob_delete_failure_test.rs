// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! P2 parity: storage blob deletion failures must not be swallowed.
//!
//! - Batch expiry cleanup tolerates per-blob failures (row removal continues,
//!   count returned, failure observable via `tracing`).
//! - Delete commands (`delete_content_item`, task/process attachment delete,
//!   cascade deletes) propagate a blob deletion failure instead of returning
//!   success over an orphaned object.
//!
//! A `FakeStorage` whose `delete` fails stands in for a filesystem/cloud backend
//! that returned an error.

use flowable_content_service::{
    ContentObject, ContentObjectStorageMetadata, ContentStorage, CreateContentItemRequest,
    FlowableContentService,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use std::sync::{Arc, Mutex};

/// In-memory storage double: records every `delete` call and optionally fails
/// them. `store` succeeds with a deterministic id so service items carry a
/// `storage_id`; all other operations are unused by the paths under test.
struct FakeStorage {
    delete_fails: bool,
    deleted_ids: Mutex<Vec<String>>,
}

impl FakeStorage {
    fn new(delete_fails: bool) -> Arc<Self> {
        Arc::new(Self {
            delete_fails,
            deleted_ids: Mutex::new(Vec::new()),
        })
    }

    fn deleted_ids(&self) -> Vec<String> {
        self.deleted_ids.lock().unwrap().clone()
    }
}

impl ContentStorage for FakeStorage {
    fn store(&self, object: &ContentObject) -> Result<ContentObjectStorageMetadata, FlowableError> {
        Ok(ContentObjectStorageMetadata {
            storage_id: format!("blob-{}", object.id),
            storage_backend: self.backend_name().to_string(),
            stored_at: "2026-01-01T00:00:00.000Z".to_string(),
            size: object.size,
            checksum: None,
        })
    }

    fn retrieve(&self, storage_id: &str) -> Result<Vec<u8>, FlowableError> {
        Err(FlowableError::NotFound(format!(
            "fake storage has no object '{storage_id}'"
        )))
    }

    fn delete(&self, storage_id: &str) -> Result<(), FlowableError> {
        self.deleted_ids
            .lock()
            .unwrap()
            .push(storage_id.to_string());
        if self.delete_fails {
            Err(FlowableError::ExecutionError(format!(
                "forced fake delete failure for '{storage_id}'"
            )))
        } else {
            Ok(())
        }
    }

    fn exists(&self, _storage_id: &str) -> Result<bool, FlowableError> {
        Ok(false)
    }

    fn backend_name(&self) -> &str {
        "fake-failing"
    }

    fn get_metadata(
        &self,
        storage_id: &str,
    ) -> Result<ContentObjectStorageMetadata, FlowableError> {
        Err(FlowableError::NotFound(format!(
            "fake storage has no object '{storage_id}'"
        )))
    }
}

fn service_with(
    name: &str,
    storage: Arc<FakeStorage>,
) -> (FlowableContentService, Arc<FakeStorage>, Arc<ProcessEngine>) {
    let engine = Arc::new(ProcessEngine::new(name.to_string()).unwrap());
    let service = FlowableContentService::with_storage(
        Arc::clone(&engine),
        storage.clone() as Arc<dyn ContentStorage>,
    );
    (service, storage, engine)
}

fn create_item(
    service: &FlowableContentService,
    name: &str,
    task_id: Option<&str>,
    process_instance_id: Option<&str>,
    expires_in_seconds: Option<u64>,
) -> flowable_content_service::ContentItem {
    service
        .create_content_item(CreateContentItemRequest {
            name: name.to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("payload".to_string()),
            task_id: task_id.map(str::to_string),
            process_instance_id: process_instance_id.map(str::to_string),
            scope_type: None,
            scope_id: None,
            created_by: Some("tester".to_string()),
            expires_in_seconds,
        })
        .unwrap()
}

fn execution_message(error: FlowableError) -> String {
    match error {
        FlowableError::ExecutionError(message) => message,
        other => panic!("expected ExecutionError, got {other:?}"),
    }
}

#[test]
fn cleanup_expired_items_continues_after_blob_delete_failures() {
    let (service, storage, _engine) = service_with("content-blob-cleanup", FakeStorage::new(true));

    let first = create_item(&service, "expired-1", None, None, Some(1));
    let second = create_item(&service, "expired-2", None, None, Some(1));
    std::thread::sleep(std::time::Duration::from_millis(1_100));

    // Batch semantics: undeleteable blobs do not abort the run; every expired
    // DB row is still removed and the processed count is returned.
    let cleaned = service.cleanup_expired_items().unwrap();
    assert_eq!(cleaned, 2);
    assert_eq!(storage.deleted_ids().len(), 2);
    assert!(service.get_content_item(&first.id).is_err());
    assert!(service.get_content_item(&second.id).is_err());
}

#[test]
fn delete_content_item_propagates_blob_failure_and_preserves_row() {
    let (service, storage, _engine) =
        service_with("content-blob-item-delete", FakeStorage::new(true));
    let item = create_item(&service, "doc.txt", None, None, None);

    let error = service.delete_content_item(&item.id).unwrap_err();
    let message = execution_message(error);
    assert!(message.contains("forced fake delete failure"), "{message}");

    // Blob is deleted before the row, so the row survives and the call is
    // retryable instead of leaving an orphan behind a success response.
    let still_present = service.get_content_item(&item.id).unwrap();
    assert_eq!(still_present.id, item.id);
    assert_eq!(storage.deleted_ids().len(), 1);
}

#[test]
fn delete_content_item_succeeds_with_healthy_storage() {
    let (service, storage, _engine) =
        service_with("content-blob-item-delete-ok", FakeStorage::new(false));
    let item = create_item(&service, "doc.txt", None, None, None);

    service.delete_content_item(&item.id).unwrap();
    assert!(service.get_content_item(&item.id).is_err());
    assert_eq!(storage.deleted_ids(), vec![item.storage_id.unwrap()]);
}

#[test]
fn cascade_delete_by_task_propagates_blob_failure_and_preserves_rows() {
    let (service, storage, _engine) =
        service_with("content-blob-cascade-delete", FakeStorage::new(true));
    create_item(&service, "a.txt", Some("task-cascade"), None, None);
    create_item(&service, "b.txt", Some("task-cascade"), None, None);

    // Cascade is a deletion command: first blob failure aborts before any DB
    // row is removed, so callers can retry the whole cascade.
    let error = service
        .delete_content_items_by_task_id("task-cascade")
        .unwrap_err();
    assert!(
        execution_message(error).contains("forced fake delete failure"),
        "storage error must propagate from cascade helper"
    );
    assert_eq!(
        storage.deleted_ids().len(),
        1,
        "cascade must stop at the first blob failure"
    );

    let remaining: Vec<_> = service
        .create_content_item_query()
        .task_id("task-cascade")
        .list()
        .unwrap();
    assert_eq!(
        remaining.len(),
        2,
        "no DB rows may be removed on blob failure"
    );
}

/// Deploy a process with one active user task; returns (process_instance_id, task_id).
fn deploy_user_task(engine: &Arc<ProcessEngine>, process_key: &str) -> (String, String) {
    let repository = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="{process_key}">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Task 1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );
    repository
        .deploy(
            repository
                .create_deployment()
                .add_string(format!("{process_key}.bpmn20.xml"), xml),
        )
        .unwrap();
    let process_instance = runtime.start_process_instance_by_key(process_key).unwrap();
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    (process_instance.id, tasks[0].id.clone())
}

#[test]
fn delete_task_attachment_reports_orphaned_blob_after_commit() {
    let (service, _storage, engine) =
        service_with("content-blob-task-attachment", FakeStorage::new(true));
    let (_process_instance_id, task_id) = deploy_user_task(&engine, "blobTaskAttachProcess");

    // Extension-path item (FS-backed storage_id) attached to a runtime task.
    let item = create_item(&service, "attach.txt", Some(&task_id), None, None);
    let storage_id = item.storage_id.clone().unwrap();

    // The engine command (row + DeleteAttachment event) commits first; the blob
    // failure must still surface as 500 naming the orphaned object.
    let error = service
        .delete_task_attachment(&task_id, &item.id, None)
        .unwrap_err();
    let message = execution_message(error);
    assert!(message.contains("orphaned object"), "{message}");
    assert!(message.contains(&storage_id), "{message}");

    // Post-commit semantics: the row is already gone.
    assert!(service.get_content_item(&item.id).is_err());
}

#[test]
fn delete_process_attachment_reports_orphaned_blob_after_commit() {
    let (service, _storage, engine) =
        service_with("content-blob-process-attachment", FakeStorage::new(true));
    let (process_instance_id, _task_id) = deploy_user_task(&engine, "blobProcAttachProcess");

    let item = create_item(
        &service,
        "attach.txt",
        None,
        Some(&process_instance_id),
        None,
    );
    let storage_id = item.storage_id.clone().unwrap();

    let error = service
        .delete_process_attachment(&process_instance_id, &item.id, None)
        .unwrap_err();
    let message = execution_message(error);
    assert!(message.contains("orphaned object"), "{message}");
    assert!(message.contains(&storage_id), "{message}");
    assert!(service.get_content_item(&item.id).is_err());
}
