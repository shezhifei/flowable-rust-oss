//! P119 — EngineEventType completion for missing Java `FlowableEngineEventType`s.
//!
//! Verifies typed-event bus listeners receive newly wired types with key
//! payload fields. Required e2e families: MULTI_INSTANCE / TASK_*_CHANGED /
//! TIMER_*. Also covers HISTORIC_* on the default audit history path.
//!
//! Java throw-point references (verified against flowable-engine sources):
//! - MULTI_INSTANCE_ACTIVITY_STARTED: `ContinueProcessOperation.java:276-279`
//! - MULTI_INSTANCE_ACTIVITY_COMPLETED: `MultiInstanceActivityBehavior.java:431-435`
//! - MULTI_INSTANCE_ACTIVITY_COMPLETED_WITH_CONDITION: `…java:424-428`
//! - TASK_*_CHANGED: `TaskEntityManagerImpl.java:276-302`
//! - TIMER_SCHEDULED: `TimerJobSchedulerImpl.java:69-73`
//! - TIMER_FIRED: `TriggerTimerEventJobHandler.java:44-46`
//! - HISTORIC_*: `DefaultHistoryManager.java:90-95,120-126,215-218,234-237`

use chrono::{TimeZone, Utc};
use flowable_engine::engine::event_dispatcher::{
    EngineEvent, EngineEventDispatcher, EngineEventListener, EngineEventType, EntityKind,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::task_service::TaskUpdate;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct EventCollector {
    events: Arc<Mutex<Vec<EngineEvent>>>,
}

impl EventCollector {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn types(&self) -> Vec<EngineEventType> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.event_type())
            .collect()
    }

    fn entity_events_of(&self, ty: EngineEventType) -> Vec<(EntityKind, String, Option<String>)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                EngineEvent::Entity { event_type, data } if *event_type == ty => Some((
                    data.entity_kind,
                    data.entity_id.clone(),
                    data.process_instance_id.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    fn job_events_of(&self, ty: EngineEventType) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                EngineEvent::Job { event_type, job } if *event_type == ty => {
                    Some(job.timer_job_id.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

impl EngineEventListener for EventCollector {
    fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

fn engine_with_collector(name: &str) -> (ProcessEngine, EventCollector) {
    let mut config = ProcessEngineConfiguration::default();
    let collector = EventCollector::new();
    config.engine_event_dispatcher = EngineEventDispatcher::new();
    config
        .engine_event_dispatcher
        .add_event_listener(Arc::new(collector.clone()));
    let engine = ProcessEngine::new_with_config(name.to_string(), config);
    (engine, collector)
}

fn engine_with_typed_listener(
    name: &str,
    event_type: EngineEventType,
) -> (ProcessEngine, EventCollector) {
    let mut config = ProcessEngineConfiguration::default();
    let collector = EventCollector::new();
    config.engine_event_dispatcher = EngineEventDispatcher::new();
    config
        .engine_event_dispatcher
        .add_typed_event_listener(event_type, Arc::new(collector.clone()));
    let engine = ProcessEngine::new_with_config(name.to_string(), config);
    (engine, collector)
}

// ---------------------------------------------------------------------------
// MULTI_INSTANCE_* e2e
// ---------------------------------------------------------------------------

#[test]
fn multi_instance_started_and_completed_fire() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p119-mi" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toMi" sourceRef="start" targetRef="miTask" />
            <userTask id="miTask" name="MI">
                <multiInstanceLoopCharacteristics isSequential="true">
                    <loopCardinality>2</loopCardinality>
                </multiInstanceLoopCharacteristics>
            </userTask>
            <sequenceFlow id="toEnd" sourceRef="miTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p119-mi");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p119-mi.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::MultiInstanceActivityStarted),
        "MULTI_INSTANCE_ACTIVITY_STARTED must fire: {:?}",
        types
    );
    // MI path must NOT also emit ACTIVITY_STARTED for the MI body itself
    // (Java exclusive branch). Child instance activity starts are separate.
    let mi_started = collector.entity_events_of(EngineEventType::MultiInstanceActivityStarted);
    assert!(!mi_started.is_empty());
    assert_eq!(
        mi_started[0].2.as_deref(),
        Some(pi.id.as_str()),
        "MI started event must carry processInstanceId"
    );
    assert!(
        mi_started[0].1.starts_with("miTask:"),
        "entity_id should be activityId:type, got {}",
        mi_started[0].1
    );

    // Complete both sequential instances → MULTI_INSTANCE_ACTIVITY_COMPLETED.
    let task_service = engine.get_task_service();
    for _ in 0..2 {
        let tasks = task_service
            .get_tasks_by_process_instance_id(pi.id.clone())
            .unwrap();
        assert_eq!(tasks.len(), 1);
        task_service.complete_task_by_id(tasks[0].id.clone()).unwrap();
    }

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::MultiInstanceActivityCompleted),
        "MULTI_INSTANCE_ACTIVITY_COMPLETED must fire after all instances: {:?}",
        types
    );
    let completed = collector.entity_events_of(EngineEventType::MultiInstanceActivityCompleted);
    assert_eq!(completed[0].2.as_deref(), Some(pi.id.as_str()));
}

#[test]
fn multi_instance_completed_with_condition_fires() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p119-mi-cond" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toMi" sourceRef="start" targetRef="miTask" />
            <userTask id="miTask" name="MI">
                <multiInstanceLoopCharacteristics isSequential="true">
                    <loopCardinality>5</loopCardinality>
                    <completionCondition>${nrOfCompletedInstances == 1}</completionCondition>
                </multiInstanceLoopCharacteristics>
            </userTask>
            <sequenceFlow id="toEnd" sourceRef="miTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p119-mi-cond");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p119-mi-cond.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let task_service = engine.get_task_service();
    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    task_service.complete_task_by_id(tasks[0].id.clone()).unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::MultiInstanceActivityCompletedWithCondition),
        "MULTI_INSTANCE_ACTIVITY_COMPLETED_WITH_CONDITION must fire: {:?}",
        types
    );
}

#[test]
fn multi_instance_typed_listener_only_receives_mi_started() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p119-mi-typed" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toMi" sourceRef="start" targetRef="miTask" />
            <userTask id="miTask" name="MI">
                <multiInstanceLoopCharacteristics isSequential="true">
                    <loopCardinality>1</loopCardinality>
                </multiInstanceLoopCharacteristics>
            </userTask>
            <sequenceFlow id="toEnd" sourceRef="miTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_typed_listener(
        "p119-mi-typed",
        EngineEventType::MultiInstanceActivityStarted,
    );
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p119-mi-typed.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let types = collector.types();
    assert_eq!(
        types,
        vec![EngineEventType::MultiInstanceActivityStarted],
        "typed listener must only receive MULTI_INSTANCE_ACTIVITY_STARTED"
    );
}

// ---------------------------------------------------------------------------
// TASK_*_CHANGED e2e
// ---------------------------------------------------------------------------

#[test]
fn task_field_changed_events_fire_on_update_and_setters() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p119-task" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toTask" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Original" />
            <sequenceFlow id="toEnd" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p119-task-changed");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p119-task.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let task_service = engine.get_task_service();
    let task = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap()
        .into_iter()
        .next()
        .expect("user task");
    let task_id = task.id.clone();

    collector.clear();

    // update_task_by_id → owner / name (Java logTaskUpdateEvents).
    task_service
        .update_task_by_id(
            task_id.clone(),
            TaskUpdate {
                name: Some("Renamed".to_string()),
                owner: Some(Some("owner-1".to_string())),
                ..Default::default()
            },
        )
        .unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::TaskOwnerChanged),
        "TASK_OWNER_CHANGED: {:?}",
        types
    );
    assert!(
        types.contains(&EngineEventType::TaskNameChanged),
        "TASK_NAME_CHANGED: {:?}",
        types
    );
    let owner_ev = collector.entity_events_of(EngineEventType::TaskOwnerChanged);
    assert_eq!(owner_ev[0].1, task_id);
    assert_eq!(owner_ev[0].2.as_deref(), Some(pi.id.as_str()));

    collector.clear();

    // Dedicated setters → priority / dueDate.
    let due = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
    task_service
        .set_task_priority(task_id.clone(), 42)
        .unwrap();
    task_service
        .set_task_due_date(task_id.clone(), Some(due))
        .unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::TaskPriorityChanged),
        "TASK_PRIORITY_CHANGED: {:?}",
        types
    );
    assert!(
        types.contains(&EngineEventType::TaskDuedateChanged),
        "TASK_DUEDATE_CHANGED: {:?}",
        types
    );
}

// ---------------------------------------------------------------------------
// TIMER_* e2e
// ---------------------------------------------------------------------------

#[test]
fn timer_scheduled_and_fired_e2e() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p119-timer" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toCatch" sourceRef="start" targetRef="timerCatch" />
            <intermediateCatchEvent id="timerCatch" name="Wait">
                <timerEventDefinition>
                    <timeDuration>PT5M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="toEnd" sourceRef="timerCatch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p119-timer");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p119-timer.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::TimerScheduled),
        "TIMER_SCHEDULED must fire when intermediate timer is created: {:?}",
        types
    );
    let scheduled_ids = collector.job_events_of(EngineEventType::TimerScheduled);
    assert_eq!(scheduled_ids.len(), 1);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let jobs = store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(jobs.len(), 1);
    let job_id = jobs[0].timer_job_id.clone();
    session.rollback().unwrap();

    collector.clear();

    // Fire via manual execute_timer_job_by_id (dispatches TIMER_FIRED).
    runtime.execute_timer_job_by_id(&job_id).unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::TimerFired),
        "TIMER_FIRED must fire on execute: {:?}",
        types
    );
    let fired_ids = collector.job_events_of(EngineEventType::TimerFired);
    assert_eq!(fired_ids, vec![job_id]);
}

// ---------------------------------------------------------------------------
// HISTORIC_* (sync audit path)
// ---------------------------------------------------------------------------

#[test]
fn historic_process_and_activity_events_fire_on_audit_history() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p119-hist" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toEnd" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p119-hist");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p119-hist.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::HistoricProcessInstanceCreated),
        "HISTORIC_PROCESS_INSTANCE_CREATED: {:?}",
        types
    );
    assert!(
        types.contains(&EngineEventType::HistoricProcessInstanceEnded),
        "HISTORIC_PROCESS_INSTANCE_ENDED (instant process): {:?}",
        types
    );
    assert!(
        types.contains(&EngineEventType::HistoricActivityInstanceCreated),
        "HISTORIC_ACTIVITY_INSTANCE_CREATED: {:?}",
        types
    );
    assert!(
        types.contains(&EngineEventType::HistoricActivityInstanceEnded),
        "HISTORIC_ACTIVITY_INSTANCE_ENDED: {:?}",
        types
    );

    let created = collector.entity_events_of(EngineEventType::HistoricProcessInstanceCreated);
    assert_eq!(created[0].0, EntityKind::HistoricProcessInstance);
    assert_eq!(created[0].1, pi.id);
    assert_eq!(created[0].2.as_deref(), Some(pi.id.as_str()));
}

#[test]
fn boundary_timer_schedules_timer_scheduled_event() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p119-btimer" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toTask" sourceRef="start" targetRef="host" />
            <userTask id="host" name="Host" />
            <boundaryEvent id="timeout" attachedToRef="host" cancelActivity="true">
                <timerEventDefinition>
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="toEnd" sourceRef="host" targetRef="end" />
            <endEvent id="end" />
            <sequenceFlow id="toTimeoutEnd" sourceRef="timeout" targetRef="timeoutEnd" />
            <endEvent id="timeoutEnd" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p119-btimer");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p119-btimer.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::TimerScheduled),
        "boundary timer must dispatch TIMER_SCHEDULED: {:?}",
        types
    );
}
