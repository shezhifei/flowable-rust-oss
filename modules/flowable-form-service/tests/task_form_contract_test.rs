//! Engine-side Java parity contract for CompleteTaskWithForm (P2-FORM).
//!
//! Java: `CompleteTaskWithFormCmd` + `FormService.saveFormInstance(..., outcome)`
//! — form instance, variables, and task complete share one command session.

mod test_support;

use flowable_engine::error::FlowableError;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use flowable_form_service::{
    CompleteTaskWithFormCmd, CompleteTaskWithFormInput, FORCE_FAIL_FORM_OUTCOME,
    FormDeploymentRequest, FormDeploymentResource, FormModel, FormSubmissionProperty,
    FormSubmissionRequest, FormSubmissionResult, validate_form_model,
};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use test_support::{deploy_runtime_forms, deploy_runtime_process, runtime_fixture};

#[test]
fn complete_with_form_definition_writes_variables_instance_and_outcome() {
    let (engine, service) = runtime_fixture("form-complete-happy");
    deploy_runtime_forms(&service);
    let process_definition_id = deploy_runtime_process(
        &engine,
        "formCompleteHappy",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id),
            task_id: None,
            business_key: None,
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "requester".into(),
                    value: json!("alice"),
                },
                FormSubmissionProperty {
                    id: "amount".into(),
                    value: json!(10),
                },
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(pi) => pi,
        other => panic!("expected process instance, got {other:?}"),
    };

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .expect("task");

    let form_definition_id = service
        .create_form_definition_query()
        .key("expenseApproval")
        .list()
        .unwrap()
        .into_iter()
        .max_by_key(|d| d.version)
        .unwrap()
        .id;

    let mut variables = HashMap::new();
    variables.insert("approved".into(), json!(true));
    variables.insert("comment".into(), json!("ok"));

    let instance = service
        .complete_task_with_form_definition(
            task.id.clone(),
            form_definition_id.clone(),
            Some("approve".into()),
            variables,
            false,
            HashMap::new(),
            Some("admin".into()),
        )
        .unwrap();

    assert_eq!(instance.form_definition_id, form_definition_id);
    assert_eq!(instance.task_id.as_deref(), Some(task.id.as_str()));
    assert_eq!(instance.outcome.as_deref(), Some("approve"));
    assert_eq!(instance.values.get("approved"), Some(&json!(true)));
    assert_eq!(instance.submitted_by.as_deref(), Some("admin"));

    // Task completed
    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .is_empty()
    );

    // Outcome variable written (default form_{key}_outcome)
    let vars = engine
        .get_variable_service()
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(vars.get("approved"), Some(&json!(true)));
    assert_eq!(
        vars.get("form_expenseApproval_outcome"),
        Some(&json!("approve"))
    );

    // Form instance queryable
    let listed = service
        .create_form_instance_query()
        .task_id(task.id.clone())
        .list()
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].outcome.as_deref(), Some("approve"));
}

#[test]
fn unsupported_field_type_deploys_leniently_and_the_boundary_validator_flags_it() {
    let (_engine, service) = runtime_fixture("form-complete-rollback-type");
    // Java 6.8 deploys vendor field types generically and defers rejection to
    // runtime; strict validation lives at the modeler boundary, so a form like
    // this can never be published from the designer even though it deploys.
    let resource = json!({
        "key": "unsupportedRuntime",
        "name": "Unsupported runtime",
        "fields": [
            { "fieldType": "BaseField", "id": "attachment", "name": "Attachment", "type": "custom_widget", "required": true }
        ]
    });
    service
        .deploy(FormDeploymentRequest {
            name: "Unsupported form".into(),
            resources: vec![FormDeploymentResource {
                resource_name: "unsupported-runtime.form".into(),
                resource: resource.clone().to_string(),
            }],
        })
        .unwrap();

    let model: FormModel = serde_json::from_value(resource).unwrap();
    let issues = validate_form_model(&model);
    assert!(
        issues
            .iter()
            .any(|issue| issue.code == "flowable-form-field-type-unsupported"),
        "expected a type-unsupported issue, got {issues:?}"
    );
}

#[test]
fn complete_with_missing_form_definition_is_not_found_without_side_effects() {
    let (engine, service) = runtime_fixture("form-complete-missing-def");
    deploy_runtime_forms(&service);
    let process_definition_id = deploy_runtime_process(
        &engine,
        "formCompleteMissingDef",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_key("formCompleteMissingDef")
        .unwrap();
    let _ = process_definition_id;

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .expect("task");

    let err = service
        .complete_task_with_form_definition(
            task.id.clone(),
            "form-definition-does-not-exist",
            Some("approve".into()),
            HashMap::from([("approved".into(), json!(true))]),
            false,
            HashMap::new(),
            None,
        )
        .unwrap_err();

    assert!(matches!(err, FlowableError::NotFound(_)));

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let vars = engine
        .get_variable_service()
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert!(vars.get("approved").is_none());

    assert!(
        service
            .create_form_instance_query()
            .task_id(task.id)
            .list()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn complete_without_form_definition_id_leaves_existing_path_unchanged() {
    // Covered by CompleteTaskByIdCmd path — variables only, no form instance.
    let (engine, service) = runtime_fixture("form-complete-no-form-id");
    deploy_runtime_forms(&service);
    let _ = deploy_runtime_process(
        &engine,
        "formCompleteNoFormId",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_key("formCompleteNoFormId")
        .unwrap();

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .expect("task");

    engine
        .get_task_service()
        .complete_task_by_id_with_variables(
            task.id.clone(),
            HashMap::from([("approved".into(), json!(true))]),
        )
        .unwrap();

    assert!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .is_empty()
    );
    assert!(
        service
            .create_form_instance_query()
            .task_id(task.id)
            .list()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn complete_with_form_rejects_suspended_task_without_side_effects() {
    let (engine, service) = runtime_fixture("form-complete-suspended");
    service
        .deploy(FormDeploymentRequest {
            name: "suspend form".into(),
            resources: vec![FormDeploymentResource {
                resource_name: "simple.form".into(),
                resource: json!({
                    "key": "simpleForm",
                    "name": "Simple",
                    "resourceName": "simple.form",
                    "fields": [
                        { "id": "note", "type": "string" }
                    ]
                })
                .to_string(),
            }],
        })
        .unwrap();

    let form_definition_id = service
        .create_form_definition_query()
        .key("simpleForm")
        .list()
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .id;

    // Standalone suspended task (no process) — command still rejects.
    let store = engine.get_runtime_store();
    let mut task = flowable_engine::task::Task::new(
        "task-suspended-form".into(),
        "pi-1".into(),
        "pi-1".into(),
        "review".into(),
        "Review".into(),
    );
    task.set_suspension_state(true);
    let mut session = store.create_session().unwrap();
    store.insert_task(&task, &mut session);
    session.flush_and_commit().unwrap();

    let err = service
        .complete_task_with_form_definition(
            "task-suspended-form",
            form_definition_id,
            Some("ok".into()),
            HashMap::from([("note".into(), json!("x"))]),
            false,
            HashMap::new(),
            None,
        )
        .unwrap_err();

    assert!(
        matches!(err, FlowableError::ExecutionError(ref m) if m.contains("suspended")),
        "got {err:?}"
    );
    assert!(
        service
            .create_form_instance_query()
            .task_id("task-suspended-form")
            .list()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn mid_command_failure_rolls_back_form_instance_and_task_complete() {
    let (engine, service) = runtime_fixture("form-complete-force-fail");
    deploy_runtime_forms(&service);
    let _ = deploy_runtime_process(
        &engine,
        "formCompleteForceFail",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_key("formCompleteForceFail")
        .unwrap();

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .expect("task");

    let form_definition_id = service
        .create_form_definition_query()
        .key("expenseApproval")
        .list()
        .unwrap()
        .into_iter()
        .max_by_key(|d| d.version)
        .unwrap()
        .id;

    let definition = service.get_form_definition(&form_definition_id).unwrap();
    let mut form_values = BTreeMap::new();
    form_values.insert("approved".into(), json!(true));
    let mut task_variables = HashMap::new();
    task_variables.insert("approved".into(), json!(true));

    // Bypass field validation; exercise command-level force-fail after insert.
    let cmd = CompleteTaskWithFormCmd::new(CompleteTaskWithFormInput {
        task_id: task.id.clone(),
        form_definition_id,
        outcome: Some(FORCE_FAIL_FORM_OUTCOME.into()),
        task_variables,
        form_instance_values: form_values,
        local_scope: false,
        transient_variables: HashMap::new(),
        submitted_by: None,
        form_definition: Some(definition),
        process_definition_id: None,
        form_properties: Vec::new(),
        handlers: BTreeMap::new(),
    });
    let err = engine.get_command_executor().execute(&cmd).unwrap_err();
    assert!(matches!(err, FlowableError::BadRequest(_)));

    assert_eq!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .len(),
        1
    );
    assert!(
        service
            .create_form_instance_query()
            .task_id(task.id)
            .list()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn submit_form_task_path_persists_outcome_on_form_instance() {
    let (engine, service) = runtime_fixture("form-submit-outcome-field");
    service
        .deploy(FormDeploymentRequest {
            name: "Outcome field forms".into(),
            resources: vec![
                FormDeploymentResource {
                    resource_name: "travel-request.form".into(),
                    resource: json!({
                        "key": "travelRequest",
                        "name": "Travel request",
                        "resourceName": "travel-request.form",
                        "fields": [
                            { "id": "requester", "type": "string", "required": true }
                        ]
                    })
                    .to_string(),
                },
                FormDeploymentResource {
                    resource_name: "expense-approval.form".into(),
                    resource: json!({
                        "key": "expenseApproval",
                        "name": "Expense approval",
                        "resourceName": "expense-approval.form",
                        "fields": [
                            { "id": "approved", "type": "boolean", "required": true }
                        ]
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
    let process_definition_id = deploy_runtime_process(
        &engine,
        "formSubmitOutcomeField",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id),
            task_id: None,
            business_key: None,
            outcome: None,
            properties: vec![FormSubmissionProperty {
                id: "requester".into(),
                value: json!("bob"),
            }],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(pi) => pi,
        other => panic!("expected process instance, got {other:?}"),
    };

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .unwrap();

    let instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: None,
            task_id: Some(task.id.clone()),
            business_key: None,
            outcome: Some("approve".into()),
            properties: vec![FormSubmissionProperty {
                id: "approved".into(),
                value: json!(true),
            }],
        })
        .unwrap()
    {
        FormSubmissionResult::TaskCompleted(fi) => fi,
        other => panic!("expected task completed, got {other:?}"),
    };

    assert_eq!(instance.outcome.as_deref(), Some("approve"));
}
