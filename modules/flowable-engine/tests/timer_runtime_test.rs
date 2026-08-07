use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_timer_intermediate_catch_event() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="timerIntermediateCatchProcess" name="Timer Intermediate Catch Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="intermediateCatchEvent1" />
            <intermediateCatchEvent id="intermediateCatchEvent1" name="Timer Catch">
                <timerEventDefinition>
                    <timeDuration>PT5M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="intermediateCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("Timer Intermediate Catch Deployment".to_string())
        .add_string("timer_process.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id);
    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    // Assert that a timer job state has been created
    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    let timer_execution = executions
        .values()
        .find(|e| e.activity_id.as_deref() == Some("intermediateCatchEvent1"))
        .expect("Execution for timer catch event should exist");

    assert!(!timer_execution.is_active);

    let timer_states = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(timer_states.len(), 1);

    let timer_state = &timer_states[0];
    assert_eq!(timer_state.activity_id, "intermediateCatchEvent1");
    assert!(!timer_state.is_boundary);
    assert_eq!(timer_state.time_duration.as_deref(), Some("PT5M"));
    assert_eq!(timer_state.category, None);

    session.rollback().unwrap();
    drop(session);

    // Trigger the timer deterministically
    process_engine.trigger_timer_intermediate_catch_event(timer_execution.id.clone());

    let mut session_after = runtime_store.create_session().unwrap();
    let process_instance_ended = runtime_store
        .find_process_instance(&process_instance.id, &mut session_after)
        .expect("Process instance should exist");
    assert!(process_instance_ended.is_ended);

    let timer_states_after = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session_after);
    assert!(timer_states_after.is_empty());
}

#[test]
fn intermediate_timer_job_category_is_populated() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="timerJobCategoryProcess" name="Timer Job Category Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="intermediateCatchEvent1" />
            <intermediateCatchEvent id="intermediateCatchEvent1" name="Timer Catch">
                <extensionElements>
                    <flowable:jobCategory>${categoryValue}</flowable:jobCategory>
                </extensionElements>
                <timerEventDefinition>
                    <timeDuration>PT5M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="intermediateCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let process_engine = ProcessEngine::new("timer-job-category".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Timer Job Category Deployment".to_string())
                .add_string("timer_job_category.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable(
                    "categoryValue".to_string(),
                    serde_json::json!("timer-orders"),
                ),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let timer_states = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(timer_states.len(), 1);
    assert_eq!(timer_states[0].category.as_deref(), Some("timer-orders"));
    assert_eq!(timer_states[0].activity_id, "intermediateCatchEvent1");
}

#[test]
fn test_timer_boundary_event_on_user_task() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="timerBoundaryProcess" name="Timer Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            
            <userTask id="userTask1" name="User Task" />
            <boundaryEvent id="timerBoundary1" attachedToRef="userTask1" cancelActivity="true">
                <timerEventDefinition>
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
            
            <sequenceFlow id="flow3" sourceRef="timerBoundary1" targetRef="endEvent2" />
            <endEvent id="endEvent2" />
        </process>
    </definitions>"#;

    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let builder = repository_service
        .create_deployment()
        .name("Timer Boundary Deployment".to_string())
        .add_string("timer_boundary.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id);
    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();

    // Check timer boundary registration
    let mut session = runtime_store.create_session().unwrap();
    let timer_states = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(timer_states.len(), 1);

    let timer_state = &timer_states[0];
    assert_eq!(timer_state.activity_id, "timerBoundary1");
    assert!(timer_state.is_boundary);
    assert!(timer_state.cancel_activity);
    assert_eq!(
        timer_state.attached_activity_id.as_deref(),
        Some("userTask1")
    );

    session.rollback().unwrap();
    drop(session);

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    // Trigger boundary event
    process_engine
        .trigger_timer_boundary_event("timerBoundary1".to_string(), process_instance.id.clone());

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after.is_empty(),
        "Interrupting boundary should cancel task"
    );

    let mut session_after = runtime_store.create_session().unwrap();
    let process_instance_ended = runtime_store
        .find_process_instance(&process_instance.id, &mut session_after)
        .expect("Process instance should exist");
    assert!(process_instance_ended.is_ended);
}

#[test]
fn test_mixed_boundary_events() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="mixedBoundaryProcess" name="Mixed Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            
            <receiveTask id="receiveTask1" name="Receive Task" />
            
            <boundaryEvent id="timerBoundary1" attachedToRef="receiveTask1" cancelActivity="false">
                <timerEventDefinition>
                    <timeDuration>PT5M</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            
            <boundaryEvent id="messageBoundary1" attachedToRef="receiveTask1" cancelActivity="true">
                <messageEventDefinition messageRef="myMessage" />
            </boundaryEvent>
            
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
            
            <sequenceFlow id="flow3" sourceRef="timerBoundary1" targetRef="task2" />
            <userTask id="task2" name="Task 2" />
            <sequenceFlow id="flow4" sourceRef="task2" targetRef="endEvent2" />
            <endEvent id="endEvent2" />
            
            <sequenceFlow id="flow5" sourceRef="messageBoundary1" targetRef="endEvent3" />
            <endEvent id="endEvent3" />
        </process>
    </definitions>"#;

    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("Mixed Boundary Deployment".to_string())
        .add_string("mixed.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id);
    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();

    let mut session = runtime_store.create_session().unwrap();
    let timer_states = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(timer_states.len(), 1);
    assert_eq!(timer_states[0].activity_id, "timerBoundary1");
    assert!(!timer_states[0].cancel_activity);

    let message_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(message_states.len(), 1);
    assert_eq!(message_states[0].boundary_event_id, "messageBoundary1");

    session.rollback().unwrap();
    drop(session);

    // Trigger the non-interrupting timer
    process_engine
        .trigger_timer_boundary_event("timerBoundary1".to_string(), process_instance.id.clone());

    let mut session_after = runtime_store.create_session().unwrap();
    let timer_states_after = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session_after);
    assert!(timer_states_after.is_empty(), "Timer should be consumed");

    // The message boundary event should still be active because the timer was non-interrupting
    let message_states_after = runtime_store.find_boundary_event_states_by_process_instance_id(
        &process_instance.id,
        &mut session_after,
    );
    assert_eq!(message_states_after.len(), 1);

    session_after.rollback().unwrap();
    drop(session_after);

    // Trigger the interrupting message event
    process_engine.trigger_boundary_event_by_message_ref(
        "myMessage".to_string(),
        process_instance.id.clone(),
    );

    let mut session_final = runtime_store.create_session().unwrap();
    let message_states_final = runtime_store.find_boundary_event_states_by_process_instance_id(
        &process_instance.id,
        &mut session_final,
    );
    assert!(
        message_states_final.is_empty(),
        "Message boundary should be consumed"
    );
}
