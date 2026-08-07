use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;
use std::sync::Arc;

#[test]
fn boundary_timer_job_category_is_populated_from_boundary_event() {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 4, 18, 11, 0, 0).unwrap(),
    ));
    let engine =
        ProcessEngine::with_time_source("boundary-timer-job-category".to_string(), time_source);
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="boundaryTimerJobCategory" name="Boundary Timer Job Category">
            <startEvent id="startEvent" />
            <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="hostTask" />
            <userTask id="hostTask" name="Host">
                <extensionElements>
                    <flowable:jobCategory>host-category</flowable:jobCategory>
                </extensionElements>
            </userTask>
            <boundaryEvent id="timeoutBoundary" attachedToRef="hostTask" cancelActivity="true">
                <extensionElements>
                    <flowable:jobCategory>boundary-orders</flowable:jobCategory>
                </extensionElements>
                <timerEventDefinition>
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow2" sourceRef="hostTask" targetRef="normalEnd" />
            <endEvent id="normalEnd" />
            <sequenceFlow id="flow3" sourceRef="timeoutBoundary" targetRef="timeoutEnd" />
            <endEvent id="timeoutEnd" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Boundary Timer Job Category Deployment".to_string())
                .add_string(
                    "boundaryTimerJobCategory.bpmn20.xml".to_string(),
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

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let timer_jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(timer_jobs.len(), 1);
    assert_eq!(timer_jobs[0].activity_id, "timeoutBoundary");
    assert!(timer_jobs[0].is_boundary);
    assert_eq!(
        timer_jobs[0].category.as_deref(),
        Some("boundary-orders"),
        "category must come from the boundary event, not the host activity"
    );
}

#[test]
fn interrupting_user_task_boundary_timer_worker_cleans_host_and_preserves_unrelated_message_boundary()
 {
    let time_source = Arc::new(TestTimeSource::new(
        Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap(),
    ));
    let engine = ProcessEngine::with_time_source(
        "boundary-timer-worker-contract".to_string(),
        time_source.clone(),
    );
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="boundaryTimerWorkerContract" name="Boundary Timer Worker Contract">
            <startEvent id="startEvent" />
            <sequenceFlow id="flow_start_fork" sourceRef="startEvent" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="flow_fork_timer" sourceRef="fork" targetRef="timerHostTask" />
            <sequenceFlow id="flow_fork_message" sourceRef="fork" targetRef="messageHostTask" />

            <userTask id="timerHostTask" name="Timer Host" />
            <boundaryEvent id="timeoutBoundary" attachedToRef="timerHostTask" cancelActivity="true">
                <timerEventDefinition>
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="flow_timer_normal" sourceRef="timerHostTask" targetRef="timerNormalEnd" />
            <endEvent id="timerNormalEnd" />
            <sequenceFlow id="flow_timer_timeout" sourceRef="timeoutBoundary" targetRef="timeoutReviewTask" />
            <userTask id="timeoutReviewTask" name="Timeout Review" />

            <userTask id="messageHostTask" name="Message Host" />
            <boundaryEvent id="unrelatedMessageBoundary" attachedToRef="messageHostTask" cancelActivity="true">
                <messageEventDefinition messageRef="unrelatedMessage" />
            </boundaryEvent>
            <sequenceFlow id="flow_message_normal" sourceRef="messageHostTask" targetRef="messageNormalEnd" />
            <endEvent id="messageNormalEnd" />
            <sequenceFlow id="flow_message_boundary" sourceRef="unrelatedMessageBoundary" targetRef="messageBoundaryTask" />
            <userTask id="messageBoundaryTask" name="Message Boundary Task" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Boundary Timer Worker Contract Deployment".to_string())
                .add_string(
                    "boundaryTimerWorkerContract.bpmn20.xml".to_string(),
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

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let timer_jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(timer_jobs.len(), 1);
    assert_eq!(timer_jobs[0].activity_id, "timeoutBoundary");
    assert!(timer_jobs[0].is_boundary);
    assert!(timer_jobs[0].cancel_activity);
    assert_eq!(
        timer_jobs[0].attached_activity_id.as_deref(),
        Some("timerHostTask")
    );

    let boundary_states = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(boundary_states.len(), 1);
    assert_eq!(
        boundary_states[0].boundary_event_id,
        "unrelatedMessageBoundary"
    );
    assert_eq!(
        boundary_states[0].event_subscription.kind,
        EventSubscriptionKind::Message
    );

    time_source.advance_time(10 * 60 * 1000);
    drop(session);
    let executed = engine.run_due_timers();
    assert_eq!(executed.len(), 1);

    let mut session = runtime_store.create_session().unwrap();
    let remaining_timer_jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert!(
        remaining_timer_jobs.is_empty(),
        "interrupting timer boundary job should be consumed after worker execution"
    );

    drop(session);

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();
    assert_eq!(
        task_keys,
        vec![
            "messageHostTask".to_string(),
            "timeoutReviewTask".to_string()
        ],
        "timer host task should be gone, boundary outgoing task should be active, and unrelated host task should remain"
    );

    let mut session = runtime_store.create_session().unwrap();
    let executions_after = runtime_store.snapshot_executions(&mut session);
    assert!(
        executions_after
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("timerHostTask")),
        "interrupting timer boundary should remove the host user task execution"
    );
    assert!(
        executions_after
            .values()
            .any(|execution| execution.activity_id.as_deref() == Some("timeoutReviewTask")),
        "timer boundary outgoing path should reach the timeout review user task"
    );

    let boundary_states_after = runtime_store
        .find_boundary_event_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(boundary_states_after.len(), 1);
    assert_eq!(
        boundary_states_after[0].boundary_event_id, "unrelatedMessageBoundary",
        "timer execution must not consume unrelated message boundary state"
    );
}
