#![allow(dead_code)]

use flowable_content_service::{CreateContentItemRequest, FlowableContentService};
use flowable_engine::engine::process_engine::ProcessEngine;
use std::sync::Arc;

pub fn service(name: &str) -> FlowableContentService {
    FlowableContentService::new(Arc::new(ProcessEngine::new(name.to_string())))
}

pub fn persistent_service(name: &str, path: &str) -> FlowableContentService {
    FlowableContentService::new(Arc::new(ProcessEngine::new_with_db_path(
        name.to_string(),
        path,
    )))
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
