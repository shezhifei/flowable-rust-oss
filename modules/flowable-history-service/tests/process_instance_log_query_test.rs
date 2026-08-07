mod common;

use common::create_process_engine;
use flowable_history_service::FlowableHistoryService;

#[test]
fn test_process_instance_log_query() {
    let process_engine = create_process_engine();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = FlowableHistoryService::new(process_engine.clone());

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="logProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("log.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pi = runtime_service
        .start_process_instance_by_key("logProcess")
        .unwrap();

    let log = history_service
        .create_process_instance_log_query(pi.id.clone())
        .include_tasks()
        .include_activities()
        .include_variables()
        .single_result()
        .unwrap();

    assert!(log.is_some());
    let log = log.unwrap();
    assert_eq!(log.process_instance_id(), &pi.id);
}
