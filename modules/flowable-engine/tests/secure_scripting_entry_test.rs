use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;

#[test]
fn test_secure_javascript_execution() {
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string()],
        ..Default::default()
    };

    let process_engine = ProcessEngine::new_with_config("default".to_string(), config);
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="jsProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="scriptTask" />
            <scriptTask id="scriptTask" scriptFormat="javascript">
                <script>
                    var x = 10;
                    var y = 20;
                    var result = x + y;
                </script>
            </scriptTask>
            <sequenceFlow id="f2" sourceRef="scriptTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("js.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pi = runtime_service
        .start_process_instance_by_key("jsProcess")
        .unwrap();

    let result_var = runtime_service
        .get_variable(pi.id.clone(), "result".to_string())
        .unwrap();
    assert_eq!(result_var, Some(json!(30)));
}
