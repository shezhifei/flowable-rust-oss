//! P82b — start event initiator variable (G9).
//!
//! Java: ProcessInstanceHelper.java:197-199 + ExecutionEntityManagerImpl.java:298-300.
//! Rust deviation: only write when start_user_id is present (no null write).

use flowable_engine::cmd::trigger_start_event_subscription_cmd::TriggerProcessStartByEventCmd;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;
use serde_json::json;

const INITIATOR_PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="initiatorProcess" isExecutable="true">
        <startEvent id="startEvent" flowable:initiator="initiator" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="userTask" />
        <userTask id="userTask" name="User Task" />
        <sequenceFlow id="flow2" sourceRef="userTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const NO_INITIATOR_PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="noInitiatorProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="userTask" />
        <userTask id="userTask" name="User Task" />
        <sequenceFlow id="flow2" sourceRef="userTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const MESSAGE_INITIATOR_PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <message id="startMsg" name="startWithInitiator" />
    <process id="messageInitiatorProcess" isExecutable="true">
        <startEvent id="messageStart" flowable:initiator="startedBy">
            <messageEventDefinition messageRef="startMsg" />
        </startEvent>
        <sequenceFlow id="flow1" sourceRef="messageStart" targetRef="userTask" />
        <userTask id="userTask" name="User Task" />
        <sequenceFlow id="flow2" sourceRef="userTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

fn deploy(engine: &ProcessEngine, name: &str, xml: &str, resource: &str) -> String {
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name(name.to_string())
                .add_string(resource.to_string(), xml.to_string()),
        )
        .unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

/// initiator + start_user_id → PI variable + history.
#[test]
fn p82b_initiator_and_start_user_writes_pi_variable_and_history() {
    let engine = ProcessEngine::new("p82b-initiator-with-user".to_string());
    let process_definition_id = deploy(
        &engine,
        "p82b initiator",
        INITIATOR_PROCESS_XML,
        "p82b_initiator.bpmn20.xml",
    );

    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .start_user_id("kermit".to_string()),
        )
        .unwrap();

    let initiator = engine
        .get_runtime_service()
        .get_variable(pi.id.clone(), "initiator".to_string())
        .unwrap();
    assert_eq!(
        initiator,
        Some(json!("kermit")),
        "initiator variable must be set on PI scope"
    );

    let historic = engine
        .get_history_service()
        .create_historic_variable_instance_query()
        .process_instance_id(pi.id.clone())
        .variable_name("initiator".to_string())
        .list()
        .unwrap();
    assert_eq!(historic.len(), 1, "initiator must be historicized");
    assert_eq!(historic[0].value, json!("kermit"));
}

/// No initiator attribute → no initiator variable even with start_user_id.
#[test]
fn p82b_no_initiator_attribute_writes_no_variable() {
    let engine = ProcessEngine::new("p82b-no-initiator".to_string());
    let process_definition_id = deploy(
        &engine,
        "p82b no initiator",
        NO_INITIATOR_PROCESS_XML,
        "p82b_no_initiator.bpmn20.xml",
    );

    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .start_user_id("kermit".to_string()),
        )
        .unwrap();

    let initiator = engine
        .get_runtime_service()
        .get_variable(pi.id.clone(), "initiator".to_string())
        .unwrap();
    assert_eq!(initiator, None, "no initiator attribute → no variable");
}

/// Initiator present but no start_user_id → do not write (Rust deviation).
#[test]
fn p82b_initiator_without_start_user_does_not_write() {
    let engine = ProcessEngine::new("p82b-initiator-no-user".to_string());
    let process_definition_id = deploy(
        &engine,
        "p82b initiator no user",
        INITIATOR_PROCESS_XML,
        "p82b_initiator_no_user.bpmn20.xml",
    );

    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let initiator = engine
        .get_runtime_service()
        .get_variable(pi.id.clone(), "initiator".to_string())
        .unwrap();
    assert_eq!(
        initiator, None,
        "Rust: without start_user_id, initiator is not written"
    );
}

/// Message start path with start_user_id covers override start_event_id + initiator.
#[test]
fn p82b_message_start_path_writes_initiator() {
    let engine = ProcessEngine::new("p82b-message-initiator".to_string());
    deploy(
        &engine,
        "p82b message initiator",
        MESSAGE_INITIATOR_PROCESS_XML,
        "p82b_message_initiator.bpmn20.xml",
    );

    let cmd = TriggerProcessStartByEventCmd::new(
        EventSubscriptionKind::Message,
        "startWithInitiator".to_string(),
    )
    .with_start_user_id("fozzie".to_string());
    let pi = engine
        .get_command_executor()
        .execute(&cmd)
        .expect("message start should succeed");

    let started_by = engine
        .get_runtime_service()
        .get_variable(pi.id.clone(), "startedBy".to_string())
        .unwrap();
    assert_eq!(
        started_by,
        Some(json!("fozzie")),
        "message start with start_user_id must set initiator variable"
    );

    let historic = engine
        .get_history_service()
        .create_historic_variable_instance_query()
        .process_instance_id(pi.id.clone())
        .variable_name("startedBy".to_string())
        .list()
        .unwrap();
    assert_eq!(historic.len(), 1);
    assert_eq!(historic[0].value, json!("fozzie"));
}
