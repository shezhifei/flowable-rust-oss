mod test_support;

use flowable_form_service::{
    FormDeploymentRequest, FormDeploymentResource, FormModel, FormSubmissionProperty,
    FormSubmissionRequest, FormSubmissionResult, validate_form_model,
};
use serde_json::{Value, json};
use test_support::{deploy_runtime_forms, deploy_runtime_process, runtime_fixture};

#[test]
fn runtime_forms_resolve_bindings_submit_values_and_persist_instances() {
    let (engine, service) = runtime_fixture("form-runtime");
    deploy_runtime_forms(&service);
    let process_definition_id = deploy_runtime_process(
        &engine,
        "travelRequestProcess",
        "travelRequest",
        "expenseApproval",
    );

    let start_form = service.get_start_form_data(&process_definition_id).unwrap();
    assert_eq!(start_form.form_key.as_deref(), Some("travelRequest"));
    assert_eq!(
        start_form.process_definition_id.as_deref(),
        Some(process_definition_id.as_str())
    );
    assert_eq!(start_form.task_id, None);
    assert_eq!(start_form.form_properties.len(), 2);
    assert_eq!(start_form.form_properties[0].id, "requester");
    assert!(start_form.form_properties[0].required);

    let process_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id.clone()),
            task_id: None,
            business_key: Some("travel-001".to_string()),
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "requester".to_string(),
                    value: json!("alice"),
                },
                FormSubmissionProperty {
                    id: "amount".to_string(),
                    value: json!("42.5"),
                },
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(process_instance) => process_instance,
        other => panic!("expected process instance result, got {other:?}"),
    };

    let root_variables = engine
        .get_variable_service()
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        root_variables.get("requester"),
        Some(&Value::String("alice".to_string()))
    );
    assert_eq!(root_variables.get("amount"), Some(&json!(42.5)));

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let task_form = service.get_task_form_data(&tasks[0].id).unwrap();
    assert_eq!(task_form.form_key.as_deref(), Some("expenseApproval"));
    assert_eq!(task_form.task_id.as_deref(), Some(tasks[0].id.as_str()));
    assert_eq!(task_form.form_properties.len(), 2);
    assert_eq!(task_form.form_properties[0].id, "approved");

    let task_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: None,
            task_id: Some(tasks[0].id.clone()),
            business_key: None,
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "approved".to_string(),
                    value: json!("true"),
                },
                FormSubmissionProperty {
                    id: "comment".to_string(),
                    value: json!("approved"),
                },
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::TaskCompleted(form_instance) => form_instance,
        other => panic!("expected task completion result, got {other:?}"),
    };

    assert_eq!(task_instance.task_id.as_deref(), Some(tasks[0].id.as_str()));
    assert_eq!(
        task_instance.process_instance_id.as_deref(),
        Some(process_instance.id.as_str())
    );

    let instances = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .unwrap();
    assert_eq!(instances.len(), 2);
    assert!(instances.iter().any(|instance| instance.task_id.is_none()));
    assert!(instances.iter().any(|instance| {
        instance.task_id.as_deref() == Some(tasks[0].id.as_str())
            && instance.values.get("approved") == Some(&json!(true))
    }));

    let stored = service.get_form_instance(&task_instance.id).unwrap();
    assert_eq!(stored.values.get("approved"), Some(&json!(true)));
    assert_eq!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn deployment_accepts_unsupported_types_and_the_boundary_validator_flags_them() {
    let (_engine, service) = runtime_fixture("form-runtime-errors");
    // Java 6.8 parses field types generically at deployment time and defers
    // rejection to runtime, so a vendor type must deploy cleanly. Strict type
    // validation lives at the modeler boundary, not on the deployment path.
    let resource = json!({
        "key": "unsupportedRuntime",
        "name": "Unsupported runtime",
        "fields": [
            { "fieldType": "BaseField", "id": "attachment", "name": "Attachment", "type": "custom_widget", "required": true }
        ]
    });
    service
        .deploy(FormDeploymentRequest {
            name: "Unsupported form".to_string(),
            resources: vec![FormDeploymentResource {
                resource_name: "unsupported-runtime.form".to_string(),
                resource: resource.clone().to_string(),
            }],
        })
        .unwrap();

    let model: FormModel = serde_json::from_value(resource).unwrap();
    let issues = validate_form_model(&model);
    assert_eq!(issues.len(), 1);
    assert_eq!(
        issues[0].code,
        "flowable-form-field-type-unsupported",
        "the modeler boundary must still reject the vendor type"
    );
}

#[test]
fn runtime_form_submission_maps_selected_outcome_to_process_variables() {
    let (engine, service) = runtime_fixture("form-runtime-outcome-mapping");
    service
        .deploy(FormDeploymentRequest {
            name: "Outcome forms".to_string(),
            resources: vec![
                FormDeploymentResource {
                    resource_name: "travel-request.form".to_string(),
                    resource: json!({
                        "key": "travelRequest",
                        "name": "Travel request",
                        "resourceName": "travel-request.form",
                        "outcomeVariableName": "startDecision",
                        "outcomes": [
                            { "id": "submit", "name": "Submit" },
                            { "id": "save", "name": "Save Draft" }
                        ],
                        "fields": [
                            { "id": "requester", "name": "Requester", "type": "string", "required": true }
                        ]
                    })
                    .to_string(),
                },
                FormDeploymentResource {
                    resource_name: "expense-approval.form".to_string(),
                    resource: json!({
                        "key": "expenseApproval",
                        "name": "Expense approval",
                        "resourceName": "expense-approval.form",
                        "outcomes": [
                            { "id": "approve", "name": "Approve" },
                            { "id": "reject", "name": "Reject" }
                        ],
                        "fields": [
                            { "id": "approved", "name": "Approved", "type": "boolean", "required": true }
                        ]
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
    let process_definition_id = deploy_runtime_process(
        &engine,
        "outcomeMappingProcess",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id),
            task_id: None,
            business_key: None,
            outcome: Some("submit".to_string()),
            properties: vec![FormSubmissionProperty {
                id: "requester".to_string(),
                value: json!("alice"),
            }],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(process_instance) => process_instance,
        other => panic!("expected process instance result, got {other:?}"),
    };

    let root_variables = engine
        .get_variable_service()
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(root_variables.get("startDecision"), Some(&json!("submit")));

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .unwrap();
    let task_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: None,
            task_id: Some(task.id.clone()),
            business_key: None,
            outcome: Some("approve".to_string()),
            properties: vec![FormSubmissionProperty {
                id: "approved".to_string(),
                value: json!(true),
            }],
        })
        .unwrap()
    {
        FormSubmissionResult::TaskCompleted(form_instance) => form_instance,
        other => panic!("expected task completion result, got {other:?}"),
    };

    let root_variables = engine
        .get_variable_service()
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        root_variables.get("form_expenseApproval_outcome"),
        Some(&json!("approve"))
    );
    assert_eq!(
        task_instance.values.get("form_expenseApproval_outcome"),
        Some(&json!("approve"))
    );
}
