//! P125 — ACTIVITY_*_WAITING residual event types + terminate / job reschedule.
//!
//! Verifies typed-event bus listeners receive newly wired types with key
//! payload fields. Required e2e: PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT.
//!
//! Java throw-point references (verified against flowable-engine sources):
//! - ACTIVITY_SIGNAL_WAITING: `IntermediateCatchSignalEventActivityBehavior.java:74-76`
//!   / `BoundarySignalEventActivityBehavior.java:79-81`
//! - ACTIVITY_MESSAGE_WAITING: `IntermediateCatchMessageEventActivityBehavior.java:67-73`
//!   / `BoundaryMessageEventActivityBehavior.java:71-73`
//! - ACTIVITY_CONDITIONAL_WAITING / RECEIVED:
//!   `IntermediateCatchConditionalEventActivityBehavior.java:46-49,63-65`
//!   / `BoundaryConditionalEventActivityBehavior.java:52-54,70-72`
//! - ACTIVITY_ESCALATION_WAITING: `BoundaryEscalationEventActivityBehavior.java:60-62`
//! - ACTIVITY_MESSAGE_CANCELLED: `ExecutionEntityManagerImpl.java:1063-1066`
//! - PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT:
//!   `TerminateEndEventActivityBehavior.java:247-248`
//! - JOB_RESCHEDULED: `TimerUtil.java:277-278`

use flowable_engine::engine::event_dispatcher::{
    EngineEvent, EngineEventDispatcher, EngineEventListener, EngineEventType, EntityKind,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;
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

    fn entity_events_of(
        &self,
        ty: EngineEventType,
    ) -> Vec<(EntityKind, String, Option<String>, Option<String>)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                EngineEvent::Entity { event_type, data } if *event_type == ty => Some((
                    data.entity_kind,
                    data.entity_id.clone(),
                    data.process_instance_id.clone(),
                    data.scope_id.clone(),
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

// ---------------------------------------------------------------------------
// ACTIVITY_SIGNAL_WAITING
// ---------------------------------------------------------------------------

#[test]
fn activity_signal_waiting_fires_on_intermediate_catch() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="alertSignal" />
        <process id="p125-signal-wait" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toCatch" sourceRef="start" targetRef="signalCatch" />
            <intermediateCatchEvent id="signalCatch">
                <signalEventDefinition signalRef="alertSignal" />
            </intermediateCatchEvent>
            <sequenceFlow id="toEnd" sourceRef="signalCatch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p125-signal-wait");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p125-signal.bpmn20.xml".to_string(), xml.to_string()),
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
        types.contains(&EngineEventType::ActivitySignalWaiting),
        "ACTIVITY_SIGNAL_WAITING must fire: {:?}",
        types
    );
    let evs = collector.entity_events_of(EngineEventType::ActivitySignalWaiting);
    assert_eq!(evs[0].2.as_deref(), Some(pi.id.as_str()));
    assert!(
        evs[0].1.starts_with("signalCatch:"),
        "entity_id should be activityId:signalName, got {}",
        evs[0].1
    );
    assert_eq!(evs[0].3.as_deref(), Some("alertSignal"));
}

// ---------------------------------------------------------------------------
// ACTIVITY_MESSAGE_WAITING
// ---------------------------------------------------------------------------

#[test]
fn activity_message_waiting_fires_on_intermediate_catch() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="orderMsg" name="orderMsg" />
        <process id="p125-msg-wait" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toCatch" sourceRef="start" targetRef="msgCatch" />
            <intermediateCatchEvent id="msgCatch">
                <messageEventDefinition messageRef="orderMsg" />
            </intermediateCatchEvent>
            <sequenceFlow id="toEnd" sourceRef="msgCatch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p125-msg-wait");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p125-msg.bpmn20.xml".to_string(), xml.to_string()),
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
        types.contains(&EngineEventType::ActivityMessageWaiting),
        "ACTIVITY_MESSAGE_WAITING must fire: {:?}",
        types
    );
    let evs = collector.entity_events_of(EngineEventType::ActivityMessageWaiting);
    assert_eq!(evs[0].2.as_deref(), Some(pi.id.as_str()));
    assert_eq!(evs[0].3.as_deref(), Some("orderMsg"));
}

// ---------------------------------------------------------------------------
// ACTIVITY_CONDITIONAL_WAITING + ACTIVITY_CONDITIONAL_RECEIVED
// ---------------------------------------------------------------------------

#[test]
fn activity_conditional_waiting_and_received_fire() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p125-cond" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toCatch" sourceRef="start" targetRef="condCatch" />
            <intermediateCatchEvent id="condCatch">
                <conditionalEventDefinition>
                    <condition>${approve == true}</condition>
                </conditionalEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="toEnd" sourceRef="condCatch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p125-cond");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p125-cond.bpmn20.xml".to_string(), xml.to_string()),
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
        types.contains(&EngineEventType::ActivityConditionalWaiting),
        "ACTIVITY_CONDITIONAL_WAITING must fire: {:?}",
        types
    );
    let waiting = collector.entity_events_of(EngineEventType::ActivityConditionalWaiting);
    assert_eq!(waiting[0].2.as_deref(), Some(pi.id.as_str()));
    assert!(waiting[0].1.starts_with("condCatch:"));

    collector.clear();

    let wait_states = runtime.get_event_wait_states_by_process_instance_id(pi.id.clone());
    let catch_exec = wait_states
        .iter()
        .find(|e| e.activity_id.as_deref() == Some("condCatch"))
        .expect("conditional wait");
    runtime.trigger_event_intermediate_catch(
        EventSubscriptionKind::Conditional,
        "${approve == true}".to_string(),
        catch_exec.execution_id.clone(),
    );

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::ActivityConditionalReceived),
        "ACTIVITY_CONDITIONAL_RECEIVED must fire: {:?}",
        types
    );
    let received = collector.entity_events_of(EngineEventType::ActivityConditionalReceived);
    assert_eq!(received[0].2.as_deref(), Some(pi.id.as_str()));
}

// ---------------------------------------------------------------------------
// ACTIVITY_ESCALATION_WAITING (boundary)
// ---------------------------------------------------------------------------

#[test]
fn activity_escalation_waiting_fires_on_boundary() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <escalation id="esc1" escalationCode="ESC_CODE" name="Escalation1" />
        <process id="p125-esc-wait" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toTask" sourceRef="start" targetRef="task1" />
            <userTask id="task1" />
            <boundaryEvent id="escBoundary" attachedToRef="task1">
                <escalationEventDefinition escalationRef="esc1" />
            </boundaryEvent>
            <sequenceFlow id="toHandler" sourceRef="escBoundary" targetRef="handler" />
            <userTask id="handler" />
            <sequenceFlow id="toEnd" sourceRef="task1" targetRef="end" />
            <sequenceFlow id="handlerEnd" sourceRef="handler" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p125-esc-wait");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p125-esc.bpmn20.xml".to_string(), xml.to_string()),
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
        types.contains(&EngineEventType::ActivityEscalationWaiting),
        "ACTIVITY_ESCALATION_WAITING must fire: {:?}",
        types
    );
    let evs = collector.entity_events_of(EngineEventType::ActivityEscalationWaiting);
    assert_eq!(evs[0].2.as_deref(), Some(pi.id.as_str()));
    assert!(
        evs[0].1.starts_with("escBoundary:"),
        "entity_id should start with boundary id, got {}",
        evs[0].1
    );
}

// ---------------------------------------------------------------------------
// ACTIVITY_MESSAGE_CANCELLED
// ---------------------------------------------------------------------------

#[test]
fn activity_message_cancelled_fires_when_execution_deleted() {
    // Terminate end destroys the parallel message wait via
    // `delete_execution_and_related_data` (Java ExecutionEntityManagerImpl
    // deleteEventSubScriptions) — bulk delete_process_instance bypasses that
    // path and must not be used as the probe.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="cancelMsg" name="cancelMsg" />
        <process id="p125-msg-cancel" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="f2" sourceRef="fork" targetRef="msgCatch" />
            <sequenceFlow id="f3" sourceRef="fork" targetRef="preTerminate" />
            <intermediateCatchEvent id="msgCatch">
                <messageEventDefinition messageRef="cancelMsg" />
            </intermediateCatchEvent>
            <sequenceFlow id="f4" sourceRef="msgCatch" targetRef="join" />
            <userTask id="preTerminate" />
            <sequenceFlow id="f5" sourceRef="preTerminate" targetRef="terminateEnd" />
            <endEvent id="terminateEnd">
                <terminateEventDefinition />
            </endEvent>
            <parallelGateway id="join" />
            <sequenceFlow id="f6" sourceRef="join" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p125-msg-cancel");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p125-msg-cancel.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    assert!(
        collector
            .types()
            .contains(&EngineEventType::ActivityMessageWaiting),
        "precondition: MESSAGE_WAITING fired"
    );
    collector.clear();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::ActivityMessageCancelled),
        "ACTIVITY_MESSAGE_CANCELLED must fire when terminate deletes the message wait: {:?}",
        types
    );
    let evs = collector.entity_events_of(EngineEventType::ActivityMessageCancelled);
    assert_eq!(evs[0].2.as_deref(), Some(pi.id.as_str()));
    assert_eq!(evs[0].3.as_deref(), Some("cancelMsg"));
}

// ---------------------------------------------------------------------------
// PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT (required e2e)
// ---------------------------------------------------------------------------

#[test]
fn process_completed_with_terminate_end_event_e2e() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p125-terminate" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="f2" sourceRef="fork" targetRef="waitTask" />
            <sequenceFlow id="f3" sourceRef="fork" targetRef="preTerminate" />
            <userTask id="waitTask" />
            <userTask id="preTerminate" />
            <sequenceFlow id="f4" sourceRef="preTerminate" targetRef="terminateEnd" />
            <endEvent id="terminateEnd">
                <terminateEventDefinition />
            </endEvent>
            <sequenceFlow id="f5" sourceRef="waitTask" targetRef="normalEnd" />
            <endEvent id="normalEnd" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p125-terminate");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p125-terminate.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    collector.clear();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    let pre = tasks
        .iter()
        .find(|t| t.task_definition_key == "preTerminate")
        .expect("preTerminate task");
    task_service.complete_task_by_id(pre.id.clone()).unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::ProcessCompletedWithTerminateEndEvent),
        "PROCESS_COMPLETED_WITH_TERMINATE_END_EVENT must fire: {:?}",
        types
    );
    // Java terminate path fires the terminate variant instead of plain
    // PROCESS_COMPLETED (ExecutionEntityManagerImpl only fires COMPLETED when
    // !cancel; terminate uses createTerminateEvent).
    assert!(
        !types.contains(&EngineEventType::ProcessCompleted),
        "plain PROCESS_COMPLETED must not fire for terminate end: {:?}",
        types
    );
    let evs = collector.entity_events_of(EngineEventType::ProcessCompletedWithTerminateEndEvent);
    assert_eq!(evs[0].1, pi.id);
    assert_eq!(evs[0].2.as_deref(), Some(pi.id.as_str()));
}

// ---------------------------------------------------------------------------
// JOB_RESCHEDULED
// ---------------------------------------------------------------------------

#[test]
fn job_rescheduled_fires_on_management_reschedule() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p125-job-resched" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toCatch" sourceRef="start" targetRef="timerCatch" />
            <intermediateCatchEvent id="timerCatch">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="toEnd" sourceRef="timerCatch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p125-job-resched");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p125-job-resched.bpmn20.xml".to_string(), xml.to_string()),
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

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let jobs = store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(jobs.len(), 1);
    let job_id = jobs[0].timer_job_id.clone();
    session.rollback().unwrap();

    collector.clear();

    let management = engine.get_management_service();
    management
        .reschedule_timer_job(
            &job_id,
            None,
            Some("PT2H".to_string()),
            None,
            None,
            None,
        )
        .unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::JobRescheduled),
        "JOB_RESCHEDULED must fire: {:?}",
        types
    );
    // Java TimerUtil.rescheduleTimerJob: JOB_RESCHEDULED then TIMER_SCHEDULED.
    assert!(
        types.contains(&EngineEventType::TimerScheduled),
        "TIMER_SCHEDULED must follow reschedule: {:?}",
        types
    );
    let resched_ids = collector.job_events_of(EngineEventType::JobRescheduled);
    assert_eq!(resched_ids, vec![job_id.clone()]);
    // Order: JOB_RESCHEDULED before TIMER_SCHEDULED.
    let resched_pos = types
        .iter()
        .position(|t| *t == EngineEventType::JobRescheduled)
        .unwrap();
    let scheduled_pos = types
        .iter()
        .position(|t| *t == EngineEventType::TimerScheduled)
        .unwrap();
    assert!(
        resched_pos < scheduled_pos,
        "JOB_RESCHEDULED must precede TIMER_SCHEDULED: {:?}",
        types
    );
}

// ---------------------------------------------------------------------------
// Boundary MESSAGE_WAITING (extra coverage for boundary insert path)
// ---------------------------------------------------------------------------

#[test]
fn activity_message_waiting_fires_on_boundary() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="boundMsg" name="boundMsg" />
        <process id="p125-bound-msg" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toTask" sourceRef="start" targetRef="task1" />
            <userTask id="task1" />
            <boundaryEvent id="msgBoundary" attachedToRef="task1" cancelActivity="true">
                <messageEventDefinition messageRef="boundMsg" />
            </boundaryEvent>
            <sequenceFlow id="toHandler" sourceRef="msgBoundary" targetRef="handler" />
            <userTask id="handler" />
            <sequenceFlow id="toEnd" sourceRef="task1" targetRef="end" />
            <sequenceFlow id="handlerEnd" sourceRef="handler" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p125-bound-msg");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p125-bound-msg.bpmn20.xml".to_string(), xml.to_string()),
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
        types.contains(&EngineEventType::ActivityMessageWaiting),
        "boundary ACTIVITY_MESSAGE_WAITING must fire: {:?}",
        types
    );
    let evs = collector.entity_events_of(EngineEventType::ActivityMessageWaiting);
    assert_eq!(evs[0].2.as_deref(), Some(pi.id.as_str()));
    assert!(evs[0].1.starts_with("msgBoundary:"));
    assert_eq!(evs[0].3.as_deref(), Some("boundMsg"));
}

// ---------------------------------------------------------------------------
// P134/P125: event-subprocess registration → ACTIVITY_*_WAITING
// Java: ProcessInstanceHelper.java:343-358
// ---------------------------------------------------------------------------

#[test]
fn activity_message_waiting_fires_on_event_subprocess_register() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="espMsg" name="espMsg" />
        <process id="p134-esp-msg-wait" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toTask" sourceRef="start" targetRef="mainTask" />
            <userTask id="mainTask" />
            <sequenceFlow id="toEnd" sourceRef="mainTask" targetRef="end" />
            <endEvent id="end" />
            <subProcess id="esp" triggeredByEvent="true">
                <startEvent id="espMsgStart" isInterrupting="false">
                    <messageEventDefinition messageRef="espMsg" />
                </startEvent>
                <sequenceFlow id="espF1" sourceRef="espMsgStart" targetRef="espTask" />
                <userTask id="espTask" />
                <sequenceFlow id="espF2" sourceRef="espTask" targetRef="espEnd" />
                <endEvent id="espEnd" />
            </subProcess>
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p134-esp-msg-wait");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p134-esp-msg.bpmn20.xml".to_string(), xml.to_string()),
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
        types.contains(&EngineEventType::ActivityMessageWaiting),
        "event-subprocess register must fire ACTIVITY_MESSAGE_WAITING: {:?}",
        types
    );
    let evs = collector.entity_events_of(EngineEventType::ActivityMessageWaiting);
    assert_eq!(evs[0].2.as_deref(), Some(pi.id.as_str()));
    assert!(
        evs[0].1.starts_with("espMsgStart:"),
        "entity_id should be startEventId:messageRef, got {}",
        evs[0].1
    );
    assert_eq!(evs[0].3.as_deref(), Some("espMsg"));
}

#[test]
fn activity_signal_waiting_fires_on_event_subprocess_register() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="espSig" name="espSig" />
        <process id="p134-esp-sig-wait" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toTask" sourceRef="start" targetRef="mainTask" />
            <userTask id="mainTask" />
            <sequenceFlow id="toEnd" sourceRef="mainTask" targetRef="end" />
            <endEvent id="end" />
            <subProcess id="esp" triggeredByEvent="true">
                <startEvent id="espSigStart" isInterrupting="false">
                    <signalEventDefinition signalRef="espSig" />
                </startEvent>
                <sequenceFlow id="espF1" sourceRef="espSigStart" targetRef="espTask" />
                <userTask id="espTask" />
                <sequenceFlow id="espF2" sourceRef="espTask" targetRef="espEnd" />
                <endEvent id="espEnd" />
            </subProcess>
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p134-esp-sig-wait");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p134-esp-sig.bpmn20.xml".to_string(), xml.to_string()),
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
        types.contains(&EngineEventType::ActivitySignalWaiting),
        "event-subprocess register must fire ACTIVITY_SIGNAL_WAITING: {:?}",
        types
    );
    let evs = collector.entity_events_of(EngineEventType::ActivitySignalWaiting);
    assert_eq!(evs[0].2.as_deref(), Some(pi.id.as_str()));
    assert!(
        evs[0].1.starts_with("espSigStart:"),
        "entity_id should be startEventId:signalRef, got {}",
        evs[0].1
    );
    assert_eq!(evs[0].3.as_deref(), Some("espSig"));
}

// ---------------------------------------------------------------------------
// P134/P125: bulk delete → ACTIVITY_MESSAGE_CANCELLED before row delete
// Java: ExecutionEntityManagerImpl.java:1050-1077
// ---------------------------------------------------------------------------

#[test]
fn activity_message_cancelled_fires_on_bulk_delete_with_message_wait() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="bulkMsg" name="bulkMsg" />
        <process id="p134-bulk-msg-cancel" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="toCatch" sourceRef="start" targetRef="msgCatch" />
            <intermediateCatchEvent id="msgCatch">
                <messageEventDefinition messageRef="bulkMsg" />
            </intermediateCatchEvent>
            <sequenceFlow id="toEnd" sourceRef="msgCatch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (engine, collector) = engine_with_collector("p134-bulk-msg-cancel");
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("p134-bulk-msg.bpmn20.xml".to_string(), xml.to_string()),
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

    // Drop WAITING noise from subscription create.
    collector.clear();

    runtime
        .bulk_delete_process_instances(vec![pi.id.clone()], Some("p134-bulk".to_string()))
        .unwrap();

    let types = collector.types();
    assert!(
        types.contains(&EngineEventType::ActivityMessageCancelled),
        "bulk delete must fire ACTIVITY_MESSAGE_CANCELLED for message wait: {:?}",
        types
    );
    let evs = collector.entity_events_of(EngineEventType::ActivityMessageCancelled);
    assert_eq!(evs[0].2.as_deref(), Some(pi.id.as_str()));
    assert_eq!(evs[0].3.as_deref(), Some("bulkMsg"));

    // Instance is gone after delete — cancel was dispatched while wait still existed.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store
            .find_process_instance(&pi.id, &mut session)
            .is_none(),
        "process instance must be deleted"
    );
    assert!(
        store
            .find_event_wait_states_by_process_instance_id(&pi.id, &mut session)
            .is_empty(),
        "message wait rows must be deleted after MESSAGE_CANCELLED dispatch"
    );
}
