use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_runtime_state_persists_after_start_process_instance() {
    let process_engine = ProcessEngine::new("default".to_string());

    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="myProcess" name="My First Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="First Task" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="end1" />
            <endEvent id="end1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Runtime State Deployment".to_string())
        .add_string("myProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("Runtime State Instance".to_string());

    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let shared_runtime_store = process_engine.get_runtime_store();
    let mut session = shared_runtime_store.create_session().unwrap();
    let execution = shared_runtime_store
        .find_execution(&process_instance.id, &mut session)
        .expect("execution should remain in shared runtime store after command returns");

    assert_eq!(execution.id, process_instance.id);
    assert_eq!(execution.process_definition_id, Some(process_definition_id));
    assert_eq!(execution.activity_id.as_deref(), Some("userTask1"));
    assert!(!execution.is_active);
}
