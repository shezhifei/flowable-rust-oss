use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;

#[test]
fn test_historic_variable_instance_query() {
    let process_engine = ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    );
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = process_engine.get_history_service();
    let variable_service = process_engine.get_variable_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="varHistoryProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("var_history.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pi = runtime_service
        .start_process_instance_by_key("varHistoryProcess")
        .unwrap();

    variable_service
        .set_variable(pi.id.clone(), "testVar".to_string(), json!("testValue"))
        .unwrap();

    let hist_vars = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(pi.id.clone())
        .variable_name("testVar".to_string())
        .list()
        .unwrap();

    assert_eq!(hist_vars.len(), 1);
    assert_eq!(hist_vars[0].variable_name(), "testVar");
    assert_eq!(hist_vars[0].value(), &json!("testValue"));
}
