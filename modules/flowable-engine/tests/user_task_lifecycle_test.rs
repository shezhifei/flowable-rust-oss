use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

#[test]
fn test_user_task_lifecycle_complete_by_task_id() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="userTaskLifecycle" name="User Task Lifecycle">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("User Task Lifecycle Deployment".to_string())
        .add_string("userTaskLifecycle.bpmn20.xml".to_string(), xml.to_string());

    let deployment = repository_service.deploy(builder).unwrap();
    assert_eq!(
        deployment.name.as_deref(),
        Some("User Task Lifecycle Deployment")
    );

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Lifecycle Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let task = &tasks[0];
    assert_eq!(task.process_instance_id, process_instance.id);
    assert_eq!(task.execution_id, process_instance.id);
    assert_eq!(task.task_definition_key, "userTask1");
    assert_eq!(task.name, "Approve Request");
    assert!(!task.is_completed);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .snapshot_executions(&mut session)
        .get(&process_instance.id)
        .cloned()
        .expect("execution should be persisted after start");
    assert!(!execution.is_active);
    assert_eq!(execution.activity_id.as_deref(), Some("userTask1"));

    session.rollback().unwrap();
    drop(session);

    task_service.complete_task_by_id(task.id.clone()).unwrap();

    let remaining_tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(remaining_tasks.is_empty());
}

#[test]
fn user_task_skip_expression_true_skips_to_outgoing_flow() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="userTaskSkipExpression" name="User Task Skip Expression">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Skippable Task" flowable:skipExpression="${shouldSkip}" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("User Task Skip Expression Deployment".to_string())
        .add_string(
            "userTaskSkipExpression.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Skip Expression True Instance".to_string())
        .variable("_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(), json!(true))
        .variable("shouldSkip".to_string(), json!(true));

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks.is_empty(),
        "skipExpression=true should not create a user task"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be persisted");
    assert!(
        stored_pi.is_ended,
        "skipped user task should continue to the end event"
    );
}

#[test]
fn user_task_skip_expression_false_preserves_wait_state() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="userTaskSkipExpressionFalse" name="User Task Skip Expression False">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Skippable Task" flowable:skipExpression="${shouldSkip}" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("User Task Skip Expression False Deployment".to_string())
        .add_string(
            "userTaskSkipExpressionFalse.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Skip Expression False Instance".to_string())
        .variable("_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(), json!(true))
        .variable("shouldSkip".to_string(), json!(false));

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "userTask1");
    assert_eq!(tasks[0].name, "Skippable Task");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be persisted");
    assert!(
        !stored_pi.is_ended,
        "skipExpression=false should keep the user task wait-state"
    );
}

#[test]
fn user_task_skip_expression_ignored_when_not_enabled() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="userTaskSkipExpressionDisabled" name="User Task Skip Expression Disabled">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Skippable Task" flowable:skipExpression="${shouldSkip}" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("User Task Skip Expression Disabled Deployment".to_string())
        .add_string(
            "userTaskSkipExpressionDisabled.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Skip Expression Disabled Instance".to_string())
        .variable("shouldSkip".to_string(), json!(true));

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "userTask1");
    assert_eq!(tasks[0].name, "Skippable Task");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be persisted");
    assert!(
        !stored_pi.is_ended,
        "disabled skipExpression should keep the user task wait-state"
    );
}
