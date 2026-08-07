use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use serde_json::json;

fn receive_task_skip_expression_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="{process_id}" name="Receive Task Skip Expression">
    <startEvent id="startEvent1" />
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
    <receiveTask id="receiveTask1" name="Wait for callback" flowable:skipExpression="${{shouldSkip}}" />
    <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#
    )
}

fn deploy_receive_task_process(process_engine: &ProcessEngine, process_id: &str) -> String {
    let repository_service = process_engine.get_repository_service();
    let xml = receive_task_skip_expression_xml(process_id);
    let builder = repository_service
        .create_deployment()
        .add_string(format!("{process_id}.bpmn20.xml"), xml);

    repository_service.deploy(builder).unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

#[test]
fn receive_task_skip_expression_true_skips_wait_state_and_takes_outgoing_flow() {
    let process_engine = ProcessEngine::new("default".to_string());
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let process_definition_id =
        deploy_receive_task_process(&process_engine, "receiveTaskSkipExpressionTrue");

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(), json!(true))
                .variable("shouldSkip".to_string(), json!(true)),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks.is_empty(),
        "skipExpression=true should not create a receive task"
    );

    let wait_states =
        task_service.get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert!(
        wait_states.is_empty(),
        "skipExpression=true should not create a receive-task wait state"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be persisted");
    assert!(
        stored_pi.is_ended,
        "skipped receive task should continue to the end event"
    );
}

#[test]
fn receive_task_skip_expression_false_preserves_wait_state() {
    let process_engine = ProcessEngine::new("default".to_string());
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let process_definition_id =
        deploy_receive_task_process(&process_engine, "receiveTaskSkipExpressionFalse");

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(), json!(true))
                .variable("shouldSkip".to_string(), json!(false)),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "receiveTask1");
    assert_eq!(tasks[0].name, "Wait for callback");

    let wait_states =
        task_service.get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(wait_states.len(), 1);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be persisted");
    assert!(
        !stored_pi.is_ended,
        "skipExpression=false should keep the receive task wait-state"
    );
}

#[test]
fn receive_task_skip_expression_ignored_when_not_enabled() {
    let process_engine = ProcessEngine::new("default".to_string());
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let process_definition_id =
        deploy_receive_task_process(&process_engine, "receiveTaskSkipExpressionDisabled");

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("shouldSkip".to_string(), json!(true)),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "receiveTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be persisted");
    assert!(
        !stored_pi.is_ended,
        "disabled skipExpression should keep the receive task wait-state"
    );
}

#[test]
fn receive_task_skip_expression_requires_boolean_result() {
    let process_engine = ProcessEngine::new("default".to_string());
    let runtime_service = process_engine.get_runtime_service();
    let process_definition_id =
        deploy_receive_task_process(&process_engine, "receiveTaskSkipExpressionInvalid");

    let result = runtime_service.start_process_instance(
        runtime_service
            .create_process_instance_builder()
            .process_definition_id(process_definition_id)
            .variable("_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(), json!(true))
            .variable("shouldSkip".to_string(), json!("yes")),
    );

    match result {
        Err(FlowableError::ExecutionError(message)) => {
            assert!(message.contains("ReceiveTask 'receiveTask1' skipExpression"));
            assert!(message.contains("must evaluate to a boolean"));
        }
        other => panic!("expected non-boolean skipExpression to fail, got {other:?}"),
    }
}
