//! Upload field submit association + enrich lifecycle (ADR-4 transactional).
//!
//! Java truth sources:
//! - FormFieldHandler.handleFormFieldsOnSubmit / enrichFormFields
//! - DefaultFormFieldHandler (upload content association)

mod test_support;

use flowable_content_service::{CreateContentItemRequest, FlowableContentService};
use flowable_engine::error::FlowableError;
use flowable_form_service::{
    default_handlers, FormDeploymentRequest, FormDeploymentResource, FormFieldHandler,
    FormSubmissionProperty, FormSubmissionRequest, FormSubmissionResult, UploadFieldHandler,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
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
) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="uploadProcess" name="uploadProcess" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="uploadRequest" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review" flowable:formKey="uploadRequest" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("upload process".to_string())
                .add_string("upload-process.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    engine
        .get_repository_service()
        .latest_process_definition_by_key("uploadProcess", None)
        .unwrap()
        .unwrap()
        .id
}

#[test]
fn upload_handler_parses_string_and_list_ids_with_trim_dedupe() {
    let ids = UploadFieldHandler::parse_content_item_ids(&json!(" a , b , a ,  ")).unwrap();
    assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);

    let ids = UploadFieldHandler::parse_content_item_ids(&json!([" x ", "y", "x"])).unwrap();
    assert_eq!(ids, vec!["x".to_string(), "y".to_string()]);
}

#[test]
fn upload_submit_associates_content_and_enrichment_replaces_ids() {
    let (engine, service) = runtime_fixture("form-upload-happy");
    deploy_upload_form(&service);
    let process_definition_id = deploy_upload_process(&engine);
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item_a = content
        .create_content_item(CreateContentItemRequest {
            name: "a.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("aaa".to_string()),
            task_id: None,
            process_instance_id: None,
            scope_type: None,
            scope_id: None,
            created_by: Some("u1".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();
    let item_b = content
        .create_content_item(CreateContentItemRequest {
            name: "b.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("bbb".to_string()),
            task_id: None,
            process_instance_id: None,
            scope_type: None,
            scope_id: None,
            created_by: Some("u1".to_string()),
            expires_in_seconds: None,
        })
        .unwrap();

    let process_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id),
            task_id: None,
            business_key: Some("upload-1".to_string()),
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "title".to_string(),
                    value: json!("docs"),
                },
                FormSubmissionProperty {
                    id: "files".to_string(),
                    value: json!(format!("{}, {}", item_a.id, item_b.id)),
                },
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(pi) => pi,
        other => panic!("expected process instance, got {other:?}"),
    };

    // Content associated with process/scope/field
    let associated_a = content.get_content_item(&item_a.id).unwrap();
    assert_eq!(
        associated_a.process_instance_id.as_deref(),
        Some(process_instance.id.as_str())
    );
    assert_eq!(associated_a.field.as_deref(), Some("files"));
    assert_eq!(associated_a.scope_type.as_deref(), Some("start"));

    let associated_b = content.get_content_item(&item_b.id).unwrap();
    assert_eq!(associated_b.field.as_deref(), Some("files"));

    // Form instance stores ids (canonical string), not content metadata
    let instance = service
        .create_form_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .unwrap()
        .pop()
        .unwrap();
    let stored = instance.values.get("files").unwrap().as_str().unwrap();
    assert!(stored.contains(&item_a.id));
    assert!(stored.contains(&item_b.id));

    // Complete task form with upload ids, then read task form data for enrichment
    // after re-seeding variables via a second process (variables set on start form).
    // Start form already set process variables including files; get_task_form_data
    // loads task execution variables which inherit process variables.
    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .unwrap();

    let form_data = service.get_task_form_data(&task.id).unwrap();
    let files_prop = form_data
        .form_properties
        .iter()
        .find(|p| p.id == "files")
        .expect("files field");
    let enriched = files_prop.value.as_ref().expect("enriched value");
    assert!(enriched.is_array(), "enrichment should yield content array, got {enriched}");
    let arr = enriched.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let names: Vec<_> = arr
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(names.contains(&"a.txt"));
    assert!(names.contains(&"b.txt"));

    // Persisted form values unchanged after enrichment
    let reloaded = service.get_form_instance(&instance.id).unwrap();
    assert_eq!(reloaded.values.get("files"), instance.values.get("files"));
}

#[test]
fn upload_submit_requires_existing_items_and_rejects_cross_tenant() {
    let (engine, service) = runtime_fixture("form-upload-reject");
    deploy_upload_form(&service);
    let process_definition_id = deploy_upload_process(&engine);
    let content = FlowableContentService::new(Arc::clone(&engine));

    // Missing content item
    let err = service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(process_definition_id.clone()),
            task_id: None,
            business_key: None,
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "title".to_string(),
                    value: json!("x"),
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

    // Cross-tenant rejection
    let foreign = content
        .create_content_item(CreateContentItemRequest {
            name: "foreign.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            description: None,
            attachment_type: None,
            external_url: None,
            content: Some("x".to_string()),
            task_id: None,
            process_instance_id: None,
            scope_type: None,
            scope_id: None,
            created_by: None,
            expires_in_seconds: None,
        })
        .unwrap();
    // Tag content with tenant-b
    {
        let store = engine.get_runtime_store();
        let mut session = store.db_store().create_session().unwrap();
        flowable_content_service::repository::associate_content_item_in_session(
            &mut session,
            &foreign.id,
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

    // Deploy tenant-a process and attempt to associate tenant-b content
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("tenant upload".to_string())
                .tenant_id("tenant-a".to_string())
                .add_string(
                    "tenant-upload.bpmn20.xml".to_string(),
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="tenantUploadProcess" name="tenantUploadProcess" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="uploadRequest" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#
                        .to_string(),
                ),
        )
        .unwrap();
    let tenant_pd = engine
        .get_repository_service()
        .latest_process_definition_by_key("tenantUploadProcess", Some("tenant-a"))
        .unwrap()
        .unwrap()
        .id;

    let err = service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(tenant_pd),
            task_id: None,
            business_key: None,
            outcome: None,
            properties: vec![
                FormSubmissionProperty {
                    id: "title".to_string(),
                    value: json!("x"),
                },
                FormSubmissionProperty {
                    id: "files".to_string(),
                    value: json!(foreign.id),
                },
            ],
        })
        .unwrap_err();
    match err {
        FlowableError::BadRequest(msg) => {
            assert!(msg.contains("tenant"), "msg={msg}");
        }
        other => panic!("expected BadRequest cross-tenant, got {other:?}"),
    }

    // Form instance must not exist for the failed tenant submit
    assert_eq!(
        service
            .create_form_instance_query()
            .tenant_id("tenant-a")
            .count()
            .unwrap(),
        0
    );

    // P1-2: both failed submits must not leave an orphan process instance
    // (start + form instance + association are one atomic command).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let runtime_instances = store.snapshot_process_instances(&mut session);
    session.rollback().ok();
    assert!(
        runtime_instances.is_empty(),
        "failed start-form submits must roll back the process instance, found {:?}",
        runtime_instances.keys().collect::<Vec<_>>()
    );
}

#[test]
fn tenant_content_created_via_tenant_entry_is_claimable_by_same_tenant_forms() {
    let (engine, service) = runtime_fixture("form-upload-tenant-positive");
    deploy_upload_form(&service);
    let content = FlowableContentService::new(Arc::clone(&engine));

    // Tenant-a process with an upload start form and an upload task form.
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("tenant upload positive".to_string())
                .tenant_id("tenant-a".to_string())
                .add_string(
                    "tenant-upload-positive.bpmn20.xml".to_string(),
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="tenantUploadPositive" name="tenantUploadPositive" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="uploadRequest" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review" flowable:formKey="uploadRequest" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#
                        .to_string(),
                ),
        )
        .unwrap();
    let tenant_pd = engine
        .get_repository_service()
        .latest_process_definition_by_key("tenantUploadPositive", Some("tenant-a"))
        .unwrap()
        .unwrap()
        .id;

    let new_item = |name: &str| CreateContentItemRequest {
        name: name.to_string(),
        mime_type: Some("text/plain".to_string()),
        description: None,
        attachment_type: None,
        external_url: None,
        content: Some("data".to_string()),
        task_id: None,
        process_instance_id: None,
        scope_type: None,
        scope_id: None,
        created_by: Some("tenant-a-user".to_string()),
        expires_in_seconds: None,
    };

    // Pre-upload through the trusted tenant-aware entry point.
    let start_item = content
        .create_content_item_for_tenant(new_item("start.txt"), Some("tenant-a"))
        .unwrap();
    assert_eq!(start_item.tenant_id.as_deref(), Some("tenant-a"));
    let task_item = content
        .create_content_item_for_tenant(new_item("task.txt"), Some("tenant-a"))
        .unwrap();

    // Tenant-a start form claims the same-tenant content.
    let process_instance = match service
        .submit_form(FormSubmissionRequest {
            process_definition_id: Some(tenant_pd),
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
                    value: json!(start_item.id),
                },
            ],
        })
        .unwrap()
    {
        FormSubmissionResult::ProcessInstance(pi) => pi,
        other => panic!("expected process instance, got {other:?}"),
    };
    let claimed = content.get_content_item(&start_item.id).unwrap();
    assert_eq!(
        claimed.process_instance_id.as_deref(),
        Some(process_instance.id.as_str())
    );
    assert_eq!(claimed.field.as_deref(), Some("files"));
    // Claiming never rewrites the tenant.
    assert_eq!(claimed.tenant_id.as_deref(), Some("tenant-a"));

    // Tenant-a task form claims the second same-tenant content item.
    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .unwrap();
    let form_definition_id = service
        .create_form_definition_query()
        .key("uploadRequest")
        .list()
        .unwrap()
        .into_iter()
        .max_by_key(|d| d.version)
        .unwrap()
        .id;
    let mut variables = HashMap::new();
    variables.insert("title".into(), json!("review"));
    variables.insert("files".into(), json!(task_item.id));
    service
        .complete_task_with_form_definition(
            task.id.clone(),
            form_definition_id,
            None,
            variables,
            false,
            HashMap::new(),
            Some("tenant-a-user".into()),
        )
        .unwrap();
    let claimed_task_item = content.get_content_item(&task_item.id).unwrap();
    assert_eq!(claimed_task_item.task_id.as_deref(), Some(task.id.as_str()));
    assert_eq!(claimed_task_item.field.as_deref(), Some("files"));
    assert_eq!(claimed_task_item.tenant_id.as_deref(), Some("tenant-a"));
}

#[test]
fn upload_handler_error_rolls_back_task_complete_and_form_instance() {
    let (engine, service) = runtime_fixture("form-upload-rollback");
    deploy_upload_form(&service);
    let _process_definition_id = deploy_upload_process(&engine);

    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_key("uploadProcess")
        .unwrap();

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .pop()
        .unwrap();

    let form_definition_id = service
        .create_form_definition_query()
        .key("uploadRequest")
        .list()
        .unwrap()
        .into_iter()
        .max_by_key(|d| d.version)
        .unwrap()
        .id;

    let mut variables = HashMap::new();
    variables.insert("title".into(), json!("t"));
    variables.insert("files".into(), json!("content-item:does-not-exist"));

    let err = service
        .complete_task_with_form_definition(
            task.id.clone(),
            form_definition_id,
            None,
            variables,
            false,
            HashMap::new(),
            Some("admin".into()),
        )
        .unwrap_err();
    match err {
        FlowableError::NotFound(msg) => assert!(msg.contains("was not found"), "msg={msg}"),
        other => panic!("expected NotFound, got {other:?}"),
    }

    // Task still open
    assert_eq!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap()
            .len(),
        1
    );
    // No form instance
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
fn unknown_custom_type_still_requires_explicit_registration() {
    let handlers = default_handlers();
    assert!(handlers.contains_key("upload"));
    assert!(!handlers.contains_key("custom_widget"));

    // Default methods exist so custom handlers compile without implementing lifecycle.
    struct MinimalHandler;
    impl FormFieldHandler for MinimalHandler {
        fn supported_type(&self) -> &str {
            "custom_widget"
        }
        fn validate(
            &self,
            _field: &flowable_form_service::FormProperty,
            _value: &Value,
        ) -> Result<(), FlowableError> {
            Ok(())
        }
        fn coerce(
            &self,
            _field: &flowable_form_service::FormProperty,
            value: Value,
        ) -> Result<Value, FlowableError> {
            Ok(value)
        }
        fn render_metadata(&self, _field: &flowable_form_service::FormProperty) -> Value {
            json!({"type": "custom_widget"})
        }
    }
    let mut map: BTreeMap<String, Arc<dyn FormFieldHandler>> = BTreeMap::new();
    map.insert("custom_widget".into(), Arc::new(MinimalHandler));
    assert!(map.contains_key("custom_widget"));
}
