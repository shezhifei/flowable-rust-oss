use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;

fn deploy_xml(
    xml: &str,
    config: ProcessEngineConfiguration,
) -> Result<(), flowable_engine::error::FlowableError> {
    let process_engine = ProcessEngine::new_with_config("default".to_string(), config);
    let repository_service = process_engine.get_repository_service();

    let builder = repository_service
        .create_deployment()
        .name("CXF Service Task Test".to_string())
        .add_string("test_process.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).map(|_| ())
}

#[test]
fn cxf_service_task_deploy_succeeds_with_valid_bpmn() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
        xmlns:flowable="http://flowable.org/bpmn"
        targetNamespace="Examples">
        <process id="cxfProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="cxfTask1" />
            <serviceTask id="cxfTask1" flowable:type="cxf" />
            <sequenceFlow id="flow2" sourceRef="cxfTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let config = ProcessEngineConfiguration::default();
    let result = deploy_xml(xml, config);
    assert!(
        result.is_ok(),
        "CXF service task should deploy (runtime behavior is separate)"
    );
}
