use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::task_service::MessageStyleWaitKind;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_message_intermediate_catch_accepts_trigger_variables_and_records_history() {
    let engine = ProcessEngine::new("message-catch-variables-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history_service = engine.get_history_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="messageCatchVariableProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="waitForMessage" />
            <intermediateCatchEvent id="waitForMessage">
                <messageEventDefinition messageRef="approvalMessage" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="waitForMessage" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review" />
            <sequenceFlow id="flow3" sourceRef="reviewTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Message Catch Variables Deployment".to_string())
                .add_string(
                    "message_catch_variables.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let wait_state = runtime_service
        .get_event_wait_states_by_process_instance_id(process_instance.id.clone())
        .into_iter()
        .find(|state| state.activity_id.as_deref() == Some("waitForMessage"))
        .expect("message catch should be waiting");

    let mut variables = HashMap::new();
    variables.insert("approvedBy".to_string(), json!("ops"));
    runtime_service
        .trigger_event_intermediate_catch_with_variables(
            EventSubscriptionKind::Message,
            "approvalMessage".to_string(),
            wait_state.execution_id.clone(),
            variables,
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "reviewTask");
    assert_eq!(
        runtime_service
            .get_variable(tasks[0].execution_id.clone(), "approvedBy".to_string())
            .unwrap(),
        Some(json!("ops"))
    );

    let historic_variables = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(process_instance.id.clone())
        .variable_name("approvedBy".to_string())
        .list()
        .unwrap();
    assert_eq!(historic_variables.len(), 1);
    assert_eq!(historic_variables[0].value, json!("ops"));
}

// P129: Java IntermediateThrowEventParseHandler.java:51-56 — MessageEventDefinition
// on intermediate throw is unsupported (LOGGER.warn only, no behavior). Formerly this
// test asserted Rust's supersets delivery into a non-interrupting event-subprocess;
// that was flipped to Java no-op semantics.
#[test]
fn test_intermediate_message_throw_is_noop_does_not_activate_event_subprocess() {
    let engine = ProcessEngine::new("message-throw-event-subprocess-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history_service = engine.get_history_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="messageThrowEventSubprocessProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="mainTask" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="throwMessage" />
            <userTask id="mainTask" name="Main" />
            <sequenceFlow id="flow4" sourceRef="mainTask" targetRef="mainEnd" />
            <intermediateThrowEvent id="throwMessage">
                <messageEventDefinition messageRef="eventMessage" />
            </intermediateThrowEvent>
            <sequenceFlow id="flow5" sourceRef="throwMessage" targetRef="throwEnd" />
            <endEvent id="throwEnd" />
            <endEvent id="mainEnd" />
            <subProcess id="messageEventSubProcess" triggeredByEvent="true">
                <startEvent id="messageEventStart" isInterrupting="false">
                    <messageEventDefinition messageRef="eventMessage" />
                </startEvent>
                <sequenceFlow id="eventFlow1" sourceRef="messageEventStart" targetRef="eventTask" />
                <userTask id="eventTask" name="Event Task" />
                <sequenceFlow id="eventFlow2" sourceRef="eventTask" targetRef="eventEnd" />
                <endEvent id="eventEnd" />
            </subProcess>
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Message Throw Event Subprocess Deployment".to_string())
                .add_string(
                    "message_throw_event_subprocess.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("requestId".to_string(), json!("REQ-42")),
        )
        .unwrap();

    // Java no-op: throwMessage takes outgoing without delivering. Only mainTask
    // remains; eventTask must NOT appear.
    // IntermediateThrowEventParseHandler.java:51-56
    let task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    assert_eq!(
        task_keys,
        vec!["mainTask".to_string()],
        "message intermediate throw must not activate the event-subprocess (Java no-op)"
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let subscriptions = store.find_event_subprocess_event_subscriptions_by_process_instance_id(
        &process_instance.id,
        &mut session,
    );
    assert!(
        subscriptions.iter().any(|subscription| {
            !subscription.interrupting
                && subscription.event_kind == EventSubscriptionKind::Message
                && subscription.event_ref == "eventMessage"
        }),
        "non-interrupting message event subprocess subscription must remain untriggered"
    );
    drop(session);

    // No throw-delivery audit: message throw no longer records bpmn-message-throw.
    // IntermediateThrowEventParseHandler.java:51-56
    let audits = history_service
        .create_historic_audit_log_query()
        .process_instance_id(process_instance.id.clone())
        .event_type("bpmn-message-throw".to_string())
        .list()
        .unwrap();
    assert!(
        audits.is_empty(),
        "message intermediate throw is a no-op and must not record a delivery audit"
    );
}

/// P129: intermediate message throw does not fire a waiting intermediate message
/// catch; throw token still takes outgoing (Java IntermediateThrowEventParseHandler.java:51-56).
#[test]
fn test_intermediate_message_throw_does_not_trigger_waiting_message_catch() {
    let engine = ProcessEngine::new("message-throw-catch-noop-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="orderMessage" name="Order Message" />
        <process id="messageThrowCatchNoopProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="waitForMessage" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="throwMessage" />
            <intermediateCatchEvent id="waitForMessage">
                <messageEventDefinition messageRef="orderMessage" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow4" sourceRef="waitForMessage" targetRef="afterCatch" />
            <userTask id="afterCatch" name="After Catch" />
            <sequenceFlow id="flow5" sourceRef="afterCatch" targetRef="catchEnd" />
            <endEvent id="catchEnd" />
            <intermediateThrowEvent id="throwMessage">
                <messageEventDefinition messageRef="orderMessage" />
            </intermediateThrowEvent>
            <sequenceFlow id="flow6" sourceRef="throwMessage" targetRef="afterThrow" />
            <userTask id="afterThrow" name="After Throw" />
            <sequenceFlow id="flow7" sourceRef="afterThrow" targetRef="throwEnd" />
            <endEvent id="throwEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Message Throw Catch Noop Deployment".to_string())
                .add_string(
                    "message_throw_catch_noop.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    // Throw path continues to afterThrow; catch stays waiting (not triggered).
    // IntermediateThrowEventParseHandler.java:51-56
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterThrow");

    let wait_states = runtime_service
        .get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert!(
        wait_states
            .iter()
            .any(|state| state.activity_id.as_deref() == Some("waitForMessage")),
        "message intermediate catch must remain waiting; message throw is a no-op"
    );
}

/// P129: message end event is none-end (no delivery) — Java EndEventParseHandler.java:72-73.
#[test]
fn test_message_end_event_is_noop_does_not_trigger_message_subscription() {
    let engine = ProcessEngine::new("message-end-event-noop-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="doneMessage" name="Done Message" />
        <process id="messageEndEventNoopProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="waitForMessage" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="messageEnd" />
            <intermediateCatchEvent id="waitForMessage">
                <messageEventDefinition messageRef="doneMessage" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow4" sourceRef="waitForMessage" targetRef="afterCatch" />
            <userTask id="afterCatch" name="After Catch" />
            <sequenceFlow id="flow5" sourceRef="afterCatch" targetRef="catchEnd" />
            <endEvent id="catchEnd" />
            <endEvent id="messageEnd">
                <messageEventDefinition messageRef="doneMessage" />
            </endEvent>
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Message End Event Noop Deployment".to_string())
                .add_string(
                    "message_end_event_noop.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    // Message end completes its own token without delivering. Catch remains waiting.
    // EndEventParseHandler.java:72-73 → createNoneEndEventActivityBehavior
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks.is_empty(),
        "message end must not wake the intermediate catch into afterCatch"
    );

    let wait_states = runtime_service
        .get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert!(
        wait_states
            .iter()
            .any(|state| state.activity_id.as_deref() == Some("waitForMessage")),
        "message catch must remain waiting after message end event (Java none-end no-op)"
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let pi = store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should exist");
    assert!(
        !pi.is_ended,
        "process stays alive on the waiting catch branch after message end"
    );
}

/// P129: models with MessageEventDefinition on throw/end events deploy successfully
/// (Java only warns for intermediate message throw; end falls through to none-end).
#[test]
fn test_message_throw_and_message_end_event_deploy_successfully() {
    let engine = ProcessEngine::new("message-throw-end-deploy-test".to_string());
    let repository_service = engine.get_repository_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="m1" name="M1" />
        <process id="messageThrowEndDeployProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="throwMessage" />
            <intermediateThrowEvent id="throwMessage">
                <messageEventDefinition messageRef="m1" />
            </intermediateThrowEvent>
            <sequenceFlow id="flow2" sourceRef="throwMessage" targetRef="messageEnd" />
            <endEvent id="messageEnd">
                <messageEventDefinition messageRef="m1" />
            </endEvent>
        </process>
    </definitions>"#;

    let deployment = repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Message Throw End Deploy".to_string())
                .add_string(
                    "message_throw_end_deploy.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .expect("message throw + message end must deploy (Java warn-not-reject)");

    assert!(!deployment.id.is_empty());
    let defs = repository_service.get_process_definition_ids().unwrap();
    assert_eq!(defs.len(), 1);

    // Runtime path also completes (both nodes no-op).
    let runtime_service = engine.get_runtime_service();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(defs[0].clone()),
        )
        .unwrap();
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let pi = store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should exist");
    assert!(
        pi.is_ended,
        "message throw + message end should pass through to completion"
    );
}

#[test]
fn test_intermediate_signal_throw_triggers_boundary_path_and_records_audit() {
    let engine = ProcessEngine::new("signal-throw-boundary-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history_service = engine.get_history_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="notifySignal" name="Notify Signal" />
        <process id="signalThrowBoundaryProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="mainTask" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="throwSignal" />
            <userTask id="mainTask" name="Main" />
            <boundaryEvent id="notifyBoundary" attachedToRef="mainTask" cancelActivity="false">
                <signalEventDefinition signalRef="notifySignal" />
            </boundaryEvent>
            <sequenceFlow id="flow4" sourceRef="mainTask" targetRef="mainEnd" />
            <sequenceFlow id="flow5" sourceRef="notifyBoundary" targetRef="notificationTask" />
            <intermediateThrowEvent id="throwSignal">
                <signalEventDefinition signalRef="notifySignal" />
            </intermediateThrowEvent>
            <sequenceFlow id="flow6" sourceRef="throwSignal" targetRef="throwEnd" />
            <userTask id="notificationTask" name="Notification" />
            <endEvent id="throwEnd" />
            <endEvent id="mainEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Signal Throw Boundary Deployment".to_string())
                .add_string(
                    "signal_throw_boundary.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("notice".to_string(), json!("sent")),
        )
        .unwrap();

    let mut tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    tasks.sort_by(|left, right| left.task_definition_key.cmp(&right.task_definition_key));
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].task_definition_key, "mainTask");
    assert_eq!(tasks[1].task_definition_key, "notificationTask");
    assert_eq!(
        runtime_service
            .get_variable(tasks[1].execution_id.clone(), "notice".to_string())
            .unwrap(),
        Some(json!("sent"))
    );

    let audits = history_service
        .create_historic_audit_log_query()
        .process_instance_id(process_instance.id.clone())
        .event_type("bpmn-signal-throw".to_string())
        .list()
        .unwrap();
    assert_eq!(audits.len(), 1);
    assert!(
        audits[0]
            .details
            .as_deref()
            .is_some_and(|details| details.contains("notifySignal"))
    );
}

#[test]
fn test_signal_boundary_event_triggers_by_resolved_global_name() {
    let engine = ProcessEngine::new("signal-boundary-name-compat-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="sig1" name="external-signal" />
        <process id="signalBoundaryNameCompatProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="userTask" />
            <userTask id="userTask" name="Wait For Signal" />
            <boundaryEvent id="signalBoundary" attachedToRef="userTask" cancelActivity="true">
                <signalEventDefinition signalRef="sig1" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask" targetRef="normalEnd" />
            <sequenceFlow id="flow3" sourceRef="signalBoundary" targetRef="boundaryEnd" />
            <endEvent id="normalEnd" />
            <endEvent id="boundaryEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Signal Boundary Name Compat Deployment".to_string())
                .add_string(
                    "signal_boundary_name_compat.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // Instance 1: trigger by the resolved global name "external-signal".
    let process_instance_by_name = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.clone())
                .name("Triggered By Name".to_string()),
        )
        .unwrap();
    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(process_instance_by_name.id.clone())
            .unwrap()
            .len(),
        1
    );
    runtime_service.trigger_boundary_event_by_signal_ref(
        "external-signal".to_string(),
        process_instance_by_name.id.clone(),
    );
    let tasks_after_name = task_service
        .get_tasks_by_process_instance_id(process_instance_by_name.id.clone())
        .unwrap();
    assert!(
        tasks_after_name.is_empty(),
        "Triggering signal boundary by resolved global name should consume the user task"
    );
    let store_after_name = engine.get_runtime_store();
    let mut session_after_name = store_after_name.create_session().unwrap();
    let pi_after_name = store_after_name
        .find_process_instance(&process_instance_by_name.id, &mut session_after_name)
        .expect("process instance should exist");
    assert!(
        pi_after_name.is_ended,
        "Process should be ended after signal boundary triggered by name"
    );
    drop(session_after_name);

    // Instance 2: trigger by the raw BPMN id "sig1".
    let process_instance_by_id = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.clone())
                .name("Triggered By Id".to_string()),
        )
        .unwrap();
    assert_eq!(
        task_service
            .get_tasks_by_process_instance_id(process_instance_by_id.id.clone())
            .unwrap()
            .len(),
        1
    );
    runtime_service.trigger_boundary_event_by_signal_ref(
        "sig1".to_string(),
        process_instance_by_id.id.clone(),
    );
    let tasks_after_id = task_service
        .get_tasks_by_process_instance_id(process_instance_by_id.id.clone())
        .unwrap();
    assert!(
        tasks_after_id.is_empty(),
        "Triggering signal boundary by raw BPMN id should consume the user task"
    );
    let store_after_id = engine.get_runtime_store();
    let mut session_after_id = store_after_id.create_session().unwrap();
    let pi_after_id = store_after_id
        .find_process_instance(&process_instance_by_id.id, &mut session_after_id)
        .expect("process instance should exist");
    assert!(
        pi_after_id.is_ended,
        "Process should be ended after signal boundary triggered by id"
    );
}

#[test]
fn test_signal_intermediate_catch_event_triggers_by_raw_id() {
    let engine = ProcessEngine::new("signal-catch-id-compat-test".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let history_service = engine.get_history_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="sig1" name="external-signal" />
        <process id="signalCatchIdCompatProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="waitForSignal" />
            <intermediateCatchEvent id="waitForSignal" name="Wait For External Signal">
                <signalEventDefinition signalRef="sig1" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="waitForSignal" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Signal Catch Id Compat Deployment".to_string())
                .add_string(
                    "signal_catch_id_compat.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // The subscription is stored under the resolved global name; the raw id must
    // also match the same subscription after the dual id/name compatibility fix.
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.clone())
                .name("Trigger By Raw Id".to_string()),
        )
        .unwrap();

    let wait_states = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(wait_states.len(), 1);
    assert_eq!(
        wait_states[0].wait_kind,
        MessageStyleWaitKind::SignalIntermediateCatchEvent
    );
    assert_eq!(
        wait_states[0].signal_ref.as_deref(),
        Some("external-signal")
    );
    let execution_id = wait_states[0].execution_id.clone();

    runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        "sig1".to_string(),
        execution_id,
    );

    let store_after = engine.get_runtime_store();
    let mut session_after = store_after.create_session().unwrap();
    let pi_after = store_after
        .find_process_instance(&process_instance.id, &mut session_after)
        .expect("process instance should exist");
    assert!(
        pi_after.is_ended,
        "Triggering signal intermediate catch by raw id should complete the process"
    );
    drop(session_after);

    let audits = history_service
        .create_historic_audit_log_query()
        .process_instance_id(process_instance.id.clone())
        .event_type("bpmn-signal-throw".to_string())
        .list()
        .unwrap();
    assert!(
        audits.is_empty(),
        "Direct runtime trigger of signal catch should not record a throw audit"
    );
}
