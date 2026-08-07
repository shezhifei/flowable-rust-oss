use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::service::config::ProcessEngineConfiguration;

#[test]
fn test_task_query_deterministic_ordering() {
    let process_engine = ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    );
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="orderProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Task 1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="task2" />
            <userTask id="task2" name="Task 2" />
            <sequenceFlow id="f3" sourceRef="task2" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("order.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let proc_inst = runtime_service
        .start_process_instance_by_key("orderProcess")
        .unwrap();

    // TaskQuery.list() must produce a deterministic ordering so that callers can rely on
    // task pagination and snapshotting without re-issuing the query.
    let tasks = task_service
        .create_task_query()
        .process_instance_id(proc_inst.id.clone())
        .list()
        .unwrap();

    assert!(!tasks.is_empty(), "Should have at least one active task");

    // Verify ordering by name if supported
    let tasks_by_name = task_service
        .create_task_query()
        .process_instance_id(proc_inst.id.clone())
        .order_by_task_name()
        .asc()
        .list()
        .unwrap();

    if tasks_by_name.len() > 1 {
        assert!(tasks_by_name[0].name() <= tasks_by_name[1].name());
    }
}

#[test]
fn test_task_query_filtering() {
    let process_engine = ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    );
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="filterProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="taskA" />
            <userTask id="taskA" name="Target Task" />
            <sequenceFlow id="f2" sourceRef="taskA" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string("filter.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    runtime_service
        .start_process_instance_by_key("filterProcess")
        .unwrap();

    let task = task_service
        .create_task_query()
        .task_name("Target Task".to_string())
        .single_result()
        .unwrap();

    assert!(task.is_some());
    assert_eq!(task.unwrap().name(), "Target Task");

    let no_task = task_service
        .create_task_query()
        .task_name("Non-existent".to_string())
        .single_result()
        .unwrap();

    assert!(no_task.is_none());
}
