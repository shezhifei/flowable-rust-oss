use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;

fn deploy_and_start(
    xml: &str,
    config: ProcessEngineConfiguration,
) -> Result<(), flowable_engine::error::FlowableError> {
    let engine = ProcessEngine::new_with_config("groovy-test".to_string(), config);
    let repo = engine.get_repository_service();

    let builder = repo
        .create_deployment()
        .name("Groovy Static Test".to_string())
        .add_string("test.bpmn20.xml".to_string(), xml.to_string());

    repo.deploy(builder)?;
    let runtime = engine.get_runtime_service();
    runtime.start_process_instance_by_key("groovyProcess")?;
    Ok(())
}

#[test]
fn groovy_static_executes_simple_expression() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="groovyProcess" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="scriptTask1" />
            <scriptTask id="scriptTask1" scriptFormat="groovy">
                <script>def x = 1 + 2</script>
            </scriptTask>
            <sequenceFlow id="flow2" sourceRef="scriptTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["groovy".to_string()],
        ..Default::default()
    };
    let result = deploy_and_start(xml, config);
    assert!(
        result.is_ok(),
        "Groovy static should execute simple expression: {:?}",
        result.err()
    );
}

#[test]
fn groovy_rejected_when_not_in_supported_languages() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="groovyProcess" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="scriptTask1" />
            <scriptTask id="scriptTask1" scriptFormat="groovy">
                <script>def x = 1</script>
            </scriptTask>
            <sequenceFlow id="flow2" sourceRef="scriptTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let config = ProcessEngineConfiguration::default();
    let result = deploy_and_start(xml, config);
    assert!(
        result.is_err(),
        "Groovy should be rejected when not in supported languages"
    );
}
