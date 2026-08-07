//! P97 contract tests — history tail fixes.
//!
//! ⑥ `insert_task` no longer silently syncs the historic row: the
//!    user-task create/assignment listener path must still produce the
//!    assignee identity link (previously eaten by the order of
//!    update-then-record).
//! ⑦ standalone `create_task` routes through `HistoryManager
//!    .record_task_created` (gating applies; P90b no-IL pin preserved).
//! ⑧ claim/unclaim `AddUserLink`/`DeleteUserLink` task events are gated by
//!    `history_disabled` (previously written unconditionally).

use flowable_engine::bpmn::listener::{
    LocalTaskListener, LocalTaskListenerRegistry, TaskListenerContext,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::{HistoryLevel, ProcessEngineConfiguration};
use flowable_engine::task::Task;
use std::sync::Arc;

/// ⑥ — a `create` task listener that assigns the task must still produce the
/// historic assignee identity link. Before P97 the listener side-effect was
/// persisted *before* `record_task_updated` ran, so `insert_task`'s silent
/// historic sync had already overwritten the row and the IL diff was empty.
struct AssigningCreateListener;

impl LocalTaskListener for AssigningCreateListener {
    fn notify(&self, ctx: &mut TaskListenerContext<'_>) -> Result<(), FlowableError> {
        ctx.task.assignee = Some("listenerKermit".to_string());
        Ok(())
    }
}

fn assigning_listener_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="p97ListenerProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="userTask1" />
            <userTask id="userTask1" name="Review">
                <extensionElements>
                    <flowable:taskListener event="create" class="assigningCreateListener" />
                </extensionElements>
            </userTask>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
        .to_string()
}

#[test]
fn create_listener_assignee_change_produces_historic_identity_link() {
    let mut registry = LocalTaskListenerRegistry::new();
    registry.register("assigningCreateListener", Arc::new(AssigningCreateListener));
    let mut config = ProcessEngineConfiguration::default();
    config.task_listener_registry = Some(registry);
    let engine = ProcessEngine::new_with_config("p97-listener-il".to_string(), config);

    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("p97-listener-il".to_string())
                .add_string("p97.bpmn20.xml".to_string(), assigning_listener_xml()),
        )
        .unwrap();

    let runtime = engine.get_runtime_service();
    let process_definition_id = repository.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .next()
        .expect("one user task");
    assert_eq!(task.assignee.as_deref(), Some("listenerKermit"));

    let historic_ils = engine
        .get_history_service()
        .get_historic_identity_links_for_task(&task.id)
        .unwrap();
    assert!(
        historic_ils
            .iter()
            .any(|il| il.link_type == "assignee"
                && il.user_id.as_deref() == Some("listenerKermit")),
        "listener-assigned assignee must produce a historic identity link \
         (Java HistoricTaskServiceImpl.recordTaskInfoChange:142-152): {historic_ils:?}"
    );
}

/// ⑥ — regression for the silent-sync removal: the model priority must reach
/// the historic row through the HistoryManager, not through insert_task's
/// throwaway clone. Before P97 the runtime Task never carried the resolved
/// priority, and `record_task_updated` overwrote the historic row with None.
#[test]
fn model_priority_and_due_date_reach_historic_task() {
    let engine = ProcessEngine::new("p97-priority-historic".to_string());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="p97PriorityProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task" />
            <userTask id="task" name="Probe" flowable:priority="70" flowable:dueDate="2026-01-31T10:20:30Z" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("p97-priority".to_string())
                .add_string("p97-priority.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let runtime = engine.get_runtime_service();
    let process_definition_id = repository.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();
    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()[0]
        .clone();
    assert_eq!(task.priority, Some(70));

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let historic = store
        .get_historic_task_instance(&task.id, &mut session)
        .expect("historic row");
    session.rollback().unwrap();
    assert_eq!(historic.priority, Some(70));
    assert!(historic.due_date.is_some());
}

/// ⑦ — standalone create routes through the HistoryManager: with history
/// disabled no historic task row is written (the old hand-built insert in
/// CreateTaskCmd ignored `history_disabled`).
#[test]
fn standalone_create_task_respects_history_disabled() {
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = HistoryLevel::None;
    let engine = ProcessEngine::new_with_config("p97-standalone-none".to_string(), config);

    let task = Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "standalone-no-history".to_string(),
    );
    let created = engine
        .get_task_service()
        .create_task(task)
        .expect("create standalone task");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store
            .get_historic_task_instance(&created.id, &mut session)
            .is_none(),
        "history=None must not write a historic task instance"
    );
    session.rollback().unwrap();
}

/// ⑦ — positive path: default (FULL) history still records the standalone
/// task with its assignee, and the P90b pin holds (no IL for the initial
/// standalone assignee).
#[test]
fn standalone_create_task_records_history_without_identity_link() {
    let engine = ProcessEngine::new("p97-standalone-full".to_string());
    let mut task = Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "standalone-full-history".to_string(),
    );
    task.assignee = Some("kermit".to_string());
    let created = engine
        .get_task_service()
        .create_task(task)
        .expect("create standalone task");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let historic = store
        .get_historic_task_instance(&created.id, &mut session)
        .expect("historic task instance recorded via HistoryManager");
    session.rollback().unwrap();
    assert_eq!(historic.assignee.as_deref(), Some("kermit"));
    assert_eq!(historic.name.as_deref(), Some("standalone-full-history"));

    let historic_ils = engine
        .get_history_service()
        .get_historic_identity_links_for_task(&created.id)
        .unwrap();
    assert!(
        historic_ils.is_empty(),
        "P90b pin: standalone initial assignee must not write historic IL: {historic_ils:?}"
    );
}

/// ⑧ — claim/unclaim task events are gated by history configuration.
/// FULL records AddUserLink/DeleteUserLink; NONE records nothing (previously
/// written unconditionally via a direct store call).
#[test]
fn claim_unclaim_task_events_respect_history_level() {
    // FULL: events recorded.
    let engine = ProcessEngine::new("p97-claim-full".to_string());
    let task = Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "claim-events-full".to_string(),
    );
    let created = engine
        .get_task_service()
        .create_task(task)
        .expect("create task");
    let task_service = engine.get_task_service();
    task_service
        .claim_task_by_id(created.id.clone(), "kermit".to_string())
        .unwrap();
    task_service.unclaim_task_by_id(created.id.clone()).unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let actions: Vec<String> = engine
        .get_history_service()
        .get_task_events(&created.id, &mut session)
        .iter()
        .map(|event| event.action.clone())
        .collect();
    session.rollback().unwrap();
    assert!(
        actions.iter().any(|action| action == "AddUserLink"),
        "claim must record AddUserLink under FULL history: {actions:?}"
    );
    assert!(
        actions.iter().any(|action| action == "DeleteUserLink"),
        "unclaim must record DeleteUserLink under FULL history: {actions:?}"
    );

    // NONE: nothing recorded.
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = HistoryLevel::None;
    let engine_none = ProcessEngine::new_with_config("p97-claim-none".to_string(), config);
    let task_none = Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "claim-events-none".to_string(),
    );
    let created_none = engine_none
        .get_task_service()
        .create_task(task_none)
        .expect("create task");
    let task_service_none = engine_none.get_task_service();
    task_service_none
        .claim_task_by_id(created_none.id.clone(), "kermit".to_string())
        .unwrap();
    task_service_none
        .unclaim_task_by_id(created_none.id.clone())
        .unwrap();

    let store_none = engine_none.get_runtime_store();
    let mut session_none = store_none.create_session().unwrap();
    let events_none = engine_none
        .get_history_service()
        .get_task_events(&created_none.id, &mut session_none);
    session_none.rollback().unwrap();
    assert!(
        events_none.is_empty(),
        "history=None must not record claim/unclaim task events: {events_none:?}"
    );
}
