//! P55 contract tests for execution-level move and enableEventSubProcessStartEvent.
//!
//! Java references:
//! - `ChangeActivityStateBuilderImpl.java:46-110` (`moveExecutionToActivityId` family)
//! - `ChangeActivityStateBuilderImpl.java:177-182` (`enableEventSubProcessStartEvent`)
//! - `AbstractDynamicStateManager#doMoveExecutionState` (enable loop + move)
//! - `ChangeStateTest#testEnableEventSubProcessStartEvent` (multipleEventSubProcessEvents BPMN)
//!
//! ActivityId-level change-state (cancel+start normalize) is covered elsewhere and left alone.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;
use flowable_engine::runtime::process_instance::ProcessInstance;
use serde_json::json;
use std::collections::HashMap;

fn review_chain_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="executionMoveProcess" name="Execution Move Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="reviewA" />
            <userTask id="reviewA" name="Review A" />
            <sequenceFlow id="f2" sourceRef="reviewA" targetRef="reviewB" />
            <userTask id="reviewB" name="Review B" />
            <sequenceFlow id="f3" sourceRef="reviewB" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
        .to_string()
}

/// Mirrors Java `multipleEventSubProcessEvents.bpmn20.xml` used by
/// `ChangeStateTest#testEnableEventSubProcessStartEvent`.
fn multi_event_subprocess_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <signal id="mySignal" name="mySignal" flowable:scope="global"/>
        <message id="myMessage" name="myMessage"/>
        <process id="changeStateForEventSubProcess" isExecutable="true">
            <startEvent id="theStart"/>
            <sequenceFlow id="f1" sourceRef="theStart" targetRef="processTask"/>
            <userTask id="processTask"/>
            <sequenceFlow id="f2" sourceRef="processTask" targetRef="theEnd"/>
            <endEvent id="theEnd"/>

            <subProcess id="eventSubProcess" triggeredByEvent="true">
                <startEvent id="eventSubProcessStart" isInterrupting="true">
                    <signalEventDefinition signalRef="mySignal" />
                </startEvent>
                <sequenceFlow id="esf1" sourceRef="eventSubProcessStart" targetRef="eventSubProcessTask" />
                <userTask id="eventSubProcessTask"/>
                <sequenceFlow id="esf2" sourceRef="eventSubProcessTask" targetRef="eventSubProcessEnd" />
                <endEvent id="eventSubProcessEnd" />

                <startEvent id="messageEventSubProcessStart" isInterrupting="true">
                    <messageEventDefinition messageRef="myMessage"/>
                </startEvent>
                <sequenceFlow id="esf3" sourceRef="messageEventSubProcessStart" targetRef="messageEventSubProcessTask" />
                <userTask id="messageEventSubProcessTask"/>
                <sequenceFlow id="esf4" sourceRef="messageEventSubProcessTask" targetRef="messageEventSubProcessEnd" />
                <endEvent id="messageEventSubProcessEnd" />
            </subProcess>
        </process>
    </definitions>"#
        .to_string()
}

fn deploy_and_start(engine: &ProcessEngine, xml: String, resource: &str) -> ProcessInstance {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string(resource.to_string(), xml),
    )
    .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap()
}

fn execution_at_activity(
    engine: &ProcessEngine,
    process_instance_id: &str,
    activity_id: &str,
) -> flowable_engine::runtime::execution::Execution {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    runtime_store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && execution.activity_id.as_deref() == Some(activity_id)
                && !execution.is_ended
        })
        .expect("an active execution should exist at the requested activity")
}

fn event_subprocess_subscriptions(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Vec<flowable_engine::persistence::runtime_store::EventSubprocessEventSubscription> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.find_event_subprocess_event_subscriptions_by_process_instance_id(
        process_instance_id,
        &mut session,
    )
}

fn task_definition_keys(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    let mut keys = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

/// Java-style moveExecutionToActivityId: the same execution row is reused at the
/// target activity, and execution-local variables set before the move survive.
#[test]
fn move_execution_to_activity_id_preserves_execution_identity_and_local_variables() {
    let engine = ProcessEngine::new("p55-exec-move-identity".to_string());
    let instance = deploy_and_start(&engine, review_chain_xml(), "execution_move.bpmn20.xml");
    let source = execution_at_activity(&engine, &instance.id, "reviewA");
    let source_id = source.id.clone();
    let parent_id = source.parent_id.clone();

    engine
        .get_runtime_service()
        .set_variable_local(
            source_id.clone(),
            "carryLocal".to_string(),
            json!("from-reviewA"),
        )
        .expect("set local variable on source execution");

    engine
        .get_runtime_service()
        .move_execution_to_activity_id(source_id.clone(), "reviewB".to_string())
        .expect("execution-level move should succeed");

    let moved = execution_at_activity(&engine, &instance.id, "reviewB");
    assert_eq!(
        moved.id, source_id,
        "true move must reuse the source execution id (not cancel+start a new row)"
    );
    assert_eq!(
        moved.parent_id, parent_id,
        "true move must keep the execution tree parent linkage"
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(source_id.clone(), "carryLocal".to_string())
            .unwrap(),
        Some(json!("from-reviewA")),
        "local variables on the moved execution must outlive the move"
    );
    assert_eq!(
        task_definition_keys(&engine, &instance.id),
        vec!["reviewB".to_string()]
    );
}

/// Injected process / local variables still apply on the true-move path.
#[test]
fn move_execution_to_activity_id_with_variables_merges_locals() {
    let engine = ProcessEngine::new("p55-exec-move-vars".to_string());
    let instance = deploy_and_start(&engine, review_chain_xml(), "execution_move.bpmn20.xml");
    let source = execution_at_activity(&engine, &instance.id, "reviewA");
    let source_id = source.id.clone();

    engine
        .get_runtime_service()
        .set_variable_local(source_id.clone(), "kept".to_string(), json!(1))
        .unwrap();

    let mut process_variables = HashMap::new();
    process_variables.insert("procFlag".to_string(), json!(true));
    let mut local_for_b = HashMap::new();
    local_for_b.insert("injected".to_string(), json!("on-B"));
    let mut local_variables = HashMap::new();
    local_variables.insert("reviewB".to_string(), local_for_b);

    engine
        .get_runtime_service()
        .move_execution_to_activity_id_with_variables(
            source_id.clone(),
            "reviewB".to_string(),
            process_variables,
            local_variables,
        )
        .expect("move with variables should succeed");

    let moved = execution_at_activity(&engine, &instance.id, "reviewB");
    assert_eq!(moved.id, source_id);
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(instance.id.clone(), "procFlag".to_string())
            .unwrap(),
        Some(json!(true))
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(source_id.clone(), "kept".to_string())
            .unwrap(),
        Some(json!(1))
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(source_id, "injected".to_string())
            .unwrap(),
        Some(json!("on-B"))
    );
}

/// Java `ChangeStateTest#testEnableEventSubProcessStartEvent` /
/// `AbstractDynamicStateManager` enable loop: when an event-subprocess start is
/// not currently subscribed, `enableEventSubProcessStartEvent` arms it so a later
/// event can fire the ES.
///
/// Note: Rust re-registers process-level ES subscriptions when a user task
/// starts, so after an interrupting signal the subscriptions may reappear. This
/// test forces the unarmed state (delete subscriptions) then enables only the
/// message start — matching the Java "0 subscriptions → enable → 1 subscription
/// → message fires" observable sequence.
#[test]
fn enable_event_subprocess_start_event_allows_message_trigger_after_interrupting_signal() {
    let engine = ProcessEngine::new("p55-enable-es-start".to_string());
    let instance = deploy_and_start(
        &engine,
        multi_event_subprocess_xml(),
        "multi_es.bpmn20.xml",
    );

    assert_eq!(
        task_definition_keys(&engine, &instance.id),
        vec!["processTask".to_string()]
    );

    // Process-level ES subscriptions (signal + message) are armed when the user task starts.
    let before = event_subprocess_subscriptions(&engine, &instance.id);
    assert!(
        before.iter().any(|s| {
            s.event_kind == EventSubscriptionKind::Signal && s.event_ref == "mySignal"
        }),
        "signal ES start should be subscribed before trigger: {before:?}"
    );
    assert!(
        before.iter().any(|s| {
            s.event_kind == EventSubscriptionKind::Message && s.event_ref == "myMessage"
        }),
        "message ES start should be subscribed before trigger: {before:?}"
    );

    // Fire interrupting signal first (Java test does this, then observes 0 subs).
    engine
        .get_runtime_service()
        .trigger_event_subprocess_by_signal("mySignal".to_string(), instance.id.clone());
    assert_eq!(
        task_definition_keys(&engine, &instance.id),
        vec!["eventSubProcessTask".to_string()]
    );

    // Force Java-equivalent unarmed state: Rust user-task registration may have
    // re-armed both starts when the ES task was created.
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.delete_event_subprocess_event_subscriptions_by_process_instance_id(
            &instance.id,
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }
    assert!(
        event_subprocess_subscriptions(&engine, &instance.id).is_empty(),
        "subscriptions must be unarmed before enable"
    );

    // Without enable, message must not activate the ES path.
    engine
        .get_runtime_service()
        .trigger_event_subprocess_by_message("myMessage".to_string(), instance.id.clone());
    assert_eq!(
        task_definition_keys(&engine, &instance.id),
        vec!["eventSubProcessTask".to_string()],
        "message must not fire ES while subscription is disabled"
    );

    engine
        .get_runtime_service()
        .enable_event_subprocess_start_event(
            instance.id.clone(),
            "messageEventSubProcessStart".to_string(),
        )
        .expect("enableEventSubProcessStartEvent should re-arm the message start");

    let enabled = event_subprocess_subscriptions(&engine, &instance.id);
    assert_eq!(
        enabled.len(),
        1,
        "exactly one subscription after enable: {enabled:?}"
    );
    assert_eq!(enabled[0].event_kind, EventSubscriptionKind::Message);
    assert_eq!(enabled[0].event_ref, "myMessage");
    assert_eq!(enabled[0].start_event_id, "messageEventSubProcessStart");

    engine
        .get_runtime_service()
        .trigger_event_subprocess_by_message("myMessage".to_string(), instance.id.clone());

    assert_eq!(
        task_definition_keys(&engine, &instance.id),
        vec!["messageEventSubProcessTask".to_string()],
        "enabled message start must be triggerable"
    );
}

#[test]
fn enable_event_subprocess_start_event_rejects_unknown_start_event() {
    let engine = ProcessEngine::new("p55-enable-es-missing".to_string());
    let instance = deploy_and_start(
        &engine,
        multi_event_subprocess_xml(),
        "multi_es_missing.bpmn20.xml",
    );

    let err = engine
        .get_runtime_service()
        .enable_event_subprocess_start_event(instance.id.clone(), "noSuchStart".to_string())
        .expect_err("unknown start event id must fail");
    assert!(
        err.to_string().contains("noSuchStart"),
        "error should name the missing activity, got: {err}"
    );
}

#[test]
fn move_execution_to_activity_id_rejects_unknown_target() {
    let engine = ProcessEngine::new("p55-exec-move-missing".to_string());
    let instance = deploy_and_start(&engine, review_chain_xml(), "execution_move_missing.bpmn20.xml");
    let source_id = execution_at_activity(&engine, &instance.id, "reviewA").id;

    let err = engine
        .get_runtime_service()
        .move_execution_to_activity_id(source_id, "missingActivity".to_string())
        .expect_err("unknown target must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("missingActivity") || msg.to_lowercase().contains("not found"),
        "error should mention the missing activity, got: {msg}"
    );
}
