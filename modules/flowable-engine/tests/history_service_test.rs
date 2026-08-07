use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_history_service::FlowableHistoryService;
use std::sync::Arc;

#[test]
fn test_history_process_instance() {
    let engine = Arc::new(ProcessEngine::new("history-test".to_string()));
    let history_service = FlowableHistoryService::new(Arc::clone(&engine));

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="historyProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="User Task" />
        <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

    let builder = engine
        .get_repository_service()
        .create_deployment()
        .name("History Deployment".to_string())
        .add_string("history.bpmn20.xml".to_string(), xml.to_string());

    engine
        .get_repository_service()
        .deploy(builder)
        .expect("deploy failed");

    let pds = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap();
    let pd_id = pds[0].clone();

    let mut builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(pd_id);
    builder = builder
        .business_key("HistoryBK".to_string())
        .variable("my_var".to_string(), "my_value".into());

    let pi = engine
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap();
    let pi_id = pi.id.clone();

    // Check variable created
    // We don't have get_historic_variable_instance_by_pi_id so we just rely on audit log to show start was called, or we could find the variable ID if we had an API.
    // Wait, FlowableHistoryService doesn't have an API to list by process_instance_id. Let me just test historic process instance and audit log first.

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    let task_id = tasks[0].id.clone();

    engine
        .get_task_service()
        .complete_task_by_id(task_id.clone())
        .unwrap();

    // Check history PI
    let historic_pi = history_service
        .get_historic_process_instance(&pi_id)
        .expect("history should exist");
    assert_eq!(historic_pi.id, pi_id);
    assert_eq!(historic_pi.business_key, Some("HistoryBK".to_string()));
    assert!(historic_pi.end_time.is_some());
    assert!(historic_pi.duration_ms.is_some());

    // Check historic task
    let historic_task = history_service
        .get_historic_task_instance(&task_id)
        .expect("historic task should exist");
    assert_eq!(historic_task.id, task_id);
    assert!(historic_task.end_time.is_some());

    // For audit and variables, we just ensure they don't panic and are recorded. We can fetch them via DB directly if we need to.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let count: i64 = session
        .raw_query_one(
            "SELECT count(*) AS RES_ FROM historic_variable_instances WHERE data LIKE '%my_var%'",
            flowable_engine::persistence::DbParams::new(),
        )
        .unwrap()
        .and_then(|r| r.get_integer("RES_"))
        .unwrap_or(0);
    assert_eq!(count, 1, "Variable should be recorded");

    let audit_count: i64 = session
        .raw_query_one(
            "SELECT count(*) AS RES_ FROM historic_audit_logs",
            flowable_engine::persistence::DbParams::new(),
        )
        .unwrap()
        .and_then(|r| r.get_integer("RES_"))
        .unwrap_or(0);
    assert!(
        audit_count >= 2,
        "Audit logs should record start and complete"
    );

    let _ = session.rollback();
}
