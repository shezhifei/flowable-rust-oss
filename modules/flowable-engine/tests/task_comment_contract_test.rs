//! Engine contract tests for task comments/events (P2-COMMENT).
//!
//! Java parity sources:
//! - `AddCommentCmd.java` (whitespace collapse, 163-char event message, author)
//! - `Comment.xml` selectCommentsByTaskId / selectEventsByTaskId (TIME_ desc)
//! - REST historic-task list/get vs runtime create/delete split

use flowable_engine::cmd::create_task_comment_cmd::normalize_comment_event_message;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::runtime::process_instance::ProcessInstance;
use flowable_engine::task::Task;
use std::thread;
use std::time::Duration;

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

fn insert_standalone_task(engine: &ProcessEngine, task_id: &str) {
    let task = Task::new(
        task_id.to_string(),
        String::new(),
        String::new(),
        "standaloneTask".to_string(),
        "Standalone task".to_string(),
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_task(&task, &mut session);
    session.flush_and_commit().unwrap();
}

#[test]
fn normalize_comment_event_message_matches_java_rules() {
    // Java: replaceAll("\\s+", " ") keeps a single space for each whitespace run.
    assert_eq!(
        normalize_comment_event_message("hello   world\n\tfoo"),
        "hello world foo"
    );
    assert_eq!(normalize_comment_event_message(""), "");
    assert_eq!(normalize_comment_event_message("   "), " ");
    assert_eq!(normalize_comment_event_message("  a  "), " a ");

    // Exactly 163 chars → no truncation.
    let exact_163: String = "a".repeat(163);
    assert_eq!(normalize_comment_event_message(&exact_163), exact_163);

    // 164+ → first 160 chars + "..." (total length 163).
    let long: String = "x".repeat(200);
    let normalized = normalize_comment_event_message(&long);
    assert_eq!(normalized.len(), 163);
    assert!(normalized.ends_with("..."));
    assert_eq!(&normalized[..160], &"x".repeat(160));

    // Whitespace collapse happens before length check.
    let with_spaces = format!("{}{}", "y".repeat(100), " ".repeat(50));
    let collapsed = normalize_comment_event_message(&with_spaces);
    assert_eq!(collapsed, format!("{} ", "y".repeat(100)));
}

#[test]
fn create_comment_stores_full_message_and_normalized_event() {
    let engine = ProcessEngine::new("comment-normalize-create".to_string());
    let (_pi, task_id) = deploy_and_start(&engine, "commentNormalizeProcess");

    let raw = "Please   review\nthis   invoice   carefully";
    let comment = engine
        .get_history_service()
        .create_task_comment(&task_id, None, raw, Some("kermit"))
        .unwrap();

    assert_eq!(comment.message, raw);
    assert_eq!(comment.author.as_deref(), Some("kermit"));

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let events = engine
        .get_history_service()
        .get_task_events(&task_id, &mut session);
    let add_comment = events
        .iter()
        .find(|e| e.action == "AddComment")
        .expect("AddComment event");
    assert_eq!(
        add_comment.message,
        vec![normalize_comment_event_message(raw)]
    );
    assert_eq!(add_comment.user_id.as_deref(), Some("kermit"));
}

#[test]
fn create_comment_truncates_event_message_at_163() {
    let engine = ProcessEngine::new("comment-truncate-event".to_string());
    let (_pi, task_id) = deploy_and_start(&engine, "commentTruncateProcess");

    let raw: String = "z".repeat(200);
    engine
        .get_history_service()
        .create_task_comment(&task_id, None, &raw, None)
        .unwrap();

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let events = engine
        .get_history_service()
        .get_task_events(&task_id, &mut session);
    let add_comment = events.iter().find(|e| e.action == "AddComment").unwrap();
    assert_eq!(add_comment.message[0].len(), 163);
    assert!(add_comment.message[0].ends_with("..."));

    let comment = engine
        .get_history_service()
        .get_task_comments(&task_id, &mut session)
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(comment.message, raw);
}

#[test]
fn empty_and_whitespace_messages_are_accepted() {
    let engine = ProcessEngine::new("comment-empty-ws".to_string());
    let (_pi, task_id) = deploy_and_start(&engine, "commentEmptyWsProcess");
    let history = engine.get_history_service();

    let empty = history
        .create_task_comment(&task_id, None, "", Some("admin"))
        .unwrap();
    assert_eq!(empty.message, "");

    let whitespace = history
        .create_task_comment(&task_id, None, "   \t\n  ", Some("admin"))
        .unwrap();
    assert_eq!(whitespace.message, "   \t\n  ");

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let comments = history.get_task_comments(&task_id, &mut session);
    assert_eq!(comments.len(), 2);

    let events = history.get_task_events(&task_id, &mut session);
    let add_events: Vec<_> = events.iter().filter(|e| e.action == "AddComment").collect();
    assert_eq!(add_events.len(), 2);
    // Newest first (TIME_ desc).
    assert_eq!(
        add_events[0].message,
        vec![normalize_comment_event_message("   \t\n  ")]
    );
    assert_eq!(
        add_events[1].message,
        vec![normalize_comment_event_message("")]
    );
}

#[test]
fn comments_and_events_are_ordered_newest_first() {
    let engine = ProcessEngine::new("comment-order-desc".to_string());
    let (_pi, task_id) = deploy_and_start(&engine, "commentOrderProcess");
    let history = engine.get_history_service();

    history
        .create_task_comment(&task_id, None, "first", None)
        .unwrap();
    thread::sleep(Duration::from_millis(5));
    history
        .create_task_comment(&task_id, None, "second", None)
        .unwrap();
    thread::sleep(Duration::from_millis(5));
    history
        .create_task_comment(&task_id, None, "third", None)
        .unwrap();

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let comments = history.get_task_comments(&task_id, &mut session);
    assert_eq!(
        comments
            .iter()
            .map(|c| c.message.as_str())
            .collect::<Vec<_>>(),
        vec!["third", "second", "first"]
    );

    let add_events: Vec<_> = history
        .get_task_events(&task_id, &mut session)
        .into_iter()
        .filter(|e| e.action == "AddComment")
        .collect();
    assert_eq!(
        add_events
            .iter()
            .map(|e| e.message[0].as_str())
            .collect::<Vec<_>>(),
        vec!["third", "second", "first"]
    );
}

#[test]
fn comments_and_events_readable_after_task_completion() {
    let engine = ProcessEngine::new("comment-after-complete".to_string());
    let (_pi, task_id) = deploy_and_start(&engine, "commentAfterCompleteProcess");
    let history = engine.get_history_service();

    let comment = history
        .create_task_comment(&task_id, None, "survives completion", Some("admin"))
        .unwrap();

    engine
        .get_task_service()
        .complete_task_by_id(task_id.clone())
        .unwrap();

    // Runtime task is gone.
    let mut session = engine.get_runtime_store().create_session().unwrap();
    assert!(
        engine
            .get_runtime_store()
            .find_task(&task_id, &mut session)
            .is_none()
    );
    // Historic task remains.
    assert!(
        engine
            .get_runtime_store()
            .get_historic_task_instance(&task_id, &mut session)
            .is_some()
    );

    let comments = history.get_task_comments(&task_id, &mut session);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].id, comment.id);
    assert_eq!(comments[0].message, "survives completion");

    let events = history.get_task_events(&task_id, &mut session);
    assert!(events.iter().any(|e| e.action == "AddComment"));
    assert!(history.get_comment(&comment.id, &mut session).is_some());
}

#[test]
fn missing_and_suspended_task_guards_remain() {
    let engine = ProcessEngine::new("comment-guards".to_string());

    let err = engine
        .get_history_service()
        .create_task_comment("missing", None, "x", None)
        .unwrap_err();
    assert!(matches!(err, FlowableError::NotFound(_)));

    insert_standalone_task(&engine, "task-suspended");
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let mut task = store.find_task("task-suspended", &mut session).unwrap();
        task.set_suspension_state(true);
        store.update_task(&task, &mut session);
        session.flush_and_commit().unwrap();
    }

    let err = engine
        .get_history_service()
        .create_task_comment("task-suspended", None, "x", None)
        .unwrap_err();
    assert!(err.to_string().contains("suspended task"));
}

#[test]
fn suspended_process_instance_still_rejected_when_linked() {
    let engine = ProcessEngine::new("comment-suspended-pi".to_string());
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_task(
        &Task::new(
            "task-1".to_string(),
            "process-1".to_string(),
            "process-1".to_string(),
            "def".to_string(),
            "Review".to_string(),
        ),
        &mut session,
    );
    store.insert_process_instance(
        &ProcessInstance {
            id: "process-1".to_string(),
            name: None,
            process_definition_id: "definition-1".to_string(),
            process_definition_key: "definition".to_string(),
            process_definition_name: None,
            process_definition_version: 1,
            business_key: None,
            business_status: None,
            is_suspended: true,
            tenant_id: None,
            start_time: None,
            start_user_id: None,
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
            is_ended: false,
            super_execution_id: None,
            root_process_instance_id: Some("process-1".to_string()),
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let err = engine
        .get_history_service()
        .create_task_comment("task-1", Some("process-1"), "x", None)
        .unwrap_err();
    assert!(err.to_string().contains("suspended process instance"));
}

// ── P65-comment-type ────────────────────────────────────────────────────────

#[test]
fn default_comment_type_is_comment() {
    let engine = ProcessEngine::new("comment-default-type".to_string());
    let (_pi, task_id) = deploy_and_start(&engine, "commentDefaultTypeProcess");

    let comment = engine
        .get_history_service()
        .create_task_comment(&task_id, None, "hello", Some("kermit"))
        .unwrap();
    assert_eq!(comment.resolved_type(), "comment");
    assert_eq!(comment.comment_type.as_deref(), Some("comment"));

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let loaded = engine
        .get_history_service()
        .get_comment(&comment.id, &mut session)
        .unwrap();
    assert_eq!(loaded.resolved_type(), "comment");
}

#[test]
fn custom_comment_type_is_persisted_and_queryable() {
    let engine = ProcessEngine::new("comment-custom-type".to_string());
    let (pi_id, task_id) = deploy_and_start(&engine, "commentCustomTypeProcess");
    let history = engine.get_history_service();

    let custom = history
        .create_task_comment_with_type(
            &task_id,
            Some(&pi_id),
            "audit",
            "typed message",
            Some("kermit"),
        )
        .unwrap();
    assert_eq!(custom.resolved_type(), "audit");
    assert_eq!(custom.message, "typed message");

    // Default list only returns TYPE_COMMENT (Java selectCommentsByTaskId).
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let default_list = history.get_task_comments(&task_id, &mut session);
    assert!(
        default_list.iter().all(|c| c.resolved_type() == "comment"),
        "default task comments must not include custom type"
    );
    assert!(!default_list.iter().any(|c| c.id == custom.id));

    // Typed task list.
    let typed = history.get_task_comments_by_type(&task_id, "audit", &mut session);
    assert_eq!(typed.len(), 1);
    assert_eq!(typed[0].id, custom.id);
    assert_eq!(typed[0].resolved_type(), "audit");

    // Process + type.
    let process_typed =
        history.get_process_instance_comments_by_type(&pi_id, "audit", &mut session);
    assert_eq!(process_typed.len(), 1);
    assert_eq!(process_typed[0].id, custom.id);

    // Global type list.
    let global = history.get_comments_by_type("audit", &mut session);
    assert_eq!(global.len(), 1);
    assert_eq!(global[0].id, custom.id);
}

#[test]
fn typed_and_default_comments_newest_first_and_do_not_conflate_events() {
    let engine = ProcessEngine::new("comment-type-order".to_string());
    let (pi_id, task_id) = deploy_and_start(&engine, "commentTypeOrderProcess");
    let history = engine.get_history_service();

    history
        .create_task_comment(&task_id, Some(&pi_id), "default-first", None)
        .unwrap();
    thread::sleep(Duration::from_millis(5));
    history
        .create_task_comment_with_type(&task_id, Some(&pi_id), "note", "note-first", None)
        .unwrap();
    thread::sleep(Duration::from_millis(5));
    history
        .create_task_comment(&task_id, Some(&pi_id), "default-second", None)
        .unwrap();
    thread::sleep(Duration::from_millis(5));
    history
        .create_task_comment_with_type(&task_id, Some(&pi_id), "note", "note-second", None)
        .unwrap();

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let defaults = history.get_task_comments(&task_id, &mut session);
    assert_eq!(
        defaults
            .iter()
            .map(|c| c.message.as_str())
            .collect::<Vec<_>>(),
        vec!["default-second", "default-first"]
    );

    let notes = history.get_task_comments_by_type(&task_id, "note", &mut session);
    assert_eq!(
        notes
            .iter()
            .map(|c| c.message.as_str())
            .collect::<Vec<_>>(),
        vec!["note-second", "note-first"]
    );

    // Task events remain a separate observable concept (not migrated into comments).
    let events = history.get_task_events(&task_id, &mut session);
    assert!(events.iter().any(|e| e.action == "AddComment"));
    assert!(
        !notes.iter().any(|c| c.resolved_type() == "event"),
        "typed comment list must not surface HistoricTaskEvent rows as comments"
    );
}

#[test]
fn process_instance_typed_comment_and_global_type_list() {
    let engine = ProcessEngine::new("comment-pi-type".to_string());
    let (pi_id, _task_id) = deploy_and_start(&engine, "commentPiTypeProcess");
    let history = engine.get_history_service();

    let plain = history
        .create_process_instance_comment(&pi_id, "plain pi", Some("admin"))
        .unwrap();
    assert_eq!(plain.resolved_type(), "comment");

    let tagged = history
        .create_process_instance_comment_with_type(&pi_id, "review", "needs review", Some("admin"))
        .unwrap();
    assert_eq!(tagged.resolved_type(), "review");

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let all = history.get_process_instance_comments(&pi_id, &mut session);
    assert!(all.iter().any(|c| c.id == plain.id));
    assert!(all.iter().any(|c| c.id == tagged.id));

    let review = history.get_process_instance_comments_by_type(&pi_id, "review", &mut session);
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].id, tagged.id);

    let global_review = history.get_comments_by_type("review", &mut session);
    assert_eq!(global_review.len(), 1);
    assert_eq!(global_review[0].id, tagged.id);
}

#[test]
fn save_comment_preserves_id_and_can_update_type() {
    let engine = ProcessEngine::new("comment-save-type".to_string());
    let (pi_id, task_id) = deploy_and_start(&engine, "commentSaveTypeProcess");
    let history = engine.get_history_service();

    let mut comment = history
        .create_task_comment(&task_id, Some(&pi_id), "original", Some("kermit"))
        .unwrap();
    let original_id = comment.id.clone();
    assert_eq!(comment.resolved_type(), "comment");

    comment.message = "updated body".to_string();
    comment.comment_type = Some("escalation".to_string());
    history.save_comment(comment).unwrap();

    let mut session = engine.get_runtime_store().create_session().unwrap();
    let reloaded = history.get_comment(&original_id, &mut session).unwrap();
    assert_eq!(reloaded.id, original_id);
    assert_eq!(reloaded.message, "updated body");
    assert_eq!(reloaded.resolved_type(), "escalation");
    assert_eq!(reloaded.task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(reloaded.process_instance_id.as_deref(), Some(pi_id.as_str()));
    assert_eq!(reloaded.author.as_deref(), Some("kermit"));

    // After type change it leaves the default list and appears under the new type.
    let defaults = history.get_task_comments(&task_id, &mut session);
    assert!(!defaults.iter().any(|c| c.id == original_id));
    let escalations = history.get_task_comments_by_type(&task_id, "escalation", &mut session);
    assert_eq!(escalations.len(), 1);
    assert_eq!(escalations[0].id, original_id);
}

#[test]
fn legacy_comment_json_without_type_resolves_as_comment() {
    use chrono::Utc;
    use flowable_engine::history::historic_entities::HistoricComment;

    let engine = ProcessEngine::new("comment-legacy-type".to_string());
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();

    // Simulate a pre-P65 row: no comment_type field in JSON, no projected column.
    let legacy = HistoricComment {
        id: "legacy-comment-1".to_string(),
        task_id: Some("task-legacy".to_string()),
        process_instance_id: None,
        message: "old row".to_string(),
        author: Some("admin".to_string()),
        time: Utc::now(),
        action: None,
        comment_type: None,
    };
    // Insert via JSON only so comment_type column stays NULL (backward-compat path).
    session
        .insert("historic_comments", &legacy.id, &legacy)
        .unwrap();
    // Also project task_id/time so list-by-task still works.
    store.insert_historic_comment(
        HistoricComment {
            id: "legacy-comment-2".to_string(),
            task_id: Some("task-legacy".to_string()),
            process_instance_id: None,
            message: "new default".to_string(),
            author: None,
            time: Utc::now(),
            action: None,
            comment_type: Some("comment".to_string()),
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let mut session = store.create_session().unwrap();
    let loaded = store
        .find_historic_comment("legacy-comment-1", &mut session)
        .unwrap();
    assert_eq!(loaded.comment_type, None);
    assert_eq!(loaded.resolved_type(), "comment");

    // Global type=comment includes legacy null-type user comments.
    let by_type = engine
        .get_history_service()
        .get_comments_by_type("comment", &mut session);
    assert!(by_type.iter().any(|c| c.id == "legacy-comment-1"));
    assert!(by_type.iter().any(|c| c.id == "legacy-comment-2"));
}
