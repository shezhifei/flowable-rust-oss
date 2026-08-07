use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;

fn deploy_xml(xml: &str) -> Result<(), FlowableError> {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();

    let builder = repository_service
        .create_deployment()
        .name("Validation Test Deployment".to_string())
        .add_string("test_process.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).map(|_| ())
}

#[test]
fn test_supported_process_deploys_successfully() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="validProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="task1" />
            <task id="task1" />
            <sequenceFlow id="flow2" sourceRef="task1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    assert!(
        deploy_xml(xml).is_ok(),
        "Supported generic task process should deploy successfully in M2"
    );
}

#[test]
fn test_subprocess_accepted_at_deployment() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="invalidProcess">
            <subProcess id="subProcess1">
            </subProcess>
        </process>
    </definitions>"#;

    // M3 - SubProcess runtime semantics are implemented.
    assert!(
        deploy_xml(xml).is_ok(),
        "Supported generic task process should deploy successfully in M3"
    );
}

#[test]
fn test_callactivity_accepted_at_deployment() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="invalidProcess">
            <callActivity id="callActivity1" calledElement="someProcess" />
        </process>
    </definitions>"#;

    assert!(
        deploy_xml(xml).is_ok(),
        "CallActivity should deploy successfully in M3"
    );
}
