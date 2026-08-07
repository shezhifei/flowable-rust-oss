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
        .name("Validation Test Deployment".to_string())
        .add_string("test_process.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).map(|_| ())
}

#[test]
fn test_script_task_without_secure_scripting_fails() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="scriptProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="scriptTask1" />
            <scriptTask id="scriptTask1" scriptFormat="javascript">
                <script>var x = 1;</script>
            </scriptTask>
            <sequenceFlow id="flow2" sourceRef="scriptTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    // Secure scripting is disabled by default
    let config = ProcessEngineConfiguration::default();
    let result = deploy_xml(xml, config);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Secure scripting is not enabled"));
}

#[test]
fn test_script_task_with_unsupported_language_fails() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="scriptProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="scriptTask1" />
            <scriptTask id="scriptTask1" scriptFormat="python">
                <script>x = 1</script>
            </scriptTask>
            <sequenceFlow id="flow2" sourceRef="scriptTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        ..Default::default()
    };
    let result = deploy_xml(xml, config);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Script format 'python' is not supported"));
}

#[test]
fn test_m9_excluded_service_tasks_fail() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
        <process id="httpProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
            <serviceTask id="httpTask1" flowable:type="http" />
            <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let config = ProcessEngineConfiguration::default();
    let result = deploy_xml(xml, config);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Task type 'http' is not supported in M9"));
}

#[test]
fn test_supported_m9_shapes_deploy_successfully() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="transactionProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="transaction1" />
            <transaction id="transaction1">
                <startEvent id="subStartEvent" />
                <sequenceFlow id="subFlow1" sourceRef="subStartEvent" targetRef="subEndEvent" />
                <endEvent id="subEndEvent">
                    <cancelEventDefinition />
                </endEvent>
            </transaction>
            <boundaryEvent id="boundaryCancel1" attachedToRef="transaction1">
                <cancelEventDefinition />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="transaction1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let config = ProcessEngineConfiguration::default();
    let result = deploy_xml(xml, config);
    assert!(
        result.is_ok(),
        "Supported transaction/cancel shapes should deploy successfully"
    );
}
