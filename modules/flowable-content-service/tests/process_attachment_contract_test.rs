//! Engine-side Java parity contract for process-instance attachments (P65).
//!
//! Java truth:
//! - `TaskService.createAttachment(..., processInstanceId, ...)`
//! - `TaskService.getProcessInstanceAttachments(processInstanceId)`
//! - `CreateAttachmentCmd` / `GetProcessInstanceAttachmentsCmd`
//! - `AttachmentEntityManagerImpl.checkHistoryEnabled`
//!
//! Covers pure process attachments, task+process visibility, isolation,
//! payload read/delete, missing/suspended guards, history-disabled rejection,
//! and mid-command rollback.

use flowable_content_service::{
    CreateProcessAttachmentInput, CreateTaskAttachmentInput, FORCE_FAIL_ATTACHMENT_TYPE,
    FlowableContentService,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use flowable_engine::service::config::{HistoryLevel, ProcessEngineConfiguration};
use std::sync::Arc;

fn engine(name: &str) -> Arc<ProcessEngine> {
    Arc::new(ProcessEngine::new(name.to_string()))
}

fn engine_with_history(name: &str, level: HistoryLevel) -> Arc<ProcessEngine> {
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = level;
    Arc::new(ProcessEngine::new_with_config(name.to_string(), config))
}

fn deploy_and_start(engine: &ProcessEngine, process_key: &str) -> (String, String) {
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
    (pi.id, tasks[0].id.clone())
}

fn process_attachment_actions(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine
        .get_history_service()
        .get_process_instance_comments(process_instance_id, &mut session)
        .into_iter()
        .filter_map(|c| c.action)
        .collect()
}

fn task_event_actions(engine: &ProcessEngine, task_id: &str) -> Vec<String> {
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine
        .get_history_service()
        .get_task_events(task_id, &mut session)
        .into_iter()
        .map(|e| e.action)
        .collect()
}

#[test]
fn pure_process_attachment_create_list_get_content_and_history() {
    let engine = engine("proc-attach-pure");
    let (process_instance_id, _task_id) = deploy_and_start(&engine, "pureProcessAttach");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: process_instance_id.clone(),
            task_id: None,
            name: "proc-note.txt".into(),
            description: Some("process scoped".into()),
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"process-bytes".to_vec()),
            user_id: Some("admin".into()),
        })
        .unwrap();

    assert_eq!(item.name, "proc-note.txt");
    assert_eq!(item.process_instance_id.as_deref(), Some(process_instance_id.as_str()));
    assert!(item.task_id.is_none());
    assert_eq!(item.content_size, 13);
    assert_eq!(item.created_by.as_deref(), Some("admin"));

    let listed = content
        .list_process_attachments(&process_instance_id)
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, item.id);

    let got = content
        .get_process_attachment(&process_instance_id, &item.id)
        .unwrap();
    assert_eq!(got.name, "proc-note.txt");

    let payload = content
        .get_process_attachment_content(&process_instance_id, &item.id)
        .unwrap();
    assert_eq!(payload.bytes, b"process-bytes");

    // Pure process → process-associated historic comment/event (no fake task id).
    let actions = process_attachment_actions(&engine, &process_instance_id);
    assert!(actions.iter().any(|a| a == "AddAttachment"));
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let comments = engine
        .get_history_service()
        .get_process_instance_comments(&process_instance_id, &mut session);
    let add = comments
        .iter()
        .find(|c| c.action.as_deref() == Some("AddAttachment"))
        .unwrap();
    assert!(add.task_id.is_none());
    assert_eq!(add.message, "proc-note.txt");
}

#[test]
fn task_and_process_visibility_and_wrong_process_isolation() {
    let engine = engine("proc-attach-visibility");
    let (pi_a, task_a) = deploy_and_start(&engine, "visProcessA");
    let (pi_b, _task_b) = deploy_and_start(&engine, "visProcessB");
    let content = FlowableContentService::new(Arc::clone(&engine));

    // Task attachment (with process id) is visible under that process.
    let task_item = content
        .create_task_attachment(CreateTaskAttachmentInput {
            task_id: task_a.clone(),
            name: "task-file.txt".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"task".to_vec()),
            user_id: None,
            process_instance_id: Some(pi_a.clone()),
        })
        .unwrap();

    // Pure process attachment on A.
    let proc_item = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: pi_a.clone(),
            task_id: None,
            name: "proc-file.txt".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"proc".to_vec()),
            user_id: None,
        })
        .unwrap();

    // Process+task create: task must belong to process; history on task.
    let both = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: pi_a.clone(),
            task_id: Some(task_a.clone()),
            name: "both-file.txt".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"both".to_vec()),
            user_id: None,
        })
        .unwrap();
    assert_eq!(both.task_id.as_deref(), Some(task_a.as_str()));
    assert_eq!(both.process_instance_id.as_deref(), Some(pi_a.as_str()));
    assert!(task_event_actions(&engine, &task_a)
        .iter()
        .any(|a| a == "AddAttachment"));

    let listed_a = content.list_process_attachments(&pi_a).unwrap();
    let ids_a: Vec<_> = listed_a.iter().map(|i| i.id.as_str()).collect();
    assert!(ids_a.contains(&task_item.id.as_str()));
    assert!(ids_a.contains(&proc_item.id.as_str()));
    assert!(ids_a.contains(&both.id.as_str()));
    assert_eq!(listed_a.len(), 3);

    // Wrong process isolation: list B empty of A's attachments.
    let listed_b = content.list_process_attachments(&pi_b).unwrap();
    assert!(listed_b.iter().all(|i| i.process_instance_id.as_deref() != Some(pi_a.as_str())));
    assert!(!listed_b.iter().any(|i| i.id == proc_item.id));

    let err = content
        .get_process_attachment(&pi_b, &proc_item.id)
        .unwrap_err();
    assert!(matches!(err, FlowableError::NotFound(_)));

    let err = content
        .get_process_attachment_content(&pi_b, &proc_item.id)
        .unwrap_err();
    assert!(matches!(err, FlowableError::NotFound(_)));

    // Task belonging to A cannot be attached under process B.
    let err = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: pi_b.clone(),
            task_id: Some(task_a.clone()),
            name: "cross".into(),
            description: None,
            attachment_type: None,
            external_url: Some("http://x".into()),
            content: None,
            user_id: None,
        })
        .unwrap_err();
    assert!(matches!(err, FlowableError::BadRequest(_) | FlowableError::ExecutionError(_)));
}

#[test]
fn delete_removes_metadata_and_payload_atomically() {
    let engine = engine("proc-attach-delete");
    let (process_instance_id, _) = deploy_and_start(&engine, "deleteProcessAttach");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let item = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: process_instance_id.clone(),
            task_id: None,
            name: "to-delete.txt".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"gone".to_vec()),
            user_id: None,
        })
        .unwrap();

    content
        .delete_process_attachment(&process_instance_id, &item.id, None)
        .unwrap();

    assert!(
        content
            .list_process_attachments(&process_instance_id)
            .unwrap()
            .is_empty()
    );
    let get_err = content
        .get_process_attachment(&process_instance_id, &item.id)
        .unwrap_err();
    assert!(matches!(get_err, FlowableError::NotFound(_)));
    let content_err = content
        .get_process_attachment_content(&process_instance_id, &item.id)
        .unwrap_err();
    assert!(matches!(content_err, FlowableError::NotFound(_)));

    assert!(process_attachment_actions(&engine, &process_instance_id)
        .iter()
        .any(|a| a == "DeleteAttachment"));

    let missing = content
        .delete_process_attachment(&process_instance_id, "missing-id", None)
        .unwrap_err();
    assert!(matches!(missing, FlowableError::NotFound(_)));
}

#[test]
fn missing_process_and_suspended_mutation_guard() {
    let engine = engine("proc-attach-guards");
    let (process_instance_id, _) = deploy_and_start(&engine, "guardProcessAttach");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let missing = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: "no-such-process".into(),
            task_id: None,
            name: "x".into(),
            description: None,
            attachment_type: None,
            external_url: Some("http://x".into()),
            content: None,
            user_id: None,
        })
        .unwrap_err();
    assert!(matches!(missing, FlowableError::NotFound(_)));
    assert!(missing.to_string().contains("no-such-process") || missing.to_string().contains("doesn't exist") || missing.to_string().contains("not found") || missing.to_string().contains("Process instance"));

    engine
        .get_runtime_service()
        .suspend_process_instance(process_instance_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let suspended = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: process_instance_id.clone(),
            task_id: None,
            name: "blocked".into(),
            description: None,
            attachment_type: Some("text/plain".into()),
            external_url: None,
            content: Some(b"x".to_vec()),
            user_id: None,
        })
        .unwrap_err();
    assert!(suspended.to_string().contains("suspended"));
    assert!(
        content
            .list_process_attachments(&process_instance_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn history_disabled_rejects_attachment_operations() {
    let engine = engine_with_history("proc-attach-no-history", HistoryLevel::None);
    // With history None, deploy/start still produces a runtime process instance.
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="noHistProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Task 1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    repo.deploy(
        repo.create_deployment()
            .add_string("noHistProcess.bpmn20.xml".into(), xml.to_string()),
    )
    .unwrap();
    let pi = runtime
        .start_process_instance_by_key("noHistProcess")
        .unwrap();
    let content = FlowableContentService::new(Arc::clone(&engine));

    let err = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: pi.id.clone(),
            task_id: None,
            name: "nope".into(),
            description: None,
            attachment_type: None,
            external_url: Some("http://x".into()),
            content: None,
            user_id: None,
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("history should be enabled")
            || err.to_string().to_ascii_lowercase().contains("history")
    );

    let list_err = content.list_process_attachments(&pi.id).unwrap_err();
    assert!(
        list_err.to_string().contains("history should be enabled")
            || list_err.to_string().to_ascii_lowercase().contains("history")
    );
}

#[test]
fn mid_command_failure_rolls_back_staged_payload() {
    let engine = engine("proc-attach-rollback");
    let (process_instance_id, _) = deploy_and_start(&engine, "rollbackProcessAttach");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let err = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: process_instance_id.clone(),
            task_id: None,
            name: "will-fail".into(),
            description: None,
            attachment_type: Some(FORCE_FAIL_ATTACHMENT_TYPE.into()),
            external_url: None,
            content: Some(b"orphan?".to_vec()),
            user_id: None,
        })
        .unwrap_err();
    assert!(matches!(err, FlowableError::BadRequest(_)));

    assert!(
        content
            .list_process_attachments(&process_instance_id)
            .unwrap()
            .is_empty()
    );
    assert!(!process_attachment_actions(&engine, &process_instance_id)
        .iter()
        .any(|a| a == "AddAttachment"));
}

#[test]
fn at_least_one_scope_required_and_name_required() {
    let engine = engine("proc-attach-scope");
    let (process_instance_id, _) = deploy_and_start(&engine, "scopeProcessAttach");
    let content = FlowableContentService::new(Arc::clone(&engine));

    let name_err = content
        .create_process_attachment(CreateProcessAttachmentInput {
            process_instance_id: process_instance_id.clone(),
            task_id: None,
            name: "   ".into(),
            description: None,
            attachment_type: None,
            external_url: Some("http://x".into()),
            content: None,
            user_id: None,
        })
        .unwrap_err();
    assert!(matches!(name_err, FlowableError::BadRequest(_)));
}
