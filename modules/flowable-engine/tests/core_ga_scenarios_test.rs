use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_core_ga_linear_process() {
    let engine = ProcessEngine::new("default".to_string());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="linearProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Task 1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_builder = engine
        .get_repository_service()
        .create_deployment()
        .add_string("linear.bpmn".to_string(), xml.to_string());
    engine
        .get_repository_service()
        .deploy(deploy_builder)
        .unwrap();

    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .business_key("my-business-key".to_string())
        .variable(
            "myVar".to_string(),
            serde_json::Value::String("myValue".to_string()),
        );

    let pi = engine
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap();

    assert_eq!(pi.business_key.as_deref(), Some("my-business-key"));
    // Single storage: start variables live on the process-instance scope
    // execution row and are read back through the runtime service.
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(pi.id.clone(), "myVar".to_string())
            .unwrap(),
        Some(serde_json::Value::String("myValue".to_string()))
    );

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "task1");

    engine
        .get_task_service()
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored_pi = store.find_process_instance(&pi.id, &mut session).unwrap();
    assert!(stored_pi.is_ended);
}

#[test]
fn test_core_ga_branching_process() {
    let engine = ProcessEngine::new("default".to_string());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="branchingProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="excGateway" />
            <exclusiveGateway id="excGateway" default="fDefault" />
            <sequenceFlow id="f2" sourceRef="excGateway" targetRef="parGateway">
                <conditionExpression><![CDATA[${goParallel == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="fDefault" sourceRef="excGateway" targetRef="defaultEnd" />
            <endEvent id="defaultEnd" />
            
            <parallelGateway id="parGateway" />
            <sequenceFlow id="f3" sourceRef="parGateway" targetRef="task1" />
            <sequenceFlow id="f4" sourceRef="parGateway" targetRef="task2" />
            
            <userTask id="task1" />
            <userTask id="task2" />
            
            <sequenceFlow id="f5" sourceRef="task1" targetRef="parGateway2" />
            <sequenceFlow id="f6" sourceRef="task2" targetRef="parGateway2" />
            
            <parallelGateway id="parGateway2" />
            <sequenceFlow id="f7" sourceRef="parGateway2" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_builder = engine
        .get_repository_service()
        .create_deployment()
        .add_string("branching.bpmn".to_string(), xml.to_string());
    engine
        .get_repository_service()
        .deploy(deploy_builder)
        .unwrap();

    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .variable("goParallel".to_string(), serde_json::Value::Bool(true));

    let pi = engine
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap();

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);

    engine
        .get_task_service()
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
    engine
        .get_task_service()
        .complete_task_by_id(tasks[1].id.clone())
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored_pi = store.find_process_instance(&pi.id, &mut session).unwrap();
    assert!(stored_pi.is_ended);
}

#[test]
fn test_core_ga_user_task_boundary_event() {
    let time_source = std::sync::Arc::new(
        flowable_engine::engine::time_source::TestTimeSource::new(chrono::Utc::now()),
    );
    let engine = ProcessEngine::with_time_source("default".to_string(), time_source.clone());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="boundaryProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            
            <userTask id="task1" />
            <boundaryEvent id="timerBoundary" attachedToRef="task1" cancelActivity="true">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end1" />
            <endEvent id="end1" />
            
            <sequenceFlow id="f3" sourceRef="timerBoundary" targetRef="task2" />
            <userTask id="task2" />
            <sequenceFlow id="f4" sourceRef="task2" targetRef="end2" />
            <endEvent id="end2" />
        </process>
    </definitions>"#;

    let deploy_builder = engine
        .get_repository_service()
        .create_deployment()
        .add_string("boundary.bpmn".to_string(), xml.to_string());
    engine
        .get_repository_service()
        .deploy(deploy_builder)
        .unwrap();

    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(def_id);

    let pi = engine
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap();

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "task1");

    // Fire timer
    time_source.advance_time(2 * 60 * 60 * 1000);
    engine.run_due_timers();

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "task2");

    engine
        .get_task_service()
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored_pi = store.find_process_instance(&pi.id, &mut session).unwrap();
    assert!(stored_pi.is_ended);
}

#[test]
fn test_core_ga_receive_task_wakeup() {
    let engine = ProcessEngine::new("default".to_string());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="msg" name="myMessage" />
        <process id="receiveProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="receive1" />
            <receiveTask id="receive1" messageRef="msg" />
            <sequenceFlow id="f2" sourceRef="receive1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_builder = engine
        .get_repository_service()
        .create_deployment()
        .add_string("receive.bpmn".to_string(), xml.to_string());
    engine
        .get_repository_service()
        .deploy(deploy_builder)
        .unwrap();

    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(def_id);

    let pi = engine
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored_pi = store.find_process_instance(&pi.id, &mut session).unwrap();
    assert!(!stored_pi.is_ended);
    drop(session);

    engine
        .get_task_service()
        .wake_up_message_by_message_ref(pi.id.clone(), "myMessage".to_string())
        .unwrap();

    let mut session = store.create_session().unwrap();
    let stored_pi = store.find_process_instance(&pi.id, &mut session).unwrap();
    assert!(stored_pi.is_ended);
}

#[test]
fn test_core_ga_timer_start_intermediate_boundary() {
    let time_source = std::sync::Arc::new(
        flowable_engine::engine::time_source::TestTimeSource::new(chrono::Utc::now()),
    );
    let engine = ProcessEngine::with_time_source("default".to_string(), time_source.clone());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="timerProcess" isExecutable="true">
            <startEvent id="start">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="start" targetRef="timerInt" />
            
            <intermediateCatchEvent id="timerInt">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="timerInt" targetRef="task1" />
            
            <userTask id="task1" />
            <boundaryEvent id="timerBound" attachedToRef="task1" cancelActivity="true">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="f3" sourceRef="task1" targetRef="end1" />
            <endEvent id="end1" />
            
            <sequenceFlow id="f4" sourceRef="timerBound" targetRef="end2" />
            <endEvent id="end2" />
        </process>
    </definitions>"#;

    let deploy_builder = engine
        .get_repository_service()
        .create_deployment()
        .add_string("timer_all.bpmn".to_string(), xml.to_string());
    engine
        .get_repository_service()
        .deploy(deploy_builder)
        .unwrap();

    let _def_ids = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap();

    // trigger start timer
    time_source.advance_time(2 * 60 * 60 * 1000);
    engine.run_due_timers();

    let __runtime_store = engine.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let instances = __runtime_store.snapshot_process_instances(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(instances.len(), 1);
    let pi_id = instances.keys().next().unwrap().clone();

    // trigger intermediate timer
    time_source.advance_time(2 * 60 * 60 * 1000);
    engine.run_due_timers();

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "task1");

    // trigger boundary timer
    time_source.advance_time(2 * 60 * 60 * 1000);
    engine.run_due_timers();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored_pi = store.find_process_instance(&pi_id, &mut session).unwrap();
    assert!(stored_pi.is_ended);
}
