use flowable_engine::engine::process_engine::ProcessEngine;

fn deploy_parallel_gateway_process(
    repository_service: &flowable_engine::engine::repository_service::RepositoryService,
) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="parallelGatewayProcess" name="Parallel Gateway Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow_start_split" sourceRef="startEvent1" targetRef="parallelGatewaySplit" />

            <parallelGateway id="parallelGatewaySplit" />
            <sequenceFlow id="flow_split_task1" sourceRef="parallelGatewaySplit" targetRef="userTask1" />
            <sequenceFlow id="flow_split_task2" sourceRef="parallelGatewaySplit" targetRef="userTask2" />

            <userTask id="userTask1" name="First Approval" />
            <sequenceFlow id="flow_task1_join" sourceRef="userTask1" targetRef="parallelGatewayJoin" />

            <userTask id="userTask2" name="Second Approval" />
            <sequenceFlow id="flow_task2_join" sourceRef="userTask2" targetRef="parallelGatewayJoin" />

            <parallelGateway id="parallelGatewayJoin" />
            <sequenceFlow id="flow_join_end" sourceRef="parallelGatewayJoin" targetRef="endEvent1" />

            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let deployment = repository_service
        .create_deployment()
        .name("Parallel Gateway Deployment".to_string())
        .add_string(
            "parallelGatewayProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(deployment).unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

fn get_tasks_by_process_instance(
    task_service: &flowable_engine::engine::task_service::TaskService,
    process_instance_id: &str,
) -> Vec<flowable_engine::task::Task> {
    task_service
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
}

#[test]
fn parallel_gateway_waits_for_both_branches_before_continuing() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let process_definition_id = deploy_parallel_gateway_process(&repository_service);

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Parallel Gateway Instance".to_string()),
        )
        .unwrap();

    let initial_tasks = get_tasks_by_process_instance(&task_service, &process_instance.id);
    assert_eq!(initial_tasks.len(), 2);

    let first_task = initial_tasks
        .iter()
        .find(|task| task.task_definition_key == "userTask1")
        .cloned()
        .expect("expected first parallel branch task");

    task_service
        .complete_task_by_id(first_task.id.clone())
        .unwrap();

    let tasks_after_first_complete =
        get_tasks_by_process_instance(&task_service, &process_instance.id);
    assert_eq!(tasks_after_first_complete.len(), 1);
    assert_eq!(
        tasks_after_first_complete[0].task_definition_key,
        "userTask2"
    );

    let mut session = runtime_store.create_session().unwrap();
    let snapshot_after_first = runtime_store.snapshot_executions(&mut session);
    assert!(
        !snapshot_after_first
            .values()
            .any(|execution| execution.activity_id.as_deref() == Some("endEvent1")),
        "parallel join should not reach end event after only one branch completes"
    );
    drop(session);

    let second_task = tasks_after_first_complete[0].clone();
    task_service
        .complete_task_by_id(second_task.id.clone())
        .unwrap();

    let remaining_tasks = get_tasks_by_process_instance(&task_service, &process_instance.id);
    assert!(remaining_tasks.is_empty());

    let mut session = runtime_store.create_session().unwrap();
    let snapshot_after_second = runtime_store.snapshot_executions(&mut session);
    assert!(
        snapshot_after_second.values().any(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance.id.as_str())
                && execution.activity_id.as_deref() == Some("endEvent1")
        }),
        "expected the joined parallel flow to continue to the end event"
    );
}
