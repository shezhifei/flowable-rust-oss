use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::task_service::MessageStyleWaitKind;

fn start_process_instance(
    xml: &str,
    deployment_name: &str,
    process_name: &str,
) -> (ProcessEngine, String) {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name(deployment_name.to_string())
        .add_string(format!("{}.bpmn20.xml", process_name), xml.to_string());

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name(process_name.to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();
    (process_engine, process_instance.id)
}

fn get_waiting_execution_id(process_engine: &ProcessEngine, process_instance_id: &str) -> String {
    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let result = store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && !execution.is_active
        })
        .map(|execution| execution.id)
        .expect("waiting execution should exist");
    session.rollback().unwrap();
    result
}

#[test]
fn test_intermediate_catch_event_without_event_definitions_waits_for_trigger() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="intermediateCatchProcess" name="Intermediate Catch Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="intermediateCatchEvent1" />
            <intermediateCatchEvent id="intermediateCatchEvent1" name="Catch Without Definitions" />
            <sequenceFlow id="flow2" sourceRef="intermediateCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = start_process_instance(
        xml,
        "Intermediate Catch Deployment",
        "Intermediate Catch Process",
    );

    let runtime_store = process_engine.get_runtime_store();
    let task_service = process_engine.get_task_service();
    let runtime_service = process_engine.get_runtime_service();
    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert!(visible_wait_states.is_empty());

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should be persisted");
    assert!(!stored_pi.is_ended);

    let execution = runtime_store
        .snapshot_executions(&mut session)
        .get(&process_instance_id)
        .cloned()
        .expect("execution should be persisted");
    session.rollback().unwrap();
    assert!(!execution.is_active);
    assert_eq!(
        execution.activity_id.as_deref(),
        Some("intermediateCatchEvent1")
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap();
    assert!(tasks.is_empty());

    runtime_service
        .trigger_intermediate_catch_event_by_process_instance_id(process_instance_id.clone());

    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert!(visible_wait_states.is_empty());

    let mut session2 = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session2)
        .expect("process instance should remain in runtime store");
    session2.rollback().unwrap();
    assert!(stored_pi.is_ended);
}

#[test]
fn test_intermediate_catch_event_with_message_event_definition_exposes_display_name_and_accepts_message_ref()
 {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="intermediateMessageCatchProcess" name="Intermediate Message Catch Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="intermediateCatchEvent1" />
            <intermediateCatchEvent id="intermediateCatchEvent1" name="Catch With Message Definition">
                <messageEventDefinition messageRef="OrderApproved" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="intermediateCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = start_process_instance(
        xml,
        "Intermediate Message Catch Deployment",
        "Intermediate Message Catch Process",
    );

    let runtime_store = process_engine.get_runtime_store();
    let runtime_service = process_engine.get_runtime_service();
    let execution_id = get_waiting_execution_id(&process_engine, &process_instance_id);
    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(visible_wait_states.len(), 1);
    assert_eq!(
        visible_wait_states[0].wait_kind,
        MessageStyleWaitKind::MessageIntermediateCatchEvent
    );
    assert_eq!(
        visible_wait_states[0].process_instance_id,
        process_instance_id.clone()
    );
    assert_eq!(visible_wait_states[0].execution_id, execution_id.clone());
    assert_eq!(
        visible_wait_states[0].message_name.as_deref(),
        Some("Catch With Message Definition")
    );
    assert_eq!(
        visible_wait_states[0].message_ref.as_deref(),
        Some("OrderApproved")
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should be persisted");
    assert!(!stored_pi.is_ended);
    drop(session);

    runtime_service.trigger_intermediate_catch_event_by_message_ref_and_execution_id(
        "OrderApproved".to_string(),
        execution_id,
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should remain in runtime store");
    assert!(stored_pi.is_ended);
    drop(session);

    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert!(visible_wait_states.is_empty());
}

#[test]
fn test_intermediate_catch_event_with_message_event_definition_rejects_wrong_display_name() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="intermediateMessageCatchWrongNameProcess" name="Intermediate Message Catch Wrong Name Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="intermediateCatchEvent1" />
            <intermediateCatchEvent id="intermediateCatchEvent1" name="Catch With Message Definition">
                <messageEventDefinition messageRef="OrderApproved" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="intermediateCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = start_process_instance(
        xml,
        "Intermediate Message Catch Wrong Name Deployment",
        "Intermediate Message Catch Wrong Name Process",
    );

    let runtime_store = process_engine.get_runtime_store();
    let runtime_service = process_engine.get_runtime_service();
    let execution_id = get_waiting_execution_id(&process_engine, &process_instance_id);

    runtime_service.trigger_intermediate_catch_event_by_message_ref_and_execution_id(
        "Catch With Message Definition".to_string(),
        execution_id,
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should remain in runtime store");
    assert!(!stored_pi.is_ended);
    drop(session);

    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .snapshot_executions(&mut session)
        .get(&process_instance_id)
        .cloned()
        .expect("execution should still be present");
    assert!(!execution.is_active);
    drop(session);

    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(visible_wait_states.len(), 1);
    assert_eq!(
        visible_wait_states[0].message_name.as_deref(),
        Some("Catch With Message Definition")
    );
    assert_eq!(
        visible_wait_states[0].message_ref.as_deref(),
        Some("OrderApproved")
    );
}

#[test]
fn test_intermediate_catch_event_with_message_event_definition_rejects_wrong_execution_target() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="intermediateMessageCatchWrongTargetProcess" name="Intermediate Message Catch Wrong Target Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="intermediateCatchEvent1" />
            <intermediateCatchEvent id="intermediateCatchEvent1" name="Catch With Message Definition">
                <messageEventDefinition messageRef="OrderApproved" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="intermediateCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = start_process_instance(
        xml,
        "Intermediate Message Catch Wrong Target Deployment",
        "Intermediate Message Catch Wrong Target Process",
    );

    let runtime_store = process_engine.get_runtime_store();
    let runtime_service = process_engine.get_runtime_service();
    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(visible_wait_states.len(), 1);

    runtime_service.trigger_intermediate_catch_event_by_message_ref_and_execution_id(
        "OrderApproved".to_string(),
        "missing-execution-id".to_string(),
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should remain in runtime store");
    assert!(!stored_pi.is_ended);
    drop(session);

    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .snapshot_executions(&mut session)
        .get(&process_instance_id)
        .cloned()
        .expect("execution should still be present");
    assert!(!execution.is_active);
    drop(session);

    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(visible_wait_states.len(), 1);
}

#[test]
fn test_intermediate_throw_event_without_event_definitions_is_pass_through() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="intermediateThrowProcess" name="Intermediate Throw Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="intermediateThrowEvent1" />
            <intermediateThrowEvent id="intermediateThrowEvent1" name="Throw Without Definitions" />
            <sequenceFlow id="flow2" sourceRef="intermediateThrowEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = start_process_instance(
        xml,
        "Intermediate Throw Deployment",
        "Intermediate Throw Process",
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should be persisted");
    assert!(stored_pi.is_ended);
}

// ============================================================================
// Signal Intermediate Catch Event Tests
// ============================================================================

#[test]
fn test_intermediate_catch_event_with_signal_event_definition_exposes_signal_ref_and_accepts_signal()
 {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="Alert Signal" />
        <process id="intermediateSignalCatchProcess" name="Intermediate Signal Catch Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="signalCatchEvent1" />
            <intermediateCatchEvent id="signalCatchEvent1" name="Catch Alert Signal">
                <signalEventDefinition signalRef="alertSignal" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="signalCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = start_process_instance(
        xml,
        "Intermediate Signal Catch Deployment",
        "Intermediate Signal Catch Process",
    );

    let runtime_store = process_engine.get_runtime_store();
    let runtime_service = process_engine.get_runtime_service();
    let execution_id = get_waiting_execution_id(&process_engine, &process_instance_id);

    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(visible_wait_states.len(), 1);
    assert_eq!(
        visible_wait_states[0].wait_kind,
        MessageStyleWaitKind::SignalIntermediateCatchEvent
    );
    assert_eq!(
        visible_wait_states[0].process_instance_id,
        process_instance_id.clone()
    );
    assert_eq!(visible_wait_states[0].execution_id, execution_id.clone());
    assert_eq!(
        visible_wait_states[0].message_name.as_deref(),
        Some("Catch Alert Signal")
    );
    assert_eq!(
        visible_wait_states[0].signal_ref.as_deref(),
        Some("Alert Signal")
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should be persisted");
    assert!(!stored_pi.is_ended);
    drop(session);

    runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        "Alert Signal".to_string(),
        execution_id,
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should remain in runtime store");
    assert!(stored_pi.is_ended);
    drop(session);

    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert!(visible_wait_states.is_empty());
}

#[test]
fn test_intermediate_catch_event_with_signal_rejects_wrong_signal_ref() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="Alert Signal" />
        <process id="intermediateSignalCatchWrongRefProcess" name="Intermediate Signal Catch Wrong Ref Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="signalCatchEvent1" />
            <intermediateCatchEvent id="signalCatchEvent1" name="Catch Alert Signal">
                <signalEventDefinition signalRef="alertSignal" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="signalCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = start_process_instance(
        xml,
        "Intermediate Signal Catch Wrong Ref Deployment",
        "Intermediate Signal Catch Wrong Ref Process",
    );

    let runtime_store = process_engine.get_runtime_store();
    let runtime_service = process_engine.get_runtime_service();
    let execution_id = get_waiting_execution_id(&process_engine, &process_instance_id);

    // Try triggering with wrong signal_ref - should be no-op
    runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        "wrongSignal".to_string(),
        execution_id.clone(),
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should remain in runtime store");
    assert!(!stored_pi.is_ended);
    drop(session);

    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .snapshot_executions(&mut session)
        .get(&process_instance_id)
        .cloned()
        .expect("execution should still be present");
    assert!(!execution.is_active);
    drop(session);

    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(visible_wait_states.len(), 1);
    assert_eq!(
        visible_wait_states[0].signal_ref.as_deref(),
        Some("Alert Signal")
    );

    // Now trigger with correct signal_ref
    runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        "Alert Signal".to_string(),
        execution_id,
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should be ended");
    assert!(stored_pi.is_ended);
}

#[test]
fn test_intermediate_catch_event_with_signal_rejects_wrong_execution_target() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="Alert Signal" />
        <process id="intermediateSignalCatchWrongTargetProcess" name="Intermediate Signal Catch Wrong Target Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="signalCatchEvent1" />
            <intermediateCatchEvent id="signalCatchEvent1" name="Catch Alert Signal">
                <signalEventDefinition signalRef="alertSignal" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="signalCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = start_process_instance(
        xml,
        "Intermediate Signal Catch Wrong Target Deployment",
        "Intermediate Signal Catch Wrong Target Process",
    );

    let runtime_store = process_engine.get_runtime_store();
    let runtime_service = process_engine.get_runtime_service();
    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(visible_wait_states.len(), 1);

    // Try triggering with wrong execution_id - should be no-op
    runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        "alertSignal".to_string(),
        "missing-execution-id".to_string(),
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .expect("process instance should remain in runtime store");
    assert!(!stored_pi.is_ended);
    drop(session);

    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .snapshot_executions(&mut session)
        .get(&process_instance_id)
        .cloned()
        .expect("execution should still be present");
    assert!(!execution.is_active);
    drop(session);

    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(visible_wait_states.len(), 1);
}

// ============================================================================
// Mixed Message + Signal Intermediate Catch Event Regression Tests
// ============================================================================

/// Verifies that a single process definition can contain both a message and a
/// signal intermediate catch event, and that triggering one leaves the other
/// untouched until it is triggered independently.
#[test]
fn test_mixed_message_and_signal_intermediate_catch_events_independent_trigger() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="Alert Signal" />
        <process id="mixedIceProcess" name="Mixed ICE Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="parallelGateway1" />
            <parallelGateway id="parallelGateway1" />
            <sequenceFlow id="flow2" sourceRef="parallelGateway1" targetRef="messageCatch1" />
            <sequenceFlow id="flow3" sourceRef="parallelGateway1" targetRef="signalCatch1" />
            <intermediateCatchEvent id="messageCatch1" name="Catch Order Approved">
                <messageEventDefinition messageRef="OrderApproved" />
            </intermediateCatchEvent>
            <intermediateCatchEvent id="signalCatch1" name="Catch Alert Signal">
                <signalEventDefinition signalRef="alertSignal" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow4" sourceRef="messageCatch1" targetRef="parallelJoin1" />
            <sequenceFlow id="flow5" sourceRef="signalCatch1" targetRef="parallelJoin1" />
            <parallelGateway id="parallelJoin1" />
            <sequenceFlow id="flow6" sourceRef="parallelJoin1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) =
        start_process_instance(xml, "Mixed ICE Deployment", "Mixed ICE Process");

    let runtime_store = process_engine.get_runtime_store();
    let runtime_service = process_engine.get_runtime_service();

    // Two wait states: one message, one signal
    let visible_wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(
        visible_wait_states.len(),
        2,
        "Should have two wait states: one message and one signal"
    );

    let message_wait = visible_wait_states
        .iter()
        .find(|ws| ws.wait_kind == MessageStyleWaitKind::MessageIntermediateCatchEvent)
        .expect("Should have message wait state");
    assert_eq!(message_wait.message_ref.as_deref(), Some("OrderApproved"));
    let message_execution_id = message_wait.execution_id.clone();

    let signal_wait = visible_wait_states
        .iter()
        .find(|ws| ws.wait_kind == MessageStyleWaitKind::SignalIntermediateCatchEvent)
        .expect("Should have signal wait state");
    assert_eq!(signal_wait.signal_ref.as_deref(), Some("Alert Signal"));
    let signal_execution_id = signal_wait.execution_id.clone();

    // Trigger message ICE only
    runtime_service.trigger_intermediate_catch_event_by_message_ref_and_execution_id(
        "OrderApproved".to_string(),
        message_execution_id,
    );

    let wait_states_after_msg = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(
        wait_states_after_msg.len(),
        1,
        "Signal wait state should remain"
    );
    assert_eq!(
        wait_states_after_msg[0].wait_kind,
        MessageStyleWaitKind::SignalIntermediateCatchEvent
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert!(
        !stored_pi.is_ended,
        "Process should not be ended yet (signal branch still waiting)"
    );
    drop(session);

    // Trigger signal ICE
    runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        "Alert Signal".to_string(),
        signal_execution_id,
    );

    let wait_states_final = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert!(
        wait_states_final.is_empty(),
        "All wait states should be cleared"
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi_final = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert!(
        stored_pi_final.is_ended,
        "Process should be ended after both branches complete"
    );
}

/// Wrong event_ref for both message and signal ICE should be no-op.
#[test]
fn test_mixed_intermediate_catch_wrong_ref_is_noop_for_both_types() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="Alert Signal" />
        <process id="mixedIceWrongRefProcess" name="Mixed ICE Wrong Ref Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="parallelGateway1" />
            <parallelGateway id="parallelGateway1" />
            <sequenceFlow id="flow2" sourceRef="parallelGateway1" targetRef="messageCatch1" />
            <sequenceFlow id="flow3" sourceRef="parallelGateway1" targetRef="signalCatch1" />
            <intermediateCatchEvent id="messageCatch1" name="Catch Order">
                <messageEventDefinition messageRef="OrderApproved" />
            </intermediateCatchEvent>
            <intermediateCatchEvent id="signalCatch1" name="Catch Signal">
                <signalEventDefinition signalRef="alertSignal" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow4" sourceRef="messageCatch1" targetRef="parallelJoin1" />
            <sequenceFlow id="flow5" sourceRef="signalCatch1" targetRef="parallelJoin1" />
            <parallelGateway id="parallelJoin1" />
            <sequenceFlow id="flow6" sourceRef="parallelJoin1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = start_process_instance(
        xml,
        "Mixed ICE Wrong Ref Deployment",
        "Mixed ICE Wrong Ref Process",
    );

    let runtime_store = process_engine.get_runtime_store();
    let runtime_service = process_engine.get_runtime_service();

    let wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(wait_states.len(), 2);

    let msg_exec = wait_states
        .iter()
        .find(|ws| ws.wait_kind == MessageStyleWaitKind::MessageIntermediateCatchEvent)
        .unwrap()
        .execution_id
        .clone();
    let sig_exec = wait_states
        .iter()
        .find(|ws| ws.wait_kind == MessageStyleWaitKind::SignalIntermediateCatchEvent)
        .unwrap()
        .execution_id
        .clone();

    // Wrong message ref
    runtime_service.trigger_intermediate_catch_event_by_message_ref_and_execution_id(
        "WrongMessage".to_string(),
        msg_exec.clone(),
    );
    // Wrong signal ref
    runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        "wrongSignal".to_string(),
        sig_exec.clone(),
    );
    // Cross-type: send signal ref to message execution
    runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        "alertSignal".to_string(),
        msg_exec.clone(),
    );
    // Cross-type: send message ref to signal execution
    runtime_service.trigger_intermediate_catch_event_by_message_ref_and_execution_id(
        "OrderApproved".to_string(),
        sig_exec.clone(),
    );

    // All four should be no-ops
    let wait_states_after = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_id.clone());
    assert_eq!(
        wait_states_after.len(),
        2,
        "Wrong refs should not clear any wait states"
    );

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert!(!stored_pi.is_ended, "Process should still be waiting");
}
