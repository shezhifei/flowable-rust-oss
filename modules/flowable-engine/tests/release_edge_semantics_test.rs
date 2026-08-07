use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_unsupported_boundary_event_returns_structured_error() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="unsupportedBoundaryProcess" name="Unsupported Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <!-- Link boundary event is unsupported right now -->
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="true">
                <linkEventDefinition name="LinkA" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Unsupported Boundary Test".to_string())
        .add_string(
            "unsupportedBoundaryProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // The boundary event is evaluated during take_outgoing_sequence_flows_operation or when it's reached.
    // If it's unsupported, what should happen?
    // In Flowable, boundary events are attached when the activity is entered.
    // So starting the process should fail, or maybe it's just parsed as unsupported and fails at runtime.
    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Unsupported Boundary Test Instance".to_string());

    let result = runtime_service.start_process_instance(process_instance_builder);
    assert!(result.is_err());

    if let Err(e) = result {
        let err_str = format!("{:?}", e);
        assert!(err_str.contains("UnsupportedElement"));
    }
}
