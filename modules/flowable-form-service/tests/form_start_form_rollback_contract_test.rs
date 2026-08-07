//! Start-form submission atomicity (P1-2): process start, form instance,
//! historic details and content association commit/roll back as one command.
//!
//! Java truth sources:
//! - `StartProcessInstanceWithFormCmd` — one command context for process
//!   start + `FormService.createFormInstanceWithScopeId`
//! - `FormFieldHandler.handleFormFieldsOnSubmit` (upload content association)
//!
//! Each failure class asserts there is NO residue: runtime process instance,
//! historic process instance, form instance, content association.

mod test_support;

use flowable_content_service::{CreateContentItemRequest, FlowableContentService};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::error::FlowableError;
use flowable_form_service::{
    FlowableFormService, FormDeploymentRequest, FormDeploymentResource, FormFieldHandler,
    FormFieldSubmitContext, FormProperty, FormSubmissionProperty, FormSubmissionRequest,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use test_support::runtime_fixture;

fn deploy_form(service: &FlowableFormService, fields: Value) {
    service
        .deploy(FormDeploymentRequest {
            name: "Start form rollback forms".to_string(),
            resources: vec![FormDeploymentResource {
                resource_name: "start-rollback.form".to_string(),
                resource: json!({
                    "key": "startRollback",
                    "name": "Start rollback",
                    "resourceName": "start-rollback.form",
                    "fields": fields
                })
                .to_string(),
            }],
        })
        .unwrap();
}

fn deploy_process(engine: &Arc<ProcessEngine>, tenant_id: Option<&str>) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="startRollbackProcess" name="startRollbackProcess" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="startRollback" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;
    let mut deployment = engine
        .get_repository_service()
        .create_deployment()
        .name("start rollback process".to_string())
        .add_string("start-rollback.bpmn20.xml".to_string(), xml.to_string());
    if let Some(tenant) = tenant_id {
        deployment = deployment.tenant_id(tenant.to_string());
    }
    engine.get_repository_service().deploy(deployment).unwrap();
    engine
        .get_repository_service()
        .latest_process_definition_by_key("startRollbackProcess", tenant_id)
        .unwrap()
        .unwrap()
        .id
}

fn create_content_item(content: &FlowableContentService, name: &str) -> String {
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

/// No residue after a failed start-form submit: runtime PI, historic PI and
/// form instances must all be absent.
fn assert_no_submission_residue(engine: &Arc<ProcessEngine>, service: &FlowableFormService) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let runtime_instances = store.snapshot_process_instances(&mut session);
    let tasks = session
        .find_all::<flowable_engine::task::Task>("tasks")
        .unwrap_or_default();
    session.rollback().ok();
    assert!(
        runtime_instances.is_empty(),
        "runtime process instances must roll back, found {:?}",
        runtime_instances.keys().collect::<Vec<_>>()
    );
    assert!(
        tasks.is_empty(),
        "tasks of the rolled-back process must not survive"
    );
    assert_eq!(
        engine
            .get_history_service()
            .create_historic_process_instance_query()
            .count()
            .unwrap(),
        0,
        "historic process instances must roll back"
    );
    assert_eq!(
        service.create_form_instance_query().count().unwrap(),
        0,
        "form instances must roll back"
    );
}

#[test]
fn start_form_missing_content_rolls_back_process_and_form_instance() {
    let (engine, service) = runtime_fixture("start-form-rollback-missing");
    deploy_form(
        &service,
        json!([
            { "id": "title", "name": "Title", "type": "string", "required": true },
            { "id": "files", "name": "Files", "type": "upload", "required": true }
        ]),
    );
    let process_definition_id = deploy_process(&engine, None);

    let err = service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id),
            task_id: None,
            business_key: Some("rollback-1".to_string()),
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "title".to_string(),
                    value: json!("docs"),
                },
                FormSubmissionProperty {
                    id: "files".to_string(),
                    value: json!("content-item:missing"),
                },
            ],
        })
        .unwrap_err();
    match err {
        FlowableError::NotFound(msg) => assert!(msg.contains("was not found"), "msg={msg}"),
        other => panic!("expected NotFound, got {other:?}"),
    }

    assert_no_submission_residue(&engine, &service);
}

#[test]
fn start_form_cross_tenant_content_rolls_back_everything() {
    let (engine, service) = runtime_fixture("start-form-rollback-tenant");
    deploy_form(
        &service,
        json!([
            { "id": "title", "name": "Title", "type": "string", "required": true },
            { "id": "files", "name": "Files", "type": "upload", "required": true }
        ]),
    );
    let process_definition_id = deploy_process(&engine, Some("tenant-a"));
    let content = FlowableContentService::new(Arc::clone(&engine));

    // Content item owned by tenant-b.
    let foreign_id = create_content_item(&content, "foreign.txt");
    {
        let store = engine.get_runtime_store();
        let mut session = store.db_store().create_session().unwrap();
        flowable_content_service::repository::associate_content_item_in_session(
            &mut session,
            &foreign_id,
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

    let err = service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id),
            task_id: None,
            business_key: None,
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "title".to_string(),
                    value: json!("docs"),
                },
                FormSubmissionProperty {
                    id: "files".to_string(),
                    value: json!(foreign_id.clone()),
                },
            ],
        })
        .unwrap_err();
    match err {
        FlowableError::BadRequest(msg) => assert!(msg.contains("tenant"), "msg={msg}"),
        other => panic!("expected BadRequest cross-tenant, got {other:?}"),
    }

    assert_no_submission_residue(&engine, &service);

    // Content association must be untouched (still tenant-b, no process link).
    let foreign = content.get_content_item(&foreign_id).unwrap();
    assert_eq!(foreign.process_instance_id, None);
    assert_eq!(foreign.field, None);
    assert_eq!(foreign.tenant_id.as_deref(), Some("tenant-b"));
}

/// Custom handler failing on submit after the upload handler already
/// associated content: the whole command rolls back, including the content
/// association written earlier in the same session.
#[test]
fn start_form_custom_handler_error_rolls_back_process_and_content_association() {
    struct FailingWidgetHandler;
    impl FormFieldHandler for FailingWidgetHandler {
        fn supported_type(&self) -> &str {
            "custom_widget"
        }
        fn validate(&self, _field: &FormProperty, _value: &Value) -> Result<(), FlowableError> {
            Ok(())
        }
        fn coerce(&self, _field: &FormProperty, value: Value) -> Result<Value, FlowableError> {
            Ok(value)
        }
        fn render_metadata(&self, _field: &FormProperty) -> Value {
            json!({"type": "custom_widget"})
        }
        fn handle_submit(
            &self,
            _field: &FormProperty,
            _value: &Value,
            _ctx: &mut FormFieldSubmitContext<'_>,
        ) -> Result<(), FlowableError> {
            Err(FlowableError::ExecutionError(
                "widget backend unavailable".to_string(),
            ))
        }
    }

    let engine = Arc::new(ProcessEngine::new(
        "start-form-rollback-handler".to_string(),
    ));
    let mut custom: BTreeMap<String, Arc<dyn FormFieldHandler>> = BTreeMap::new();
    custom.insert("custom_widget".to_string(), Arc::new(FailingWidgetHandler));
    let service = FlowableFormService::with_handlers(Arc::clone(&engine), custom);

    // BTreeMap submit order is alphabetical: "files" (upload association
    // succeeds) runs before "widget" (handler hard error) — proving the
    // association from the same submission is rolled back too.
    deploy_form(
        &service,
        json!([
            { "id": "files", "name": "Files", "type": "upload", "required": true },
            { "id": "widget", "name": "Widget", "type": "custom_widget", "required": true }
        ]),
    );
    let process_definition_id = deploy_process(&engine, None);
    let content = FlowableContentService::new(Arc::clone(&engine));
    let item_id = create_content_item(&content, "a.txt");

    let err = service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id),
            task_id: None,
            business_key: None,
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "files".to_string(),
                    value: json!(item_id.clone()),
                },
                FormSubmissionProperty {
                    id: "widget".to_string(),
                    value: json!("x"),
                },
            ],
        })
        .unwrap_err();
    match err {
        FlowableError::ExecutionError(msg) => {
            assert!(msg.contains("widget backend unavailable"), "msg={msg}")
        }
        other => panic!("expected ExecutionError from handler, got {other:?}"),
    }

    assert_no_submission_residue(&engine, &service);

    // Upload association from the same failed submission must be rolled back.
    let item = content.get_content_item(&item_id).unwrap();
    assert_eq!(item.process_instance_id, None);
    assert_eq!(item.field, None);
    assert_eq!(item.scope_type, None);
}
