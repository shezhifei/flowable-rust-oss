use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::service::config::ProcessEngineConfiguration;

const SEQUENTIAL_MI_HISTORY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="sequentialMIHistoryProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="true">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const PARALLEL_MI_HISTORY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="parallelMIHistoryProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

#[test]
fn sequential_mi_records_one_activity_instance_per_child() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let history_service = process_engine.get_history_service();

    repository_service
        .deploy(repository_service.create_deployment().add_string(
            "sequential_mi_history.bpmn20.xml".to_string(),
            SEQUENTIAL_MI_HISTORY_XML.to_string(),
        ))
        .unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_def_id),
        )
        .unwrap();

    for _ in 0..3 {
        let tasks = task_service
            .get_tasks_by_process_instance_id(pi.id.clone())
            .unwrap();
        assert_eq!(tasks.len(), 1);
        task_service
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    }

    let activities = history_service
        .create_historic_activity_instance_query()
        .process_instance_id(pi.id.clone())
        .list()
        .unwrap();

    let mi_task_activities: Vec<_> = activities
        .iter()
        .filter(|a| a.activity_id == "miTask")
        .collect();

    assert_eq!(
        mi_task_activities.len(),
        3,
        "Java parity: sequential MI with loopCardinality=3 must record 3 historic activity instances for miTask, got {}. All activities: {:?}",
        mi_task_activities.len(),
        activities
            .iter()
            .map(|a| &a.activity_id)
            .collect::<Vec<_>>()
    );

    for (i, act) in mi_task_activities.iter().enumerate() {
        assert!(
            act.end_time.is_some(),
            "miTask instance {} must have end_time set (all instances completed)",
            i
        );
        assert!(
            act.duration_ms.is_some(),
            "miTask instance {} must have duration_ms set",
            i
        );
    }
}

#[test]
fn parallel_mi_records_one_activity_instance_per_child() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let history_service = process_engine.get_history_service();

    repository_service
        .deploy(repository_service.create_deployment().add_string(
            "parallel_mi_history.bpmn20.xml".to_string(),
            PARALLEL_MI_HISTORY_XML.to_string(),
        ))
        .unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_def_id),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3);

    for task in tasks {
        task_service.complete_task_by_id(task.id.clone()).unwrap();
    }

    let activities = history_service
        .create_historic_activity_instance_query()
        .process_instance_id(pi.id.clone())
        .list()
        .unwrap();

    let mi_task_activities: Vec<_> = activities
        .iter()
        .filter(|a| a.activity_id == "miTask")
        .collect();

    assert_eq!(
        mi_task_activities.len(),
        3,
        "Java parity: parallel MI with loopCardinality=3 must record 3 historic activity instances for miTask, got {}. All activities: {:?}",
        mi_task_activities.len(),
        activities
            .iter()
            .map(|a| &a.activity_id)
            .collect::<Vec<_>>()
    );

    for (i, act) in mi_task_activities.iter().enumerate() {
        assert!(
            act.end_time.is_some(),
            "miTask instance {} must have end_time set (all instances completed)",
            i
        );
        assert!(
            act.duration_ms.is_some(),
            "miTask instance {} must have duration_ms set",
            i
        );
    }
}

#[test]
fn test_historic_process_instance_query() {
    let process_engine = ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    );
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = process_engine.get_history_service();

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
    let process_engine = ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    );
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let history_service = process_engine.get_history_service();

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

    // Should have at least start event and end event (if completed) or just start.
    // In our minimal engine, start event might be enough for now.
    assert!(!activities.is_empty());
    assert!(activities.iter().any(|a| a.activity_id() == "start"));
}
