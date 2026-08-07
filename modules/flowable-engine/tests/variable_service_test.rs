use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

#[test]
fn test_variable_service() {
    let process_engine = ProcessEngine::new("default".to_string());
    let runtime_service = process_engine.get_runtime_service();
    let variable_service = process_engine.get_variable_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p1" isExecutable="true">
            <startEvent id="start" />
            <userTask id="task1" name="My Task" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let repository_service = process_engine.get_repository_service();
    let deployment_builder = repository_service
        .create_deployment()
        .add_string("p1.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(deployment_builder).unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance_by_id(pd_id, None)
        .unwrap();

    let exec_id = pi.id.clone();

    // Set variable
    variable_service
        .set_variable(exec_id.clone(), "myVar".to_string(), json!("myValue"))
        .unwrap();

    // Get variable
    let val = variable_service
        .get_variable(exec_id.clone(), "myVar".to_string())
        .unwrap();
    assert_eq!(val, Some(json!("myValue")));

    // Get all variables
    let vars = variable_service.get_variables(exec_id).unwrap();
    assert_eq!(vars.get("myVar"), Some(&json!("myValue")));
}
