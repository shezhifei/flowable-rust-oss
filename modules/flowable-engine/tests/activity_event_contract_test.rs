use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;

#[test]
fn test_deterministic_activity_event_recording() {
    let process_engine = Arc::new(ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    ));

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="activityProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="User Task" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let builder = process_engine
        .get_repository_service()
        .create_deployment()
        .add_string("activity.bpmn20.xml".to_string(), xml.to_string());

    process_engine
        .get_repository_service()
        .deploy(builder)
        .unwrap();

    let pds = process_engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap();
    let pd_id = pds[0].clone();

    let pi = process_engine
        .get_runtime_service()
        .start_process_instance(
            process_engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut params = flowable_engine::persistence::DbParams::new();
    params.push(pi.id.as_str());

    let rows = session
        .raw_query(
            "SELECT data FROM historic_activity_instances WHERE process_instance_id = ?",
            params,
        )
        .unwrap();

    let mut activities = Vec::new();
    for row in rows {
        if let Some(data) = row.get_text("data") {
            activities.push(data);
        }
    }

    assert!(activities.len() >= 2);
    let task1_activity = activities
        .iter()
        .find(|a| a.contains(r#""activity_id":"task1""#))
        .expect("task1 not found");
    assert!(task1_activity.contains(r#""end_time":null"#));

    let _ = session.rollback();

    let tasks = process_engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    process_engine
        .get_task_service()
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let mut session2 = store.create_session().unwrap();
    let mut params2 = flowable_engine::persistence::DbParams::new();
    params2.push(pi.id.as_str());
    let rows2 = session2
        .raw_query(
            "SELECT data FROM historic_activity_instances WHERE process_instance_id = ?",
            params2,
        )
        .unwrap();

    let mut activities2 = Vec::new();
    for row in rows2 {
        if let Some(data) = row.get_text("data") {
            activities2.push(data);
        }
    }

    assert!(activities2.len() >= 3);
    let task1_activity_ended = activities2
        .iter()
        .find(|a| a.contains(r#""activity_id":"task1""#))
        .expect("task1 not found");
    assert!(!task1_activity_ended.contains(r#""end_time":null"#));

    let _end_event = activities2
        .iter()
        .find(|a| a.contains(r#""activity_id":"end""#))
        .expect("end not found");
    let _ = session2.rollback();
}
