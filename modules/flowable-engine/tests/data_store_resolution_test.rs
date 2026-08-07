use flowable_engine::engine::data_routing::DataRoutingService;
use flowable_engine::engine::process_engine::ProcessEngine;

const DATA_STORE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <dataStore id="ds1" name="My Data Store" />
    <process id="p1" isExecutable="true">
        <startEvent id="start" />
        <endEvent id="end" />
    </process>
</definitions>"#;

#[test]
fn test_data_store_resolution() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();

    let deployment_builder = repository_service.create_deployment().add_string(
        "datastore.bpmn20.xml".to_string(),
        DATA_STORE_XML.to_string(),
    );
    repository_service.deploy(deployment_builder).unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // Test resolution
    let executor = process_engine.get_command_executor();
    let model = executor
        .deployment_manager()
        .get_bpmn_model(&pd_id)
        .unwrap();

    let ds = DataRoutingService::resolve_data_store(&model, "ds1").unwrap();
    assert_eq!(ds.name.as_deref().unwrap(), "My Data Store");

    let res = DataRoutingService::resolve_data_store(&model, "nonexistent");
    assert!(res.is_err());
}
