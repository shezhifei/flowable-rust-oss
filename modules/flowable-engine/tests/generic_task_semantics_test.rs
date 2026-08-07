use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_generic_task_semantics() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="genericTaskProcess" name="Generic Task Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="genericTask" />
            <task id="genericTask" name="Just A Generic Task" />
            <sequenceFlow id="flow2" sourceRef="genericTask" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("Generic Task Deployment".to_string())
        .add_string("generic_task.bpmn20.xml".to_string(), xml.to_string());

    let _ = repository_service.deploy(builder);

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Generic Task Instance".to_string());

    let result = runtime_service.start_process_instance(process_instance_builder);
    assert!(
        result.is_ok(),
        "Generic Task process should start successfully without panic or error"
    );
}
