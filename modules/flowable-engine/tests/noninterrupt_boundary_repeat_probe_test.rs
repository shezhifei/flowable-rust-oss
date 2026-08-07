//! Contract: non-interrupting boundary events with **repeat** semantics keep
//! their subscription after each fire and can be re-triggered while the host
//! activity is still active. The subscription is removed only when the host
//! ends (task complete / delete / interrupt of host).
//!
//! Keep-set after P10-1 / P12 (non-interrupting, state retained on fire):
//! **message**, **signal**, **conditional**, **escalation**.
//! One-shot consume kinds: **error** (always interrupting in Java parse),
//! **cancel**, **compensate**. Timer non-interrupt path reschedules cycles
//! separately (not via `boundary_event_state`).
//!
//! Java reference:
//! `MessageNonInterruptingBoundaryEventTest#testSingleNonInterruptingBoundaryMessageEvent`
//! (`flowable-engine/.../message/MessageNonInterruptingBoundaryEventTest.java:31-86`)
//! — same message fires twice ("event subscription not removed"); subscription
//! is removed when the host task completes.
//!
//! Engine path: `trigger_boundary_event_cmd.rs` `execute_boundary_trigger`
//! retains non-interrupting state for the keep-set kinds above; other kinds
//! still delete `boundary_event_state` on fire.

use flowable_engine::engine::process_engine::ProcessEngine;

const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="nonInterruptingRepeat" name="NonInterruptingRepeat">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
        <userTask id="userTask1" name="Host" />
        <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="false">
            <messageEventDefinition messageRef="notifyMessage" />
        </boundaryEvent>
        <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
        <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="receiveTask1" />
        <receiveTask id="receiveTask1" name="Ack" />
        <sequenceFlow id="flow4" sourceRef="receiveTask1" targetRef="endEvent2" />
        <endEvent id="endEvent1" />
        <endEvent id="endEvent2" />
    </process>
</definitions>"#;

fn deploy_and_start(engine: &ProcessEngine) -> String {
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("noninterrupt-repeat".to_string())
        .add_string(
            "noninterrupt_repeat.bpmn20.xml".to_string(),
            XML.to_string(),
        );
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();
    instance.id
}

#[test]
fn non_interrupting_message_boundary_subscription_survives_and_fires_twice() {
    let engine = ProcessEngine::new("p9-4-repeat".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let instance_id = deploy_and_start(&engine);

    // First trigger: boundary path produces one receiveTask1.
    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), instance_id.clone())
        .unwrap();
    let tasks = task_service
        .get_tasks_by_process_instance_id(instance_id.clone())
        .unwrap();
    let receive_count_1 = tasks
        .iter()
        .filter(|t| t.task_definition_key == "receiveTask1")
        .count();
    assert_eq!(
        receive_count_1, 1,
        "first trigger should spawn boundary path"
    );

    // Java: "event subscription not removed" after fire.
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states =
        runtime_store.find_boundary_event_states_by_process_instance_id(&instance_id, &mut session);
    assert_eq!(
        states.len(),
        1,
        "non-interrupting boundary subscription must survive its own trigger"
    );
    drop(session);

    // Second trigger of the same message: another boundary path fires.
    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), instance_id.clone())
        .unwrap();
    let tasks = task_service
        .get_tasks_by_process_instance_id(instance_id.clone())
        .unwrap();
    let receive_count_2 = tasks
        .iter()
        .filter(|t| t.task_definition_key == "receiveTask1")
        .count();
    assert_eq!(
        receive_count_2, 2,
        "second trigger of the same non-interrupting boundary must fire again"
    );

    // Subscription still present while host is open.
    let mut session = runtime_store.create_session().unwrap();
    let states =
        runtime_store.find_boundary_event_states_by_process_instance_id(&instance_id, &mut session);
    assert_eq!(
        states.len(),
        1,
        "subscription remains after repeated fires until host ends"
    );
}

#[test]
fn non_interrupting_boundary_subscription_removed_when_host_task_completes() {
    // Cleanup-side contract: after a non-interrupting fire, completing the host
    // user task must remove the boundary state (Java: subscription removed with
    // host). Guards against over-fix that leaks subscriptions forever.
    let engine = ProcessEngine::new("p9-4-host-cleanup".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let instance_id = deploy_and_start(&engine);

    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), instance_id.clone())
        .unwrap();

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states_after_fire =
        runtime_store.find_boundary_event_states_by_process_instance_id(&instance_id, &mut session);
    assert_eq!(
        states_after_fire.len(),
        1,
        "subscription must still be present after non-interrupting fire"
    );
    drop(session);

    let host_task = task_service
        .get_tasks_by_process_instance_id(instance_id.clone())
        .unwrap()
        .into_iter()
        .find(|t| t.task_definition_key == "userTask1")
        .expect("host user task must still exist");
    task_service.complete_task_by_id(host_task.id).unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let states_after_host =
        runtime_store.find_boundary_event_states_by_process_instance_id(&instance_id, &mut session);
    assert!(
        states_after_host.is_empty(),
        "boundary subscription must be removed when host user task completes"
    );

    // Boundary path (receiveTask1) should still be open; wake it so the process
    // can finish cleanly (guards against leaking wait state / stuck PI).
    let receive_executions: Vec<_> = runtime_store
        .snapshot_executions(&mut session)
        .values()
        .filter(|e| {
            e.process_instance_id.as_deref() == Some(instance_id.as_str())
                && e.activity_id.as_deref() == Some("receiveTask1")
        })
        .cloned()
        .collect();
    assert_eq!(
        receive_executions.len(),
        1,
        "boundary path receive execution should remain after host completes"
    );
    drop(session);

    engine.wake_up_message_by_process_instance_id(instance_id.clone());

    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&instance_id, &mut session)
        .expect("process instance row should exist");
    assert!(
        pi.is_ended,
        "process should end after host complete and boundary path complete"
    );
}
