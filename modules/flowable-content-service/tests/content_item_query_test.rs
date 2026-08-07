mod test_support;

use flowable_engine::error::FlowableError;
use std::{thread, time::Duration};
use test_support::{create_sample_items, service};

#[test]
fn content_item_query_returns_deterministic_results_and_supported_filters() {
    let service = service("content-item-query");
    create_sample_items(&service);

    let page = service
        .create_content_item_query()
        .page(0, 10)
        .list_page()
        .unwrap();

    assert_eq!(page.start, 0);
    assert_eq!(page.size, 2);
    assert_eq!(page.total, 2);
    assert_eq!(
        page.data
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        vec!["invoice.pdf", "notes.txt"]
    );

    let task_scoped = service
        .create_content_item_query()
        .task_id("task-001")
        .list()
        .unwrap();
    assert_eq!(task_scoped.len(), 1);
    assert_eq!(task_scoped[0].created_by.as_deref(), Some("kermit"));

    let process_scoped = service
        .create_content_item_query()
        .process_instance_id("process-002")
        .list()
        .unwrap();
    assert_eq!(process_scoped.len(), 1);
    assert_eq!(process_scoped[0].name, "notes.txt");
}

#[test]
fn content_item_query_rejects_unsupported_filters_structurally() {
    let service = service("content-item-query-errors");
    create_sample_items(&service);

    let error = service
        .create_content_item_query()
        .unsupported_filter("tenantId", "tenant-a")
        .list_page()
        .unwrap_err();

    match error {
        FlowableError::ExecutionError(message) | FlowableError::Generic(message) => {
            assert!(message.contains("tenantId"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn content_item_query_orders_by_created_date_descending() {
    let service = service("content-item-created-sort");

    service
        .create_content_item(flowable_content_service::CreateContentItemRequest {
            name: "old.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("old".to_string()),
            task_id: None,
            process_instance_id: None,
            scope_type: None,
            scope_id: None,
            created_by: Some("kermit".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();

    thread::sleep(Duration::from_millis(25));

    service
        .create_content_item(flowable_content_service::CreateContentItemRequest {
            name: "new.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("new".to_string()),
            task_id: None,
            process_instance_id: None,
            scope_type: None,
            scope_id: None,
            created_by: Some("kermit".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();

    let items = service
        .create_content_item_query()
        .created_by("kermit")
        .order_by_created_date()
        .desc()
        .list()
        .unwrap();

    assert_eq!(
        items
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        vec!["new.txt", "old.txt"]
    );
    assert!(items[0].created_at > items[1].created_at);
}
