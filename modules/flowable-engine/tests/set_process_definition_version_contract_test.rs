//! Contract tests for `SetProcessDefinitionVersionCmd`, mirroring Java
//! `org.flowable.engine.impl.cmd.SetProcessDefinitionVersionCmd` semantics:
//!   - constructor validation -> FlowableIllegalArgumentException (BadRequest);
//!   - unknown process instance -> FlowableObjectNotFoundException (NotFound);
//!   - child execution id -> FlowableIllegalArgumentException (BadRequest);
//!   - unknown target version -> FlowableObjectNotFoundException (NotFound);
//!   - current activity missing in new version -> FlowableException (ExecutionError);
//!   - happy path switches runtime executions and the historic process instance.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::error::FlowableError;
use flowable_engine::runtime::process_instance::ProcessInstance;

fn user_task_xml(task_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="setVersionProcess" name="Set Version Process">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="{task_id}" />
            <userTask id="{task_id}" name="Task" />
            <sequenceFlow id="f2" sourceRef="{task_id}" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

fn deploy(engine: &ProcessEngine, xml: String) {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("setVersionProcess.bpmn20.xml".to_string(), xml),
    )
    .unwrap();
}

fn definition_id_for_version(engine: &ProcessEngine, version: i32) -> String {
    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.split(':').nth(1).and_then(|v| v.parse::<i32>().ok()) == Some(version))
        .expect("process definition with requested version should be deployed")
}

fn deploy_and_start(engine: &ProcessEngine) -> ProcessInstance {
    deploy(engine, user_task_xml("task1"));
    let definition_id = definition_id_for_version(engine, 1);
    engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap()
}

#[test]
fn blank_process_instance_id_rejected_with_bad_request() {
    let engine = ProcessEngine::new("set-version-blank-id".to_string());
    let error = engine
        .get_runtime_service()
        .set_process_definition_version("", 1)
        .expect_err("Java constructor rejects a blank process instance id");
    assert!(matches!(error, FlowableError::BadRequest(_)));
    assert!(
        error
            .to_string()
            .contains("process instance id is mandatory")
    );
}

#[test]
fn non_positive_version_rejected_with_bad_request() {
    let engine = ProcessEngine::new("set-version-non-positive".to_string());
    let error = engine
        .get_runtime_service()
        .set_process_definition_version("some-instance", 0)
        .expect_err("Java constructor rejects versions < 1");
    assert!(matches!(error, FlowableError::BadRequest(_)));
    assert!(error.to_string().contains("must be positive"));
}

#[test]
fn unknown_process_instance_yields_not_found() {
    let engine = ProcessEngine::new("set-version-unknown-instance".to_string());
    let error = engine
        .get_runtime_service()
        .set_process_definition_version("does-not-exist", 1)
        .expect_err("Java raises FlowableObjectNotFoundException for unknown instances");
    assert!(matches!(error, FlowableError::NotFound(_)));
    assert!(
        error
            .to_string()
            .contains("No process instance found for id = 'does-not-exist'")
    );
}

#[test]
fn child_execution_id_rejected_with_bad_request() {
    let engine = ProcessEngine::new("set-version-child-execution".to_string());

    // A parallel split produces child executions with their own ids, which is
    // what Java's "points to a child execution" guard is about.
    let parallel_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="setVersionParallelProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="split" />
            <parallelGateway id="split" />
            <sequenceFlow id="f2" sourceRef="split" targetRef="task1" />
            <sequenceFlow id="f3" sourceRef="split" targetRef="task2" />
            <userTask id="task1" />
            <userTask id="task2" />
        </process>
    </definitions>"#;
    deploy(&engine, parallel_xml.to_string());
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let child_execution_id = runtime_store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| {
            execution.process_instance_id.as_deref() == Some(instance.id.as_str())
                && execution.id != instance.id
        })
        .map(|execution| execution.id)
        .expect("parallel process should have a child execution");

    let error = engine
        .get_runtime_service()
        .set_process_definition_version(&child_execution_id, 1)
        .expect_err("Java raises FlowableIllegalArgumentException for child execution ids");
    assert!(matches!(error, FlowableError::BadRequest(_)));
    assert!(error.to_string().contains("child execution"));
    assert!(error.to_string().contains(&instance.id));
}

#[test]
fn unknown_target_version_yields_not_found() {
    let engine = ProcessEngine::new("set-version-unknown-version".to_string());
    let instance = deploy_and_start(&engine);

    let error = engine
        .get_runtime_service()
        .set_process_definition_version(&instance.id, 99)
        .expect_err("Java DeploymentManager raises FlowableObjectNotFoundException");
    assert!(matches!(error, FlowableError::NotFound(_)));
    assert!(
        error
            .to_string()
            .contains("no processes deployed with key = 'setVersionProcess' and version = '99'")
    );
}

#[test]
fn missing_current_activity_in_new_version_yields_execution_error() {
    let engine = ProcessEngine::new("set-version-missing-activity".to_string());
    let instance = deploy_and_start(&engine);

    // Version 2 renames the wait-state activity, so the running instance's
    // current activity no longer exists in the target version.
    deploy(&engine, user_task_xml("renamedTask"));

    let error = engine
        .get_runtime_service()
        .set_process_definition_version(&instance.id, 2)
        .expect_err("Java raises FlowableException when the current activity is missing");
    assert!(matches!(error, FlowableError::ExecutionError(_)));
    assert!(
        error
            .to_string()
            .contains("does not contain the current activity")
    );

    // Nothing may have been switched.
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let unchanged = runtime_store
        .find_process_instance(&instance.id, &mut session)
        .unwrap();
    assert_eq!(
        unchanged.process_definition_id,
        instance.process_definition_id
    );
    assert_eq!(unchanged.process_definition_version, 1);
}

#[test]
fn switches_instance_executions_and_history_to_target_version() {
    let engine = ProcessEngine::new("set-version-happy-path".to_string());
    let instance = deploy_and_start(&engine);
    assert_eq!(instance.process_definition_version, 1);

    // Version 2 keeps the same activity ids, so the switch is allowed.
    deploy(&engine, user_task_xml("task1"));
    let target_definition_id = definition_id_for_version(&engine, 2);

    engine
        .get_runtime_service()
        .set_process_definition_version(&instance.id, 2)
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    let switched = runtime_store
        .find_process_instance(&instance.id, &mut session)
        .unwrap();
    assert_eq!(switched.process_definition_id, target_definition_id);
    assert_eq!(switched.process_definition_version, 2);
    assert_eq!(switched.process_definition_key, "setVersionProcess");

    let executions: Vec<_> = runtime_store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|execution| execution.process_instance_id.as_deref() == Some(instance.id.as_str()))
        .collect();
    assert!(!executions.is_empty());
    for execution in &executions {
        assert_eq!(
            execution.process_definition_id.as_deref(),
            Some(target_definition_id.as_str()),
            "execution '{}' should reference the target definition",
            execution.id
        );
    }

    // Java `HistoryManager.recordProcessDefinitionChange` parity.
    let historic = engine
        .get_history_service()
        .create_historic_process_instance_query()
        .process_instance_id(instance.id.clone())
        .single_result()
        .unwrap()
        .expect("historic process instance should exist");
    assert_eq!(historic.process_definition_id, target_definition_id);

    // The wait state stays operational after the switch: the task can still
    // be completed and the instance ends normally.
    let task_service = engine.get_task_service();
    let tasks = task_service
        .get_tasks_by_process_instance_id(instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let finished = runtime_store
        .find_process_instance(&instance.id, &mut session)
        .expect("process instance row should still exist");
    assert!(
        finished.is_ended,
        "process instance should be ended after completing the task"
    );
}
