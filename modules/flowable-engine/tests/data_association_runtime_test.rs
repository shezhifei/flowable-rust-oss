use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_data_association_baseline() {
    let engine = ProcessEngine::new("data-assoc-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let variable_service = engine.get_variable_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="dataProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        
        <userTask id="task1">
            <dataInputAssociation>
                <sourceRef>globalVar</sourceRef>
                <targetRef>taskVar</targetRef>
            </dataInputAssociation>
            <dataOutputAssociation>
                <sourceRef>taskOutVar</sourceRef>
                <targetRef>globalOutVar</targetRef>
            </dataOutputAssociation>
        </userTask>
        
        <sequenceFlow id="f2" sourceRef="task1" targetRef="wait" />
        <userTask id="wait" />
        <sequenceFlow id="f3" sourceRef="wait" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("data.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // In Flowable, variables can be passed to start
    // We haven't fully wired variable mapping, let's just make it fail if unsupported, or pass if we implement a stub.
    // For M8 baseline, we want simple mapping or a deterministic failure.
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("dataProcess".to_string())
        .variable("globalVar".to_string(), serde_json::json!("value1"));

    let instance = runtime_service.start_process_instance(pi_builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    // check input variable is mapped to task scope
    let task_vars = variable_service
        .get_variables(tasks[0].execution_id.clone())
        .unwrap();
    assert_eq!(
        task_vars.get("taskVar").and_then(|v| v.as_str()),
        Some("value1")
    );

    // set output variable
    variable_service
        .set_variable(
            tasks[0].execution_id.clone(),
            "taskOutVar".to_string(),
            serde_json::json!("value2"),
        )
        .unwrap();

    // complete task, which should map output
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // check output variable is mapped back to process scope
    let process_vars = variable_service.get_variables(instance.id.clone()).unwrap();
    assert_eq!(
        process_vars.get("globalOutVar").and_then(|v| v.as_str()),
        Some("value2")
    );
}

#[test]
fn test_unsupported_complex_data_mapping_fails() {
    let engine = ProcessEngine::new("data-assoc-fail-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="dataFailProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        
        <userTask id="task1">
            <dataInputAssociation>
                <!-- Complex assignment is now supported -->
                <assignment>
                    <from>${1 + 1}</from>
                    <to>taskVar</to>
                </assignment>
            </dataInputAssociation>
        </userTask>
        
        <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("data_fail.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("dataFailProcess".to_string());

    let res = runtime_service.start_process_instance(pi_builder);

    // Task G now supports data association assignments
    assert!(
        res.is_ok(),
        "data association with assignments must succeed"
    );
}
