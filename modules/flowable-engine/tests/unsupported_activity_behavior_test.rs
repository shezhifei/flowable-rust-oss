use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;

fn assert_unsupported_activity_error(xml: &str, expected_type: &str, expected_id: &str) {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("Unsupported Activity Deployment".to_string())
        .add_string(
            "unsupported_process.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    let _ = repository_service.deploy(builder);

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Unsupported Instance".to_string());

    let result = runtime_service.start_process_instance(process_instance_builder);

    match result {
        Err(FlowableError::UnsupportedElement {
            element_type,
            activity_id,
        }) => {
            assert_eq!(element_type, expected_type);
            assert_eq!(activity_id, expected_id);
        }
        Err(e) => panic!("Expected UnsupportedElement error, got: {:?}", e),
        Ok(_) => panic!("Expected process start to fail on unsupported node, but it succeeded"),
    }
}

#[test]
fn test_unsupported_intermediate_throw_timer_event_behavior() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="unsupportedIntermediateThrowProcess" name="Unsupported Intermediate Throw Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="intermediateThrowEvent1" />
            <intermediateThrowEvent id="intermediateThrowEvent1" name="Timer Throw">
                <timerEventDefinition><timeDuration>PT10H</timeDuration></timerEventDefinition>
            </intermediateThrowEvent>
            <sequenceFlow id="flow2" sourceRef="intermediateThrowEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    assert_unsupported_activity_error(xml, "IntermediateThrowEvent", "intermediateThrowEvent1");
}

#[test]
fn test_supported_process_starts_normally() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="supportedProcess" name="Supported Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Normal User Task" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("Supported Activity Deployment".to_string())
        .add_string("supported_process.bpmn20.xml".to_string(), xml.to_string());

    let _ = repository_service.deploy(builder);

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Supported Instance".to_string());

    let result = runtime_service.start_process_instance(process_instance_builder);

    assert!(
        result.is_ok(),
        "Supported process should start successfully"
    );
}
