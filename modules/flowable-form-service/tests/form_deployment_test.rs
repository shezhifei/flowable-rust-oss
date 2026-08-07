mod test_support;

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_form_service::{
    FlowableFormService, FormDeploymentRequest, FormDeploymentResource, FormManagementService,
};
use serde_json::json;
use std::fs;
use std::sync::Arc;
use test_support::{persistent_service, service};
use uuid::Uuid;

#[test]
fn deployment_registers_form_definitions_with_deterministic_order_and_versions() {
    let db_path = std::env::temp_dir()
        .join(format!("flowable-form-service-{}.db", Uuid::new_v4()))
        .to_string_lossy()
        .into_owned();

    let service = persistent_service("form-deployment", &db_path);
    let first = service
        .deploy(FormDeploymentRequest {
            name: "First".to_string(),
            resources: vec![FormDeploymentResource {
                resource_name: "expense-approval.form".to_string(),
                resource: json!({
                    "key": "expenseApproval",
                    "name": "Expense approval",
                    "resourceName": "expense-approval.form",
                    "fields": []
                })
                .to_string(),
            }],
        })
        .unwrap();
    assert_eq!(first.resource_names, vec!["expense-approval.form"]);

    let reloaded = persistent_service("form-deployment-reloaded", &db_path);
    let second = reloaded
        .deploy(FormDeploymentRequest {
            name: "Second".to_string(),
            resources: vec![FormDeploymentResource {
                resource_name: "expense-approval-v2.form".to_string(),
                resource: json!({
                    "key": "expenseApproval",
                    "name": "Expense approval v2",
                    "resourceName": "expense-approval-v2.form",
                    "fields": []
                })
                .to_string(),
            }],
        })
        .unwrap();

    let versions = reloaded
        .create_form_definition_query()
        .key("expenseApproval")
        .list()
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version, 2);
    assert_eq!(versions[1].version, 1);
    assert_eq!(versions[0].deployment_id, second.id);

    let _ = fs::remove_file(db_path);
}

#[test]
fn deployment_allows_duplicate_form_keys_in_the_same_deployment_with_incrementing_versions() {
    let service = service("form-deployment-duplicate-keys");

    let deployment = service
        .deploy(FormDeploymentRequest {
            name: "Duplicate keys".to_string(),
            resources: vec![
                FormDeploymentResource {
                    resource_name: "travel-request.form".to_string(),
                    resource: json!({
                        "key": "travelRequest",
                        "name": "Travel request",
                        "resourceName": "travel-request.form",
                        "fields": []
                    })
                    .to_string(),
                },
                FormDeploymentResource {
                    resource_name: "travel-request-copy.form".to_string(),
                    resource: json!({
                        "key": "travelRequest",
                        "name": "Travel request copy",
                        "resourceName": "travel-request-copy.form",
                        "fields": []
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();

    let versions = service
        .create_form_definition_query()
        .key("travelRequest")
        .list()
        .unwrap();
    assert_eq!(versions.len(), 2);
    // Both definitions share the same deployment
    assert_eq!(versions[0].deployment_id, deployment.id);
    assert_eq!(versions[1].deployment_id, deployment.id);
    // They get incrementing version numbers
    assert_eq!(versions[0].version, 2);
    assert_eq!(versions[1].version, 1);
}

#[test]
fn deployment_persists_layout_outcomes_and_outcome_variable_name() {
    let db_path = std::env::temp_dir()
        .join(format!(
            "flowable-form-service-layout-{}.db",
            Uuid::new_v4()
        ))
        .to_string_lossy()
        .into_owned();

    let service = persistent_service("form-deployment-layout", &db_path);
    let deployment = service
        .deploy(FormDeploymentRequest {
            name: "Layout outcomes deployment".to_string(),
            resources: vec![FormDeploymentResource {
                resource_name: "employee-onboarding.form".to_string(),
                resource: json!({
                    "key": "employeeOnboarding",
                    "name": "Employee Onboarding",
                    "resourceName": "employee-onboarding.form",
                    "outcomes": [
                        { "id": "submit", "name": "Submit" },
                        { "id": "save", "name": "Save Draft" }
                    ],
                    "outcomeVariableName": "formOutcome",
                    "layout": {
                        "columns": 2,
                        "type": "two-column"
                    },
                    "fields": []
                })
                .to_string(),
            }],
        })
        .unwrap();

    let definition = service
        .get_form_definition(&format!("{}:{}", deployment.id, "employee-onboarding.form"))
        .unwrap();

    assert_eq!(
        definition.outcome_variable_name.as_deref(),
        Some("formOutcome")
    );
    assert_eq!(
        definition.layout,
        Some(json!({
            "columns": 2,
            "type": "two-column"
        }))
    );
    let outcomes = definition.outcomes.expect("outcomes should be persisted");
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].id.as_deref(), Some("submit"));
    assert_eq!(outcomes[0].name.as_deref(), Some("Submit"));
    assert_eq!(outcomes[1].id.as_deref(), Some("save"));
    assert_eq!(outcomes[1].name.as_deref(), Some("Save Draft"));

    let reloaded_engine = Arc::new(ProcessEngine::new_with_db_path(
        "form-deployment-layout-reloaded".to_string(),
        &db_path,
    ));
    let _reloaded_form_service = FlowableFormService::new(Arc::clone(&reloaded_engine));
    let management = FormManagementService::new(Arc::clone(&reloaded_engine));
    let versions = management.list_versions("employeeOnboarding").unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(
        versions[0].outcome_variable_name.as_deref(),
        Some("formOutcome")
    );
    assert_eq!(
        versions[0].layout,
        Some(json!({
            "columns": 2,
            "type": "two-column"
        }))
    );
    assert_eq!(versions[0].outcomes.as_ref().map(Vec::len), Some(2));

    let latest = management.get_latest_version("employeeOnboarding").unwrap();
    assert_eq!(latest.outcome_variable_name.as_deref(), Some("formOutcome"));
    assert_eq!(latest.outcomes.as_ref().map(Vec::len), Some(2));

    let _ = fs::remove_file(db_path);
}
