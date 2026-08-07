use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;

fn deploy_and_start(
    xml: &str,
    config: ProcessEngineConfiguration,
) -> Result<(), flowable_engine::error::FlowableError> {
    let engine = ProcessEngine::new_with_config("secure-js-test".to_string(), config);
    let repo = engine.get_repository_service();

    let builder = repo
        .create_deployment()
        .name("Secure JS Test".to_string())
        .add_string("test.bpmn20.xml".to_string(), xml.to_string());

    repo.deploy(builder)?;
    let runtime = engine.get_runtime_service();
    runtime.start_process_instance_by_key("jsProcess")?;
    Ok(())
}

#[test]
fn secure_javascript_executes_simple_expression() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="jsProcess" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="scriptTask1" />
            <scriptTask id="scriptTask1" scriptFormat="javascript">
                <script>var x = 1 + 2;</script>
            </scriptTask>
            <sequenceFlow id="flow2" sourceRef="scriptTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        ..Default::default()
    };
    let result = deploy_and_start(xml, config);
    assert!(
        result.is_ok(),
        "Secure JS should execute simple expression: {:?}",
        result.err()
    );
}

#[test]
fn secure_javascript_rejected_when_disabled() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="jsProcess" isExecutable="true">
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
    let result = deploy_and_start(xml, config);
    assert!(
        result.is_err(),
        "JS should be rejected when secure scripting is disabled"
    );
}
