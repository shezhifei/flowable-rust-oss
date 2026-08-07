use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;

#[test]
fn test_error_boundary_event_baseline() {
    let engine = ProcessEngine::new("error-boundary-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="errorProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="subProcess" />
        
        <subProcess id="subProcess">
            <startEvent id="subStart" />
            <sequenceFlow id="subF1" sourceRef="subStart" targetRef="subTask" />
            <userTask id="subTask" />
            <sequenceFlow id="subF2" sourceRef="subTask" targetRef="throwError" />
            <endEvent id="throwError">
                <errorEventDefinition errorRef="error1" />
            </endEvent>
        </subProcess>
        
        <boundaryEvent id="catchError" attachedToRef="subProcess">
            <errorEventDefinition errorRef="error1" />
        </boundaryEvent>
        
        <sequenceFlow id="f2" sourceRef="catchError" targetRef="errorTask" />
        <userTask id="errorTask" />
        <sequenceFlow id="f3" sourceRef="errorTask" targetRef="end" />
        
        <sequenceFlow id="f4" sourceRef="subProcess" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("error.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("errorProcess".to_string());

    let instance = runtime_service.start_process_instance(pi_builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key.as_str(), "subTask");

    // Complete the task, triggering the error
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // Verify error was caught and routed to errorTask
    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should have 1 task (errorTask)");
    assert_eq!(tasks[0].task_definition_key.as_str(), "errorTask");
}

#[test]
fn test_error_boundary_error_code_exact_handler_beats_catch_all() {
    let engine = ProcessEngine::new("error-code-boundary-priority-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <error id="businessError" errorCode="BUSINESS" />
      <process id="errorCodePriorityProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="subProcess" />

        <subProcess id="subProcess">
            <startEvent id="subStart" />
            <sequenceFlow id="subF1" sourceRef="subStart" targetRef="subTask" />
            <userTask id="subTask" />
            <sequenceFlow id="subF2" sourceRef="subTask" targetRef="throwBusinessError" />
            <endEvent id="throwBusinessError">
                <errorEventDefinition errorRef="businessError" />
            </endEvent>
        </subProcess>

        <boundaryEvent id="catchAnyError" attachedToRef="subProcess">
            <errorEventDefinition />
        </boundaryEvent>
        <sequenceFlow id="anyFlow" sourceRef="catchAnyError" targetRef="catchAllTask" />
        <userTask id="catchAllTask" />
        <sequenceFlow id="anyEndFlow" sourceRef="catchAllTask" targetRef="end" />

        <boundaryEvent id="catchBusinessError" attachedToRef="subProcess">
            <errorEventDefinition errorCode="BUSINESS" />
        </boundaryEvent>
        <sequenceFlow id="businessFlow" sourceRef="catchBusinessError" targetRef="businessTask" />
        <userTask id="businessTask" />
        <sequenceFlow id="businessEndFlow" sourceRef="businessTask" targetRef="end" />

        <sequenceFlow id="normalFlow" sourceRef="subProcess" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service.create_deployment().add_string(
        "error-code-priority.bpmn20.xml".to_string(),
        bpmn_xml.to_string(),
    );
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("errorCodePriorityProcess".to_string());

    let instance = runtime_service.start_process_instance(pi_builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key.as_str(), "subTask");

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "the thrown BUSINESS error should be caught once"
    );
    assert_eq!(
        tasks[0].task_definition_key.as_str(),
        "businessTask",
        "an exact errorCode boundary must win over the catch-all boundary"
    );
}

#[test]
fn test_cancel_boundary_transaction_baseline() {
    let engine = ProcessEngine::new("cancel-boundary-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="cancelProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="transaction" />
        
        <transaction id="transaction">
            <startEvent id="txStart" />
            <sequenceFlow id="txF1" sourceRef="txStart" targetRef="txTask" />
            <userTask id="txTask" />
            <sequenceFlow id="txF2" sourceRef="txTask" targetRef="throwCancel" />
            <endEvent id="throwCancel">
                <cancelEventDefinition />
            </endEvent>
        </transaction>
        
        <boundaryEvent id="catchCancel" attachedToRef="transaction">
            <cancelEventDefinition />
        </boundaryEvent>
        
        <sequenceFlow id="f2" sourceRef="catchCancel" targetRef="cancelTask" />
        <userTask id="cancelTask" />
        <sequenceFlow id="f3" sourceRef="cancelTask" targetRef="end" />
        
        <sequenceFlow id="f4" sourceRef="transaction" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("cancel.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("cancelProcess".to_string());

    let instance = runtime_service.start_process_instance(pi_builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key.as_str(), "txTask");

    // Complete the task, triggering the cancel
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // Verify cancel was caught and routed to cancelTask
    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should have 1 task (cancelTask)");
    assert_eq!(tasks[0].task_definition_key.as_str(), "cancelTask");
}

#[test]
fn test_unsupported_boundary_variants() {
    let engine = ProcessEngine::new("unsupported-boundary-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="unsupportedProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="subProcess" />
        
        <subProcess id="subProcess">
            <startEvent id="subStart" />
            <sequenceFlow id="subF1" sourceRef="subStart" targetRef="subTask" />
            <userTask id="subTask" />
            <sequenceFlow id="subF2" sourceRef="subTask" targetRef="subEnd" />
            <endEvent id="subEnd" />
        </subProcess>
        
        <boundaryEvent id="catchLink" attachedToRef="subProcess">
            <linkEventDefinition name="LinkA" />
        </boundaryEvent>
        
        <sequenceFlow id="f2" sourceRef="catchLink" targetRef="end" />
        <sequenceFlow id="f4" sourceRef="subProcess" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("unsupported.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("unsupportedProcess".to_string());

    let res = runtime_service.start_process_instance(pi_builder);

    assert!(res.is_err());
    if let Err(FlowableError::UnsupportedElement {
        element_type,
        activity_id: _,
    }) = res
    {
        assert!(
            element_type.contains("BoundaryEvent") || element_type.contains("LinkEventDefinition")
        );
    } else {
        panic!("Expected UnsupportedElement error");
    }
}
