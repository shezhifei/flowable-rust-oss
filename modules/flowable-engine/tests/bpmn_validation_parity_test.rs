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
        .name("BPMN Validation Test".to_string())
        .add_string("test_process.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).map(|_| ())
}

#[test]
fn bpmn_validation_rejects_script_task_without_secure_scripting() {
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

    let config = ProcessEngineConfiguration::default();
    let result = deploy_xml(xml, config);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Secure scripting is not enabled")
    );
}

#[test]
fn bpmn_validation_rejects_unsupported_script_language() {
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Script format 'python' is not supported")
    );
}

#[test]
fn bpmn_validation_rejects_http_service_task() {
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Task type 'http' is not supported")
    );
}

#[test]
fn bpmn_validation_accepts_valid_user_task_process() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="userTaskProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let config = ProcessEngineConfiguration::default();
    let result = deploy_xml(xml, config);
    assert!(result.is_ok(), "Valid user task process should deploy");
}

#[test]
fn bpmn_validation_accepts_script_task_with_secure_scripting_enabled() {
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

    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        ..Default::default()
    };
    let result = deploy_xml(xml, config);
    assert!(
        result.is_ok(),
        "JS script task with secure scripting enabled should deploy"
    );
}
