use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_recovery_snapshot_user_task() {
    let engine1 = ProcessEngine::new("test_engine".to_string());

    let deployment_builder = engine1.get_repository_service().create_deployment()
        .add_string(
            "user_task.bpmn".to_string(),
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
              <process id="myProcess" isExecutable="true">
                <startEvent id="start"/>
                <sequenceFlow id="flow1" sourceRef="start" targetRef="userTask"/>
                <userTask id="userTask" name="My Task"/>
                <sequenceFlow id="flow2" sourceRef="userTask" targetRef="end"/>
                <endEvent id="end"/>
              </process>
            </definitions>"#.to_string(),
        );
    engine1
        .get_repository_service()
        .deploy(deployment_builder)
        .unwrap();

    let process_definition_id = engine1
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let builder = engine1
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(process_definition_id);
    let pi = engine1
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap();
    let __runtime_store = engine1.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let executions = __runtime_store.snapshot_executions(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    let user_task_execution = executions
        .values()
        .find(|e| e.activity_id.as_deref() == Some("userTask"))
        .unwrap();

    let snapshot = engine1.export_recovery_snapshot();

    // Simulate restart
    let engine2 = ProcessEngine::new("test_engine_2".to_string());
    engine2.import_recovery_snapshot(snapshot);

    let __runtime_store = engine2.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let executions2 = __runtime_store.snapshot_executions(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    let recovered_user_task_execution = executions2
        .values()
        .find(|e| e.activity_id.as_deref() == Some("userTask"))
        .unwrap();
    assert_eq!(user_task_execution.id, recovered_user_task_execution.id);

    // Can complete task
    let tasks = engine2
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone());
    engine2
        .get_task_service()
        .complete_task_by_id(tasks.unwrap()[0].id.clone())
        .unwrap();
    let runtime_store = engine2.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi2 = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .unwrap();
    assert!(pi2.is_ended);
}

#[test]
fn test_recovery_snapshot_message_start() {
    let engine1 = ProcessEngine::new("test_engine".to_string());
    let bpmn = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
      <message id="msg" name="myMessage" />
      <process id="myProcess" isExecutable="true">
        <startEvent id="start">
          <messageEventDefinition messageRef="myMessage" />
        </startEvent>
        <sequenceFlow id="flow1" sourceRef="start" targetRef="end"/>
        <endEvent id="end"/>
      </process>
    </definitions>"#;

    let deployment_builder = engine1
        .get_repository_service()
        .create_deployment()
        .add_string("msg_start.bpmn".to_string(), bpmn.to_string());
    engine1
        .get_repository_service()
        .deploy(deployment_builder)
        .unwrap();

    let snapshot = engine1.export_recovery_snapshot();

    let engine2 = ProcessEngine::new("test_engine_2".to_string());
    engine2.import_recovery_snapshot(snapshot);

    // Start instance by message on the recovered engine
    let pi = engine2.start_process_instance_by_message("myMessage".to_string());
    let runtime_store = engine2.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi2 = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .unwrap();
    assert!(pi2.is_ended);
}

#[test]
fn test_recovery_snapshot_timer_job() {
    let time_source = std::sync::Arc::new(
        flowable_engine::engine::time_source::TestTimeSource::new(chrono::Utc::now()),
    );
    let engine1 = ProcessEngine::with_time_source("test_engine".to_string(), time_source.clone());
    let bpmn = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
      <process id="myProcess" isExecutable="true">
        <startEvent id="start"/>
        <sequenceFlow id="flow1" sourceRef="start" targetRef="timer"/>
        <intermediateCatchEvent id="timer">
          <timerEventDefinition>
            <timeDuration>PT1H</timeDuration>
          </timerEventDefinition>
        </intermediateCatchEvent>
        <sequenceFlow id="flow2" sourceRef="timer" targetRef="end"/>
        <endEvent id="end"/>
      </process>
    </definitions>"#;

    let deployment_builder = engine1
        .get_repository_service()
        .create_deployment()
        .add_string("timer.bpmn".to_string(), bpmn.to_string());
    engine1
        .get_repository_service()
        .deploy(deployment_builder)
        .unwrap();

    let process_definition_id = engine1
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let builder = engine1
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(process_definition_id);
    let pi = engine1
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap();

    let snapshot = engine1.export_recovery_snapshot();

    let engine2 = ProcessEngine::with_time_source("test_engine_2".to_string(), time_source.clone());
    engine2.import_recovery_snapshot(snapshot);

    time_source.advance_time(2 * 60 * 60 * 1000);

    let dispatched = engine2.run_due_timers();
    assert_eq!(dispatched.len(), 1);

    let runtime_store = engine2.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi2 = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .unwrap();
    assert!(pi2.is_ended);
}

#[test]
fn test_recovery_snapshot_timer_start_subscription() {
    let time_source = std::sync::Arc::new(
        flowable_engine::engine::time_source::TestTimeSource::new(chrono::Utc::now()),
    );
    let engine1 = ProcessEngine::with_time_source("test_engine".to_string(), time_source.clone());
    let bpmn = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
      <process id="myProcess" isExecutable="true">
        <startEvent id="start">
          <timerEventDefinition>
            <timeDuration>PT1H</timeDuration>
          </timerEventDefinition>
        </startEvent>
        <sequenceFlow id="flow1" sourceRef="start" targetRef="end"/>
        <endEvent id="end"/>
      </process>
    </definitions>"#;

    let deployment_builder = engine1
        .get_repository_service()
        .create_deployment()
        .add_string("timer_start.bpmn".to_string(), bpmn.to_string());
    engine1
        .get_repository_service()
        .deploy(deployment_builder)
        .unwrap();

    let snapshot = engine1.export_recovery_snapshot();

    let engine2 = ProcessEngine::with_time_source("test_engine_2".to_string(), time_source.clone());
    engine2.import_recovery_snapshot(snapshot);

    let timer_subs = engine2.get_timer_start_subscriptions();
    assert_eq!(timer_subs.len(), 1);
    assert!(
        !timer_subs[0].id.is_empty(),
        "Recovered timer start subscription should retain its row id"
    );

    time_source.advance_time(2 * 60 * 60 * 1000);
    let dispatched = engine2.run_due_timers();
    assert_eq!(dispatched.len(), 1);
    let runtime_store = engine2.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert!(runtime_store.snapshot_process_instances(&mut session).len() == 1);
}
