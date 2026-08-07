//! P24 sub-item 1: getDataObjects / getDataObjectsLocal (runtime + task).
//! Java parity: DataObjectsTest.testRetrieveDataObjectsFromNestedSubprocess.

use flowable_engine::engine::process_engine::ProcessEngine;

const DATA_OBJECTS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="http://www.flowable.org/Test">
    <process id="DataObjectsTest" name="DataObjectsTest" isExecutable="true">
        <dataObject itemSubjectRef="xsd:string" name="VariableA" id="doA"/>
        <dataObject itemSubjectRef="xsd:string" name="VariableB" id="doB"/>
        <startEvent id="startevent1" name="Start"/>
        <endEvent id="endevent1" name="End"/>
        <subProcess id="subProcess1" name="SubProcess">
            <dataObject itemSubjectRef="xsd:string" name="VariableB" id="doB1"/>
            <dataObject itemSubjectRef="xsd:string" name="VariableC" id="doC"/>
            <startEvent id="startevent2" name="Start"/>
            <endEvent id="endevent2" name="End"/>
            <subProcess id="subProcess2" name="NestedSubProcess">
                <dataObject itemSubjectRef="xsd:string" name="VariableC" id="doC2"/>
                <dataObject itemSubjectRef="xsd:string" name="VariableD" id="doD"/>
                <startEvent id="startevent3" name="Start"/>
                <endEvent id="endevent3" name="End"/>
                <userTask id="usertask2" name="Task B"/>
                <sequenceFlow id="Start3" sourceRef="startevent3" targetRef="usertask2"/>
                <sequenceFlow id="Done3" sourceRef="usertask2" targetRef="endevent3"/>
            </subProcess>
            <sequenceFlow id="Start2" sourceRef="startevent2" targetRef="subProcess2"/>
            <sequenceFlow id="Done2" sourceRef="subProcess2" targetRef="endevent2"/>
        </subProcess>
        <sequenceFlow id="Start1" sourceRef="startevent1" targetRef="subProcess1"/>
        <sequenceFlow id="Done1" sourceRef="subProcess1" targetRef="endevent1"/>
    </process>
</definitions>"#;

#[test]
fn test_get_data_objects_from_nested_subprocess() {
    let engine = ProcessEngine::new("default".to_string());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_svc = engine.get_task_service();

    repo.deploy(
        repo.create_deployment()
            .add_string("data-objects.bpmn20.xml".to_string(), DATA_OBJECTS_XML.to_string()),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime.start_process_instance_by_id(def_id, None).unwrap();

    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key.as_str(), "usertask2");

    // Process instance scope: VariableA, VariableB only.
    let data_objects = runtime.get_data_objects(pi.id.clone()).unwrap();
    assert_eq!(data_objects.len(), 2);
    assert!(data_objects.contains_key("VariableA"));
    assert!(data_objects.contains_key("VariableB"));
    assert!(runtime
        .get_data_object(pi.id.clone(), "VariableA".to_string())
        .unwrap()
        .is_some());
    assert!(runtime
        .get_data_object(pi.id.clone(), "VariableZ".to_string())
        .unwrap()
        .is_none());

    // Local on process instance same set.
    let local = runtime.get_data_objects_local(pi.id.clone()).unwrap();
    assert_eq!(local.len(), 2);

    // Task execution can see A,B,C,D (nested scope visibility).
    let task_execution_id = tasks[0].execution_id.clone();
    let from_task_exec = runtime
        .get_data_objects(task_execution_id.clone())
        .unwrap();
    assert!(
        from_task_exec.contains_key("VariableA")
            && from_task_exec.contains_key("VariableB")
            && from_task_exec.contains_key("VariableC")
            && from_task_exec.contains_key("VariableD"),
        "expected A-D from nested task execution, got {:?}",
        from_task_exec.keys().collect::<Vec<_>>()
    );

    // TaskService entry
    let task_dos = task_svc.get_data_objects(tasks[0].id.clone()).unwrap();
    assert_eq!(task_dos.len(), 4);
    assert!(task_svc
        .get_data_object(tasks[0].id.clone(), "VariableD".to_string())
        .unwrap()
        .is_some());
    assert!(task_svc
        .get_data_object(tasks[0].id.clone(), "VariableZ".to_string())
        .unwrap()
        .is_none());

    // Null validation
    assert!(runtime
        .get_data_object("".to_string(), "VariableA".to_string())
        .is_err());
    assert!(runtime
        .get_data_object(pi.id.clone(), "".to_string())
        .is_err());
    assert!(task_svc
        .get_data_object("".to_string(), "VariableA".to_string())
        .is_err());
}

#[test]
fn test_get_data_objects_local_only_current_scope() {
    let engine = ProcessEngine::new("default".to_string());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_svc = engine.get_task_service();

    repo.deploy(
        repo.create_deployment()
            .add_string("data-objects.bpmn20.xml".to_string(), DATA_OBJECTS_XML.to_string()),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime.start_process_instance_by_id(def_id, None).unwrap();
    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();

    // Local on process instance: only main-process definitions present as vars.
    let local_root = runtime.get_data_objects_local(pi.id.clone()).unwrap();
    assert_eq!(local_root.len(), 2);
    assert!(local_root.contains_key("VariableA"));
    assert!(local_root.contains_key("VariableB"));
    assert!(!local_root.contains_key("VariableC"));
    assert!(!local_root.contains_key("VariableD"));

    // Task execution local: may be empty if vars live on ancestor scopes.
    let local_task_exec = runtime
        .get_data_objects_local(tasks[0].execution_id.clone())
        .unwrap();
    // Nested data objects are on subprocess scopes, not the leaf task execution.
    assert!(
        !local_task_exec.contains_key("VariableA"),
        "process-level VariableA must not appear as local on task execution"
    );
}
