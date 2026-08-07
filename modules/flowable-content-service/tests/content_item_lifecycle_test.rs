mod test_support;

use flowable_content_service::CreateContentItemRequest;
use flowable_engine::error::FlowableError;
use std::fs;
use test_support::persistent_service;
use uuid::Uuid;

#[test]
fn content_item_create_get_delete_round_trip_is_durable() {
    let db_path = std::env::temp_dir()
        .join(format!("flowable-content-service-{}.db", Uuid::new_v4()))
        .to_string_lossy()
        .into_owned();

    let service = persistent_service("content-item-lifecycle", &db_path);
    let created = service
        .create_content_item(CreateContentItemRequest {
            name: "contract.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("signed".to_string()),
            task_id: Some("task-123".to_string()),
            process_instance_id: Some("process-123".to_string()),
            scope_type: Some("task".to_string()),
            scope_id: Some("task-123".to_string()),
            created_by: Some("animal".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();

    let reloaded = persistent_service("content-item-lifecycle-reloaded", &db_path);
    let stored = reloaded.get_content_item(&created.id).unwrap();
    assert_eq!(stored.name, "contract.txt");
    assert_eq!(stored.content_size, "signed".len());

    reloaded.delete_content_item(&created.id).unwrap();

    let error = reloaded.get_content_item(&created.id).unwrap_err();
    match error {
        FlowableError::NotFound(message) => assert!(message.contains(&created.id)),
        other => panic!("unexpected error: {other:?}"),
    }

    let _ = fs::remove_file(db_path);
}

#[test]
fn deleting_missing_content_item_returns_not_found() {
    let service = persistent_service(
        "content-item-delete-errors",
        &std::env::temp_dir()
            .join(format!(
                "flowable-content-service-delete-{}.db",
                Uuid::new_v4()
            ))
            .to_string_lossy(),
    );

    let error = service.delete_content_item("missing-item").unwrap_err();
    match error {
        FlowableError::NotFound(message) => assert!(message.contains("missing-item")),
        other => panic!("unexpected error: {other:?}"),
    }
}
