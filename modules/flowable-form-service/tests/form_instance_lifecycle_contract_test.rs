//! Form instance values bytes + delete lifecycle (Java FormService contract).
//!
//! Java truth sources:
//! - FormService.getFormInstanceValues / deleteFormInstance /
//!   deleteFormInstancesByFormDefinition / deleteFormInstancesByProcessDefinition /
//!   deleteFormInstancesByScopeDefinition
//! - FormInstance.formValuesId / formValueBytes / scopeDefinitionId / tenantId

mod test_support;

use flowable_form_service::{
    form_instance_values_bytes, FormInstance, FormSubmissionProperty, FormSubmissionRequest,
    FormSubmissionResult,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use test_support::{deploy_runtime_forms, deploy_runtime_process, runtime_fixture};

#[test]
fn form_instance_persists_values_id_bytes_scope_definition_and_tenant() {
    let (engine, service) = runtime_fixture("form-instance-lifecycle-values");
    deploy_runtime_forms(&service);

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Tenant lifecycle process".to_string())
                .tenant_id("tenant-lifecycle".to_string())
                .add_string(
                    "lifecycle-process.bpmn20.xml".to_string(),
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="lifecycleProcess" name="lifecycleProcess" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="travelRequest" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="approveTask" />
        <userTask id="approveTask" name="Approve" flowable:formKey="expenseApproval" />
        <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#
                        .to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("lifecycleProcess", Some("tenant-lifecycle"))
        .unwrap()
        .unwrap()
        .id;

    let process_instance = match service
        .submit_form_as(
            FormSubmissionRequest {
                process_definition_id: Some(process_definition_id.clone()),
                task_id: None,
                business_key: Some("lifecycle-1".to_string()),
                outcome: None,
                properties: vec![
                    FormSubmissionProperty {
                        id: "requester".to_string(),
                        value: json!("alice"),
                    },
                    FormSubmissionProperty {
                        id: "amount".to_string(),
                        value: json!(99),
                    },
                ],
            },
            "alice",
        )
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(pi) => pi,
        other => panic!("expected process instance, got {other:?}"),
    };

    let instances = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .unwrap();
    assert_eq!(instances.len(), 1);
    let instance = &instances[0];

    assert!(
        instance.form_values_id.as_deref().is_some_and(|id| !id.is_empty()),
        "form_values_id must be assigned at write"
    );
    let stored_bytes = instance
        .form_value_bytes
        .as_ref()
        .expect("form_value_bytes must be stored at write");
    let parsed: BTreeMap<String, Value> = serde_json::from_slice(stored_bytes).unwrap();
    assert_eq!(parsed.get("requester"), Some(&json!("alice")));
    assert_eq!(parsed.get("amount"), Some(&json!(99.0)).or(Some(&json!(99))));

    let via_api = service
        .get_form_instance_values(&instance.id)
        .unwrap();
    assert_eq!(via_api, *stored_bytes);

    assert_eq!(
        instance.scope_definition_id.as_deref(),
        Some(process_definition_id.as_str())
    );
    assert_eq!(instance.tenant_id.as_deref(), Some("tenant-lifecycle"));
    assert_eq!(
        service
            .create_form_instance_query()
            .tenant_id("tenant-lifecycle")
            .count()
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .create_form_instance_query()
            .tenant_id_like("tenant-%")
            .count()
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .create_form_instance_query()
            .without_tenant_id()
            .count()
            .unwrap(),
        0
    );
}

#[test]
fn legacy_form_instance_without_value_bytes_derives_on_read() {
    let (_engine, service) = runtime_fixture("form-instance-legacy-bytes");
    deploy_runtime_forms(&service);

    // Simulate a legacy row: values present, form_value_bytes absent.
    let mut values = BTreeMap::new();
    values.insert("requester".to_string(), json!("legacy-user"));
    values.insert("amount".to_string(), json!(7));
    let legacy = FormInstance {
        id: "form-instance:legacy-1".to_string(),
        form_definition_id: "def-legacy".to_string(),
        form_definition_key: "travelRequest".to_string(),
        form_definition_name: "Travel request".to_string(),
        deployment_id: "dep-legacy".to_string(),
        process_definition_id: None,
        process_instance_id: None,
        task_id: None,
        scope_type: "start".to_string(),
        scope_id: "scope-legacy".to_string(),
        scope_definition_id: None,
        submitted_at: 1,
        submitted_by: Some("legacy".to_string()),
        tenant_id: None,
        form_values_id: None,
        form_value_bytes: None,
        outcome: None,
        values: values.clone(),
    };

    // Direct repository insert via service internals is not public; use values helper.
    let derived = form_instance_values_bytes(&legacy);
    let parsed: BTreeMap<String, Value> = serde_json::from_slice(&derived).unwrap();
    assert_eq!(parsed, values);
    // Ensure derivation does not require mutation: original still has None bytes.
    assert!(legacy.form_value_bytes.is_none());
}

#[test]
fn delete_form_instance_by_id_and_bulk_deletes() {
    let (engine, service) = runtime_fixture("form-instance-lifecycle-delete");
    deploy_runtime_forms(&service);
    let process_definition_id = deploy_runtime_process(
        &engine,
        "instanceDeleteProcess",
        "travelRequest",
        "expenseApproval",
    );

    let process_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id.clone()),
            task_id: None,
            business_key: Some("delete-1".to_string()),
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "requester".to_string(),
                    value: json!("dave"),
                },
                FormSubmissionProperty {
                    id: "amount".to_string(),
                    value: json!(12),
                },
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(pi) => pi,
        other => panic!("expected process instance, got {other:?}"),
    };

    let start = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .unwrap()
        .pop()
        .unwrap();

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
            outcome: None,
            properties: vec![FormSubmissionProperty {
                id: "approved".to_string(),
                value: json!(false),
            }],
        })
        .unwrap()
    {
        FormSubmissionResult::TaskCompleted(instance) => instance,
        other => panic!("expected task completed, got {other:?}"),
    };

    // Single delete by id
    service.delete_form_instance(&start.id).unwrap();
    assert!(service.get_form_instance(&start.id).is_err());
    assert!(service.get_form_instance(&task_instance.id).is_ok());

    // Bulk by form definition removes remaining instance
    let deleted = service
        .delete_form_instances_by_form_definition(&task_instance.form_definition_id)
        .unwrap();
    assert_eq!(deleted, 1);
    assert!(service.get_form_instance(&task_instance.id).is_err());

    // Create fresh instances for process-definition bulk delete
    let process_instance2 = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id.clone()),
            task_id: None,
            business_key: Some("delete-2".to_string()),
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "requester".to_string(),
                    value: json!("erin"),
                },
                FormSubmissionProperty {
                    id: "amount".to_string(),
                    value: json!(15),
                },
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(pi) => pi,
        other => panic!("expected process instance, got {other:?}"),
    };
    assert_eq!(
        service
            .create_form_instance_query()
            .process_instance_id(process_instance2.id.clone())
            .count()
            .unwrap(),
        1
    );
    let deleted_pd = service
        .delete_form_instances_by_process_definition(&process_definition_id)
        .unwrap();
    assert!(deleted_pd >= 1);
    assert_eq!(
        service
            .create_form_instance_query()
            .process_definition_id(process_definition_id.clone())
            .count()
            .unwrap(),
        0
    );

    // Scope-definition bulk delete
    let process_instance3 = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id.clone()),
            task_id: None,
            business_key: Some("delete-3".to_string()),
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "requester".to_string(),
                    value: json!("frank"),
                },
                FormSubmissionProperty {
                    id: "amount".to_string(),
                    value: json!(18),
                },
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(pi) => pi,
        other => panic!("expected process instance, got {other:?}"),
    };
    let start3 = service
        .create_form_instance_query()
        .process_instance_id(process_instance3.id)
        .list()
        .unwrap()
        .pop()
        .unwrap();
    let scope_def = start3
        .scope_definition_id
        .clone()
        .expect("scope_definition_id");
    let deleted_scope = service
        .delete_form_instances_by_scope_definition(&scope_def)
        .unwrap();
    assert!(deleted_scope >= 1);
    assert!(service.get_form_instance(&start3.id).is_err());

    // Missing id is NotFound
    let err = service
        .delete_form_instance("form-instance:missing")
        .unwrap_err();
    assert!(format!("{err}").contains("was not found") || format!("{err}").contains("NotFound"));
}
