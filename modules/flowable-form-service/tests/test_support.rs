#![allow(dead_code)]

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_form_service::{
    FlowableFormService, FormDeployment, FormDeploymentRequest, FormDeploymentResource,
};
use serde_json::json;
use std::sync::Arc;

pub fn service(name: &str) -> FlowableFormService {
    FlowableFormService::new(Arc::new(ProcessEngine::new(name.to_string())))
}

pub fn persistent_service(name: &str, path: &str) -> FlowableFormService {
    FlowableFormService::new(Arc::new(ProcessEngine::new_with_db_path(
        name.to_string(),
        path,
    )))
}

pub fn runtime_fixture(name: &str) -> (Arc<ProcessEngine>, FlowableFormService) {
    let engine = Arc::new(ProcessEngine::new(name.to_string()));
    let service = FlowableFormService::new(Arc::clone(&engine));
    (engine, service)
}

pub fn deploy_sample_forms(service: &FlowableFormService) -> FormDeployment {
    service
        .deploy(FormDeploymentRequest {
            name: "Sample forms".to_string(),
            resources: vec![
                FormDeploymentResource {
                    resource_name: "expense-approval.form".to_string(),
                    resource: json!({
                        "key": "expenseApproval",
                        "name": "Expense approval",
                        "description": "Expense approval form",
                        "resourceName": "expense-approval.form",
                        "fields": [
                            { "id": "amount", "type": "number" }
                        ]
                    })
                    .to_string(),
                },
                FormDeploymentResource {
                    resource_name: "travel-request.form".to_string(),
                    resource: json!({
                        "key": "travelRequest",
                        "name": "Travel request",
                        "description": "Travel request form",
                        "resourceName": "travel-request.form",
                        "fields": [
                            { "id": "destination", "type": "string" }
                        ]
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap()
}

pub fn deploy_runtime_forms(service: &FlowableFormService) -> FormDeployment {
    service
        .deploy(FormDeploymentRequest {
            name: "Runtime forms".to_string(),
            resources: vec![
                FormDeploymentResource {
                    resource_name: "travel-request.form".to_string(),
                    resource: json!({
                        "key": "travelRequest",
                        "name": "Travel request",
                        "description": "Travel request start form",
                        "resourceName": "travel-request.form",
                        "fields": [
                            { "id": "requester", "name": "Requester", "type": "string", "required": true },
                            { "id": "amount", "name": "Amount", "type": "number", "required": true }
                        ]
                    })
                    .to_string(),
                },
                FormDeploymentResource {
                    resource_name: "expense-approval.form".to_string(),
                    resource: json!({
                        "key": "expenseApproval",
                        "name": "Expense approval",
                        "description": "Expense approval task form",
                        "resourceName": "expense-approval.form",
                        "fields": [
                            { "id": "approved", "name": "Approved", "type": "boolean", "required": true },
                            { "id": "comment", "name": "Comment", "type": "string" }
                        ]
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap()
}

pub fn deploy_runtime_process(
    engine: &Arc<ProcessEngine>,
    process_key: &str,
    start_form_key: &str,
    task_form_key: &str,
) -> String {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="{process_key}" name="{process_key}" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="{start_form_key}" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="approveTask" />
        <userTask id="approveTask" name="Approve request" flowable:formKey="{task_form_key}" />
        <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#
    );

    let deployment = engine
        .get_repository_service()
        .create_deployment()
        .name(format!("{process_key} deployment"))
        .add_string(format!("{process_key}.bpmn20.xml"), xml);

    engine.get_repository_service().deploy(deployment).unwrap();

    engine
        .get_repository_service()
        .latest_process_definition_by_key(process_key, None)
        .unwrap()
        .unwrap()
        .id
}
