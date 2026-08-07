use flowable_engine::engine::process_engine::ProcessEngine;

const ADHOC_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="adhocProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="adhoc" />
        <adHocSubProcess id="adhoc">
            <startEvent id="adhocStart" />
            <sequenceFlow id="adhocFlow1" sourceRef="adhocStart" targetRef="adhocTask" />
            <userTask id="adhocTask" name="Adhoc Task" />
            <sequenceFlow id="adhocFlow2" sourceRef="adhocTask" targetRef="adhocEnd" />
            <endEvent id="adhocEnd" />
        </adHocSubProcess>
        <sequenceFlow id="flow2" sourceRef="adhoc" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const ADHOC_MANUAL_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="adhocManualProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="adhocSubProcess" />
        <adHocSubProcess id="adhocSubProcess">
            <userTask id="innerUserTask" name="Inner User Task" />
        </adHocSubProcess>
        <sequenceFlow id="flow2" sourceRef="adhocSubProcess" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

#[test]
fn test_adhoc_subprocess_runtime_semantics() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .add_string("adhoc.bpmn20.xml".to_string(), ADHOC_XML.to_string());
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance_by_id(process_def_id, None)
        .unwrap();

    // 1. Inside adhoc (acting like a subprocess for now)
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Adhoc Task");

    // 2. Complete task in adhoc
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // 3. Process should end (because adhoc completed)
    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(pi.is_ended);
}

#[test]
fn test_adhoc_subprocess_without_inner_start_event_manual_activation_completes_parent() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service.create_deployment().add_string(
        "adhoc-manual.bpmn20.xml".to_string(),
        ADHOC_MANUAL_XML.to_string(),
    );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance_by_id(process_def_id, None)
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks.is_empty(),
        "ad-hoc subprocess without an inner start event should wait for manual activation"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let adhoc_execution_id = runtime_store
        .snapshot_executions(&mut session)
        .values()
        .find(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance.id.as_str())
                && execution.activity_id.as_deref() == Some("adhocSubProcess")
        })
        .expect("ad-hoc subprocess execution should be waiting")
        .id
        .clone();
    drop(session);

    runtime_service
        .activate_adhoc_task(&adhoc_execution_id, "innerUserTask")
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "innerUserTask");
    assert_eq!(tasks[0].name, "Inner User Task");

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // Java: completing the last ad-hoc activity does not leave the ad-hoc
    // unless a completion condition fires; the engine API must complete it.
    runtime_service
        .complete_adhoc_subprocess(&adhoc_execution_id)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(
        pi.is_ended,
        "completing the manually activated task should complete the ad-hoc subprocess outgoing flow"
    );
}
