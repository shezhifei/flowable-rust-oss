mod test_support;

use flowable_form_service::{FormSubmissionProperty, FormSubmissionRequest, FormSubmissionResult};
use serde_json::json;
use test_support::{deploy_runtime_forms, deploy_runtime_process, runtime_fixture};

#[test]
fn form_instance_query_supports_runtime_scope_filters() {
    let (engine, service) = runtime_fixture("form-instance-query");
    deploy_runtime_forms(&service);
    let process_definition_id = deploy_runtime_process(
        &engine,
        "instanceQueryProcess",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id.clone()),
            task_id: None,
            business_key: Some("travel-002".to_string()),
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "requester".to_string(),
                    value: json!("bob"),
                },
                FormSubmissionProperty {
                    id: "amount".to_string(),
                    value: json!(10),
                },
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(process_instance) => process_instance,
        other => panic!("expected process instance result, got {other:?}"),
    };

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .unwrap();

    let task_instance_id = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: None,
            task_id: Some(task.id.clone()),
            business_key: None,
            outcome: None,
            properties: vec![FormSubmissionProperty {
                id: "approved".to_string(),
                value: json!(false),
            }],
        })
        .unwrap()
    {
        FormSubmissionResult::TaskCompleted(form_instance) => form_instance.id,
        other => panic!("expected task completion result, got {other:?}"),
    };

    let by_process_definition = service
        .create_form_instance_query()
        .process_definition_id(process_definition_id.clone())
        .list()
        .unwrap();
    assert_eq!(by_process_definition.len(), 2);

    let by_process_instance = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .unwrap();
    assert_eq!(by_process_instance.len(), 2);

    let by_task = service
        .create_form_instance_query()
        .task_id(task.id.clone())
        .list()
        .unwrap();
    assert_eq!(by_task.len(), 1);
    assert_eq!(by_task[0].id, task_instance_id);

    let paged = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .page(1, 1)
        .list_page()
        .unwrap();
    assert_eq!(paged.start, 1);
    assert_eq!(paged.size, 1);
    assert_eq!(paged.total, 2);
}

#[test]
fn form_instance_query_filters_by_submitted_by() {
    let (engine, service) = runtime_fixture("form-instance-query-submitted-by");
    deploy_runtime_forms(&service);
    let process_definition_id = deploy_runtime_process(
        &engine,
        "instanceQuerySubmittedByProcess",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = match service
        .submit_form_as(
            FormSubmissionRequest {
                process_definition_id: Some(process_definition_id.clone()),
                task_id: None,
                business_key: Some("travel-003".to_string()),
                outcome: None,
                properties: vec![
                    FormSubmissionProperty {
                        id: "requester".to_string(),
                        value: json!("carol"),
                    },
                    FormSubmissionProperty {
                        id: "amount".to_string(),
                        value: json!(25),
                    },
                ],
            },
            "carol",
        )
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(process_instance) => process_instance,
        other => panic!("expected process instance result, got {other:?}"),
    };

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .unwrap();

    let task_instance = match service
        .submit_form_as(
            FormSubmissionRequest {
                process_definition_id: None,
                task_id: Some(task.id.clone()),
                business_key: None,
                outcome: None,
                properties: vec![FormSubmissionProperty {
                    id: "approved".to_string(),
                    value: json!(true),
                }],
            },
            "manager",
        )
        .unwrap()
    {
        FormSubmissionResult::TaskCompleted(form_instance) => form_instance,
        other => panic!("expected task completion result, got {other:?}"),
    };

    let by_requester = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .submitted_by("carol")
        .list()
        .unwrap();
    assert_eq!(by_requester.len(), 1);
    assert_eq!(by_requester[0].submitted_by.as_deref(), Some("carol"));
    assert_eq!(by_requester[0].form_definition_key, "travelRequest");

    let by_manager = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .submitted_by("manager")
        .list()
        .unwrap();
    assert_eq!(by_manager.len(), 1);
    assert_eq!(by_manager[0].id, task_instance.id);
    assert_eq!(by_manager[0].submitted_by.as_deref(), Some("manager"));
}

#[test]
fn form_instance_query_filters_by_submitted_dates_and_submitter_like() {
    let (engine, service) = runtime_fixture("form-instance-query-date-like");
    deploy_runtime_forms(&service);
    let process_definition_id = deploy_runtime_process(
        &engine,
        "instanceQueryDateLikeProcess",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = match service
        .submit_form_as(
            FormSubmissionRequest {
                process_definition_id: Some(process_definition_id),
                task_id: None,
                business_key: Some("travel-004".to_string()),
                outcome: None,
                properties: vec![
                    FormSubmissionProperty {
                        id: "requester".to_string(),
                        value: json!("dave"),
                    },
                    FormSubmissionProperty {
                        id: "amount".to_string(),
                        value: json!(31),
                    },
                ],
            },
            "regional-manager",
        )
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(process_instance) => process_instance,
        other => panic!("expected process instance result, got {other:?}"),
    };

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .unwrap();
    let task_instance = match service
        .submit_form_as(
            FormSubmissionRequest {
                process_definition_id: None,
                task_id: Some(task.id.clone()),
                business_key: None,
                outcome: None,
                properties: vec![FormSubmissionProperty {
                    id: "approved".to_string(),
                    value: json!(true),
                }],
            },
            "regional-approver",
        )
        .unwrap()
    {
        FormSubmissionResult::TaskCompleted(form_instance) => form_instance,
        other => panic!("expected task completion result, got {other:?}"),
    };

    let submitted_at = task_instance.submitted_at;
    // Scope by task_id: start-form and task-form can share the same millisecond
    // timestamp on fast machines, so date-only filter is not unique.
    let exact = service
        .create_form_instance_query()
        .submitted_date(submitted_at)
        .task_id(task.id.clone())
        .list()
        .unwrap();
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].id, task_instance.id);

    let after = service
        .create_form_instance_query()
        .submitted_date_after(submitted_at - 1)
        .list()
        .unwrap();
    assert!(after.iter().any(|instance| instance.id == task_instance.id));

    let before = service
        .create_form_instance_query()
        .submitted_date_before(submitted_at + 1)
        .list()
        .unwrap();
    assert!(
        before
            .iter()
            .any(|instance| instance.id == task_instance.id)
    );

    let by_like = service
        .create_form_instance_query()
        .submitted_by_like("regional-%")
        .list()
        .unwrap();
    assert_eq!(by_like.len(), 2);
}

#[test]
fn form_instance_query_supports_ids_likes_scope_tenant_and_without_task() {
    let (engine, service) = runtime_fixture("form-instance-query-extended");
    deploy_runtime_forms(&service);
    let process_definition_id = deploy_runtime_process(
        &engine,
        "instanceQueryExtendedProcess",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = match service
        .submit_form_as(
            FormSubmissionRequest {
                process_definition_id: Some(process_definition_id.clone()),
                task_id: None,
                business_key: Some("travel-extended".to_string()),
                outcome: None,
                properties: vec![
                    FormSubmissionProperty {
                        id: "requester".to_string(),
                        value: json!("erin"),
                    },
                    FormSubmissionProperty {
                        id: "amount".to_string(),
                        value: json!(40),
                    },
                ],
            },
            "erin",
        )
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(process_instance) => process_instance,
        other => panic!("expected process instance result, got {other:?}"),
    };

    let start_instances = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .without_task_id()
        .list()
        .unwrap();
    assert_eq!(start_instances.len(), 1);
    let start_instance = &start_instances[0];
    assert!(start_instance.task_id.is_none() || start_instance.task_id.as_deref() == Some(""));
    assert_eq!(
        start_instance.scope_definition_id.as_deref(),
        Some(process_definition_id.as_str())
    );
    assert!(start_instance.form_values_id.is_some());
    assert!(start_instance.form_value_bytes.is_some());

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .unwrap();

    let task_instance = match service
        .submit_form_as(
            FormSubmissionRequest {
                process_definition_id: None,
                task_id: Some(task.id.clone()),
                business_key: None,
                outcome: None,
                properties: vec![FormSubmissionProperty {
                    id: "approved".to_string(),
                    value: json!(true),
                }],
            },
            "manager",
        )
        .unwrap()
    {
        FormSubmissionResult::TaskCompleted(form_instance) => form_instance,
        other => panic!("expected task completion result, got {other:?}"),
    };

    // ids filter
    let by_ids = service
        .create_form_instance_query()
        .ids([start_instance.id.clone(), task_instance.id.clone()])
        .list()
        .unwrap();
    assert_eq!(by_ids.len(), 2);

    // id-like filters
    let by_def_like = service
        .create_form_instance_query()
        .form_definition_id_like("%expense%")
        .list()
        .unwrap();
    assert!(
        by_def_like
            .iter()
            .any(|instance| instance.id == task_instance.id)
    );

    let by_task_like = service
        .create_form_instance_query()
        .task_id_like(&format!("{}%", &task.id[..task.id.len().min(8)]))
        .list()
        .unwrap();
    assert!(
        by_task_like
            .iter()
            .any(|instance| instance.id == task_instance.id)
    );

    let by_pi_like = service
        .create_form_instance_query()
        .process_instance_id_like(&format!("{}%", &process_instance.id[..8.min(process_instance.id.len())]))
        .list()
        .unwrap();
    assert!(by_pi_like.len() >= 2);

    let by_pd_like = service
        .create_form_instance_query()
        .process_definition_id_like(&format!(
            "{}%",
            &process_definition_id[..8.min(process_definition_id.len())]
        ))
        .list()
        .unwrap();
    assert!(by_pd_like.len() >= 2);

    // scope filters
    let by_scope = service
        .create_form_instance_query()
        .scope_type("task")
        .scope_id(task.id.clone())
        .list()
        .unwrap();
    assert_eq!(by_scope.len(), 1);
    assert_eq!(by_scope[0].id, task_instance.id);

    let by_scope_def = service
        .create_form_instance_query()
        .scope_definition_id(process_definition_id.clone())
        .list()
        .unwrap();
    assert_eq!(by_scope_def.len(), 2);

    // withoutTaskId excludes the task form
    let without_task = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .without_task_id()
        .list()
        .unwrap();
    assert_eq!(without_task.len(), 1);
    assert_eq!(without_task[0].id, start_instance.id);

    // sorting + paging
    let sorted = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .order_by_submitted_date()
        .desc()
        .list()
        .unwrap();
    assert_eq!(sorted.len(), 2);
    assert!(sorted[0].submitted_at >= sorted[1].submitted_at);

    let page = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .order_by_submitted_date()
        .asc()
        .page(0, 1)
        .list_page()
        .unwrap();
    assert_eq!(page.start, 0);
    assert_eq!(page.size, 1);
    assert_eq!(page.total, 2);
}
