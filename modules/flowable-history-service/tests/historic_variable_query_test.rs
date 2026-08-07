mod common;

use common::create_process_engine;
use flowable_engine::engine::query::Query;
use flowable_history_service::FlowableHistoryService;
use serde_json::json;

#[test]
fn test_historic_variable_instance_query() {
    let process_engine = create_process_engine();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = FlowableHistoryService::new(process_engine.clone());
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

    let historic_variables = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(pi.id.clone())
        .variable_name("testVar".to_string())
        .list()
        .unwrap();

    assert_eq!(historic_variables.len(), 1);
    assert_eq!(historic_variables[0].variable_name(), "testVar");
    assert_eq!(historic_variables[0].value(), &json!("testValue"));
}

#[test]
fn test_historic_variable_query_filters_task_local_and_process_variables() {
    let process_engine = create_process_engine();
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = FlowableHistoryService::new(process_engine.clone());
    let variable_service = process_engine.get_variable_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="varScopeHistoryProcess">
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
                .add_string("var_scope_history.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let pi = runtime_service
        .start_process_instance_by_key("varScopeHistoryProcess")
        .unwrap();
    let task = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap()
        .pop()
        .unwrap();

    variable_service
        .set_variable(pi.id.clone(), "globalFlag".to_string(), json!(true))
        .unwrap();
    task_service
        .set_task_local_variable(
            task.id.clone(),
            "localDecision".to_string(),
            json!("approved"),
        )
        .unwrap();
    task_service
        .set_task_local_variable(
            task.id.clone(),
            "LocalDecision".to_string(),
            json!("rejected"),
        )
        .unwrap();

    let process_variables = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(pi.id.clone())
        .exclude_task_variables()
        .list()
        .unwrap();

    assert_eq!(process_variables.len(), 1);
    assert_eq!(process_variables[0].variable_name(), "globalFlag");
    assert!(process_variables[0].task_id.is_none());

    let task_variables = history_service
        .create_historic_variable_instance_query()
        .task_id(task.id.clone())
        .variable_name_like("local%".to_string())
        .variable_type("string".to_string())
        .list()
        .unwrap();

    assert_eq!(task_variables.len(), 1);
    assert_eq!(task_variables[0].variable_name(), "localDecision");
    assert_eq!(task_variables[0].task_id.as_deref(), Some(task.id.as_str()));
}
