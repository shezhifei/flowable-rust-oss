mod test_support;

use flowable_content_service::CreateContentItemRequest;
use flowable_engine::error::FlowableError;
use std::fs;
use test_support::persistent_service;
use uuid::Uuid;

#[test]
fn content_item_data_is_durable_and_readable_separately_from_metadata() {
    let db_path = std::env::temp_dir()
        .join(format!(
            "flowable-content-service-data-{}.db",
            Uuid::new_v4()
        ))
        .to_string_lossy()
        .into_owned();

    let service = persistent_service("content-item-data", &db_path);
    let created = service
        .create_content_item(CreateContentItemRequest {
            name: "invoice.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("approved-payload".to_string()),
            task_id: Some("task-101".to_string()),
            process_instance_id: Some("process-101".to_string()),
            scope_type: Some("bpmn".to_string()),
            scope_id: Some("scope-101".to_string()),
            created_by: Some("kermit".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();

    let reloaded = persistent_service("content-item-data-reloaded", &db_path);
    let metadata = reloaded.get_content_item(&created.id).unwrap();
    let data = reloaded.get_content_item_data(&created.id).unwrap();

    assert_eq!(metadata.id, created.id);
    assert_eq!(metadata.content_size, "approved-payload".len());
    assert_eq!(data.content_item_id, created.id);
    assert_eq!(data.mime_type.as_deref(), Some("text/plain"));
    assert_eq!(data.content, b"approved-payload");
    assert_eq!(data.content_size, "approved-payload".len());

    let _ = fs::remove_file(db_path);
}

#[test]
fn scoped_content_cleanup_removes_metadata_and_payload_for_owned_subset() {
    let db_path = std::env::temp_dir()
        .join(format!(
            "flowable-content-service-scoped-cleanup-{}.db",
            Uuid::new_v4()
        ))
        .to_string_lossy()
        .into_owned();

    let service = persistent_service("content-item-scoped-cleanup", &db_path);

    let process_item = service
        .create_content_item(CreateContentItemRequest {
            name: "process.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("process-body".to_string()),
            task_id: None,
            process_instance_id: Some("process-cleanup".to_string()),
            scope_type: Some("bpmn".to_string()),
            scope_id: Some("scope-process".to_string()),
            created_by: None,
            expires_in_seconds: None,
        })
        .unwrap();
    let task_item = service
        .create_content_item(CreateContentItemRequest {
            name: "task.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("task-body".to_string()),
            task_id: Some("task-cleanup".to_string()),
            process_instance_id: Some("process-other".to_string()),
            scope_type: Some("task".to_string()),
            scope_id: Some("scope-task".to_string()),
            created_by: None,
            expires_in_seconds: None,
        })
        .unwrap();
    let scope_item = service
        .create_content_item(CreateContentItemRequest {
            name: "scope.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("scope-body".to_string()),
            task_id: None,
            process_instance_id: Some("process-scope".to_string()),
            scope_type: Some("cmmn".to_string()),
            scope_id: Some("scope-cleanup".to_string()),
            created_by: None,
            expires_in_seconds: None,
        })
        .unwrap();
    let survivor = service
        .create_content_item(CreateContentItemRequest {
            name: "survivor.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("survivor-body".to_string()),
            task_id: Some("task-survivor".to_string()),
            process_instance_id: Some("process-survivor".to_string()),
            scope_type: Some("bpmn".to_string()),
            scope_id: Some("scope-survivor".to_string()),
            created_by: None,
            expires_in_seconds: None,
        })
        .unwrap();

    assert_eq!(
        service
            .delete_content_items_by_process_instance_id("process-cleanup")
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .delete_content_items_by_task_id("task-cleanup")
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .delete_content_items_by_scope_id_and_scope_type("scope-cleanup", "cmmn")
            .unwrap(),
        1
    );

    for deleted_id in [&process_item.id, &task_item.id, &scope_item.id] {
        let metadata_error = service.get_content_item(deleted_id).unwrap_err();
        let data_error = service.get_content_item_data(deleted_id).unwrap_err();
        assert!(matches!(metadata_error, FlowableError::NotFound(_)));
        assert!(matches!(data_error, FlowableError::NotFound(_)));
    }

    let survivor_metadata = service.get_content_item(&survivor.id).unwrap();
    let survivor_data = service.get_content_item_data(&survivor.id).unwrap();
    assert_eq!(survivor_metadata.name, "survivor.txt");
    assert_eq!(survivor_data.content, b"survivor-body");

    let _ = fs::remove_file(db_path);
}

#[test]
fn missing_content_item_data_returns_not_found() {
    let service = persistent_service(
        "content-item-data-missing",
        &std::env::temp_dir()
            .join(format!(
                "flowable-content-service-data-missing-{}.db",
                Uuid::new_v4()
            ))
            .to_string_lossy(),
    );

    let error = service.get_content_item_data("missing-item").unwrap_err();
    match error {
        FlowableError::NotFound(message) => assert!(message.contains("missing-item")),
        other => panic!("unexpected error: {other:?}"),
    }
}
