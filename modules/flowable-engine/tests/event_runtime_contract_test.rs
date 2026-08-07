use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn test_transaction_compensation_e2e() {
    let config = flowable_engine::service::config::ProcessEngineConfiguration::default();
    let engine = ProcessEngine::new_with_config("event-runtime-contract-test".to_string(), config);

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let runtime_store = engine.get_runtime_store();

    let bpmn_xml = r#"
    <definitions targetNamespace="http://flowable.org/bpmn">
      <process id="transactionCompProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="tx" />
        
        <transaction id="tx">
            <startEvent id="txStart" />
            <sequenceFlow id="txF1" sourceRef="txStart" targetRef="doWork" />
            
            <userTask id="doWork" />
            
            <boundaryEvent id="compBoundary" attachedToRef="doWork">
                <compensateEventDefinition />
            </boundaryEvent>
            
            <userTask id="undoWork" isForCompensation="true" />
            
            <sequenceFlow id="txF2" sourceRef="doWork" targetRef="throwCancel" />
            
            <endEvent id="throwCancel">
                <cancelEventDefinition />
            </endEvent>
            
            <association sourceRef="compBoundary" targetRef="undoWork" />
        </transaction>
        
        <boundaryEvent id="catchCancel" attachedToRef="tx">
            <cancelEventDefinition />
        </boundaryEvent>
        
        <sequenceFlow id="f2" sourceRef="catchCancel" targetRef="cancelledTask" />
        <userTask id="cancelledTask" />
        <sequenceFlow id="f3" sourceRef="cancelledTask" targetRef="end" />
        
        <sequenceFlow id="f4" sourceRef="tx" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;

    let builder = repository_service
        .create_deployment()
        .add_string("tx.bpmn20.xml".to_string(), bpmn_xml.to_string());
    repository_service.deploy(builder).unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .name("transactionCompProcess".to_string());

    let instance = runtime_service.start_process_instance(pi_builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key.as_str(), "doWork");

    // Complete doWork. This should trigger TakeOutgoingSequenceFlowsOperation which should register compensation.
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // After doWork, it hits throwCancel, which triggers compensation (undoWork)
    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();

    // It should be at undoWork
    assert_eq!(tasks.len(), 1, "Should be at undoWork for compensation");
    assert_eq!(tasks[0].task_definition_key.as_str(), "undoWork");

    // Complete compensation task
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // After compensation is done, the transaction is cancelled, caught by catchCancel -> cancelledTask
    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at cancelledTask");
    assert_eq!(tasks[0].task_definition_key.as_str(), "cancelledTask");

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&instance.id, &mut session)
        .unwrap();
    assert!(pi.is_ended, "Process should be ended");
}
