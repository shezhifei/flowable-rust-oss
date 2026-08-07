mod common;

use common::create_process_engine;
use flowable_engine::engine::query::Query;
use flowable_history_service::{FlowableHistoryService, HistoricActivityInstanceQueryRequest};

#[test]
fn test_historic_process_instance_query() {
    let process_engine = create_process_engine();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = FlowableHistoryService::new(process_engine.clone());

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="historyProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("history.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pi = runtime_service
        .start_process_instance_by_key("historyProcess")
        .unwrap();

    let hist_instances = history_service
        .create_historic_process_instance_query()
        .process_instance_id(pi.id.clone())
        .list()
        .unwrap();

    assert_eq!(hist_instances.len(), 1);
    assert_eq!(hist_instances[0].id(), &pi.id);
}

#[test]
fn test_historic_activity_instance_query() {
    let process_engine = create_process_engine();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = FlowableHistoryService::new(process_engine.clone());

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="activityHistoryProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("act_history.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pi = runtime_service
        .start_process_instance_by_key("activityHistoryProcess")
        .unwrap();

    let activities = history_service
        .create_historic_activity_instance_query()
        .process_instance_id(pi.id.clone())
        .list()
        .unwrap();

    assert!(!activities.is_empty());
    assert!(
        activities
            .iter()
            .any(|activity| activity.activity_id() == "start")
    );
}

#[test]
fn test_historic_activity_service_filters_activity_and_execution() {
    let process_engine = create_process_engine();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = FlowableHistoryService::new(process_engine.clone());

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="activityServiceFilterProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Filtered Task" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(repository_service.create_deployment().add_string(
            "activity_service_filter.bpmn20.xml".to_string(),
            xml.to_string(),
        ))
        .unwrap();

    let pi = runtime_service
        .start_process_instance_by_key("activityServiceFilterProcess")
        .unwrap();

    let task_activity = history_service
        .list_historic_activity_instances(HistoricActivityInstanceQueryRequest {
            process_instance_id: Some(pi.id.clone()),
            execution_id: None,
            activity_id: Some("task1".to_string()),
        })
        .unwrap();

    assert_eq!(task_activity.len(), 1);
    let execution_id = task_activity[0].execution_id.clone();

    let filtered_by_execution = history_service
        .list_historic_activity_instances(HistoricActivityInstanceQueryRequest {
            process_instance_id: Some(pi.id.clone()),
            execution_id: Some(execution_id),
            activity_id: Some("task1".to_string()),
        })
        .unwrap();

    assert_eq!(filtered_by_execution.len(), 1);
    assert_eq!(filtered_by_execution[0].activity_id(), "task1");
}
