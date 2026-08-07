use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::task_service::EventWaitKind;
use std::collections::HashMap;

#[test]
fn test_start_process_instance_uses_deployed_start_event_id() {
    let process_engine = ProcessEngine::new("default".to_string());

    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="customStartProcess" name="Custom Start Process">
            <startEvent id="kickoff" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Start Event Resolution Deployment".to_string())
        .add_string("customStartProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("Start Event Resolution Instance".to_string());

    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let shared_runtime_store = process_engine.get_runtime_store();
    let mut session = shared_runtime_store.create_session().unwrap();
    let execution = shared_runtime_store
        .find_execution(&process_instance.id, &mut session)
        .expect("execution should be stored after start");

    assert_eq!(execution.process_definition_id, Some(process_definition_id));
    assert_eq!(execution.activity_id.as_deref(), Some("kickoff"));
}

#[test]
fn test_single_bpmn_resource_starts_each_process_definition_by_id() {
    let process_engine = ProcessEngine::new("default".to_string());

    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="messageA" name="ProcessBMessage" />
        <process id="userTaskProcess" name="User Task Process">
            <startEvent id="userStart" />
            <sequenceFlow id="userFlow1" sourceRef="userStart" targetRef="userTaskA" />
            <userTask id="userTaskA" name="Review A" />
            <sequenceFlow id="userFlow2" sourceRef="userTaskA" targetRef="userEnd" />
            <endEvent id="userEnd" />
        </process>
        <process id="messageWaitProcess" name="Message Wait Process">
            <startEvent id="messageStart" />
            <sequenceFlow id="messageFlow1" sourceRef="messageStart" targetRef="messageCatchB" />
            <intermediateCatchEvent id="messageCatchB" name="Wait For B">
                <messageEventDefinition messageRef="messageA" />
            </intermediateCatchEvent>
            <sequenceFlow id="messageFlow2" sourceRef="messageCatchB" targetRef="messageEnd" />
            <endEvent id="messageEnd" />
        </process>
        <process id="timerWaitProcess" name="Timer Wait Process">
            <startEvent id="timerStart" />
            <sequenceFlow id="timerFlow1" sourceRef="timerStart" targetRef="timerCatchC" />
            <intermediateCatchEvent id="timerCatchC" name="Wait For C">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="timerFlow2" sourceRef="timerCatchC" targetRef="timerEnd" />
            <endEvent id="timerEnd" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Multi Process Single Resource Deployment".to_string())
        .add_string(
            "multiProcessSingleResource.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();

    let definitions_by_key = repository_service
        .get_process_definitions()
        .unwrap()
        .into_iter()
        .map(|definition| (definition.key.clone(), definition))
        .collect::<HashMap<_, _>>();

    assert_eq!(
        definitions_by_key.len(),
        3,
        "single BPMN resource should deploy all process definitions"
    );

    let user_task_definition_id = definitions_by_key["userTaskProcess"].id.clone();
    let message_wait_definition_id = definitions_by_key["messageWaitProcess"].id.clone();
    let timer_wait_definition_id = definitions_by_key["timerWaitProcess"].id.clone();

    let user_task_instance = runtime_service
        .start_process_instance_by_id(user_task_definition_id.clone(), None)
        .unwrap();
    let message_wait_instance = runtime_service
        .start_process_instance_by_id(message_wait_definition_id.clone(), None)
        .unwrap();
    let timer_wait_instance = runtime_service
        .start_process_instance_by_id(timer_wait_definition_id.clone(), None)
        .unwrap();

    let user_tasks = task_service
        .get_tasks_by_process_instance_id(user_task_instance.id.clone())
        .unwrap();
    assert_eq!(user_tasks.len(), 1);
    assert_eq!(user_tasks[0].task_definition_key, "userTaskA");

    let message_wait_states = runtime_service
        .get_event_wait_states_by_process_instance_id(message_wait_instance.id.clone());
    assert_eq!(message_wait_states.len(), 1);
    assert_eq!(
        message_wait_states[0].wait_kind,
        EventWaitKind::MessageIntermediateCatchEvent
    );
    assert_eq!(
        message_wait_states[0].activity_id.as_deref(),
        Some("messageCatchB")
    );
    assert_eq!(
        message_wait_states[0].message_ref.as_deref(),
        Some("ProcessBMessage")
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let timer_jobs = runtime_store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|job| job.process_instance_id == timer_wait_instance.id)
        .collect::<Vec<_>>();
    session.rollback().unwrap();
    assert_eq!(timer_jobs.len(), 1);
    assert_eq!(timer_jobs[0].activity_id, "timerCatchC");
    assert_eq!(timer_jobs[0].time_duration.as_deref(), Some("PT1H"));

    let __runtime_store = process_engine.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let executions = __runtime_store.snapshot_executions(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(
        executions
            .get(&user_task_instance.id)
            .and_then(|execution| execution.process_definition_id.as_deref()),
        Some(user_task_definition_id.as_str())
    );
    assert_eq!(
        executions
            .get(&message_wait_instance.id)
            .and_then(|execution| execution.process_definition_id.as_deref()),
        Some(message_wait_definition_id.as_str())
    );
    assert_eq!(
        executions
            .get(&timer_wait_instance.id)
            .and_then(|execution| execution.process_definition_id.as_deref()),
        Some(timer_wait_definition_id.as_str())
    );
}
