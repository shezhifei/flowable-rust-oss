mod test_support;

use flowable_engine::error::FlowableError;
use test_support::{deploy_sample_forms, service};

#[test]
fn form_definition_query_returns_deterministic_results_and_supported_filters() {
    let service = service("form-definition-query");
    let deployment = deploy_sample_forms(&service);

    let page = service
        .create_form_definition_query()
        .page(0, 10)
        .list_page()
        .unwrap();

    assert_eq!(page.start, 0);
    assert_eq!(page.size, 2);
    assert_eq!(page.total, 2);
    assert_eq!(
        page.data
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["expenseApproval", "travelRequest"]
    );
    assert!(
        page.data
            .iter()
            .all(|item| item.deployment_id == deployment.id)
    );

    let expense_only = service
        .create_form_definition_query()
        .key("expenseApproval")
        .list()
        .unwrap();
    assert_eq!(expense_only.len(), 1);
    assert_eq!(expense_only[0].resource_name, "expense-approval.form");

    let by_name = service
        .create_form_definition_query()
        .name("Travel request")
        .list()
        .unwrap();
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].key, "travelRequest");
}

#[test]
fn form_definition_query_rejects_unsupported_filters_structurally() {
    let service = service("form-definition-query-errors");
    deploy_sample_forms(&service);

    let error = service
        .create_form_definition_query()
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
