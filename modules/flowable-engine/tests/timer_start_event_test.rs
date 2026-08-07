use chrono::Utc;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::time_source::TestTimeSource;
use std::sync::Arc;

const TIMER_START_EVENT_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://flowable.org/test">
  <process id="timerStartProcess" name="Timer Start Process" isExecutable="true">
    <startEvent id="timerStartEvent" name="Timer Start">
      <timerEventDefinition>
        <timeDuration>PT10S</timeDuration>
      </timerEventDefinition>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="timerStartEvent" targetRef="task1"/>
    <userTask id="task1" name="User Task"/>
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="endEvent"/>
    <endEvent id="endEvent" name="End"/>
  </process>
</definitions>"#;

const TIMER_START_WITH_CYCLE_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://flowable.org/test">
  <process id="timerCycleProcess" name="Timer Cycle Process" isExecutable="true">
    <startEvent id="timerStartEvent" name="Timer Start">
      <timerEventDefinition>
        <timeCycle>R/PT5S</timeCycle>
      </timerEventDefinition>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="timerStartEvent" targetRef="task1"/>
    <userTask id="task1" name="User Task"/>
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="endEvent"/>
    <endEvent id="endEvent" name="End"/>
  </process>
</definitions>"#;

const MULTIPLE_START_EVENTS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://flowable.org/test">
  <process id="multipleStartProcess" name="Multiple Start Process" isExecutable="true">
    <startEvent id="normalStart" name="Normal Start"/>
    <startEvent id="timerStart" name="Timer Start">
      <timerEventDefinition>
        <timeDuration>PT10S</timeDuration>
      </timerEventDefinition>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="normalStart" targetRef="task1"/>
    <sequenceFlow id="flow2" sourceRef="timerStart" targetRef="task1"/>
    <userTask id="task1" name="User Task"/>
    <sequenceFlow id="flow3" sourceRef="task1" targetRef="endEvent"/>
    <endEvent id="endEvent" name="End"/>
  </process>
</definitions>"#;

const MULTIPLE_TIMER_START_EVENTS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://flowable.org/test">
  <process id="dualTimerStartProcess" name="Dual Timer Start Process" isExecutable="true">
    <startEvent id="timerStartA" name="Timer Start A">
      <timerEventDefinition>
        <timeDuration>PT10S</timeDuration>
      </timerEventDefinition>
    </startEvent>
    <startEvent id="timerStartB" name="Timer Start B">
      <timerEventDefinition>
        <timeDuration>PT10S</timeDuration>
      </timerEventDefinition>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="timerStartA" targetRef="task1"/>
    <sequenceFlow id="flow2" sourceRef="timerStartB" targetRef="task1"/>
    <userTask id="task1" name="User Task"/>
    <sequenceFlow id="flow3" sourceRef="task1" targetRef="endEvent"/>
    <endEvent id="endEvent" name="End"/>
  </process>
</definitions>"#;

#[test]
fn timer_start_subscription_category_is_populated_from_start_event() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://flowable.org/test">
  <process id="timerStartCategoryProcess" name="Timer Start Category" isExecutable="true">
    <startEvent id="timerStartEvent" name="Timer Start">
      <extensionElements>
        <flowable:jobCategory>start-orders</flowable:jobCategory>
      </extensionElements>
      <timerEventDefinition>
        <timeDuration>PT10S</timeDuration>
      </timerEventDefinition>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="timerStartEvent" targetRef="task1"/>
    <userTask id="task1" name="User Task"/>
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="endEvent"/>
    <endEvent id="endEvent" name="End"/>
  </process>
</definitions>"#;

    let test_time = Arc::new(TestTimeSource::new(Utc::now()));
    let engine = ProcessEngine::with_time_source("timer-start-category".to_string(), test_time);
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("timer_start_category".to_string())
                .add_string(
                    "timer_start_category.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let subs = engine.get_timer_start_subscriptions();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].start_event_id, "timerStartEvent");
    assert_eq!(subs[0].category.as_deref(), Some("start-orders"));

    // Management timer-job mapping exposes the process timer-start category.
    let timer_jobs = engine
        .get_management_service()
        .create_timer_job_query()
        .list()
        .unwrap();
    let mapped = timer_jobs
        .iter()
        .find(|job| job.activity_id == "timerStartEvent")
        .expect("timer start should appear in management timer job query");
    assert_eq!(mapped.category.as_deref(), Some("start-orders"));
}

#[test]
fn test_process_timer_start_event_creates_new_instance() {
    let test_time = Arc::new(TestTimeSource::new(Utc::now()));
    let engine = ProcessEngine::with_time_source("test".to_string(), test_time.clone());

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("test_deployment".to_string())
        .add_string(
            "timer_start.bpmn20.xml".to_string(),
            TIMER_START_EVENT_BPMN.to_string(),
        );
    engine.get_repository_service().deploy(builder).unwrap();

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()
        .pop()
        .expect("Process definition should be deployed");

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let initial_instances = runtime_store.snapshot_process_instances(&mut session).len();
    assert_eq!(initial_instances, 0, "Should have no instances initially");

    session.rollback().unwrap();
    drop(session);

    test_time.advance_time(15000);

    let triggered = engine.run_due_timers();

    assert!(
        triggered.iter().any(|id| id.contains("timer_start:")),
        "Should have triggered a timer start event"
    );

    let __runtime_store = engine.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let instances = __runtime_store.snapshot_process_instances(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(
        instances.len(),
        1,
        "Should have created one process instance"
    );

    let instance = instances.values().next().unwrap();
    assert_eq!(
        instance.process_definition_id, process_definition_id,
        "Instance should belong to correct process definition"
    );
}

#[test]
fn test_non_due_process_timer_start_does_nothing() {
    let test_time = Arc::new(TestTimeSource::new(Utc::now()));
    let engine = ProcessEngine::with_time_source("test".to_string(), test_time.clone());

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("test_deployment".to_string())
        .add_string(
            "timer_start.bpmn20.xml".to_string(),
            TIMER_START_EVENT_BPMN.to_string(),
        );
    engine.get_repository_service().deploy(builder).unwrap();

    test_time.advance_time(5000);

    let triggered = engine.run_due_timers();

    assert!(
        !triggered.iter().any(|id| id.contains("timer_start:")),
        "Should NOT have triggered a timer start event (not due yet)"
    );

    let __runtime_store = engine.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let instances = __runtime_store.snapshot_process_instances(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(instances.len(), 0, "Should have no process instances");
}

#[test]
fn test_timer_start_with_cycle_creates_instances() {
    // Java: StartTimerEventRepeatWithoutEndTest — infinite R/PT.. keeps
    // rescheduling after each fire (TimerJobSchedulerImpl).
    let test_time = Arc::new(TestTimeSource::new(Utc::now()));
    let engine = ProcessEngine::with_time_source("test".to_string(), test_time.clone());

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("test_deployment".to_string())
        .add_string(
            "timer_cycle.bpmn20.xml".to_string(),
            TIMER_START_WITH_CYCLE_BPMN.to_string(),
        );
    engine.get_repository_service().deploy(builder).unwrap();

    test_time.advance_time(5000);
    let triggered1 = engine.run_due_timers();
    assert!(
        triggered1.iter().any(|id| id.contains("timer_start:")),
        "First cycle should trigger timer start"
    );
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert_eq!(
        runtime_store.snapshot_process_instances(&mut session).len(),
        1,
        "Should have one instance after first cycle"
    );
    session.rollback().unwrap();
    drop(session);

    // Second and third fires must still produce instances (repeat).
    test_time.advance_time(5000);
    let triggered2 = engine.run_due_timers();
    assert!(
        triggered2.iter().any(|id| id.contains("timer_start:")),
        "Second cycle should trigger timer start"
    );
    test_time.advance_time(5000);
    let triggered3 = engine.run_due_timers();
    assert!(
        triggered3.iter().any(|id| id.contains("timer_start:")),
        "Third cycle should trigger timer start"
    );

    let mut session = runtime_store.create_session().unwrap();
    assert_eq!(
        runtime_store.snapshot_process_instances(&mut session).len(),
        3,
        "Infinite R cycle should create one instance per fire"
    );
    // Subscription remains due (not permanently released).
    let subs = engine.get_timer_start_subscriptions();
    assert_eq!(subs.len(), 1);
    assert!(
        subs[0].due_time.is_some(),
        "timeCycle start subscription must keep a future due_time after fire"
    );
}

#[test]
fn test_timer_start_with_r4_cycle_limit() {
    // Java StartTimerEventTest.testCycleWithLimitStartTimerEvent — R2/PT5M fires twice then stops.
    const BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://flowable.org/test">
  <process id="timerCycleLimitProcess" isExecutable="true">
    <startEvent id="timerStartEvent">
      <timerEventDefinition>
        <timeCycle>R2/PT5S</timeCycle>
      </timerEventDefinition>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="timerStartEvent" targetRef="task1"/>
    <userTask id="task1"/>
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="endEvent"/>
    <endEvent id="endEvent"/>
  </process>
</definitions>"#;

    let test_time = Arc::new(TestTimeSource::new(Utc::now()));
    let engine = ProcessEngine::with_time_source("r2-limit".to_string(), test_time.clone());
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("r2".to_string())
                .add_string("r2.bpmn20.xml".to_string(), BPMN.to_string()),
        )
        .unwrap();

    test_time.advance_time(5000);
    assert!(
        engine
            .run_due_timers()
            .iter()
            .any(|id| id.contains("timer_start:"))
    );
    test_time.advance_time(5000);
    assert!(
        engine
            .run_due_timers()
            .iter()
            .any(|id| id.contains("timer_start:"))
    );
    // Exhausted — third fire must not start another instance.
    test_time.advance_time(5000);
    let third = engine.run_due_timers();
    assert!(
        !third.iter().any(|id| id.contains("timer_start:")),
        "R2 cycle must stop after two fires"
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert_eq!(
        runtime_store.snapshot_process_instances(&mut session).len(),
        2
    );
    let subs = engine.get_timer_start_subscriptions();
    assert!(
        subs.iter().all(|s| s.due_time.is_none()),
        "exhausted cycle must clear due_time"
    );
}

#[test]
fn test_multiple_start_events_coexistence() {
    let test_time = Arc::new(TestTimeSource::new(Utc::now()));
    let engine = ProcessEngine::with_time_source("test".to_string(), test_time.clone());

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("test_deployment".to_string())
        .add_string(
            "multiple_start.bpmn20.xml".to_string(),
            MULTIPLE_START_EVENTS_BPMN.to_string(),
        );
    engine.get_repository_service().deploy(builder).unwrap();

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()
        .pop()
        .unwrap();

    let instance1 = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id.clone(), None);

    assert!(
        !instance1.unwrap().id.is_empty(),
        "Normal start event should create instance immediately"
    );

    test_time.advance_time(15000);
    let triggered = engine.run_due_timers();

    assert!(
        triggered.iter().any(|id| id.contains("timer_start:")),
        "Timer start event should also trigger"
    );

    let __runtime_store = engine.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let instances = __runtime_store.snapshot_process_instances(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(
        instances.len(),
        2,
        "Should have two instances: one from normal start, one from timer"
    );
}

#[test]
fn test_timer_start_event_subscription_model() {
    let test_time = Arc::new(TestTimeSource::new(Utc::now()));
    let engine = ProcessEngine::with_time_source("test".to_string(), test_time.clone());

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("test_deployment".to_string())
        .add_string(
            "timer_start.bpmn20.xml".to_string(),
            TIMER_START_EVENT_BPMN.to_string(),
        );
    engine.get_repository_service().deploy(builder).unwrap();

    let timer_subs = engine.get_timer_start_subscriptions();

    assert_eq!(
        timer_subs.len(),
        1,
        "Should have one timer start subscription"
    );

    let sub = &timer_subs[0];
    assert!(
        !sub.id.is_empty(),
        "Timer start subscription should have a stable row id"
    );
    assert_eq!(sub.process_definition_key, "timerStartProcess");
    assert_eq!(sub.start_event_id, "timerStartEvent");
    assert_eq!(sub.time_duration.as_deref(), Some("PT10S"));
    assert!(sub.time_date.is_none());
    assert!(sub.time_cycle.is_none());
    assert!(sub.due_time.is_some());
}

#[test]
fn test_multiple_timer_start_events_are_acquired_independently() {
    let test_time = Arc::new(TestTimeSource::new(Utc::now()));
    let engine = ProcessEngine::with_time_source("test".to_string(), test_time.clone());

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("dual_timer_start_deployment".to_string())
        .add_string(
            "dual_timer_start.bpmn20.xml".to_string(),
            MULTIPLE_TIMER_START_EVENTS_BPMN.to_string(),
        );
    engine.get_repository_service().deploy(builder).unwrap();

    let timer_subs = engine.get_timer_start_subscriptions();
    assert_eq!(
        timer_subs.len(),
        2,
        "Should have two timer start subscriptions"
    );
    assert_ne!(
        timer_subs[0].id, timer_subs[1].id,
        "Each timer start subscription needs a stable row id"
    );

    test_time.advance_time(15000);
    let triggered = engine.run_due_timers();
    assert!(
        triggered
            .iter()
            .filter(|id| id.contains("timer_start:"))
            .count()
            >= 2,
        "Both timer start events should be triggered independently"
    );

    let __runtime_store = engine.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let instances = __runtime_store.snapshot_process_instances(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(
        instances.len(),
        2,
        "Both timer start events should create separate instances"
    );
}

#[test]
fn test_timer_start_event_deletion_on_undeploy() {
    let test_time = Arc::new(TestTimeSource::new(Utc::now()));
    let engine = ProcessEngine::with_time_source("test".to_string(), test_time.clone());

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("test_deployment".to_string())
        .add_string(
            "timer_start.bpmn20.xml".to_string(),
            TIMER_START_EVENT_BPMN.to_string(),
        );
    let deployment = engine.get_repository_service().deploy(builder).unwrap();

    let initial_subs = engine.get_timer_start_subscriptions();
    assert_eq!(initial_subs.len(), 1);

    engine
        .get_repository_service()
        .delete_deployment(&deployment.id)
        .unwrap();

    let remaining_subs = engine.get_timer_start_subscriptions();
    assert_eq!(
        remaining_subs.len(),
        0,
        "Timer subscriptions should be deleted on undeploy"
    );
}
