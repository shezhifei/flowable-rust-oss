use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;

#[test]
fn test_deterministic_repository_query_ordering() {
    let process_engine = ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    );
    let repository_service = process_engine.get_repository_service();

    let xml1 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="processB"><startEvent id="start" /><endEvent id="end" /></process>
    </definitions>"#;

    let xml2 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="processA"><startEvent id="start" /><endEvent id="end" /></process>
    </definitions>"#;

    let xml3 = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="processC"><startEvent id="start" /><endEvent id="end" /></process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("B.bpmn20.xml".to_string(), xml1.to_string()),
        )
        .unwrap();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("A.bpmn20.xml".to_string(), xml2.to_string()),
        )
        .unwrap();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("C.bpmn20.xml".to_string(), xml3.to_string()),
        )
        .unwrap();

    let ids = repository_service.get_process_definition_ids().unwrap();
    assert_eq!(ids.len(), 3);

    let mut sorted_ids = ids.clone();
    sorted_ids.sort();
    assert_eq!(ids, sorted_ids);
}
