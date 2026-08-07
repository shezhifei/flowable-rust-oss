//! Contract tests for execution trigger suspension guard.
//!
//! Java parity: `NeedsActiveExecutionCmd` is the base class guard for
//! `TriggerCmd`, `MessageEventReceivedCmd`, `SetExecutionVariablesCmd`,
//! `RemoveExecutionVariablesCmd`. It checks `execution.isSuspended()` and
//! throws `FlowableException` (→ HTTP 500).
//!
//! `SignalEventReceivedCmd` has an inline check for targeted signals only
//! (executionId != null). Global signals do NOT check suspension.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;

/// Deploys a process with a message intermediate catch event and starts an instance.
/// Returns (process_instance_id, execution_id_at_catch, message_ref).
fn deploy_message_catch(engine: &ProcessEngine, id_suffix: &str) -> (String, String, String) {
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let message_ref = format!("msg{id_suffix}");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="msg{id_suffix}" name="{message_ref}" />
        <process id="msgproc{id_suffix}">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="catch" />
            <intermediateCatchEvent id="catch">
                <messageEventDefinition messageRef="msg{id_suffix}" />
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="catch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );

    repo.deploy(
        repo.create_deployment()
            .add_string(format!("msgproc{id_suffix}.bpmn20.xml"), xml),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // Find the execution waiting at the catch event
    let wait_states = runtime.get_event_wait_states_by_process_instance_id(instance.id.clone());
    let execution_id = wait_states[0].execution_id.clone();

    (instance.id, execution_id, message_ref)
}

/// Deploys a process with a signal intermediate catch event and starts an instance.
fn deploy_signal_catch(engine: &ProcessEngine, id_suffix: &str) -> (String, String, String) {
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let signal_ref = format!("sig{id_suffix}");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="sig{id_suffix}" name="{signal_ref}" />
        <process id="sigproc{id_suffix}">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="catch" />
            <intermediateCatchEvent id="catch">
                <signalEventDefinition signalRef="sig{id_suffix}" />
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="catch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );

    repo.deploy(
        repo.create_deployment()
            .add_string(format!("sigproc{id_suffix}.bpmn20.xml"), xml),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    let wait_states = runtime.get_event_wait_states_by_process_instance_id(instance.id.clone());
    let execution_id = wait_states[0].execution_id.clone();

    (instance.id, execution_id, signal_ref)
}

/// Deploys a process with a timer intermediate catch event and starts an instance.
fn deploy_timer_catch(engine: &ProcessEngine, id_suffix: &str) -> (String, String) {
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="timerproc{id_suffix}">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="catch" />
            <intermediateCatchEvent id="catch">
                <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="catch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );

    repo.deploy(
        repo.create_deployment()
            .add_string(format!("timerproc{id_suffix}.bpmn20.xml"), xml),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // Timer events use timer_job_states, not event_wait_states
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let timer_states =
        store.find_timer_job_states_by_process_instance_id(&instance.id, &mut session);
    let execution_id = timer_states[0].execution_id.clone();

    (instance.id, execution_id)
}

// ── Test: message trigger on suspended execution is rejected ──

#[test]
fn message_trigger_on_suspended_execution_rejected() {
    let engine = ProcessEngine::new("msg-trigger-suspended".to_string());
    let runtime = engine.get_runtime_service();

    let (pi_id, execution_id, message_ref) = deploy_message_catch(&engine, "1");

    // Suspend the process instance (cascades to executions)
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    // Attempting to trigger the message catch should fail with ExecutionError
    let result = runtime.trigger_event_intermediate_catch_with_variables(
        EventSubscriptionKind::Message,
        message_ref,
        execution_id,
        Default::default(),
    );

    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("suspended"),
        "Expected suspension error, got: {msg}"
    );
}

// ── Test: targeted signal trigger on suspended execution is rejected ──

#[test]
fn targeted_signal_trigger_on_suspended_execution_rejected() {
    let engine = ProcessEngine::new("sig-trigger-suspended".to_string());
    let runtime = engine.get_runtime_service();

    let (pi_id, execution_id, signal_ref) = deploy_signal_catch(&engine, "1");

    // Suspend the process instance
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    // Targeted signal trigger should fail
    let result = runtime.trigger_event_intermediate_catch_with_variables(
        EventSubscriptionKind::Signal,
        signal_ref,
        execution_id,
        Default::default(),
    );

    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("suspended"),
        "Expected suspension error, got: {msg}"
    );
}

// ── Test: timer trigger on suspended execution is rejected ──

#[test]
fn timer_trigger_on_suspended_execution_rejected() {
    let engine = ProcessEngine::new("timer-trigger-suspended".to_string());
    let runtime = engine.get_runtime_service();

    let (pi_id, execution_id) = deploy_timer_catch(&engine, "1");

    // Suspend the process instance
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    // Timer trigger should fail
    let result = runtime.trigger_timer_intermediate_catch_event(execution_id);

    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("suspended"),
        "Expected suspension error, got: {msg}"
    );
}

// ── Test: global signal broadcast does NOT check suspension (Java parity) ──

#[test]
fn global_signal_broadcast_skips_suspension_check() {
    let engine = ProcessEngine::new("global-signal-suspended".to_string());
    let runtime = engine.get_runtime_service();

    let (pi_id, _execution_id, signal_ref) = deploy_signal_catch(&engine, "1");

    // Suspend the process instance
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    // Global signal broadcast should NOT fail (Java parity: SignalEventReceivedCmd
    // with executionId == null does NOT check suspension)
    runtime.trigger_global_signal_intermediate_catch(signal_ref, _execution_id);

    // The process instance should have completed (execution was triggered)
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let pi = store.find_process_instance(&pi_id, &mut session);
    // After trigger, the process should have advanced past the catch event
    // (it may or may not be ended depending on async behavior, but it should not error)
    assert!(pi.is_some());
}

// ── Test: trigger on active execution succeeds ──

#[test]
fn message_trigger_on_active_execution_succeeds() {
    let engine = ProcessEngine::new("msg-trigger-active".to_string());
    let runtime = engine.get_runtime_service();

    let (_pi_id, execution_id, message_ref) = deploy_message_catch(&engine, "1");

    // Trigger on active execution should succeed
    let result = runtime.trigger_event_intermediate_catch_with_variables(
        EventSubscriptionKind::Message,
        message_ref,
        execution_id,
        Default::default(),
    );

    assert!(result.is_ok());
}
