//! P1 tenant contract tests for task/process attachments.
//!
//! Covers the tenant-inheritance fix:
//! - process attachments inherit the process instance tenant (pure process and
//!   task+process variants);
//! - task attachments inherit the task tenant;
//! - tenantless contexts still produce tenantless attachments;
//! - a standalone task (no process instance) must not be combined with a
//!   process scope;
//! - a task belonging to another process instance or another tenant is
//!   rejected.

use flowable_content_service::{
    CreateProcessAttachmentInput, CreateTaskAttachmentInput, FlowableContentService,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::task::Task;
use std::sync::Arc;

fn engine(name: &str) -> Arc<ProcessEngine> {
    Arc::new(ProcessEngine::new(name.to_string()))
}

/// Deploy a single-user-task process (optionally tenant-scoped) and start one
/// instance of it, returning `(process_instance_id, task_id)`.
fn deploy_and_start(
    engine: &ProcessEngine,
    process_key: &str,
    tenant_id: Option<&str>,
) -> (String, String) {
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="{process_key}">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Task 1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );

    let mut builder = repo
        .create_deployment()
        .add_string(format!("{process_key}.bpmn20.xml"), xml);
    if let Some(tenant) = tenant_id {
        builder = builder.tenant_id(tenant.to_string());
    }
    repo.deploy(builder).unwrap();

    let definition_id = repo
        .latest_process_definition_by_key(process_key, tenant_id)
        .unwrap()
        .unwrap()
        .id;
    let pi = runtime
        .start_process_instance_by_id(definition_id, None)
        .unwrap();
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    (pi.id, tasks[0].id.clone())
}

fn process_attachment_input(
    process_instance_id: &str,
    task_id: Option<&str>,
    name: &str,
) -> CreateProcessAttachmentInput {
    CreateProcessAttachmentInput {
        process_instance_id: process_instance_id.to_string(),
        task_id: task_id.map(str::to_string),
        name: name.to_string(),
        description: None,
        attachment_type: Some("text/plain".into()),
        external_url: None,
        content: Some(b"tenant-bytes".to_vec()),
        user_id: Some("admin".into()),
    }
}

#[test]
fn process_attachment_inherits_process_instance_tenant() {
    let engine = engine("attach-tenant-inherit");
    let (process_instance_id, task_id) =
        deploy_and_start(&engine, "tenantAttachProc", Some("tenant-b"));
    let content = FlowableContentService::new(Arc::clone(&engine));

    // Pure process attachment inherits the PI tenant.
    let pure = content
        .create_process_attachment(process_attachment_input(
            &process_instance_id,
            None,
            "pure.txt",
        ))
        .unwrap();
    assert_eq!(pure.tenant_id.as_deref(), Some("tenant-b"));

    // Task+process attachment inherits the PI tenant as well.
    let scoped = content
        .create_process_attachment(process_attachment_input(
            &process_instance_id,
            Some(&task_id),
            "scoped.txt",
        ))
        .unwrap();
    assert_eq!(scoped.tenant_id.as_deref(), Some("tenant-b"));
    assert_eq!(scoped.task_id.as_deref(), Some(task_id.as_str()));

    // Persisted rows carry the tenant, not only the returned value.
    let listed = content
        .list_process_attachments(&process_instance_id)
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed
        .iter()
        .all(|item| item.tenant_id.as_deref() == Some("tenant-b")));
}

#[test]
fn tenantless_process_attachment_stays_tenantless() {
    let engine = engine("attach-tenantless");
    let (process_instance_id, _task_id) =
        deploy_and_start(&engine, "tenantlessAttachProc", None);
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item = content
        .create_process_attachment(process_attachment_input(
            &process_instance_id,
            None,
            "plain.txt",
        ))
        .unwrap();
    assert_eq!(item.tenant_id, None);
}

#[test]
fn task_attachment_inherits_task_tenant() {
    let engine = engine("task-attach-tenant");
    let (_process_instance_id, task_id) =
        deploy_and_start(&engine, "tenantTaskAttach", Some("tenant-b"));
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: task_id.clone(),
            name: "task-note.txt".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"task-bytes".to_vec()),
            user_id: Some("admin".into()),
            process_instance_id: None,
        })
        .unwrap();
    assert_eq!(item.tenant_id.as_deref(), Some("tenant-b"));

    // Standalone tenantless task → tenantless attachment.
    let standalone = engine
        .get_task_service()
        .create_task(Task::new(
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "Standalone".to_string(),
        ))
        .unwrap();
    let standalone_item = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: standalone.id.clone(),
            name: "standalone-note.txt".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"standalone-bytes".to_vec()),
            user_id: None,
            process_instance_id: None,
        })
        .unwrap();
    assert_eq!(standalone_item.tenant_id, None);
}

#[test]
fn standalone_task_cannot_be_combined_with_process_scope() {
    let engine = engine("attach-standalone-reject");
    let (process_instance_id, _task_id) =
        deploy_and_start(&engine, "standaloneRejectProc", None);
    let content = FlowableContentService::new(Arc::clone(&engine));

    let standalone = engine
        .get_task_service()
        .create_task(Task::new(
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "Standalone".to_string(),
        ))
        .unwrap();

    let err = content
        .create_process_attachment(process_attachment_input(
            &process_instance_id,
            Some(&standalone.id),
            "hijack.txt",
        ))
        .unwrap_err();
    match err {
        FlowableError::BadRequest(msg) => {
            assert!(msg.contains("does not belong to process instance"), "msg={msg}")
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }

    // Nothing persisted for the rejected attempt.
    let listed = content
        .list_process_attachments(&process_instance_id)
        .unwrap();
    assert!(listed.is_empty());
}

#[test]
fn task_of_other_process_instance_is_rejected() {
    let engine = engine("attach-foreign-task");
    let (pi_one, _task_one) = deploy_and_start(&engine, "foreignTaskProcA", None);
    let (_pi_two, task_two) = deploy_and_start(&engine, "foreignTaskProcB", None);
    let content = FlowableContentService::new(Arc::clone(&engine));

    let err = content
        .create_process_attachment(process_attachment_input(
            &pi_one,
            Some(&task_two),
            "cross.txt",
        ))
        .unwrap_err();
    match err {
        FlowableError::BadRequest(msg) => {
            assert!(msg.contains("does not belong to process instance"), "msg={msg}")
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

#[test]
fn task_with_mismatching_tenant_is_rejected() {
    let engine = engine("attach-tenant-mismatch");
    let (process_instance_id, _task_id) =
        deploy_and_start(&engine, "tenantMismatchProc", Some("tenant-b"));
    let content = FlowableContentService::new(Arc::clone(&engine));

    // Synthetic task pointing at the tenant-b PI but living in no tenant —
    // the tenant consistency check must reject the combination.
    let mut foreign = Task::new(
        String::new(),
        process_instance_id.clone(),
        String::new(),
        String::new(),
        "Foreign tenant task".to_string(),
    );
    foreign.tenant_id = None;
    let foreign = engine.get_task_service().create_task(foreign).unwrap();

    let err = content
        .create_process_attachment(process_attachment_input(
            &process_instance_id,
            Some(&foreign.id),
            "mismatch.txt",
        ))
        .unwrap_err();
    match err {
        FlowableError::BadRequest(msg) => {
            assert!(msg.contains("different tenant"), "msg={msg}")
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }
}
