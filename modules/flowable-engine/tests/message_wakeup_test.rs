use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::task_service::MessageStyleWaitKind;

fn deploy_receive_task_process(process_engine: &ProcessEngine, deployment_name: &str) -> String {
    let repository_service = process_engine.get_repository_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="receiveTaskWakeup" name="Receive Task Wakeup">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Signal" />
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name(deployment_name.to_string())
        .add_string("receiveTaskWakeup.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

#[test]
fn test_message_wakeup_no_match_is_noop() {
    let process_engine = ProcessEngine::new("default".to_string());
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let process_definition_id =
        deploy_receive_task_process(&process_engine, "Receive Task Wakeup No Match");

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Wakeup Instance".to_string()),
        )
        .unwrap();

    let waiting_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(waiting_states.len(), 1);
    assert_eq!(
        waiting_states[0].wait_kind,
        MessageStyleWaitKind::ReceiveTask
    );
    assert_eq!(
        waiting_states[0].process_instance_id,
        process_instance.id.clone()
    );
    assert_eq!(waiting_states[0].execution_id, process_instance.id.clone());
    assert!(waiting_states[0].message_name.is_none());
    assert!(waiting_states[0].message_ref.is_none());

    process_engine.wake_up_message_by_process_instance_id("missing-process-instance".to_string());

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .snapshot_executions(&mut session)
        .get(&process_instance.id)
        .cloned()
        .expect("execution should remain persisted");
    assert!(!execution.is_active);
    assert_eq!(execution.activity_id.as_deref(), Some("receiveTask1"));
}

#[test]
fn test_message_wakeup_only_wakes_one_matching_instance() {
    let process_engine = ProcessEngine::new("default".to_string());
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let process_definition_id =
        deploy_receive_task_process(&process_engine, "Receive Task Wakeup Single Match");

    let first_process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.clone())
                .name("Wakeup Instance 1".to_string()),
        )
        .unwrap();
    let second_process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Wakeup Instance 2".to_string()),
        )
        .unwrap();

    let first_waiting_states = process_engine
        .get_message_style_wait_states_by_process_instance_id(first_process_instance.id.clone());
    assert_eq!(first_waiting_states.len(), 1);
    assert_eq!(
        first_waiting_states[0].wait_kind,
        MessageStyleWaitKind::ReceiveTask
    );

    let second_waiting_states = process_engine
        .get_message_style_wait_states_by_process_instance_id(second_process_instance.id.clone());
    assert_eq!(second_waiting_states.len(), 1);
    assert_eq!(
        second_waiting_states[0].wait_kind,
        MessageStyleWaitKind::ReceiveTask
    );

    process_engine.wake_up_message_by_process_instance_id(first_process_instance.id.clone());

    let first_waiting_states = process_engine
        .get_message_style_wait_states_by_process_instance_id(first_process_instance.id.clone());
    assert!(first_waiting_states.is_empty());

    let first_tasks = task_service
        .get_tasks_by_process_instance_id(first_process_instance.id.clone())
        .unwrap();
    assert!(first_tasks.is_empty());

    let second_waiting_states = process_engine
        .get_message_style_wait_states_by_process_instance_id(second_process_instance.id.clone());
    assert_eq!(second_waiting_states.len(), 1);

    let second_tasks = task_service
        .get_tasks_by_process_instance_id(second_process_instance.id.clone())
        .unwrap();
    assert_eq!(second_tasks.len(), 1);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let first_execution = runtime_store
        .snapshot_executions(&mut session)
        .get(&first_process_instance.id)
        .cloned()
        .expect("first execution should remain persisted");
    assert!(first_execution.is_active);

    let second_execution = runtime_store
        .snapshot_executions(&mut session)
        .get(&second_process_instance.id)
        .cloned()
        .expect("second execution should remain persisted");
    assert!(!second_execution.is_active);
    assert_eq!(
        second_execution.activity_id.as_deref(),
        Some("receiveTask1")
    );
}

#[test]
fn test_message_wakeup_by_message_ref() {
    let process_engine = ProcessEngine::new("default".to_string());
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let repository_service = process_engine.get_repository_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="receiveTaskMessageRef" name="Receive Task Message Ref">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Signal" messageRef="mySpecialMessage" />
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Receive Task Message Ref".to_string())
        .add_string(
            "receiveTaskMessageRef.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Wakeup Instance".to_string()),
        )
        .unwrap();

    let waiting_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(waiting_states.len(), 1);
    assert_eq!(
        waiting_states[0].wait_kind,
        MessageStyleWaitKind::ReceiveTask
    );
    assert_eq!(
        waiting_states[0].message_ref.as_deref(),
        Some("mySpecialMessage")
    );

    // Try waking up with wrong message_ref
    process_engine
        .wake_up_message_by_message_ref(process_instance.id.clone(), "wrongMessage".to_string());

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1); // Task should still be there

    // Now wake up with correct message_ref
    process_engine.wake_up_message_by_message_ref(
        process_instance.id.clone(),
        "mySpecialMessage".to_string(),
    );

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(tasks_after.is_empty()); // Task should be completed
}
