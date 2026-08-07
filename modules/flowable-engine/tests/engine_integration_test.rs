use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_deploy_and_start_process_instance() {
    // 1. Create ProcessEngine
    let process_engine = ProcessEngine::new("default".to_string());

    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    // 2. Mock BPMN XML content
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="myProcess" name="My First Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="First Task" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="exclusiveGateway1" />
            
            <exclusiveGateway id="exclusiveGateway1" />
            <sequenceFlow id="flow3" sourceRef="exclusiveGateway1" targetRef="parallelGateway1">
                <conditionExpression><![CDATA[${approved == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="flow4" sourceRef="exclusiveGateway1" targetRef="end1">
                <conditionExpression><![CDATA[${approved == false}]]></conditionExpression>
            </sequenceFlow>

            <parallelGateway id="parallelGateway1" />
            <sequenceFlow id="flow5" sourceRef="parallelGateway1" targetRef="userTask2" />
            <sequenceFlow id="flow6" sourceRef="parallelGateway1" targetRef="serviceTask1" />

            <userTask id="userTask2" name="Second Task" />
            <sequenceFlow id="flow7" sourceRef="userTask2" targetRef="end2" />

            <serviceTask id="serviceTask1" name="Service Task" />
            <sequenceFlow id="flow8" sourceRef="serviceTask1" targetRef="end3" />

            <endEvent id="end1" />
            <endEvent id="end2" />
            <endEvent id="end3" />
        </process>
    </definitions>"#;

    // 3. Deploy process definition
    let builder = repository_service
        .create_deployment()
        .name("My First Deployment".to_string())
        .add_string("myProcess.bpmn20.xml".to_string(), xml.to_string());

    let deployment = repository_service.deploy(builder).unwrap();

    assert_eq!(deployment.name.as_deref(), Some("My First Deployment"));

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // 4. Start Process Instance
    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("Test Instance".to_string())
        .variable(
            "someVar".to_string(),
            serde_json::Value::String("hello".to_string()),
        )
        .variable("approved".to_string(), serde_json::Value::Bool(true)); // Needed for exclusive gateway

    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    // 5. Assertions on start
    assert_eq!(
        process_instance.process_definition_id,
        process_definition_id
    );
    assert_eq!(process_instance.name.as_deref(), Some("Test Instance"));
    // Single storage: start variables live on the process-instance scope
    // execution row and are read back through the runtime service.
    assert_eq!(
        runtime_service
            .get_variable(process_instance.id.clone(), "someVar".to_string())
            .unwrap(),
        Some(serde_json::Value::String("hello".to_string()))
    );

    // 6. Complete the user task 1 (which triggers the exclusive gateway -> parallel gateway -> split to userTask2 and serviceTask1)
    let task_service = process_engine.get_task_service();
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "userTask1");

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 1);
    assert_eq!(tasks_after[0].task_definition_key, "userTask2");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(!pi_after.is_ended);
}
