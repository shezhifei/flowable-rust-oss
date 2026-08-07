use flowable_engine::engine::process_engine::ProcessEngine;

const TRANSACTION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="transactionProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="tx" />
        <transaction id="tx">
            <startEvent id="txStart" />
            <sequenceFlow id="txFlow1" sourceRef="txStart" targetRef="txTask" />
            <userTask id="txTask" name="Transaction Task" />
            <sequenceFlow id="txFlow2" sourceRef="txTask" targetRef="txEnd" />
            <endEvent id="txEnd" />
        </transaction>
        <sequenceFlow id="flow2" sourceRef="tx" targetRef="afterTx" />
        <userTask id="afterTx" name="After Transaction" />
        <sequenceFlow id="flow3" sourceRef="afterTx" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

#[test]
fn test_transaction_runtime_semantics() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service.create_deployment().add_string(
        "transaction.bpmn20.xml".to_string(),
        TRANSACTION_XML.to_string(),
    );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance_by_id(process_def_id, None)
        .unwrap();

    // 1. Inside transaction
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Transaction Task");

    // 2. Complete task in transaction
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // 3. After transaction
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "After Transaction");

    // 4. Complete final task
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(pi.is_ended);
}
