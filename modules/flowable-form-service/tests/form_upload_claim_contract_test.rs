//! Form upload claim contract at the service level (P1 tenant fix).
//!
//! Covers the review gaps: tenantless submit referencing tenant content,
//! tenant submit referencing tenantless content, and resubmission of content
//! already associated with another process.

mod test_support;

use flowable_content_service::{CreateContentItemRequest, FlowableContentService};
use flowable_engine::error::FlowableError;
use flowable_form_service::{
    FormDeploymentRequest, FormDeploymentResource, FormSubmissionProperty, FormSubmissionRequest,
    FormSubmissionResult,
};
use serde_json::json;
use std::sync::Arc;
use test_support::runtime_fixture;

fn deploy_upload_form(service: &flowable_form_service::FlowableFormService) {
    service
        .deploy(FormDeploymentRequest {
            name: "Upload forms".to_string(),
            resources: vec![FormDeploymentResource {
                resource_name: "upload-request.form".to_string(),
                resource: json!({
                    "key": "uploadRequest",
                    "name": "Upload request",
                    "resourceName": "upload-request.form",
                    "fields": [
                        { "id": "title", "name": "Title", "type": "string", "required": true },
                        { "id": "files", "name": "Files", "type": "upload", "required": true }
                    ]
                })
                .to_string(),
            }],
        })
        .unwrap();
}

fn deploy_upload_process(
    engine: &Arc<flowable_engine::engine::process_engine::ProcessEngine>,
    tenant_id: Option<&str>,
) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="uploadClaimProcess" name="uploadClaimProcess" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="uploadRequest" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;
    let mut builder = engine
        .get_repository_service()
        .create_deployment()
        .name("upload claim process".to_string())
        .add_string("upload-claim.bpmn20.xml".to_string(), xml.to_string());
    if let Some(tenant) = tenant_id {
        builder = builder.tenant_id(tenant.to_string());
    }
    engine.get_repository_service().deploy(builder).unwrap();
    engine
        .get_repository_service()
        .latest_process_definition_by_key("uploadClaimProcess", tenant_id)
        .unwrap()
        .unwrap()
        .id
}

fn create_unowned_item(content: &FlowableContentService, name: &str) -> String {
    content
        .create_content_item(CreateContentItemRequest {
            name: name.to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("payload".to_string()),
            task_id: None,
            process_instance_id: None,
            scope_type: None,
            scope_id: None,
            created_by: Some("u1".to_string()),
            expires_in_seconds: None,
        })
        .unwrap()
        .id
}

fn submit_start_form(
    service: &flowable_form_service::FlowableFormService,
    process_definition_id: &str,
    files_value: &str,
) -> Result<FormSubmissionResult, FlowableError> {
    service.submit_form(FormSubmissionRequest {
        process_definition_id: Some(process_definition_id.to_string()),
        task_id: None,
        business_key: None,
        outcome: None,
        properties: vec![
            FormSubmissionProperty {
                id: "title".to_string(),
                value: json!("t"),
            },
            FormSubmissionProperty {
                id: "files".to_string(),
                value: json!(files_value),
            },
        ],
    })
}

fn assert_no_runtime_process_instances(
    engine: &Arc<flowable_engine::engine::process_engine::ProcessEngine>,
) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let runtime_instances = store.snapshot_process_instances(&mut session);
    session.rollback().ok();
    assert!(
        runtime_instances.is_empty(),
        "rejected submit must roll back the process instance, found {:?}",
        runtime_instances.keys().collect::<Vec<_>>()
    );
}

#[test]
fn tenantless_submit_cannot_claim_tenant_content() {
    let (engine, service) = runtime_fixture("form-claim-tenantless-ctx");
    deploy_upload_form(&service);
    let process_definition_id = deploy_upload_process(&engine, None);
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item_id = create_unowned_item(&content, "foreign.txt");
    // Tag the item with tenant-b (explicit admin/seed operation).
    {
        let store = engine.get_runtime_store();
        let mut session = store.db_store().create_session().unwrap();
        flowable_content_service::repository::associate_content_item_in_session(
            &mut session,
            &item_id,
            None,
            None,
            None,
            None,
            None,
            Some("tenant-b"),
        )
        .unwrap();
        session.flush_and_commit().unwrap();
    }

    // Previous bug: a tenantless form context silently took over the item and
    // cleared its tenant_id. Now the submit must be rejected symmetrically.
    let err = submit_start_form(&service, &process_definition_id, &item_id).unwrap_err();
    match err {
        FlowableError::BadRequest(msg) => assert!(msg.contains("tenant"), "msg={msg}"),
        other => panic!("expected BadRequest, got {other:?}"),
    }

    // Ownership must be untouched: tenant intact, no association written.
    let untouched = content.get_content_item(&item_id).unwrap();
    assert_eq!(untouched.tenant_id.as_deref(), Some("tenant-b"));
    assert_eq!(untouched.process_instance_id, None);
    assert_eq!(untouched.field, None);

    assert_no_runtime_process_instances(&engine);
}

#[test]
fn tenant_submit_cannot_adopt_tenantless_content() {
    let (engine, service) = runtime_fixture("form-claim-tenant-ctx");
    deploy_upload_form(&service);
    let process_definition_id = deploy_upload_process(&engine, Some("tenant-a"));
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item_id = create_unowned_item(&content, "unowned.txt");

    // First-time tenant adoption must be an explicit claim, not a form submit.
    let err = submit_start_form(&service, &process_definition_id, &item_id).unwrap_err();
    match err {
        FlowableError::BadRequest(msg) => assert!(msg.contains("tenant"), "msg={msg}"),
        other => panic!("expected BadRequest, got {other:?}"),
    }

    let untouched = content.get_content_item(&item_id).unwrap();
    assert_eq!(untouched.tenant_id, None);
    assert_eq!(untouched.process_instance_id, None);

    assert_no_runtime_process_instances(&engine);
}

#[test]
fn resubmitting_content_owned_by_another_process_conflicts() {
    let (engine, service) = runtime_fixture("form-claim-reassign");
    deploy_upload_form(&service);
    let process_definition_id = deploy_upload_process(&engine, None);
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item_id = create_unowned_item(&content, "owned.txt");

    // First submit claims the item for process instance #1.
    let first_pi = match submit_start_form(&service, &process_definition_id, &item_id).unwrap() {
        FormSubmissionResult::ProcessInstance(pi) => pi,
        other => panic!("expected process instance, got {other:?}"),
    };
    let owned = content.get_content_item(&item_id).unwrap();
    assert_eq!(owned.process_instance_id.as_deref(), Some(first_pi.id.as_str()));
    assert_eq!(owned.field.as_deref(), Some("files"));

    // Second submit referencing the same item must be rejected as a conflict
    // and must not start a second process instance.
    let err = submit_start_form(&service, &process_definition_id, &item_id).unwrap_err();
    match err {
        FlowableError::Conflict(msg) => assert!(msg.contains("already associated"), "msg={msg}"),
        other => panic!("expected Conflict, got {other:?}"),
    }

    // Original association unchanged, only the first process instance remains.
    let still_owned = content.get_content_item(&item_id).unwrap();
    assert_eq!(
        still_owned.process_instance_id.as_deref(),
        Some(first_pi.id.as_str())
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let runtime_instances = store.snapshot_process_instances(&mut session);
    session.rollback().ok();
    assert_eq!(
        runtime_instances.len(),
        1,
        "only the first submit may leave a process instance, found {:?}",
        runtime_instances.keys().collect::<Vec<_>>()
    );
    assert!(runtime_instances.contains_key(&first_pi.id));
}
