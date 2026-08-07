use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::{
    EventSubscriptionKind, RuntimeMessageStyleWaitKind,
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_user_task_registers_interrupting_and_non_interrupting_message_boundary_event_states() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="boundaryEventTest" name="Boundary Event Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </boundaryEvent>
            <boundaryEvent id="boundaryEvent2" attachedToRef="userTask1" cancelActivity="false">
                <messageEventDefinition messageRef="notifyMessage" />
            </boundaryEvent>
            <boundaryEvent id="boundaryEvent3" attachedToRef="userTask1" cancelActivity="true">
                <timerEventDefinition><timeDuration>PT10H</timeDuration></timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <sequenceFlow id="flow4" sourceRef="boundaryEvent2" targetRef="notifyEndEvent" />
            <sequenceFlow id="flow5" sourceRef="boundaryEvent3" targetRef="timerEndEvent" />
            <endEvent id="endEvent1" />
            <endEvent id="cancelEndEvent" />
            <endEvent id="notifyEndEvent" />
            <endEvent id="timerEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Boundary Event Test Deployment".to_string())
        .add_string("boundaryEventTest.bpmn20.xml".to_string(), xml.to_string());

    let deployment = repository_service.deploy(builder).unwrap();
    assert_eq!(
        deployment.name.as_deref(),
        Some("Boundary Event Test Deployment")
    );

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Boundary Event Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    let user_task_execution = executions
        .values()
        .find(|e| e.activity_id.as_deref() == Some("userTask1"))
        .expect("User task execution should exist");
    assert!(
        !user_task_execution.is_active,
        "User task should be in wait state"
    );

    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states.len(),
        2,
        "Should register both interrupting and non-interrupting message boundary events"
    );

    let interrupting_state = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "boundaryEvent1")
        .expect("Interrupting boundary should be registered");
    assert_eq!(interrupting_state.attached_activity_id, "userTask1");
    assert_eq!(interrupting_state.process_instance_id, process_instance.id);
    assert_eq!(interrupting_state.host_execution_id, user_task_execution.id);
    assert!(
        interrupting_state.cancel_activity,
        "boundaryEvent1 should be interrupting"
    );

    let non_interrupting_state = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "boundaryEvent2")
        .expect("Non-interrupting boundary should be registered");
    assert_eq!(non_interrupting_state.attached_activity_id, "userTask1");
    assert_eq!(
        non_interrupting_state.process_instance_id,
        process_instance.id
    );
    assert_eq!(
        non_interrupting_state.host_execution_id,
        user_task_execution.id
    );
    assert!(
        !non_interrupting_state.cancel_activity,
        "boundaryEvent2 should be non-interrupting"
    );

    assert!(
        runtime_store
            .find_boundary_event_state("boundaryEvent3", &process_instance.id, &mut session)
            .is_none(),
        "Timer boundary event should not be registered"
    );
}

#[test]
fn test_interrupting_boundary_event_cancels_host_activity_and_follows_boundary_path() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="boundaryEventCancelTest" name="Boundary Event Cancel Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Boundary Event Cancel Test Deployment".to_string())
        .add_string(
            "boundaryEventCancelTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Boundary Event Cancel Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_before.len(),
        1,
        "Should have one task before boundary event trigger"
    );
    assert_eq!(tasks_before[0].task_definition_key, "userTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_before.len(),
        1,
        "Should have boundary event state before trigger"
    );
    assert!(
        boundary_states_before[0].cancel_activity,
        "Boundary event should be interrupting"
    );
    drop(session);

    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), process_instance.id.clone())
        .unwrap();

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after.is_empty(),
        "Task should be deleted after boundary event trigger"
    );

    let mut session = runtime_store.create_session().unwrap();
    let executions_after = runtime_store.snapshot_executions(&mut session);
    assert!(
        executions_after
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("userTask1")),
        "User task execution should be removed after boundary event trigger"
    );

    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states_after.is_empty(),
        "Boundary event state should be cleaned up after trigger"
    );

    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        process_instance_after.is_ended,
        "Process should be ended after boundary event triggers cancel path"
    );
}

#[test]
fn test_interrupting_conditional_boundary_event_on_user_task_evaluates_and_leaves_host_task() {
    let process_engine = ProcessEngine::new("conditional-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="conditionalBoundaryProcess" name="Conditional Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review Request" />
            <boundaryEvent id="conditionalBoundary1" attachedToRef="userTask1" cancelActivity="true">
                <conditionalEventDefinition>
                    <condition>${approved == true}</condition>
                </conditionalEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="conditionalBoundary1" targetRef="escalatedTask" />
            <userTask id="escalatedTask" name="Escalated Review" />
            <sequenceFlow id="flow4" sourceRef="escalatedTask" targetRef="conditionalEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="conditionalEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Conditional Boundary Deployment".to_string())
        .add_string(
            "conditionalBoundary.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Conditional Boundary Instance".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(boundary_states.len(), 1);
    assert_eq!(boundary_states[0].boundary_event_id, "conditionalBoundary1");
    assert_eq!(
        boundary_states[0].event_subscription.kind,
        EventSubscriptionKind::Conditional
    );
    assert_eq!(
        boundary_states[0].event_subscription.event_ref,
        "${approved == true}"
    );
    // Release the read session before invoking the next command.
    drop(session);

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 1);
    assert_eq!(tasks_before[0].task_definition_key, "userTask1");

    let mut variables = HashMap::new();
    variables.insert("approved".to_string(), json!(true));
    runtime_service
        .evaluate_conditional_events(process_instance.id.clone(), variables)
        .unwrap();

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 1);
    assert_eq!(tasks_after[0].task_definition_key, "escalatedTask");

    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states_after.is_empty(),
        "Interrupting conditional boundary should be consumed"
    );
    drop(session);
}

#[test]
fn test_non_interrupting_conditional_boundary_on_user_task_preserves_host_and_follows_path() {
    let process_engine =
        ProcessEngine::new("non-interrupting-conditional-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="nonInterruptingConditionalBoundaryProcess" name="Non Interrupting Conditional Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review Request" />
            <boundaryEvent id="conditionalBoundary1" attachedToRef="userTask1" cancelActivity="false">
                <conditionalEventDefinition>
                    <condition>${approved == true}</condition>
                </conditionalEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="conditionalBoundary1" targetRef="escalatedTask" />
            <userTask id="escalatedTask" name="Escalated Review" />
            <sequenceFlow id="flow4" sourceRef="escalatedTask" targetRef="conditionalEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="conditionalEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Non Interrupting Conditional Boundary Deployment".to_string())
        .add_string(
            "nonInterruptingConditionalBoundary.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Non Interrupting Conditional Boundary Instance".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(boundary_states.len(), 1);
    assert_eq!(boundary_states[0].boundary_event_id, "conditionalBoundary1");
    assert!(!boundary_states[0].cancel_activity);
    assert_eq!(
        boundary_states[0].event_subscription.kind,
        EventSubscriptionKind::Conditional
    );
    assert_eq!(
        boundary_states[0].event_subscription.event_ref,
        "${approved == true}"
    );
    drop(session);

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 1);
    assert_eq!(tasks_before[0].task_definition_key, "userTask1");

    runtime_service.trigger_boundary_event_by_event_ref(
        EventSubscriptionKind::Conditional,
        "${approved == false}".to_string(),
        process_instance.id.clone(),
    );

    let tasks_after_wrong_ref = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_wrong_ref.len(),
        1,
        "Wrong conditional event ref should not trigger the boundary path"
    );
    assert_eq!(tasks_after_wrong_ref[0].task_definition_key, "userTask1");

    let mut variables = HashMap::new();
    variables.insert("approved".to_string(), json!(true));
    runtime_service
        .evaluate_conditional_events(process_instance.id.clone(), variables)
        .unwrap();

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 2);
    assert!(
        tasks_after
            .iter()
            .any(|task| task.task_definition_key == "userTask1"),
        "Host user task should remain after non-interrupting conditional boundary fires"
    );
    assert!(
        tasks_after
            .iter()
            .any(|task| task.task_definition_key == "escalatedTask"),
        "Boundary path should create the escalation user task"
    );

    let host_task = tasks_after
        .iter()
        .find(|task| task.task_definition_key == "userTask1")
        .unwrap();
    let mut session = runtime_store.create_session().unwrap();
    let executions_after = runtime_store.snapshot_executions(&mut session);
    let host_execution = executions_after
        .get(&host_task.execution_id)
        .expect("Host execution should still exist");
    assert_eq!(host_execution.activity_id.as_deref(), Some("userTask1"));
    assert!(
        !host_execution.is_active,
        "Host user task execution should remain in wait state"
    );

    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_after.len(),
        1,
        "Triggered non-interrupting conditional boundary should retain state (repeat)"
    );
    assert_eq!(
        boundary_states_after[0].boundary_event_id,
        "conditionalBoundary1"
    );

    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        !process_instance_after.is_ended,
        "Process should not end while the host user task remains open"
    );
}

/// Java: `BoundaryConditionalEventTest#testCatchNonInterruptingConditionalOnEmbeddedSubprocess`
/// (lines 79–117): same non-interrupting conditional boundary fires twice while the
/// host remains; task count grows by one each fire; process stays open.
#[test]
fn test_non_interrupting_conditional_boundary_on_embedded_subprocess_fires_twice() {
    let process_engine =
        ProcessEngine::new("non-interrupting-conditional-subprocess-repeat".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="nonInterruptingCondSubProcess" name="Non Interrupting Conditional SubProcess">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="reviewScope" />
            <subProcess id="reviewScope">
                <startEvent id="subStart" />
                <sequenceFlow id="subFlow1" sourceRef="subStart" targetRef="innerReviewTask" />
                <userTask id="innerReviewTask" name="Inner Review" />
                <sequenceFlow id="subFlow2" sourceRef="innerReviewTask" targetRef="subEnd" />
                <endEvent id="subEnd" />
            </subProcess>
            <boundaryEvent id="conditionalBoundary" attachedToRef="reviewScope" cancelActivity="false">
                <conditionalEventDefinition>
                    <condition>${myVar == true}</condition>
                </conditionalEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="reviewScope" targetRef="normalEnd" />
            <sequenceFlow id="flow3" sourceRef="conditionalBoundary" targetRef="taskAfterConditionalCatch" />
            <userTask id="taskAfterConditionalCatch" name="After Conditional" />
            <sequenceFlow id="flow4" sourceRef="taskAfterConditionalCatch" targetRef="boundaryEnd" />
            <endEvent id="normalEnd" />
            <endEvent id="boundaryEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Non Interrupting Conditional SubProcess Deployment".to_string())
                .add_string(
                    "nonInterruptingCondSubProcess.bpmn20.xml".to_string(),
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
                .name("Non Interrupting Conditional SubProcess Instance".to_string()),
        )
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 1);
    assert_eq!(tasks_before[0].task_definition_key, "innerReviewTask");

    // Java: BoundaryConditionalEventTest#testCatchNonInterruptingConditionalOnEmbeddedSubprocess
    // sets myVar=true before each runtimeService.trigger — the conditional gate
    // (ConditionUtil.hasTrueCondition) requires the condition to hold.
    runtime_service
        .set_variable(
            process_instance.id.clone(),
            "myVar".to_string(),
            json!(true),
        )
        .unwrap();

    // First fire via direct boundary trigger (Java: runtimeService.trigger).
    runtime_service
        .trigger_boundary_event(
            "conditionalBoundary".to_string(),
            process_instance.id.clone(),
        )
        .unwrap();

    let tasks_after_first = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_first.len(),
        2,
        "first fire should add one taskAfterConditionalCatch (1 host + 1 boundary)"
    );
    assert!(
        tasks_after_first
            .iter()
            .any(|t| t.task_definition_key == "innerReviewTask"),
        "host inner task must remain after non-interrupting fire"
    );
    assert_eq!(
        tasks_after_first
            .iter()
            .filter(|t| t.task_definition_key == "taskAfterConditionalCatch")
            .count(),
        1
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states_after_first = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        states_after_first.len(),
        1,
        "non-interrupting conditional boundary state must survive first fire"
    );
    drop(session);

    // Second fire of the same boundary while host is still open (condition still true).
    runtime_service
        .trigger_boundary_event(
            "conditionalBoundary".to_string(),
            process_instance.id.clone(),
        )
        .unwrap();

    let tasks_after_second = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_second.len(),
        3,
        "second fire should add another taskAfterConditionalCatch (1 host + 2 boundary)"
    );
    assert!(
        tasks_after_second
            .iter()
            .any(|t| t.task_definition_key == "innerReviewTask"),
        "host inner task must remain after second fire"
    );
    assert_eq!(
        tasks_after_second
            .iter()
            .filter(|t| t.task_definition_key == "taskAfterConditionalCatch")
            .count(),
        2,
        "two boundary path instances expected after two fires"
    );

    let mut session = runtime_store.create_session().unwrap();
    let states_after_second = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        states_after_second.len(),
        1,
        "boundary state must still be present after repeated fires"
    );
    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should exist");
    assert!(
        !process_instance_after.is_ended,
        "process must stay open while host task remains"
    );
}

/// P13: direct `trigger_boundary_event` on a conditional boundary must re-evaluate
/// the condition (Java `BoundaryConditionalEventActivityBehavior.trigger`). When
/// null: command error (state retained, host task stays, no downstream task).
/// False is a silent no-op, and after setting the variable true a later trigger
/// fires normally.
#[test]
fn test_conditional_boundary_trigger_noop_when_condition_false_then_fires_when_true() {
    let process_engine =
        ProcessEngine::new("conditional-trigger-gate-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="conditionalTriggerGate" name="Conditional Trigger Gate">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Host" />
            <boundaryEvent id="conditionalBoundary" attachedToRef="userTask1" cancelActivity="false">
                <conditionalEventDefinition>
                    <condition>${myVar == true}</condition>
                </conditionalEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEnd" />
            <sequenceFlow id="flow3" sourceRef="conditionalBoundary" targetRef="taskAfterCatch" />
            <userTask id="taskAfterCatch" name="After Conditional" />
            <sequenceFlow id="flow4" sourceRef="taskAfterCatch" targetRef="boundaryEnd" />
            <endEvent id="normalEnd" />
            <endEvent id="boundaryEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Conditional Trigger Gate Deployment".to_string())
                .add_string(
                    "conditionalTriggerGate.bpmn20.xml".to_string(),
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
                .name("Conditional Trigger Gate Instance".to_string()),
        )
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 1);
    assert_eq!(tasks_before[0].task_definition_key, "userTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(states_before.len(), 1);
    assert_eq!(states_before[0].boundary_event_id, "conditionalBoundary");
    drop(session);

    // Java UelExpressionCondition parity: an unset variable produces null and
    // fails the command instead of being treated as false.
    let error = runtime_service
        .trigger_boundary_event(
            "conditionalBoundary".to_string(),
            process_instance.id.clone(),
        )
        .expect_err("a null conditional boundary result must fail the command");
    assert!(matches!(
        error,
        flowable_engine::error::FlowableError::ExecutionError(message)
            if message.contains("non-Boolean") && message.ends_with("null")
    ));

    // An actual Boolean false remains the silent no-op case.
    runtime_service
        .set_variable(
            process_instance.id.clone(),
            "myVar".to_string(),
            json!(false),
        )
        .unwrap();
    runtime_service
        .trigger_boundary_event(
            "conditionalBoundary".to_string(),
            process_instance.id.clone(),
        )
        .unwrap();

    let tasks_after_false = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_false.len(),
        1,
        "condition false must not create boundary path task"
    );
    assert_eq!(tasks_after_false[0].task_definition_key, "userTask1");

    let mut session = runtime_store.create_session().unwrap();
    let states_after_false = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        states_after_false.len(),
        1,
        "boundary state must be retained when condition is false"
    );
    assert_eq!(
        states_after_false[0].boundary_event_id,
        "conditionalBoundary"
    );
    drop(session);

    // Satisfy condition, then trigger again → fire.
    runtime_service
        .set_variable(
            process_instance.id.clone(),
            "myVar".to_string(),
            json!(true),
        )
        .unwrap();
    runtime_service
        .trigger_boundary_event(
            "conditionalBoundary".to_string(),
            process_instance.id.clone(),
        )
        .unwrap();

    let tasks_after_true = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_true.len(),
        2,
        "condition true must fire boundary path (host + after-catch)"
    );
    assert!(
        tasks_after_true
            .iter()
            .any(|t| t.task_definition_key == "userTask1"),
        "host task remains for non-interrupting boundary"
    );
    assert!(
        tasks_after_true
            .iter()
            .any(|t| t.task_definition_key == "taskAfterCatch"),
        "boundary path must create taskAfterCatch"
    );

    let mut session = runtime_store.create_session().unwrap();
    let states_after_true = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        states_after_true.len(),
        1,
        "non-interrupting conditional boundary state survives fire (repeat)"
    );
}

/// Java: `BoundaryConditionalEventTest#testCatchNonInterruptingConditionalOnEmbeddedSubprocessWithEvaluation`
/// (lines 137–177): two evaluateConditionalEvents with satisfying vars each produce a
/// new instance; a third call with unsatisfied vars does not fire and does not drop state.
#[test]
fn test_non_interrupting_conditional_boundary_repeat_via_evaluate_and_skips_when_false() {
    let process_engine =
        ProcessEngine::new("non-interrupting-conditional-evaluate-repeat".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="nonInterruptingCondEvalProcess" name="Non Interrupting Conditional Eval">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review Request" />
            <boundaryEvent id="conditionalBoundary1" attachedToRef="userTask1" cancelActivity="false">
                <conditionalEventDefinition>
                    <condition>${approved == true}</condition>
                </conditionalEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="conditionalBoundary1" targetRef="taskAfterConditionalCatch" />
            <userTask id="taskAfterConditionalCatch" name="After Conditional" />
            <sequenceFlow id="flow4" sourceRef="taskAfterConditionalCatch" targetRef="conditionalEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="conditionalEndEvent" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Non Interrupting Conditional Eval Deployment".to_string())
                .add_string(
                    "nonInterruptingCondEval.bpmn20.xml".to_string(),
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
                .name("Non Interrupting Conditional Eval Instance".to_string()),
        )
        .unwrap();

    let mut true_vars = HashMap::new();
    true_vars.insert("approved".to_string(), json!(true));

    runtime_service
        .evaluate_conditional_events(process_instance.id.clone(), true_vars.clone())
        .unwrap();

    let tasks_after_first = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_after_first.len(), 2);
    assert_eq!(
        tasks_after_first
            .iter()
            .filter(|t| t.task_definition_key == "taskAfterConditionalCatch")
            .count(),
        1
    );

    // Second evaluation with condition still true: another boundary instance.
    runtime_service
        .evaluate_conditional_events(process_instance.id.clone(), true_vars)
        .unwrap();

    let tasks_after_second = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_second.len(),
        3,
        "second evaluate with true condition should spawn another boundary task"
    );
    assert_eq!(
        tasks_after_second
            .iter()
            .filter(|t| t.task_definition_key == "taskAfterConditionalCatch")
            .count(),
        2
    );
    assert!(
        tasks_after_second
            .iter()
            .any(|t| t.task_definition_key == "userTask1"),
        "host task must remain"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states_after_second = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        states_after_second.len(),
        1,
        "boundary state must survive evaluate-driven fires"
    );
    drop(session);

    // Third evaluation with unsatisfied condition: no new instance, state retained.
    let mut false_vars = HashMap::new();
    false_vars.insert("approved".to_string(), json!(false));
    runtime_service
        .evaluate_conditional_events(process_instance.id.clone(), false_vars)
        .unwrap();

    let tasks_after_false = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_false.len(),
        3,
        "unsatisfied condition must not spawn another boundary task"
    );
    assert_eq!(
        tasks_after_false
            .iter()
            .filter(|t| t.task_definition_key == "taskAfterConditionalCatch")
            .count(),
        2
    );

    let mut session = runtime_store.create_session().unwrap();
    let states_after_false = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        states_after_false.len(),
        1,
        "unsatisfied evaluate must not consume boundary state"
    );
    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should exist");
    assert!(!process_instance_after.is_ended);
}

#[test]
fn test_non_interrupting_conditional_boundary_uses_evaluated_boundary_not_matching_condition_ref() {
    let process_engine =
        ProcessEngine::new("conditional-boundary-same-condition-ref-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="sameConditionBoundaryProcess" name="Same Condition Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="userTaskA" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="userTaskB" />
            <userTask id="userTaskA" name="Review A" />
            <userTask id="userTaskB" name="Review B" />
            <boundaryEvent id="conditionalBoundaryA" attachedToRef="userTaskA" cancelActivity="false">
                <conditionalEventDefinition>
                    <condition>${approved == true}</condition>
                </conditionalEventDefinition>
            </boundaryEvent>
            <boundaryEvent id="conditionalBoundaryB" attachedToRef="userTaskB" cancelActivity="false">
                <conditionalEventDefinition>
                    <condition>${approved == true}</condition>
                </conditionalEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow4" sourceRef="userTaskA" targetRef="join" />
            <sequenceFlow id="flow5" sourceRef="userTaskB" targetRef="join" />
            <parallelGateway id="join" />
            <sequenceFlow id="flow6" sourceRef="join" targetRef="normalEndEvent" />
            <sequenceFlow id="flow7" sourceRef="conditionalBoundaryA" targetRef="escalatedTaskA" />
            <sequenceFlow id="flow8" sourceRef="conditionalBoundaryB" targetRef="escalatedTaskB" />
            <userTask id="escalatedTaskA" name="Escalated A" />
            <userTask id="escalatedTaskB" name="Escalated B" />
            <sequenceFlow id="flow9" sourceRef="escalatedTaskA" targetRef="boundaryEndA" />
            <sequenceFlow id="flow10" sourceRef="escalatedTaskB" targetRef="boundaryEndB" />
            <endEvent id="normalEndEvent" />
            <endEvent id="boundaryEndA" />
            <endEvent id="boundaryEndB" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Same Condition Boundary Deployment".to_string())
                .add_string(
                    "sameConditionBoundary.bpmn20.xml".to_string(),
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
                .name("Same Condition Boundary Instance".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let session = runtime_store.create_session().unwrap();
    drop(session);
    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 2);

    let mut session = runtime_store.create_session().unwrap();
    for task in &tasks_before {
        let mut execution = runtime_store
            .find_execution(&task.execution_id, &mut session)
            .expect("Task execution should exist");
        execution.set_process_variable(
            "approved".to_string(),
            json!(task.task_definition_key == "userTaskB"),
        );
        runtime_store.update_execution(&execution, &mut session);
    }
    session.flush_and_commit().unwrap();

    runtime_service
        .evaluate_conditional_events(process_instance.id.clone(), HashMap::new())
        .unwrap();

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after
            .iter()
            .any(|task| task.task_definition_key == "userTaskA"),
        "Host userTaskA should remain"
    );
    assert!(
        tasks_after
            .iter()
            .any(|task| task.task_definition_key == "userTaskB"),
        "Host userTaskB should remain"
    );
    assert!(
        !tasks_after
            .iter()
            .any(|task| task.task_definition_key == "escalatedTaskA"),
        "Boundary A must not trigger when only userTaskB's local condition is true"
    );
    assert!(
        tasks_after
            .iter()
            .any(|task| task.task_definition_key == "escalatedTaskB"),
        "Boundary B should trigger from the evaluated host execution"
    );

    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states_after
            .iter()
            .any(|state| state.boundary_event_id == "conditionalBoundaryA"),
        "Boundary A should remain registered"
    );
    assert!(
        boundary_states_after
            .iter()
            .any(|state| state.boundary_event_id == "conditionalBoundaryB"),
        "Non-interrupting Boundary B should retain state after fire (repeat)"
    );
}

#[test]
fn test_interrupting_conditional_boundary_on_subprocess_registers_and_cancels_scope() {
    let process_engine = ProcessEngine::new("subprocess-conditional-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="subProcessConditionalBoundaryProcess" name="SubProcess Conditional Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="reviewScope" />
            <subProcess id="reviewScope">
                <startEvent id="subStart" />
                <sequenceFlow id="subFlow1" sourceRef="subStart" targetRef="innerReviewTask" />
                <userTask id="innerReviewTask" name="Inner Review" />
                <sequenceFlow id="subFlow2" sourceRef="innerReviewTask" targetRef="subEnd" />
                <endEvent id="subEnd" />
            </subProcess>
            <boundaryEvent id="conditionalBoundary" attachedToRef="reviewScope" cancelActivity="true">
                <conditionalEventDefinition>
                    <condition>${needsEscalation == true}</condition>
                </conditionalEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="reviewScope" targetRef="normalEnd" />
            <sequenceFlow id="flow3" sourceRef="conditionalBoundary" targetRef="escalationTask" />
            <userTask id="escalationTask" name="Escalated Review" />
            <sequenceFlow id="flow4" sourceRef="escalationTask" targetRef="escalationEnd" />
            <endEvent id="normalEnd" />
            <endEvent id="escalationEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("SubProcess Conditional Boundary Deployment".to_string())
                .add_string(
                    "subProcessConditionalBoundary.bpmn20.xml".to_string(),
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
                .name("SubProcess Conditional Boundary Instance".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(boundary_states.len(), 1);
    assert_eq!(boundary_states[0].boundary_event_id, "conditionalBoundary");
    assert_eq!(
        boundary_states[0].event_subscription.kind,
        EventSubscriptionKind::Conditional
    );
    assert!(
        boundary_states[0].cancel_activity,
        "subprocess conditional boundary should be interrupting"
    );
    drop(session);

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 1);
    assert_eq!(tasks_before[0].task_definition_key, "innerReviewTask");

    let mut variables = HashMap::new();
    variables.insert("needsEscalation".to_string(), json!(true));
    runtime_service
        .evaluate_conditional_events(process_instance.id.clone(), variables)
        .unwrap();

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 1);
    assert_eq!(tasks_after[0].task_definition_key, "escalationTask");

    let mut session = runtime_store.create_session().unwrap();
    let executions_after = runtime_store.snapshot_executions(&mut session);
    assert!(
        executions_after
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("reviewScope")),
        "interrupting subprocess boundary should remove the host scope execution"
    );
    assert!(
        executions_after
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("innerReviewTask")),
        "interrupting subprocess boundary should remove child activity executions"
    );

    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states_after.is_empty(),
        "triggered subprocess conditional boundary should be consumed"
    );
}

#[test]
fn test_non_interrupting_boundary_event_preserves_host_task_and_execution() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="nonInterruptingBoundaryTest" name="Non-Interrupting Boundary Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="false">
                <messageEventDefinition messageRef="notifyMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Wait For Ack" />
            <sequenceFlow id="flow4" sourceRef="receiveTask1" targetRef="endEvent2" />
            <endEvent id="endEvent1" />
            <endEvent id="endEvent2" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Non-Interrupting Boundary Test Deployment".to_string())
        .add_string(
            "nonInterruptingBoundaryTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Non-Interrupting Boundary Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_before.len(),
        1,
        "Should have one task before boundary event trigger"
    );
    assert_eq!(tasks_before[0].task_definition_key, "userTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_before.len(),
        1,
        "Should have boundary event state before trigger"
    );
    assert!(
        !boundary_states_before[0].cancel_activity,
        "Boundary event should be non-interrupting"
    );
    drop(session);

    let _host_execution_id = tasks_before[0].execution_id.clone();

    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), process_instance.id.clone())
        .unwrap();

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    let user_tasks: Vec<_> = tasks_after
        .iter()
        .filter(|t| t.task_definition_key == "userTask1")
        .collect();
    assert_eq!(
        user_tasks.len(),
        1,
        "UserTask should still exist after non-interrupting boundary event trigger"
    );

    let mut session = runtime_store.create_session().unwrap();
    let executions_after = runtime_store.snapshot_executions(&mut session);
    let host_execution = executions_after
        .get(&user_tasks[0].execution_id)
        .expect("Host execution should still exist");
    assert_eq!(
        host_execution.activity_id.as_deref(),
        Some("userTask1"),
        "Host execution should still be on userTask1"
    );
    assert!(
        !host_execution.is_active,
        "Host execution should still be in wait state"
    );

    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    // Java MessageNonInterruptingBoundaryEventTest: subscription is not removed
    // on fire (repeat while host is active). Removed only when host ends.
    assert_eq!(
        boundary_states_after.len(),
        1,
        "Non-interrupting boundary subscription must survive its own trigger"
    );

    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        !process_instance_after.is_ended,
        "Process should NOT be ended after non-interrupting boundary event triggers"
    );
}

#[test]
fn test_non_interrupting_boundary_path_completes_correctly() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="nonInterruptingBoundaryPathTest" name="Non-Interrupting Boundary Path Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="false">
                <messageEventDefinition messageRef="notifyMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Wait For Ack" />
            <sequenceFlow id="flow4" sourceRef="receiveTask1" targetRef="endEvent2" />
            <endEvent id="endEvent1" />
            <endEvent id="endEvent2" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Non-Interrupting Boundary Path Test Deployment".to_string())
        .add_string(
            "nonInterruptingBoundaryPathTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Non-Interrupting Boundary Path Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    let executions_before = runtime_store.snapshot_executions(&mut session);
    let boundary_execution_ids_before: Vec<_> = executions_before
        .values()
        .filter(|e| e.activity_id.as_deref() == Some("boundaryEvent1"))
        .map(|e| e.id.clone())
        .collect();
    assert!(
        boundary_execution_ids_before.is_empty(),
        "No boundary execution should exist before trigger"
    );
    drop(session);

    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), process_instance.id.clone())
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let executions_after = runtime_store.snapshot_executions(&mut session);
    let receive_task_executions: Vec<_> = executions_after
        .values()
        .filter(|e| e.activity_id.as_deref() == Some("receiveTask1"))
        .collect();
    assert_eq!(
        receive_task_executions.len(),
        1,
        "Should have one execution on receive task after boundary trigger"
    );
    assert!(
        !receive_task_executions[0].is_active,
        "Receive task execution should be in wait state"
    );
}

// ============================================================================
// ReceiveTask with Interrupting Message Boundary Event Tests
// ============================================================================

#[test]
fn test_receive_task_registers_interrupting_and_non_interrupting_message_boundary_event_states() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="receiveTaskBoundaryTest" name="Receive Task Boundary Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Message" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="receiveTask1" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </boundaryEvent>
            <boundaryEvent id="boundaryEvent2" attachedToRef="receiveTask1" cancelActivity="false">
                <messageEventDefinition messageRef="notifyMessage" />
            </boundaryEvent>
            <boundaryEvent id="boundaryEvent3" attachedToRef="receiveTask1" cancelActivity="true">
                <timerEventDefinition><timeDuration>PT10H</timeDuration></timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <sequenceFlow id="flow4" sourceRef="boundaryEvent2" targetRef="notifyEndEvent" />
            <sequenceFlow id="flow5" sourceRef="boundaryEvent3" targetRef="timerEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="cancelEndEvent" />
            <endEvent id="notifyEndEvent" />
            <endEvent id="timerEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Receive Task Boundary Test Deployment".to_string())
        .add_string(
            "receiveTaskBoundaryTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Receive Task Boundary Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    let receive_task_execution = executions
        .values()
        .find(|e| e.activity_id.as_deref() == Some("receiveTask1"))
        .expect("Receive task execution should exist");
    assert!(
        !receive_task_execution.is_active,
        "Receive task should be in wait state"
    );

    // Verify both wait-state and boundary state are registered
    let wait_states = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        wait_states.len(),
        1,
        "Should have one message-style wait state"
    );
    assert_eq!(
        wait_states[0].wait_kind,
        RuntimeMessageStyleWaitKind::ReceiveTask
    );
    assert_eq!(wait_states[0].activity_id.as_deref(), Some("receiveTask1"));

    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states.len(),
        2,
        "Should register both interrupting and non-interrupting message boundary events"
    );

    let interrupting_state = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "boundaryEvent1")
        .expect("Interrupting boundary should be registered");
    assert_eq!(interrupting_state.attached_activity_id, "receiveTask1");
    assert_eq!(interrupting_state.process_instance_id, process_instance.id);
    assert_eq!(
        interrupting_state.host_execution_id,
        receive_task_execution.id
    );
    assert!(
        interrupting_state.cancel_activity,
        "boundaryEvent1 should be interrupting"
    );

    let non_interrupting_state = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "boundaryEvent2")
        .expect("Non-interrupting boundary should be registered");
    assert_eq!(non_interrupting_state.attached_activity_id, "receiveTask1");
    assert_eq!(
        non_interrupting_state.process_instance_id,
        process_instance.id
    );
    assert_eq!(
        non_interrupting_state.host_execution_id,
        receive_task_execution.id
    );
    assert!(
        !non_interrupting_state.cancel_activity,
        "boundaryEvent2 should be non-interrupting"
    );

    assert!(
        runtime_store
            .find_boundary_event_state("boundaryEvent3", &process_instance.id, &mut session)
            .is_none(),
        "Timer boundary event should not be registered"
    );
}

#[test]
fn test_receive_task_interrupting_boundary_event_cancels_host_and_follows_boundary_path() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="receiveTaskBoundaryCancelTest" name="Receive Task Boundary Cancel Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Message" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="receiveTask1" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Receive Task Boundary Cancel Test Deployment".to_string())
        .add_string(
            "receiveTaskBoundaryCancelTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Receive Task Boundary Cancel Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_before.len(),
        1,
        "Should have one task before boundary event trigger"
    );
    assert_eq!(tasks_before[0].task_definition_key, "receiveTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    // Verify wait-state and boundary state exist before trigger
    let wait_states_before = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        wait_states_before.len(),
        1,
        "Should have wait state before trigger"
    );

    let boundary_states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_before.len(),
        1,
        "Should have boundary event state before trigger"
    );
    assert!(
        boundary_states_before[0].cancel_activity,
        "Boundary event should be interrupting"
    );
    drop(session);

    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), process_instance.id.clone())
        .unwrap();

    // Verify task is deleted
    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after.is_empty(),
        "Task should be deleted after boundary event trigger"
    );

    // Verify wait-state is cleaned up
    let mut session = runtime_store.create_session().unwrap();
    let wait_states_after = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        wait_states_after.is_empty(),
        "Wait state should be cleaned up after boundary event trigger"
    );

    // Verify execution is removed
    let executions_after = runtime_store.snapshot_executions(&mut session);
    assert!(
        executions_after
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("receiveTask1")),
        "Receive task execution should be removed after boundary event trigger"
    );

    // Verify boundary state is cleaned up
    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states_after.is_empty(),
        "Boundary event state should be cleaned up after trigger"
    );

    // Verify process is ended
    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        process_instance_after.is_ended,
        "Process should be ended after boundary event triggers cancel path"
    );
}

#[test]
fn test_receive_task_non_interrupting_boundary_event_preserves_host_task_and_execution() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="receiveTaskNonInterruptingBoundaryTest" name="Receive Task Non-Interrupting Boundary Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Message" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="receiveTask1" cancelActivity="false">
                <messageEventDefinition messageRef="notifyMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Notification" />
            <sequenceFlow id="flow4" sourceRef="userTask1" targetRef="endEvent2" />
            <endEvent id="endEvent1" />
            <endEvent id="endEvent2" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Receive Task Non-Interrupting Boundary Test Deployment".to_string())
        .add_string(
            "receiveTaskNonInterruptingBoundaryTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Receive Task Non-Interrupting Boundary Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_before.len(),
        1,
        "Should have one task before boundary event trigger"
    );
    assert_eq!(tasks_before[0].task_definition_key, "receiveTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    // Verify wait-state and boundary state exist before trigger
    let wait_states_before = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        wait_states_before.len(),
        1,
        "Should have wait state before trigger"
    );

    let boundary_states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_before.len(),
        1,
        "Should have boundary event state before trigger"
    );
    assert!(
        !boundary_states_before[0].cancel_activity,
        "Boundary event should be non-interrupting"
    );
    drop(session);

    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), process_instance.id.clone())
        .unwrap();

    // Verify original ReceiveTask still exists
    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    let receive_tasks: Vec<_> = tasks_after
        .iter()
        .filter(|t| t.task_definition_key == "receiveTask1")
        .collect();
    assert_eq!(
        receive_tasks.len(),
        1,
        "ReceiveTask should still exist after non-interrupting boundary event trigger"
    );

    // Verify wait-state still exists
    let mut session = runtime_store.create_session().unwrap();
    let wait_states_after = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        wait_states_after.len(),
        1,
        "Wait state should still exist after non-interrupting boundary event trigger"
    );

    // Verify execution is still on receiveTask1
    let executions_after = runtime_store.snapshot_executions(&mut session);
    let host_execution = executions_after
        .get(&receive_tasks[0].execution_id)
        .expect("Host execution should still exist");
    assert_eq!(
        host_execution.activity_id.as_deref(),
        Some("receiveTask1"),
        "Host execution should still be on receiveTask1"
    );
    assert!(
        !host_execution.is_active,
        "Host execution should still be in wait state"
    );

    // Verify boundary state is cleaned up
    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    // Java MessageNonInterruptingBoundaryEventTest: subscription is not removed
    // on fire (repeat while host is active). Removed only when host ends.
    assert_eq!(
        boundary_states_after.len(),
        1,
        "Non-interrupting boundary subscription must survive its own trigger"
    );

    // Verify process is NOT ended
    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        !process_instance_after.is_ended,
        "Process should NOT be ended after non-interrupting boundary event triggers"
    );
}

#[test]
fn test_receive_task_non_interrupting_boundary_path_completes_correctly() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="receiveTaskNonInterruptingBoundaryPathTest" name="Receive Task Non-Interrupting Boundary Path Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Message" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="receiveTask1" cancelActivity="false">
                <messageEventDefinition messageRef="notifyMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Notification" />
            <sequenceFlow id="flow4" sourceRef="userTask1" targetRef="endEvent2" />
            <endEvent id="endEvent1" />
            <endEvent id="endEvent2" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Receive Task Non-Interrupting Boundary Path Test Deployment".to_string())
        .add_string(
            "receiveTaskNonInterruptingBoundaryPathTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Receive Task Non-Interrupting Boundary Path Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    let executions_before = runtime_store.snapshot_executions(&mut session);
    let boundary_execution_ids_before: Vec<_> = executions_before
        .values()
        .filter(|e| e.activity_id.as_deref() == Some("boundaryEvent1"))
        .map(|e| e.id.clone())
        .collect();
    assert!(
        boundary_execution_ids_before.is_empty(),
        "No boundary execution should exist before trigger"
    );
    drop(session);

    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), process_instance.id.clone())
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let executions_after = runtime_store.snapshot_executions(&mut session);
    let user_task_executions: Vec<_> = executions_after
        .values()
        .filter(|e| e.activity_id.as_deref() == Some("userTask1"))
        .collect();
    assert_eq!(
        user_task_executions.len(),
        1,
        "Should have one execution on user task after boundary trigger"
    );
    assert!(
        !user_task_executions[0].is_active,
        "User task execution should be in wait state"
    );
}

#[test]
fn test_receive_task_normal_wake_up_still_works_when_boundary_not_triggered() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="receiveTaskNormalWakeupTest" name="Receive Task Normal Wakeup Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Message" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="receiveTask1" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <endEvent id="endEvent1" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Receive Task Normal Wakeup Test Deployment".to_string())
        .add_string(
            "receiveTaskNormalWakeupTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Receive Task Normal Wakeup Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let session = runtime_store.create_session().unwrap();
    drop(session);

    // Verify initial state
    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 1, "Should have one task before wake up");

    let mut session = runtime_store.create_session().unwrap();
    let wait_states_before = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        wait_states_before.len(),
        1,
        "Should have wait state before wake up"
    );

    let boundary_states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_before.len(),
        1,
        "Should have boundary event state before wake up"
    );
    drop(session);

    // Use normal wake-up (not boundary trigger)
    process_engine.wake_up_message_by_process_instance_id(process_instance.id.clone());

    // Verify normal path completed
    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after.is_empty(),
        "Task should be deleted after normal wake up"
    );

    let mut session = runtime_store.create_session().unwrap();
    let wait_states_after = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        wait_states_after.is_empty(),
        "Wait state should be cleaned up after normal wake up"
    );

    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states_after.is_empty(),
        "Boundary event state should be cleaned up after normal wake up"
    );

    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        process_instance_after.is_ended,
        "Process should be ended after normal wake up"
    );
}

#[test]
fn test_receive_task_ignores_unsupported_boundary_events() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="receiveTaskUnsupportedBoundaryTest" name="Receive Task Unsupported Boundary Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Message" />
            <boundaryEvent id="timerBoundary" attachedToRef="receiveTask1" cancelActivity="true">
                <timerEventDefinition><timeDuration>PT10H</timeDuration></timerEventDefinition>
            </boundaryEvent>
            <boundaryEvent id="interruptingMessageBoundary" attachedToRef="receiveTask1" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="timerBoundary" targetRef="timerEndEvent" />
            <sequenceFlow id="flow5" sourceRef="interruptingMessageBoundary" targetRef="cancelEndEvent" />
            <endEvent id="endEvent1" />
            <endEvent id="timerEndEvent" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Receive Task Unsupported Boundary Test Deployment".to_string())
        .add_string(
            "receiveTaskUnsupportedBoundaryTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Receive Task Unsupported Boundary Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    // Only the interrupting message boundary should be registered
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states.len(),
        1,
        "Should only register interrupting message boundary event"
    );
    assert_eq!(
        boundary_states[0].boundary_event_id, "interruptingMessageBoundary",
        "Only interrupting message boundary should be registered"
    );

    // Verify timer is not registered
    assert!(
        runtime_store
            .find_boundary_event_state("timerBoundary", &process_instance.id, &mut session)
            .is_none(),
        "Timer boundary event should not be registered"
    );
}

#[test]
fn test_concurrent_instances_interrupting_boundary_isolation() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="concurrentInterruptingTest" name="Concurrent Interrupting Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Concurrent Interrupting Deployment".to_string())
        .add_string(
            "concurrentInterruptingTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // Start instance A
    let instance_a = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.clone())
                .name("Instance A".to_string()),
        )
        .unwrap();

    // Start instance B
    let instance_b = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.clone())
                .name("Instance B".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    // Verify both have the boundary registration
    let bounds_a = runtime_store
        .find_boundary_event_states_by_process_instance_id(&instance_a.id, &mut session);
    let bounds_b = runtime_store
        .find_boundary_event_states_by_process_instance_id(&instance_b.id, &mut session);
    assert_eq!(bounds_a.len(), 1, "Instance A should have 1 boundary");
    assert_eq!(bounds_b.len(), 1, "Instance B should have 1 boundary");
    drop(session);

    // Trigger boundary on Instance A
    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), instance_a.id.clone())
        .unwrap();

    // Verify Instance A is ended (due to cancel path)
    let mut session = runtime_store.create_session().unwrap();
    let after_a = runtime_store
        .find_process_instance(&instance_a.id, &mut session)
        .unwrap();
    assert!(after_a.is_ended, "Instance A should be ended");

    // Verify Instance B is still waiting
    let after_b = runtime_store
        .find_process_instance(&instance_b.id, &mut session)
        .unwrap();
    assert!(!after_b.is_ended, "Instance B should not be ended");

    let bounds_b_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&instance_b.id, &mut session);
    assert_eq!(
        bounds_b_after.len(),
        1,
        "Instance B boundary should remain intact"
    );
    drop(session);

    let tasks_b = task_service
        .get_tasks_by_process_instance_id(instance_b.id.clone())
        .unwrap();
    assert_eq!(tasks_b.len(), 1, "Instance B task should remain intact");
}

#[test]
fn test_concurrent_instances_non_interrupting_boundary_isolation() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="concurrentNonInterruptingTest" name="Concurrent Non Interrupting Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="false">
                <messageEventDefinition messageRef="notifyMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Wait For Ack" />
            <sequenceFlow id="flow4" sourceRef="receiveTask1" targetRef="endEvent2" />
            <endEvent id="endEvent1" />
            <endEvent id="endEvent2" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Concurrent Non Interrupting Deployment".to_string())
        .add_string(
            "concurrentNonInterruptingTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // Start instance A
    let instance_a = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.clone())
                .name("Instance A".to_string()),
        )
        .unwrap();

    // Start instance B
    let instance_b = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.clone())
                .name("Instance B".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let session = runtime_store.create_session().unwrap();
    drop(session);

    // Trigger boundary on Instance A
    runtime_service
        .trigger_boundary_event("boundaryEvent1".to_string(), instance_a.id.clone())
        .unwrap();

    // Non-interrupting: subscription stays on A (repeat); isolation is that B
    // is untouched. Java MessageNonInterruptingBoundaryEventTest.
    let mut session = runtime_store.create_session().unwrap();
    let bounds_a_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&instance_a.id, &mut session);
    assert_eq!(
        bounds_a_after.len(),
        1,
        "Instance A non-interrupting boundary subscription survives its trigger"
    );

    // Check execution
    let execs = runtime_store.snapshot_executions(&mut session);
    let rcv_a: Vec<_> = execs
        .values()
        .filter(|e| {
            e.process_instance_id.as_deref() == Some(&instance_a.id)
                && e.activity_id.as_deref() == Some("receiveTask1")
        })
        .collect();
    assert_eq!(
        rcv_a.len(),
        1,
        "Instance A should reach receiveTask1 on boundary path"
    );

    // Verify Instance B's boundary is intact
    let bounds_b_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&instance_b.id, &mut session);
    assert_eq!(
        bounds_b_after.len(),
        1,
        "Instance B boundary should remain intact"
    );
    drop(session);

    let rcv_b: Vec<_> = execs
        .values()
        .filter(|e| {
            e.process_instance_id.as_deref() == Some(&instance_b.id)
                && e.activity_id.as_deref() == Some("receiveTask1")
        })
        .collect();
    assert_eq!(rcv_b.len(), 0, "Instance B should NOT reach receiveTask1");

    let tasks_b = task_service
        .get_tasks_by_process_instance_id(instance_b.id.clone())
        .unwrap();
    assert_eq!(tasks_b.len(), 1, "Instance B task should remain intact");
}

#[test]
fn test_trigger_boundary_event_by_message_ref() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="triggerByMessageRefTest" name="Trigger By Message Ref Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="true">
                <messageEventDefinition messageRef="specialCancelMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Trigger By Message Ref Test Deployment".to_string())
        .add_string(
            "triggerByMessageRefTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Trigger By Message Ref Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 1);

    // Try triggering with wrong message_ref
    process_engine.trigger_boundary_event_by_message_ref(
        "wrongMessage".to_string(),
        process_instance.id.clone(),
    );

    let tasks_after_wrong = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_wrong.len(),
        1,
        "Task should still exist after wrong message ref"
    );

    // Trigger with correct message_ref
    process_engine.trigger_boundary_event_by_message_ref(
        "specialCancelMessage".to_string(),
        process_instance.id.clone(),
    );

    let tasks_after_correct = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after_correct.is_empty(),
        "Task should be deleted after correct message ref"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(process_instance_after.is_ended, "Process should be ended");
}

// ============================================================================
// Signal Boundary Event Tests on UserTask
// ============================================================================

#[test]
fn test_user_task_registers_interrupting_and_non_interrupting_signal_boundary_event_states() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="cancelSignal" name="Cancel Signal" />
        <signal id="notifySignal" name="Notify Signal" />
        <process id="signalBoundaryEventTest" name="Signal Boundary Event Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="true">
                <signalEventDefinition signalRef="cancelSignal" />
            </boundaryEvent>
            <boundaryEvent id="boundaryEvent2" attachedToRef="userTask1" cancelActivity="false">
                <signalEventDefinition signalRef="notifySignal" />
            </boundaryEvent>
            <boundaryEvent id="boundaryEvent3" attachedToRef="userTask1" cancelActivity="true">
                <timerEventDefinition><timeDuration>PT10H</timeDuration></timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <sequenceFlow id="flow4" sourceRef="boundaryEvent2" targetRef="notifyEndEvent" />
            <sequenceFlow id="flow5" sourceRef="boundaryEvent3" targetRef="timerEndEvent" />
            <endEvent id="endEvent1" />
            <endEvent id="cancelEndEvent" />
            <endEvent id="notifyEndEvent" />
            <endEvent id="timerEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Signal Boundary Event Test Deployment".to_string())
        .add_string(
            "signalBoundaryEventTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    let deployment = repository_service.deploy(builder).unwrap();
    assert_eq!(
        deployment.name.as_deref(),
        Some("Signal Boundary Event Test Deployment")
    );

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Signal Boundary Event Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    let user_task_execution = executions
        .values()
        .find(|e| e.activity_id.as_deref() == Some("userTask1"))
        .expect("User task execution should exist");
    assert!(
        !user_task_execution.is_active,
        "User task should be in wait state"
    );

    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states.len(),
        2,
        "Should register both interrupting and non-interrupting signal boundary events"
    );

    let interrupting_state = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "boundaryEvent1")
        .expect("Interrupting boundary should be registered");
    assert_eq!(interrupting_state.attached_activity_id, "userTask1");
    assert_eq!(interrupting_state.process_instance_id, process_instance.id);
    assert_eq!(interrupting_state.host_execution_id, user_task_execution.id);
    assert!(
        interrupting_state.cancel_activity,
        "boundaryEvent1 should be interrupting"
    );
    assert_eq!(
        interrupting_state.event_subscription.kind,
        EventSubscriptionKind::Signal
    );
    assert_eq!(
        interrupting_state.event_subscription.event_ref,
        "cancelSignal"
    );

    let non_interrupting_state = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "boundaryEvent2")
        .expect("Non-interrupting boundary should be registered");
    assert_eq!(non_interrupting_state.attached_activity_id, "userTask1");
    assert_eq!(
        non_interrupting_state.process_instance_id,
        process_instance.id
    );
    assert_eq!(
        non_interrupting_state.host_execution_id,
        user_task_execution.id
    );
    assert!(
        !non_interrupting_state.cancel_activity,
        "boundaryEvent2 should be non-interrupting"
    );
    assert_eq!(
        non_interrupting_state.event_subscription.kind,
        EventSubscriptionKind::Signal
    );
    assert_eq!(
        non_interrupting_state.event_subscription.event_ref,
        "notifySignal"
    );

    assert!(
        runtime_store
            .find_boundary_event_state("boundaryEvent3", &process_instance.id, &mut session)
            .is_none(),
        "Timer boundary event should not be registered"
    );
}

#[test]
fn test_signal_interrupting_boundary_event_on_user_task_cancels_host_and_follows_boundary_path() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="cancelSignal" name="Cancel Signal" />
        <process id="signalBoundaryCancelTest" name="Signal Boundary Cancel Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="true">
                <signalEventDefinition signalRef="cancelSignal" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Signal Boundary Cancel Test Deployment".to_string())
        .add_string(
            "signalBoundaryCancelTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Signal Boundary Cancel Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_before.len(),
        1,
        "Should have one task before boundary event trigger"
    );
    assert_eq!(tasks_before[0].task_definition_key, "userTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_before.len(),
        1,
        "Should have boundary event state before trigger"
    );
    assert!(
        boundary_states_before[0].cancel_activity,
        "Boundary event should be interrupting"
    );
    drop(session);

    // Trigger by signal_ref
    runtime_service.trigger_boundary_event_by_signal_ref(
        "cancelSignal".to_string(),
        process_instance.id.clone(),
    );

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after.is_empty(),
        "Task should be deleted after boundary event trigger"
    );

    let mut session = runtime_store.create_session().unwrap();
    let executions_after = runtime_store.snapshot_executions(&mut session);
    assert!(
        executions_after
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("userTask1")),
        "User task execution should be removed after boundary event trigger"
    );

    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states_after.is_empty(),
        "Boundary event state should be cleaned up after trigger"
    );

    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        process_instance_after.is_ended,
        "Process should be ended after boundary event triggers cancel path"
    );
}

#[test]
fn test_signal_non_interrupting_boundary_event_on_user_task_preserves_host() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="notifySignal" name="Notify Signal" />
        <process id="signalNonInterruptingBoundaryTest" name="Signal Non-Interrupting Boundary Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="false">
                <signalEventDefinition signalRef="notifySignal" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Wait For Ack" />
            <sequenceFlow id="flow4" sourceRef="receiveTask1" targetRef="endEvent2" />
            <endEvent id="endEvent1" />
            <endEvent id="endEvent2" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Signal Non-Interrupting Boundary Test Deployment".to_string())
        .add_string(
            "signalNonInterruptingBoundaryTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Signal Non-Interrupting Boundary Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_before.len(),
        1,
        "Should have one task before boundary event trigger"
    );
    assert_eq!(tasks_before[0].task_definition_key, "userTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_before.len(),
        1,
        "Should have boundary event state before trigger"
    );
    assert!(
        !boundary_states_before[0].cancel_activity,
        "Boundary event should be non-interrupting"
    );
    drop(session);

    // Trigger by signal_ref
    runtime_service.trigger_boundary_event_by_signal_ref(
        "notifySignal".to_string(),
        process_instance.id.clone(),
    );

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    let user_tasks: Vec<_> = tasks_after
        .iter()
        .filter(|t| t.task_definition_key == "userTask1")
        .collect();
    assert_eq!(
        user_tasks.len(),
        1,
        "UserTask should still exist after non-interrupting boundary event trigger"
    );

    let mut session = runtime_store.create_session().unwrap();
    let executions_after = runtime_store.snapshot_executions(&mut session);
    let host_execution = executions_after
        .get(&user_tasks[0].execution_id)
        .expect("Host execution should still exist");
    assert_eq!(
        host_execution.activity_id.as_deref(),
        Some("userTask1"),
        "Host execution should still be on userTask1"
    );
    assert!(
        !host_execution.is_active,
        "Host execution should still be in wait state"
    );

    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    // Java MessageNonInterruptingBoundaryEventTest: subscription is not removed
    // on fire (repeat while host is active). Removed only when host ends.
    assert_eq!(
        boundary_states_after.len(),
        1,
        "Non-interrupting boundary subscription must survive its own trigger"
    );

    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        !process_instance_after.is_ended,
        "Process should NOT be ended after non-interrupting boundary event triggers"
    );
}

// ============================================================================
// Signal Boundary Event Tests on ReceiveTask
// ============================================================================

#[test]
fn test_receive_task_registers_interrupting_and_non_interrupting_signal_boundary_event_states() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="cancelSignal" name="Cancel Signal" />
        <signal id="notifySignal" name="Notify Signal" />
        <process id="signalReceiveTaskBoundaryTest" name="Signal Receive Task Boundary Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Message" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="receiveTask1" cancelActivity="true">
                <signalEventDefinition signalRef="cancelSignal" />
            </boundaryEvent>
            <boundaryEvent id="boundaryEvent2" attachedToRef="receiveTask1" cancelActivity="false">
                <signalEventDefinition signalRef="notifySignal" />
            </boundaryEvent>
            <boundaryEvent id="boundaryEvent3" attachedToRef="receiveTask1" cancelActivity="true">
                <timerEventDefinition><timeDuration>PT10H</timeDuration></timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <sequenceFlow id="flow4" sourceRef="boundaryEvent2" targetRef="notifyEndEvent" />
            <sequenceFlow id="flow5" sourceRef="boundaryEvent3" targetRef="timerEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="cancelEndEvent" />
            <endEvent id="notifyEndEvent" />
            <endEvent id="timerEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Signal Receive Task Boundary Test Deployment".to_string())
        .add_string(
            "signalReceiveTaskBoundaryTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Signal Receive Task Boundary Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    let receive_task_execution = executions
        .values()
        .find(|e| e.activity_id.as_deref() == Some("receiveTask1"))
        .expect("Receive task execution should exist");
    assert!(
        !receive_task_execution.is_active,
        "Receive task should be in wait state"
    );

    // Verify both wait-state and boundary state are registered
    let wait_states = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        wait_states.len(),
        1,
        "Should have one message-style wait state"
    );
    assert_eq!(
        wait_states[0].wait_kind,
        RuntimeMessageStyleWaitKind::ReceiveTask
    );
    assert_eq!(wait_states[0].activity_id.as_deref(), Some("receiveTask1"));

    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states.len(),
        2,
        "Should register both interrupting and non-interrupting signal boundary events"
    );

    let interrupting_state = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "boundaryEvent1")
        .expect("Interrupting boundary should be registered");
    assert_eq!(interrupting_state.attached_activity_id, "receiveTask1");
    assert_eq!(interrupting_state.process_instance_id, process_instance.id);
    assert_eq!(
        interrupting_state.host_execution_id,
        receive_task_execution.id
    );
    assert!(
        interrupting_state.cancel_activity,
        "boundaryEvent1 should be interrupting"
    );
    assert_eq!(
        interrupting_state.event_subscription.kind,
        EventSubscriptionKind::Signal
    );
    assert_eq!(
        interrupting_state.event_subscription.event_ref,
        "cancelSignal"
    );

    let non_interrupting_state = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "boundaryEvent2")
        .expect("Non-interrupting boundary should be registered");
    assert_eq!(non_interrupting_state.attached_activity_id, "receiveTask1");
    assert_eq!(
        non_interrupting_state.process_instance_id,
        process_instance.id
    );
    assert_eq!(
        non_interrupting_state.host_execution_id,
        receive_task_execution.id
    );
    assert!(
        !non_interrupting_state.cancel_activity,
        "boundaryEvent2 should be non-interrupting"
    );
    assert_eq!(
        non_interrupting_state.event_subscription.kind,
        EventSubscriptionKind::Signal
    );
    assert_eq!(
        non_interrupting_state.event_subscription.event_ref,
        "notifySignal"
    );

    assert!(
        runtime_store
            .find_boundary_event_state("boundaryEvent3", &process_instance.id, &mut session)
            .is_none(),
        "Timer boundary event should not be registered"
    );
}

#[test]
fn test_signal_interrupting_boundary_event_on_receive_task_cancels_host_and_follows_boundary_path()
{
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="cancelSignal" name="Cancel Signal" />
        <process id="signalReceiveTaskBoundaryCancelTest" name="Signal Receive Task Boundary Cancel Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Message" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="receiveTask1" cancelActivity="true">
                <signalEventDefinition signalRef="cancelSignal" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="normalEndEvent" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <endEvent id="normalEndEvent" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Signal Receive Task Boundary Cancel Test Deployment".to_string())
        .add_string(
            "signalReceiveTaskBoundaryCancelTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Signal Receive Task Boundary Cancel Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_before.len(),
        1,
        "Should have one task before boundary event trigger"
    );
    assert_eq!(tasks_before[0].task_definition_key, "receiveTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    // Verify wait-state and boundary state exist before trigger
    let wait_states_before = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        wait_states_before.len(),
        1,
        "Should have wait state before trigger"
    );

    let boundary_states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_before.len(),
        1,
        "Should have boundary event state before trigger"
    );
    assert!(
        boundary_states_before[0].cancel_activity,
        "Boundary event should be interrupting"
    );
    drop(session);

    // Trigger by signal_ref
    runtime_service.trigger_boundary_event_by_signal_ref(
        "cancelSignal".to_string(),
        process_instance.id.clone(),
    );

    // Verify task is deleted
    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after.is_empty(),
        "Task should be deleted after boundary event trigger"
    );

    // Verify wait-state is cleaned up
    let mut session = runtime_store.create_session().unwrap();
    let wait_states_after = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        wait_states_after.is_empty(),
        "Wait state should be cleaned up after boundary event trigger"
    );

    // Verify execution is removed
    let executions_after = runtime_store.snapshot_executions(&mut session);
    assert!(
        executions_after
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("receiveTask1")),
        "Receive task execution should be removed after boundary event trigger"
    );

    // Verify boundary state is cleaned up
    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states_after.is_empty(),
        "Boundary event state should be cleaned up after trigger"
    );

    // Verify process is ended
    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        process_instance_after.is_ended,
        "Process should be ended after boundary event triggers cancel path"
    );
}

#[test]
fn test_signal_non_interrupting_boundary_event_on_receive_task_preserves_host() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="notifySignal" name="Notify Signal" />
        <process id="signalReceiveTaskNonInterruptingBoundaryTest" name="Signal Receive Task Non-Interrupting Boundary Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="receiveTask1" />
            <receiveTask id="receiveTask1" name="Await Message" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="receiveTask1" cancelActivity="false">
                <signalEventDefinition signalRef="notifySignal" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="receiveTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Notification" />
            <sequenceFlow id="flow4" sourceRef="userTask1" targetRef="endEvent2" />
            <endEvent id="endEvent1" />
            <endEvent id="endEvent2" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Signal Receive Task Non-Interrupting Boundary Test Deployment".to_string())
        .add_string(
            "signalReceiveTaskNonInterruptingBoundaryTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Signal Receive Task Non-Interrupting Boundary Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_before.len(),
        1,
        "Should have one task before boundary event trigger"
    );
    assert_eq!(tasks_before[0].task_definition_key, "receiveTask1");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    // Verify wait-state and boundary state exist before trigger
    let wait_states_before = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        wait_states_before.len(),
        1,
        "Should have wait state before trigger"
    );

    let boundary_states_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_before.len(),
        1,
        "Should have boundary event state before trigger"
    );
    assert!(
        !boundary_states_before[0].cancel_activity,
        "Boundary event should be non-interrupting"
    );
    drop(session);

    // Trigger by signal_ref
    runtime_service.trigger_boundary_event_by_signal_ref(
        "notifySignal".to_string(),
        process_instance.id.clone(),
    );

    // Verify original ReceiveTask still exists
    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    let receive_tasks: Vec<_> = tasks_after
        .iter()
        .filter(|t| t.task_definition_key == "receiveTask1")
        .collect();
    assert_eq!(
        receive_tasks.len(),
        1,
        "ReceiveTask should still exist after non-interrupting boundary event trigger"
    );

    // Verify wait-state still exists
    let mut session = runtime_store.create_session().unwrap();
    let wait_states_after = runtime_store
        .find_message_style_wait_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        wait_states_after.len(),
        1,
        "Wait state should still exist after non-interrupting boundary event trigger"
    );

    // Verify execution is still on receiveTask1
    let executions_after = runtime_store.snapshot_executions(&mut session);
    let host_execution = executions_after
        .get(&receive_tasks[0].execution_id)
        .expect("Host execution should still exist");
    assert_eq!(
        host_execution.activity_id.as_deref(),
        Some("receiveTask1"),
        "Host execution should still be on receiveTask1"
    );
    assert!(
        !host_execution.is_active,
        "Host execution should still be in wait state"
    );

    // Verify boundary state is cleaned up
    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    // Java MessageNonInterruptingBoundaryEventTest: subscription is not removed
    // on fire (repeat while host is active). Removed only when host ends.
    assert_eq!(
        boundary_states_after.len(),
        1,
        "Non-interrupting boundary subscription must survive its own trigger"
    );

    // Verify process is NOT ended
    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        !process_instance_after.is_ended,
        "Process should NOT be ended after non-interrupting boundary event triggers"
    );
}

#[test]
fn test_signal_boundary_event_wrong_signal_ref_is_noop() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="cancelSignal" name="Cancel Signal" />
        <process id="signalBoundaryWrongRefTest" name="Signal Boundary Wrong Ref Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="true">
                <signalEventDefinition signalRef="cancelSignal" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <endEvent id="endEvent1" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Signal Boundary Wrong Ref Test Deployment".to_string())
        .add_string(
            "signalBoundaryWrongRefTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance_builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Signal Boundary Wrong Ref Test Instance".to_string());

    let process_instance = runtime_service
        .start_process_instance(process_instance_builder)
        .unwrap();

    let tasks_before = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_before.len(), 1);

    // Try triggering with wrong signal_ref - should be no-op
    runtime_service.trigger_boundary_event_by_signal_ref(
        "wrongSignal".to_string(),
        process_instance.id.clone(),
    );

    let tasks_after_wrong = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_wrong.len(),
        1,
        "Task should still exist after wrong signal ref"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states.len(),
        1,
        "Boundary state should still exist after wrong signal ref"
    );

    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        !process_instance_after.is_ended,
        "Process should NOT be ended after wrong signal ref"
    );
    drop(session);

    // Trigger with correct signal_ref
    runtime_service.trigger_boundary_event_by_signal_ref(
        "cancelSignal".to_string(),
        process_instance.id.clone(),
    );

    let tasks_after_correct = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after_correct.is_empty(),
        "Task should be deleted after correct signal ref"
    );

    let mut session = runtime_store.create_session().unwrap();
    let process_instance_final = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should exist");
    assert!(
        process_instance_final.is_ended,
        "Process should be ended after correct signal ref"
    );
}

// ============================================================================
// Multi-Instance Regression Tests
// ============================================================================

#[test]
fn test_signal_boundary_event_multi_instance_isolation() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="cancelSignal" name="Cancel Signal" />
        <process id="multiInstanceSignalTest" name="Multi-Instance Signal Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="boundaryEvent1" attachedToRef="userTask1" cancelActivity="true">
                <signalEventDefinition signalRef="cancelSignal" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="boundaryEvent1" targetRef="cancelEndEvent" />
            <endEvent id="endEvent1" />
            <endEvent id="cancelEndEvent" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Multi-Instance Signal Test Deployment".to_string())
        .add_string(
            "multiInstanceSignalTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // Start first process instance
    let process_instance_builder_1 = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("Instance 1".to_string());
    let process_instance_1 = runtime_service
        .start_process_instance(process_instance_builder_1)
        .unwrap();

    // Start second process instance
    let process_instance_builder_2 = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("Instance 2".to_string());
    let process_instance_2 = runtime_service
        .start_process_instance(process_instance_builder_2)
        .unwrap();

    // Start third process instance
    let process_instance_builder_3 = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("Instance 3".to_string());
    let process_instance_3 = runtime_service
        .start_process_instance(process_instance_builder_3)
        .unwrap();

    // Verify all three instances have tasks
    let tasks_1_before = task_service
        .get_tasks_by_process_instance_id(process_instance_1.id.clone())
        .unwrap();
    let tasks_2_before = task_service
        .get_tasks_by_process_instance_id(process_instance_2.id.clone())
        .unwrap();
    let tasks_3_before = task_service
        .get_tasks_by_process_instance_id(process_instance_3.id.clone())
        .unwrap();
    assert_eq!(tasks_1_before.len(), 1);
    assert_eq!(tasks_2_before.len(), 1);
    assert_eq!(tasks_3_before.len(), 1);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    // Verify all three instances have boundary states
    let boundary_states_1_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance_1.id, &mut session);
    let boundary_states_2_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance_2.id, &mut session);
    let boundary_states_3_before = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance_3.id, &mut session);
    assert_eq!(boundary_states_1_before.len(), 1);
    assert_eq!(boundary_states_2_before.len(), 1);
    assert_eq!(boundary_states_3_before.len(), 1);
    drop(session);

    // Trigger signal on instance 2 only
    runtime_service.trigger_boundary_event_by_signal_ref(
        "cancelSignal".to_string(),
        process_instance_2.id.clone(),
    );

    // Verify instance 1 is unaffected
    let tasks_1_after = task_service
        .get_tasks_by_process_instance_id(process_instance_1.id.clone())
        .unwrap();
    assert_eq!(tasks_1_after.len(), 1, "Instance 1 task should still exist");
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_1_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance_1.id, &mut session);
    assert_eq!(
        boundary_states_1_after.len(),
        1,
        "Instance 1 boundary state should still exist"
    );
    let process_instance_1_after = runtime_store
        .find_process_instance(&process_instance_1.id, &mut session)
        .expect("Instance 1 should exist");
    assert!(
        !process_instance_1_after.is_ended,
        "Instance 1 should NOT be ended"
    );
    drop(session);

    // Verify instance 2 is ended
    let tasks_2_after = task_service
        .get_tasks_by_process_instance_id(process_instance_2.id.clone())
        .unwrap();
    assert!(
        tasks_2_after.is_empty(),
        "Instance 2 task should be deleted"
    );
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_2_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance_2.id, &mut session);
    assert!(
        boundary_states_2_after.is_empty(),
        "Instance 2 boundary state should be cleaned up"
    );
    let process_instance_2_after = runtime_store
        .find_process_instance(&process_instance_2.id, &mut session)
        .expect("Instance 2 should exist");
    assert!(
        process_instance_2_after.is_ended,
        "Instance 2 should be ended"
    );
    drop(session);

    // Verify instance 3 is unaffected
    let tasks_3_after = task_service
        .get_tasks_by_process_instance_id(process_instance_3.id.clone())
        .unwrap();
    assert_eq!(tasks_3_after.len(), 1, "Instance 3 task should still exist");
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_3_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance_3.id, &mut session);
    assert_eq!(
        boundary_states_3_after.len(),
        1,
        "Instance 3 boundary state should still exist"
    );
    let process_instance_3_after = runtime_store
        .find_process_instance(&process_instance_3.id, &mut session)
        .expect("Instance 3 should exist");
    assert!(
        !process_instance_3_after.is_ended,
        "Instance 3 should NOT be ended"
    );
}

#[test]
fn test_signal_intermediate_catch_event_multi_instance_isolation() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="Alert Signal" />
        <process id="multiInstanceSignalCatchTest" name="Multi-Instance Signal Catch Test">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="signalCatchEvent1" />
            <intermediateCatchEvent id="signalCatchEvent1" name="Catch Alert Signal">
                <signalEventDefinition signalRef="alertSignal" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="signalCatchEvent1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Multi-Instance Signal Catch Test Deployment".to_string())
        .add_string(
            "multiInstanceSignalCatchTest.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    // Start first process instance
    let process_instance_builder_1 = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("Instance 1".to_string());
    let process_instance_1 = runtime_service
        .start_process_instance(process_instance_builder_1)
        .unwrap();

    // Start second process instance
    let process_instance_builder_2 = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("Instance 2".to_string());
    let process_instance_2 = runtime_service
        .start_process_instance(process_instance_builder_2)
        .unwrap();

    // Start third process instance
    let process_instance_builder_3 = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id.clone())
        .name("Instance 3".to_string());
    let process_instance_3 = runtime_service
        .start_process_instance(process_instance_builder_3)
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let session = runtime_store.create_session().unwrap();
    drop(session);

    // Verify all three instances have wait states
    let wait_states_1_before = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_1.id.clone());
    let wait_states_2_before = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_2.id.clone());
    let wait_states_3_before = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_3.id.clone());
    assert_eq!(wait_states_1_before.len(), 1);
    assert_eq!(wait_states_2_before.len(), 1);
    assert_eq!(wait_states_3_before.len(), 1);

    // Verify all instances are not ended
    let mut session = runtime_store.create_session().unwrap();
    let process_instance_1_before = runtime_store
        .find_process_instance(&process_instance_1.id, &mut session)
        .expect("Instance 1 should exist");
    let process_instance_2_before = runtime_store
        .find_process_instance(&process_instance_2.id, &mut session)
        .expect("Instance 2 should exist");
    let process_instance_3_before = runtime_store
        .find_process_instance(&process_instance_3.id, &mut session)
        .expect("Instance 3 should exist");
    assert!(!process_instance_1_before.is_ended);
    assert!(!process_instance_2_before.is_ended);
    assert!(!process_instance_3_before.is_ended);
    drop(session);

    // Trigger signal on instance 2 only using execution_id
    let execution_id_2 = wait_states_2_before[0].execution_id.clone();
    runtime_service.trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        "Alert Signal".to_string(),
        execution_id_2,
    );

    // Verify instance 1 is unaffected
    let wait_states_1_after = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_1.id.clone());
    assert_eq!(
        wait_states_1_after.len(),
        1,
        "Instance 1 wait state should still exist"
    );
    let mut session = runtime_store.create_session().unwrap();
    let process_instance_1_after = runtime_store
        .find_process_instance(&process_instance_1.id, &mut session)
        .expect("Instance 1 should exist");
    assert!(
        !process_instance_1_after.is_ended,
        "Instance 1 should NOT be ended"
    );
    drop(session);

    // Verify instance 2 is ended
    let wait_states_2_after = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_2.id.clone());
    assert!(
        wait_states_2_after.is_empty(),
        "Instance 2 wait state should be cleaned up"
    );
    let mut session = runtime_store.create_session().unwrap();
    let process_instance_2_after = runtime_store
        .find_process_instance(&process_instance_2.id, &mut session)
        .expect("Instance 2 should exist");
    assert!(
        process_instance_2_after.is_ended,
        "Instance 2 should be ended"
    );
    drop(session);

    // Verify instance 3 is unaffected
    let wait_states_3_after = runtime_service
        .get_message_style_wait_states_by_process_instance_id(process_instance_3.id.clone());
    assert_eq!(
        wait_states_3_after.len(),
        1,
        "Instance 3 wait state should still exist"
    );
    let mut session = runtime_store.create_session().unwrap();
    let process_instance_3_after = runtime_store
        .find_process_instance(&process_instance_3.id, &mut session)
        .expect("Instance 3 should exist");
    assert!(
        !process_instance_3_after.is_ended,
        "Instance 3 should NOT be ended"
    );
}

// ============================================================================
// Mixed Message + Signal Boundary Event Regression Tests
// ============================================================================

/// A user task with both a message boundary event and a signal boundary event:
/// triggering the message boundary should leave the signal boundary unaffected
/// (non-interrupting scenario).
#[test]
fn test_mixed_message_and_signal_boundary_events_on_same_user_task() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="Alert Signal" />
        <process id="mixedBoundaryProcess" name="Mixed Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="msgBoundary1" attachedToRef="userTask1" cancelActivity="false">
                <messageEventDefinition messageRef="notifyMessage" />
            </boundaryEvent>
            <boundaryEvent id="sigBoundary1" attachedToRef="userTask1" cancelActivity="false">
                <signalEventDefinition signalRef="alertSignal" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="msgBoundary1" targetRef="receiveTask1" />
            <sequenceFlow id="flow4" sourceRef="sigBoundary1" targetRef="receiveTask2" />
            <receiveTask id="receiveTask1" name="Wait For Msg Ack" />
            <receiveTask id="receiveTask2" name="Wait For Sig Ack" />
            <sequenceFlow id="flow5" sourceRef="receiveTask1" targetRef="endEvent2" />
            <sequenceFlow id="flow6" sourceRef="receiveTask2" targetRef="endEvent3" />
            <endEvent id="endEvent1" />
            <endEvent id="endEvent2" />
            <endEvent id="endEvent3" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mixed Boundary Deployment".to_string())
        .add_string(
            "mixedBoundaryProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mixed Boundary Instance".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    // Both boundary events registered
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states.len(),
        2,
        "Should have both message and signal boundary events"
    );

    let msg_boundary = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "msgBoundary1")
        .unwrap();
    assert_eq!(
        msg_boundary.event_subscription.kind,
        EventSubscriptionKind::Message
    );
    assert_eq!(msg_boundary.event_subscription.event_ref, "notifyMessage");

    let sig_boundary = boundary_states
        .iter()
        .find(|s| s.boundary_event_id == "sigBoundary1")
        .unwrap();
    assert_eq!(
        sig_boundary.event_subscription.kind,
        EventSubscriptionKind::Signal
    );
    assert_eq!(sig_boundary.event_subscription.event_ref, "alertSignal");
    drop(session);

    // Trigger message boundary (non-interrupting) — both subscriptions survive
    // (Java repeat semantics). Isolation: signal path not yet taken.
    runtime_service.trigger_boundary_event_by_message_ref(
        "notifyMessage".to_string(),
        process_instance.id.clone(),
    );

    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_after_msg = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_after_msg.len(),
        2,
        "both non-interrupting boundaries remain after message fire"
    );
    assert!(
        boundary_states_after_msg
            .iter()
            .any(|s| s.boundary_event_id == "msgBoundary1"),
        "message boundary subscription survives its own trigger"
    );
    assert!(
        boundary_states_after_msg
            .iter()
            .any(|s| s.boundary_event_id == "sigBoundary1"),
        "signal boundary subscription remains after message fire"
    );
    drop(session);

    let tasks_after_msg = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        !tasks_after_msg.is_empty(),
        "User task should still exist (non-interrupting)"
    );
    assert!(
        tasks_after_msg
            .iter()
            .any(|t| t.task_definition_key == "userTask1"),
        "Original user task should still exist"
    );

    let mut session = runtime_store.create_session().unwrap();
    let pi_after_msg = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(
        !pi_after_msg.is_ended,
        "Process should not be ended after non-interrupting message boundary"
    );
    drop(session);

    // Trigger signal boundary (non-interrupting) — both subscriptions still present
    runtime_service.trigger_boundary_event_by_signal_ref(
        "alertSignal".to_string(),
        process_instance.id.clone(),
    );

    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_after_sig = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_after_sig.len(),
        2,
        "both non-interrupting boundaries remain after signal fire"
    );
    drop(session);

    let tasks_after_sig = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after_sig
            .iter()
            .any(|t| t.task_definition_key == "userTask1"),
        "User task should still exist (both were non-interrupting)"
    );

    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(
        !pi.is_ended,
        "Process should not be ended - user task still active"
    );
}

/// Wrong event_ref for both message and signal boundary events should be no-op.
/// Also verifies cross-type rejection: sending a signal ref to a message boundary and vice versa.
#[test]
fn test_mixed_boundary_wrong_ref_and_cross_type_are_noop() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="alertSignal" name="Alert Signal" />
        <process id="mixedBoundaryNoopProcess" name="Mixed Boundary Noop Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Approve Request" />
            <boundaryEvent id="msgBoundary1" attachedToRef="userTask1" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </boundaryEvent>
            <boundaryEvent id="sigBoundary1" attachedToRef="userTask1" cancelActivity="true">
                <signalEventDefinition signalRef="alertSignal" />
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <sequenceFlow id="flow3" sourceRef="msgBoundary1" targetRef="cancelEnd" />
            <sequenceFlow id="flow4" sourceRef="sigBoundary1" targetRef="alertEnd" />
            <endEvent id="endEvent1" />
            <endEvent id="cancelEnd" />
            <endEvent id="alertEnd" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mixed Boundary Noop Deployment".to_string())
        .add_string(
            "mixedBoundaryNoopProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mixed Boundary Noop Instance".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let session = runtime_store.create_session().unwrap();
    drop(session);

    // Wrong message ref - no-op
    runtime_service.trigger_boundary_event_by_message_ref(
        "wrongMessage".to_string(),
        process_instance.id.clone(),
    );
    // Wrong signal ref - no-op
    runtime_service.trigger_boundary_event_by_signal_ref(
        "wrongSignal".to_string(),
        process_instance.id.clone(),
    );
    // Cross-type: sending alertSignal via message trigger - no-op (different subscription kind)
    runtime_service.trigger_boundary_event_by_message_ref(
        "alertSignal".to_string(),
        process_instance.id.clone(),
    );
    // Cross-type: sending cancelMessage via signal trigger - no-op
    runtime_service.trigger_boundary_event_by_signal_ref(
        "cancelMessage".to_string(),
        process_instance.id.clone(),
    );

    let mut session = runtime_store.create_session().unwrap();
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states.len(),
        2,
        "Both boundary events should still be registered after wrong refs"
    );
    drop(session);

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "User task should still exist after wrong refs"
    );

    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(
        !pi.is_ended,
        "Process should still be active after wrong refs"
    );
    drop(session);

    // Now trigger with correct message ref - should work and interrupt
    runtime_service.trigger_boundary_event_by_message_ref(
        "cancelMessage".to_string(),
        process_instance.id.clone(),
    );

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks_after.is_empty(),
        "User task should be deleted by interrupting message boundary"
    );

    let mut session = runtime_store.create_session().unwrap();
    let pi_final = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(
        pi_final.is_ended,
        "Process should be ended after interrupting boundary event"
    );
}

#[test]
fn test_intermediate_throw_escalation_triggers_interrupting_boundary_event() {
    let process_engine = ProcessEngine::new("escalation-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
        <process id="escalationBoundaryProcess" name="Escalation Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="reviewTask" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="throwEscalation" />
            <userTask id="reviewTask" name="Review Request" />
            <boundaryEvent id="catchEscalation" attachedToRef="reviewTask" cancelActivity="true">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </boundaryEvent>
            <intermediateThrowEvent id="throwEscalation">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </intermediateThrowEvent>
            <sequenceFlow id="flow4" sourceRef="reviewTask" targetRef="normalEnd" />
            <sequenceFlow id="flow5" sourceRef="throwEscalation" targetRef="throwEnd" />
            <sequenceFlow id="flow6" sourceRef="catchEscalation" targetRef="escalatedTask" />
            <userTask id="escalatedTask" name="Escalated Review" />
            <endEvent id="normalEnd" />
            <endEvent id="throwEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Escalation Boundary Deployment".to_string())
                .add_string(
                    "escalationBoundaryProcess.bpmn20.xml".to_string(),
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
                .name("Escalation Boundary Instance".to_string()),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "escalatedTask");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    assert!(
        executions
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("reviewTask")),
        "interrupting escalation boundary should remove the host execution"
    );

    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states.is_empty(),
        "matched escalation boundary should be consumed"
    );
}

#[test]
fn test_end_event_escalation_triggers_interrupting_boundary_event() {
    let process_engine = ProcessEngine::new("end-escalation-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
        <process id="endEscalationBoundaryProcess" name="End Escalation Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="reviewTask" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="throwingEnd" />
            <userTask id="reviewTask" name="Review Request" />
            <boundaryEvent id="catchEscalation" attachedToRef="reviewTask" cancelActivity="true">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </boundaryEvent>
            <endEvent id="throwingEnd">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </endEvent>
            <sequenceFlow id="flow4" sourceRef="reviewTask" targetRef="normalEnd" />
            <sequenceFlow id="flow5" sourceRef="catchEscalation" targetRef="escalatedTask" />
            <userTask id="escalatedTask" name="Escalated Review" />
            <endEvent id="normalEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("End Escalation Boundary Deployment".to_string())
                .add_string(
                    "endEscalationBoundaryProcess.bpmn20.xml".to_string(),
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
                .name("End Escalation Boundary Instance".to_string()),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "escalatedTask");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    assert!(
        executions
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("reviewTask")),
        "interrupting end escalation boundary should remove the host execution"
    );
}

#[test]
fn test_non_interrupting_escalation_boundary_preserves_host_activity() {
    let process_engine =
        ProcessEngine::new("non-interrupting-escalation-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
        <process id="nonInterruptingEscalationBoundaryProcess" name="Non Interrupting Escalation Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="reviewTask" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="throwEscalation" />
            <userTask id="reviewTask" name="Review Request" />
            <boundaryEvent id="catchEscalation" attachedToRef="reviewTask" cancelActivity="false">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </boundaryEvent>
            <intermediateThrowEvent id="throwEscalation">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </intermediateThrowEvent>
            <sequenceFlow id="flow4" sourceRef="reviewTask" targetRef="normalEnd" />
            <sequenceFlow id="flow5" sourceRef="throwEscalation" targetRef="throwEnd" />
            <sequenceFlow id="flow6" sourceRef="catchEscalation" targetRef="escalatedTask" />
            <userTask id="escalatedTask" name="Escalated Review" />
            <endEvent id="normalEnd" />
            <endEvent id="throwEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Non Interrupting Escalation Boundary Deployment".to_string())
                .add_string(
                    "nonInterruptingEscalationBoundaryProcess.bpmn20.xml".to_string(),
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
                .name("Non Interrupting Escalation Boundary Instance".to_string()),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(
        tasks
            .iter()
            .any(|task| task.task_definition_key == "reviewTask"),
        "host user task should remain after non-interrupting escalation boundary fires"
    );
    assert!(
        tasks
            .iter()
            .any(|task| task.task_definition_key == "escalatedTask"),
        "boundary path should create the escalation user task"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let host_task = tasks
        .iter()
        .find(|task| task.task_definition_key == "reviewTask")
        .unwrap();
    let host_execution = runtime_store
        .find_execution(&host_task.execution_id, &mut session)
        .expect("host execution should still exist");
    assert_eq!(host_execution.activity_id.as_deref(), Some("reviewTask"));
    assert!(
        !host_execution.is_active,
        "host user task execution should remain in wait state"
    );

    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should exist");
    assert!(
        !process_instance_after.is_ended,
        "process should stay active while host task remains open"
    );

    // Java: BoundaryEventActivityBehavior#executeNonInterruptingBehavior never
    // deletes the waiting boundary execution; EscalationPropagation can re-find
    // it by activityId for a subsequent throw (P12 repeat, not consume).
    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states_after.len(),
        1,
        "non-interrupting escalation boundary state must survive its own trigger"
    );
    assert_eq!(
        boundary_states_after[0].boundary_event_id, "catchEscalation"
    );
}

#[test]
fn test_escalation_boundary_matches_throw_ref_to_boundary_escalation_code() {
    let process_engine = ProcessEngine::new("escalation-code-ref-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
        <process id="escalationCodeRefBoundaryProcess" name="Escalation Code Ref Boundary Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="reviewTask" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="throwEscalation" />
            <userTask id="reviewTask" name="Review Request" />
            <boundaryEvent id="catchEscalation" attachedToRef="reviewTask" cancelActivity="true">
                <escalationEventDefinition escalationCode="APPROVAL_TIMEOUT" />
            </boundaryEvent>
            <intermediateThrowEvent id="throwEscalation">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </intermediateThrowEvent>
            <sequenceFlow id="flow4" sourceRef="reviewTask" targetRef="normalEnd" />
            <sequenceFlow id="flow5" sourceRef="throwEscalation" targetRef="throwEnd" />
            <sequenceFlow id="flow6" sourceRef="catchEscalation" targetRef="escalatedTask" />
            <userTask id="escalatedTask" name="Escalated Review" />
            <endEvent id="normalEnd" />
            <endEvent id="throwEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Escalation Code Ref Boundary Deployment".to_string())
                .add_string(
                    "escalationCodeRefBoundaryProcess.bpmn20.xml".to_string(),
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
                .name("Escalation Code Ref Boundary Instance".to_string()),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "escalatedTask");
}

#[test]
fn test_nested_subprocess_escalation_prefers_nearest_boundary_scope() {
    let process_engine =
        ProcessEngine::new("nested-subprocess-escalation-nearest-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
        <process id="nestedSubprocessEscalationProcess" name="Nested Subprocess Escalation Process">
            <startEvent id="startEvent" />
            <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="outerSubProcess" />
            <subProcess id="outerSubProcess">
                <startEvent id="outerStart" />
                <sequenceFlow id="outerFlow1" sourceRef="outerStart" targetRef="innerSubProcess" />
                <subProcess id="innerSubProcess">
                    <startEvent id="innerStart" />
                    <sequenceFlow id="innerFlow1" sourceRef="innerStart" targetRef="throwingEnd" />
                    <endEvent id="throwingEnd">
                        <escalationEventDefinition escalationRef="approvalEscalation" />
                    </endEvent>
                </subProcess>
                <boundaryEvent id="innerEscalationBoundary" attachedToRef="innerSubProcess" cancelActivity="true">
                    <escalationEventDefinition escalationRef="approvalEscalation" />
                </boundaryEvent>
                <sequenceFlow id="innerBoundaryFlow" sourceRef="innerEscalationBoundary" targetRef="innerEscalatedTask" />
                <userTask id="innerEscalatedTask" name="Inner Escalated Task" />
                <sequenceFlow id="innerEscalatedFlow" sourceRef="innerEscalatedTask" targetRef="outerEnd" />
                <endEvent id="outerEnd" />
            </subProcess>
            <boundaryEvent id="outerEscalationBoundary" attachedToRef="outerSubProcess" cancelActivity="true">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </boundaryEvent>
            <sequenceFlow id="outerBoundaryFlow" sourceRef="outerEscalationBoundary" targetRef="outerEscalatedTask" />
            <userTask id="outerEscalatedTask" name="Outer Escalated Task" />
            <sequenceFlow id="outerEscalatedFlow" sourceRef="outerEscalatedTask" targetRef="outerEscalatedEnd" />
            <endEvent id="outerEscalatedEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Nested Subprocess Escalation Deployment".to_string())
                .add_string(
                    "nestedSubprocessEscalationProcess.bpmn20.xml".to_string(),
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
                .name("Nested Subprocess Escalation Instance".to_string()),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();

    assert_eq!(
        tasks.len(),
        1,
        "only the nearest matching escalation boundary path should create a task"
    );
    assert!(
        tasks
            .iter()
            .any(|task| task.task_definition_key == "innerEscalatedTask"),
        "inner subprocess escalation boundary should catch the thrown escalation"
    );
    assert!(
        !tasks
            .iter()
            .any(|task| task.task_definition_key == "outerEscalatedTask"),
        "outer subprocess escalation boundary must not catch before the inner boundary"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states
            .iter()
            .any(|state| state.boundary_event_id == "outerEscalationBoundary"),
        "outer escalation boundary should remain registered after the inner boundary catches"
    );
    assert!(
        !boundary_states
            .iter()
            .any(|state| state.boundary_event_id == "innerEscalationBoundary"),
        "inner escalation boundary should be consumed"
    );
}

#[test]
fn test_no_code_escalation_boundary_catches_any_escalation() {
    let process_engine = ProcessEngine::new("no-code-escalation-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
        <process id="noCodeEscalationBoundaryProcess" name="No Code Escalation Boundary Process">
            <startEvent id="startEvent" />
            <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="reviewTask" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="throwEscalation" />
            <userTask id="reviewTask" name="Review Task" />
            <boundaryEvent id="catchAnyEscalation" attachedToRef="reviewTask" cancelActivity="false">
                <escalationEventDefinition />
            </boundaryEvent>
            <boundaryEvent id="catchOtherEscalation" attachedToRef="reviewTask" cancelActivity="false">
                <escalationEventDefinition escalationCode="OTHER_TIMEOUT" />
            </boundaryEvent>
            <intermediateThrowEvent id="throwEscalation">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </intermediateThrowEvent>
            <sequenceFlow id="flow4" sourceRef="reviewTask" targetRef="normalEnd" />
            <sequenceFlow id="flow5" sourceRef="throwEscalation" targetRef="throwEnd" />
            <sequenceFlow id="flow6" sourceRef="catchAnyEscalation" targetRef="catchAnyTask" />
            <sequenceFlow id="flow7" sourceRef="catchOtherEscalation" targetRef="catchOtherTask" />
            <userTask id="catchAnyTask" name="Catch Any Task" />
            <userTask id="catchOtherTask" name="Catch Other Task" />
            <endEvent id="normalEnd" />
            <endEvent id="throwEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("No Code Escalation Boundary Deployment".to_string())
                .add_string(
                    "noCodeEscalationBoundaryProcess.bpmn20.xml".to_string(),
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
                .name("No Code Escalation Boundary Instance".to_string()),
        )
        .unwrap();

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();

    assert_eq!(
        task_keys,
        vec!["catchAnyTask".to_string(), "reviewTask".to_string()],
        "the no-code escalation boundary should catch the thrown escalation and the different coded boundary should not"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states
            .iter()
            .any(|state| state.boundary_event_id == "catchOtherEscalation"),
        "the mismatched coded escalation boundary should remain registered"
    );
    // Java non-interrupting: executeNonInterruptingBehavior keeps the wait
    // execution; EscalationPropagation can re-trigger by activityId (P12 repeat).
    assert!(
        boundary_states
            .iter()
            .any(|state| state.boundary_event_id == "catchAnyEscalation"),
        "the matched no-code non-interrupting escalation boundary must retain state (repeat)"
    );
}

#[test]
fn test_escalation_boundary_prefers_exact_code_over_no_code_on_same_host() {
    let process_engine =
        ProcessEngine::new("exact-code-before-catch-all-boundary-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
        <process id="exactEscalationBoundaryProcess" name="Exact Escalation Boundary Process">
            <startEvent id="startEvent" />
            <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow2" sourceRef="fork" targetRef="reviewTask" />
            <sequenceFlow id="flow3" sourceRef="fork" targetRef="throwEscalation" />
            <userTask id="reviewTask" name="Review Task" />
            <boundaryEvent id="catchAnyEscalation" attachedToRef="reviewTask" cancelActivity="false">
                <escalationEventDefinition />
            </boundaryEvent>
            <boundaryEvent id="catchExactEscalation" attachedToRef="reviewTask" cancelActivity="false">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </boundaryEvent>
            <intermediateThrowEvent id="throwEscalation">
                <escalationEventDefinition escalationCode="APPROVAL_TIMEOUT" />
            </intermediateThrowEvent>
            <sequenceFlow id="flow4" sourceRef="reviewTask" targetRef="normalEnd" />
            <sequenceFlow id="flow5" sourceRef="throwEscalation" targetRef="throwEnd" />
            <sequenceFlow id="flow6" sourceRef="catchAnyEscalation" targetRef="catchAnyTask" />
            <sequenceFlow id="flow7" sourceRef="catchExactEscalation" targetRef="catchExactTask" />
            <userTask id="catchAnyTask" name="Catch Any Task" />
            <userTask id="catchExactTask" name="Catch Exact Task" />
            <endEvent id="normalEnd" />
            <endEvent id="throwEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Exact Escalation Boundary Deployment".to_string())
                .add_string(
                    "exactEscalationBoundaryProcess.bpmn20.xml".to_string(),
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
                .name("Exact Escalation Boundary Instance".to_string()),
        )
        .unwrap();

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();

    assert_eq!(
        task_keys,
        vec!["catchExactTask".to_string(), "reviewTask".to_string()],
        "same-host exact escalation boundary must win over the no-code catch-all"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states
            .iter()
            .any(|state| state.boundary_event_id == "catchAnyEscalation"),
        "the no-code escalation boundary should remain registered when exact catch wins"
    );
    // Java non-interrupting: keep waiting boundary (P12 repeat, not consume).
    assert!(
        boundary_states
            .iter()
            .any(|state| state.boundary_event_id == "catchExactEscalation"),
        "the exact non-interrupting escalation boundary must retain state (repeat)"
    );
}

fn run_named_message_boundary_case(case_name: &str, host_xml: &str) {
    let process_engine = ProcessEngine::new(format!("named-boundary-{case_name}"));
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <message id="cancelMessageDef" name="external.cancel" />
  <process id="{case_name}Process" name="{case_name} Process">
    <startEvent id="start" />
    <sequenceFlow id="flow_start" sourceRef="start" targetRef="host" />
    {host_xml}
    <boundaryEvent id="messageBoundary" attachedToRef="host" cancelActivity="true">
      <messageEventDefinition messageRef="cancelMessageDef" />
    </boundaryEvent>
    <sequenceFlow id="flow_normal" sourceRef="host" targetRef="normalEnd" />
    <sequenceFlow id="flow_boundary" sourceRef="messageBoundary" targetRef="boundaryTask" />
    <userTask id="boundaryTask" name="Boundary Path" />
    <sequenceFlow id="flow_boundary_end" sourceRef="boundaryTask" targetRef="boundaryEnd" />
    <endEvent id="normalEnd" />
    <endEvent id="boundaryEnd" />
  </process>
</definitions>"#
    );

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .add_string(format!("{case_name}.bpmn20.xml"), xml),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name(format!("{case_name} instance")),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(
        boundary_states.len(),
        1,
        "{case_name} should register one message boundary event"
    );

    let message_boundary = boundary_states
        .iter()
        .find(|state| state.boundary_event_id == "messageBoundary")
        .expect("message boundary should be registered");
    assert_eq!(
        message_boundary.event_subscription.kind,
        EventSubscriptionKind::Message
    );
    assert_eq!(
        message_boundary.event_subscription.event_ref, "external.cancel",
        "{case_name} boundary should subscribe by message name resolved from the global definition"
    );
    drop(session);

    runtime_service.trigger_boundary_event_by_message_ref(
        "external.cancel".to_string(),
        process_instance.id.clone(),
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(
        tasks
            .iter()
            .any(|task| task.task_definition_key == "boundaryTask"),
        "{case_name} should route to the message boundary path"
    );
    assert!(
        tasks.len() == 1,
        "{case_name} should leave only the message boundary path task"
    );

    let mut session = runtime_store.create_session().unwrap();
    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        boundary_states_after.is_empty(),
        "{case_name} interrupting boundary should be consumed"
    );
}

#[test]
fn test_named_message_boundary_definitions_register_and_trigger_across_host_activities() {
    let cases = [
        (
            "userTaskHost",
            r#"<userTask id="host" name="Host User Task" />"#,
        ),
        (
            "receiveTaskHost",
            r#"<receiveTask id="host" name="Host Receive Task" />"#,
        ),
        (
            "subProcessHost",
            r#"<subProcess id="host">
                 <startEvent id="subStart" />
                 <sequenceFlow id="subFlow1" sourceRef="subStart" targetRef="innerTask" />
                 <userTask id="innerTask" name="Inner Task" />
                 <sequenceFlow id="subFlow2" sourceRef="innerTask" targetRef="subEnd" />
                 <endEvent id="subEnd" />
               </subProcess>"#,
        ),
        (
            "transactionHost",
            r#"<transaction id="host">
                 <startEvent id="txStart" />
                 <sequenceFlow id="txFlow1" sourceRef="txStart" targetRef="innerTask" />
                 <userTask id="innerTask" name="Inner Task" />
                 <sequenceFlow id="txFlow2" sourceRef="innerTask" targetRef="txEnd" />
                 <endEvent id="txEnd" />
               </transaction>"#,
        ),
    ];

    for (case_name, host_xml) in cases {
        run_named_message_boundary_case(case_name, host_xml);
    }
}
