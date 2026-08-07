//! Engine-side Java parity contract for task attachments (P2-ATTACHMENT).
//!
//! Covers CreateAttachmentCmd / DeleteAttachmentCmd semantics via
//! `FlowableContentService` atomic commands:
//!   1. binary create → content + AddAttachment event
//!   2. URL/link create → external_url, no content stream
//!   3. mid-command failure → no orphan content / event
//!   4. delete clears content + DeleteAttachment event; missing → 404
//!   5. readable after task completion (list/get/content); create/delete need runtime
//!   6. suspended task rejects create with no side effects

use flowable_content_service::{
    CreateTaskAttachmentInput, FORCE_FAIL_ATTACHMENT_TYPE, FlowableContentService,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::task::Task;
use std::sync::Arc;

fn engine(name: &str) -> Arc<ProcessEngine> {
    Arc::new(ProcessEngine::new(name.to_string()))
}

fn deploy_and_start(engine: &ProcessEngine, process_key: &str) -> String {
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

    repo.deploy(
        repo.create_deployment()
            .add_string(format!("{process_key}.bpmn20.xml"), xml),
    )
    .unwrap();

    let pi = runtime.start_process_instance_by_key(process_key).unwrap();
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    tasks[0].id.clone()
}

fn event_actions(engine: &ProcessEngine, task_id: &str) -> Vec<String> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine
        .get_history_service()
        .get_task_events(task_id, &mut session)
        .into_iter()
        .map(|e| e.action)
        .collect()
}

#[test]
fn binary_create_writes_content_and_add_attachment_event() {
    let engine = engine("attach-binary");
    let task_id = deploy_and_start(&engine, "attachBinaryProcess");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: task_id.clone(),
            name: "note.txt".into(),
            description: Some("Review note".into()),
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"approved".to_vec()),
            user_id: Some("admin".into()),
            process_instance_id: None,
        })
        .unwrap();

    assert_eq!(item.name, "note.txt");
    assert_eq!(item.attachment_type.as_deref(), Some("text/plain"));
    assert_eq!(item.content_size, 8);
    assert!(item.external_url.is_none());
    assert_eq!(item.created_by.as_deref(), Some("admin"));

    let payload = content
        .get_task_attachment_content(&task_id, &item.id)
        .unwrap();
    assert_eq!(payload.bytes, b"approved");

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let events = engine
        .get_history_service()
        .get_task_events(&task_id, &mut session);
    let add = events.iter().find(|e| e.action == "AddAttachment").unwrap();
    assert_eq!(add.message, vec!["note.txt".to_string()]);
}

#[test]
fn url_link_create_has_external_url_and_no_content_stream() {
    let engine = engine("attach-link");
    let task_id = deploy_and_start(&engine, "attachLinkProcess");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: task_id.clone(),
            name: "docs".into(),
            description: Some("link".into()),
            attachment_type: Some("simpleType".into()),
            external_url: Some("http://flowable.org".into()),
            content: None,
            user_id: None,
            process_instance_id: None,
        })
        .unwrap();

    assert_eq!(item.external_url.as_deref(), Some("http://flowable.org"));
    let err = content
        .get_task_attachment_content(&task_id, &item.id)
        .unwrap_err();
    assert!(matches!(err, FlowableError::NotFound(_)));
    assert!(event_actions(&engine, &task_id)
        .iter()
        .any(|a| a == "AddAttachment"));
}

#[test]
fn name_required_and_mid_command_failure_rolls_back() {
    let engine = engine("attach-fail");
    let task_id = deploy_and_start(&engine, "attachFailProcess");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let err = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: task_id.clone(),
            name: "   ".into(),
            description: None,
            attachment_type: None,
            external_url: None,
            content: None,
            user_id: None,
            process_instance_id: None,
        })
        .unwrap_err();
    assert!(matches!(err, FlowableError::BadRequest(_)));

    let err = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: task_id.clone(),
            name: "will-fail".into(),
            description: None,
            attachment_type: Some(FORCE_FAIL_ATTACHMENT_TYPE.into()),
            external_url: None,
            content: Some(b"orphan?".to_vec()),
            user_id: None,
            process_instance_id: None,
        })
        .unwrap_err();
    assert!(matches!(err, FlowableError::BadRequest(_)));

    assert!(content.list_task_attachments(&task_id).unwrap().is_empty());
    assert!(!event_actions(&engine, &task_id)
        .iter()
        .any(|a| a == "AddAttachment"));
}

#[test]
fn delete_removes_content_and_writes_event_missing_is_not_found() {
    let engine = engine("attach-delete");
    let task_id = deploy_and_start(&engine, "attachDeleteProcess");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: task_id.clone(),
            name: "to-delete.txt".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"x".to_vec()),
            user_id: None,
            process_instance_id: None,
        })
        .unwrap();

    content
        .delete_task_attachment(&task_id, &item.id, None)
        .unwrap();

    assert!(content.list_task_attachments(&task_id).unwrap().is_empty());
    assert!(event_actions(&engine, &task_id)
        .iter()
        .any(|a| a == "DeleteAttachment"));

    let err = content
        .delete_task_attachment(&task_id, "missing-attachment", None)
        .unwrap_err();
    assert!(matches!(err, FlowableError::NotFound(_)));
}

#[test]
fn after_completion_reads_ok_writes_need_runtime_task() {
    let engine = engine("attach-complete");
    let task_id = deploy_and_start(&engine, "attachCompleteProcess");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: task_id.clone(),
            name: "survives.txt".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"keep-me".to_vec()),
            user_id: None,
            process_instance_id: None,
        })
        .unwrap();

    engine
        .get_task_service()
        .complete_task_by_id(task_id.clone())
        .unwrap();

    let mut session = engine.get_runtime_store().create_session().unwrap();
    assert!(
        engine
            .get_runtime_store()
            .find_task(&task_id, &mut session)
            .is_none()
    );
    assert!(
        engine
            .get_runtime_store()
            .get_historic_task_instance(&task_id, &mut session)
            .is_some()
    );

    let listed = content.list_task_attachments(&task_id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, item.id);
    let got = content.get_task_attachment(&task_id, &item.id).unwrap();
    assert_eq!(got.name, "survives.txt");
    let bytes = content
        .get_task_attachment_content(&task_id, &item.id)
        .unwrap();
    assert_eq!(bytes.bytes, b"keep-me");

    let create_err = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: task_id.clone(),
            name: "nope".into(),
            description: None,
            attachment_type: None,
            external_url: Some("http://x".into()),
            content: None,
            user_id: None,
            process_instance_id: None,
        })
        .unwrap_err();
    assert!(matches!(create_err, FlowableError::NotFound(_)));

    let delete_err = content
        .delete_task_attachment(&task_id, &item.id, None)
        .unwrap_err();
    assert!(matches!(delete_err, FlowableError::NotFound(_)));
}

#[test]
fn suspended_task_rejects_create_without_side_effects() {
    let engine = engine("attach-suspended");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut task = Task::new(
        "task-suspended".into(),
        "proc-1".into(),
        "proc-1".into(),
        "def".into(),
        "Review".into(),
    );
    task.set_suspension_state(true);
    store.insert_task(&task, &mut session);
    session.flush_and_commit().unwrap();

    let content = FlowableContentService::new(Arc::clone(&engine));
    let err = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: "task-suspended".into(),
            name: "nope".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"x".to_vec()),
            user_id: None,
            process_instance_id: None,
        })
        .unwrap_err();
    assert!(err.to_string().contains("suspended"));
    assert!(
        content
            .list_task_attachments("task-suspended")
            .unwrap()
            .is_empty()
    );
    assert!(!event_actions(&engine, "task-suspended")
        .iter()
        .any(|a| a == "AddAttachment"));
}
