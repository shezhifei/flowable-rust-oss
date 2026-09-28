// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

#![allow(dead_code)]

use flowable_content_service::{CreateContentItemRequest, FlowableContentService};
use flowable_engine::engine::process_engine::ProcessEngine;
use std::sync::Arc;

pub fn service(name: &str) -> FlowableContentService {
    FlowableContentService::new(Arc::new(ProcessEngine::new(name.to_string()).unwrap()))
}

pub fn persistent_service(name: &str, path: &str) -> FlowableContentService {
    FlowableContentService::new(Arc::new(ProcessEngine::new_with_db_path(
        name.to_string(),
        path,
    ).unwrap()))
}

pub fn create_sample_items(service: &FlowableContentService) {
    service
        .create_content_item(CreateContentItemRequest {
            name: "invoice.pdf".to_string(),
            mime_type: Some("application/pdf".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("invoice-body".to_string()),
            task_id: Some("task-001".to_string()),
            process_instance_id: Some("process-001".to_string()),
            scope_type: Some("task".to_string()),
            scope_id: Some("task-001".to_string()),
            created_by: Some("kermit".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();

    service
        .create_content_item(CreateContentItemRequest {
            name: "notes.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("trip-notes".to_string()),
            task_id: None,
            process_instance_id: Some("process-002".to_string()),
            scope_type: Some("processInstance".to_string()),
            scope_id: Some("process-002".to_string()),
            created_by: Some("gonzo".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();
}
