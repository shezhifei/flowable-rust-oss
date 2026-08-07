use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use std::sync::Arc;

fn create_engine() -> (ProcessEngine, Arc<TestTimeSource>) {
    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let engine = ProcessEngine::with_time_source("test_engine".to_string(), time_source.clone());
    (engine, time_source)
}

#[test]
fn test_timer_scheduling_duration() {
    let (engine, time_source) = create_engine();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="durationProcess" name="Duration Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="catch1" />
            <intermediateCatchEvent id="catch1">
                <timerEventDefinition>
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="catch1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("deployment".to_string())
        .add_string("process.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(pd_id);
    let pi = runtime_service.start_process_instance(pi_builder).unwrap();

    // Initial state: not due yet
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 0);

    // Advance time by 5 minutes, still not due
    time_source.advance_time(5 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 0);

    // Advance time by another 5 minutes, should be due
    time_source.advance_time(5 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1);

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi_ended = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .unwrap();
    assert!(pi_ended.is_ended);
}

#[test]
fn test_timer_scheduling_date() {
    let (engine, time_source) = create_engine();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="dateProcess" name="Date Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="catch1" />
            <intermediateCatchEvent id="catch1">
                <timerEventDefinition>
                    <timeDate>2026-04-18T13:00:00Z</timeDate>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="catch1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("deployment".to_string())
        .add_string("process.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    // Current time is 12:00:00, target is 13:00:00
    assert_eq!(engine.run_due_timers().len(), 0);

    // Advance to 12:30:00
    time_source.advance_time(30 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 0);

    // Advance to 13:00:00
    time_source.advance_time(30 * 60 * 1000);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1);

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_process_instance(&pi.id, &mut session)
            .unwrap()
            .is_ended
    );
}

#[test]
fn test_timer_scheduling_cycle() {
    let (engine, time_source) = create_engine();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="cycleProcess" name="Cycle Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="catch1" />
            <intermediateCatchEvent id="catch1">
                <timerEventDefinition>
                    <timeCycle>R3/PT1H</timeCycle>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="catch1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("deployment".to_string())
        .add_string("process.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    assert_eq!(engine.run_due_timers().len(), 0);

    time_source.advance_time(60 * 60 * 1000); // 1 hour
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1); // First fire

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_process_instance(&pi.id, &mut session)
            .unwrap()
            .is_ended
    );
}

#[test]
fn test_non_interrupting_boundary_timer_cycle_repeats_while_host_waits() {
    let (engine, time_source) = create_engine();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="boundaryCycleProcess" name="Boundary Cycle Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />

            <userTask id="userTask1" name="Review Request" />
            <boundaryEvent id="timerBoundary1" attachedToRef="userTask1" cancelActivity="false">
                <timerEventDefinition>
                    <timeCycle>R2/PT1H</timeCycle>
                </timerEventDefinition>
            </boundaryEvent>

            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEndEvent" />
            <endEvent id="normalEndEvent" />

            <sequenceFlow id="flow3" sourceRef="timerBoundary1" targetRef="escalationTask" />
            <userTask id="escalationTask" name="Escalate" />
            <sequenceFlow id="flow4" sourceRef="escalationTask" targetRef="escalationEndEvent" />
            <endEvent id="escalationEndEvent" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("deployment".to_string())
                .add_string("process.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    time_source.advance_time(60 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);

    let tasks_after_first_fire = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_first_fire
            .iter()
            .filter(|task| task.task_definition_key == "userTask1")
            .count(),
        1,
        "Non-interrupting boundary timer should preserve host task"
    );
    assert_eq!(
        tasks_after_first_fire
            .iter()
            .filter(|task| task.task_definition_key == "escalationTask")
            .count(),
        1,
        "First timer fire should create one escalation task"
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let timer_states_after_first_fire =
        runtime_store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(
        timer_states_after_first_fire.len(),
        1,
        "R2 cycle should leave one future timer job after first fire"
    );
    // After prepareRepeat the stored cycle is R2/{anchor}/PT1H; after first fire R1/{anchor}/PT1H.
    let cycle_after = timer_states_after_first_fire[0]
        .time_cycle
        .as_deref()
        .expect("rescheduled cycle");
    assert!(
        cycle_after.starts_with("R1/") && cycle_after.ends_with("/PT1H"),
        "expected R1/{{anchor}}/PT1H, got {cycle_after}"
    );

    session.rollback().unwrap();
    drop(session);

    time_source.advance_time(60 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);

    let tasks_after_second_fire = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_second_fire
            .iter()
            .filter(|task| task.task_definition_key == "userTask1")
            .count(),
        1,
        "Host task should still exist after repeated non-interrupting timer"
    );
    assert_eq!(
        tasks_after_second_fire
            .iter()
            .filter(|task| task.task_definition_key == "escalationTask")
            .count(),
        2,
        "Second timer fire should create a second escalation task"
    );

    let mut session_after = runtime_store.create_session().unwrap();
    let timer_states_after_second_fire =
        runtime_store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session_after);
    assert!(
        timer_states_after_second_fire.is_empty(),
        "R2 cycle should be consumed after the second fire"
    );
}

#[test]
fn test_timer_boundary_job_is_removed_when_host_user_task_completes() {
    let (engine, time_source) = create_engine();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="boundaryCleanupProcess" name="Boundary Cleanup Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />

            <userTask id="userTask1" name="Review Request" />
            <boundaryEvent id="timerBoundary1" attachedToRef="userTask1" cancelActivity="true">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>

            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEndEvent" />
            <endEvent id="normalEndEvent" />

            <sequenceFlow id="flow3" sourceRef="timerBoundary1" targetRef="timeoutTask" />
            <userTask id="timeoutTask" name="Timeout Follow Up" />
            <sequenceFlow id="flow4" sourceRef="timeoutTask" targetRef="timeoutEndEvent" />
            <endEvent id="timeoutEndEvent" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("deployment".to_string())
                .add_string("process.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let timer_states =
        runtime_store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(timer_states.len(), 1);
    assert_eq!(timer_states[0].activity_id, "timerBoundary1");

    session.rollback().unwrap();
    drop(session);

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "userTask1");

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let timer_states_after_completion =
        runtime_store.find_timer_job_states_by_process_instance_id(&pi.id, &mut session);
    assert!(
        timer_states_after_completion.is_empty(),
        "Completing the host activity should remove its boundary timer job"
    );

    session.rollback().unwrap();
    drop(session);

    time_source.advance_time(60 * 60 * 1000);
    assert_eq!(
        engine.run_due_timers().len(),
        0,
        "Expired boundary timer must not fire after its host activity completed"
    );

    let tasks_after_due_time = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert!(
        tasks_after_due_time
            .iter()
            .all(|task| task.task_definition_key != "timeoutTask"),
        "Boundary timeout path should not start after the host completed normally"
    );
}

#[test]
fn test_multiple_due_timers_in_one_sweep() {
    let (engine, time_source) = create_engine();
    let xml1 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p1" name="P1">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="catch1" />
            <intermediateCatchEvent id="catch1">
                <timerEventDefinition>
                    <timeDuration>PT5M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="catch1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let xml2 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p2" name="P2">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="catch1" />
            <intermediateCatchEvent id="catch1">
                <timerEventDefinition>
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="catch1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("d1".to_string())
                .add_string("p1.bpmn20.xml".to_string(), xml1.to_string()),
        )
        .unwrap();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("d2".to_string())
                .add_string("p2.bpmn20.xml".to_string(), xml2.to_string()),
        )
        .unwrap();

    let pds = repository_service.get_process_definition_ids().unwrap();
    runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pds[0].clone()),
        )
        .unwrap();
    runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pds[1].clone()),
        )
        .unwrap();

    time_source.advance_time(15 * 60 * 1000); // Advance 15 mins, both are due

    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 2);
}

#[test]
fn test_mixed_timer_message_signal_coexistence() {
    let (engine, time_source) = create_engine();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="mixedProcess" name="Mixed Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            
            <receiveTask id="receiveTask1" name="Receive Task" />
            
            <boundaryEvent id="timerBoundary1" attachedToRef="receiveTask1" cancelActivity="true">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            
            <boundaryEvent id="messageBoundary1" attachedToRef="receiveTask1" cancelActivity="true">
                <messageEventDefinition messageRef="myMessage" />
            </boundaryEvent>
            
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
            
            <sequenceFlow id="flow3" sourceRef="timerBoundary1" targetRef="endEvent2" />
            <endEvent id="endEvent2" />
            
            <sequenceFlow id="flow4" sourceRef="messageBoundary1" targetRef="endEvent3" />
            <endEvent id="endEvent3" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("deployment".to_string())
        .add_string("process.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // Scenario 1: Timer triggers first
    let pi1 = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id.clone()),
        )
        .unwrap();
    time_source.advance_time(60 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 1);

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_process_instance(&pi1.id, &mut session)
            .unwrap()
            .is_ended
    );

    session.rollback().unwrap();
    drop(session);

    // Scenario 2: Message triggers first
    let pi2 = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id.clone()),
        )
        .unwrap();
    engine.trigger_boundary_event_by_message_ref("myMessage".to_string(), pi2.id.clone());

    // Timer should be deleted because message triggered and interrupted
    time_source.advance_time(60 * 60 * 1000);
    assert_eq!(engine.run_due_timers().len(), 0);

    let mut session2 = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_process_instance(&pi2.id, &mut session2)
            .unwrap()
            .is_ended
    );
}
