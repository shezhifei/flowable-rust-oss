use chrono::Utc;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::time_source::TestTimeSource;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

fn unique_snapshot_path(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "flowable-recovery-snapshot-{test_name}-{}.json",
        Uuid::new_v4()
    ))
}

fn cleanup_snapshot_file(path: &Path) {
    if path.exists() {
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn exports_snapshot_file_and_restores_user_task_process_state() {
    let snapshot_path = unique_snapshot_path("user-task");

    let engine1 = ProcessEngine::new("snapshot_export_engine".to_string());
    let deployment_builder = engine1
        .get_repository_service()
        .create_deployment()
        .add_string(
            "user_task_snapshot.bpmn".to_string(),
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
              <process id="snapshotUserTaskProcess" isExecutable="true">
                <startEvent id="start"/>
                <sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask"/>
                <userTask id="approveTask" name="Approve"/>
                <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end"/>
                <endEvent id="end"/>
              </process>
            </definitions>"#
                .to_string(),
        );
    engine1
        .get_repository_service()
        .deploy(deployment_builder)
        .unwrap();

    let process_definition_id = engine1
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let process_instance = engine1
        .get_runtime_service()
        .start_process_instance(
            engine1
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    engine1
        .export_recovery_snapshot_to_file(&snapshot_path)
        .unwrap();

    let engine2 = ProcessEngine::new("snapshot_import_engine".to_string());
    engine2
        .import_recovery_snapshot_from_file(&snapshot_path)
        .unwrap();

    let tasks = engine2
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "approveTask");

    engine2
        .get_task_service()
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let recovered_store = engine2.get_runtime_store();
    let mut recovered_session = recovered_store.create_session().unwrap();
    let recovered_instance = recovered_store
        .find_process_instance(&process_instance.id, &mut recovered_session)
        .unwrap();
    assert!(recovered_instance.is_ended);

    cleanup_snapshot_file(&snapshot_path);
}

#[test]
fn imports_snapshot_file_and_recovers_timer_wait_and_timer_start_state() {
    let snapshot_path = unique_snapshot_path("timer-state");
    let time_source = Arc::new(TestTimeSource::new(Utc::now()));
    let engine1 = ProcessEngine::with_time_source(
        "snapshot_timer_export_engine".to_string(),
        time_source.clone(),
    );

    let deployment_builder = engine1
        .get_repository_service()
        .create_deployment()
        .add_string(
            "timer_wait_snapshot.bpmn".to_string(),
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
              <process id="snapshotTimerProcess" isExecutable="true">
                <startEvent id="start"/>
                <sequenceFlow id="flow1" sourceRef="start" targetRef="waitTimer"/>
                <intermediateCatchEvent id="waitTimer">
                  <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                  </timerEventDefinition>
                </intermediateCatchEvent>
                <sequenceFlow id="flow2" sourceRef="waitTimer" targetRef="end"/>
                <endEvent id="end"/>
              </process>
            </definitions>"#
                .to_string(),
        )
        .add_string(
            "timer_start_snapshot.bpmn".to_string(),
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
              <process id="snapshotTimerStartProcess" isExecutable="true">
                <startEvent id="timerStart">
                  <timerEventDefinition>
                    <timeDuration>PT1H</timeDuration>
                  </timerEventDefinition>
                </startEvent>
                <sequenceFlow id="flow1" sourceRef="timerStart" targetRef="timerStartTask"/>
                <userTask id="timerStartTask" name="Timer Started Task"/>
                <sequenceFlow id="flow2" sourceRef="timerStartTask" targetRef="timerStartEnd"/>
                <endEvent id="timerStartEnd"/>
              </process>
            </definitions>"#
                .to_string(),
        );
    engine1
        .get_repository_service()
        .deploy(deployment_builder)
        .unwrap();

    let waiting_instance = engine1
        .get_runtime_service()
        .start_process_instance(
            engine1
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_key("snapshotTimerProcess".to_string()),
        )
        .unwrap();

    assert_eq!(engine1.get_timer_start_subscriptions().len(), 1);

    engine1
        .export_recovery_snapshot_to_file(&snapshot_path)
        .unwrap();

    let engine2 = ProcessEngine::with_time_source(
        "snapshot_timer_import_engine".to_string(),
        time_source.clone(),
    );
    engine2
        .import_recovery_snapshot_from_file(&snapshot_path)
        .unwrap();

    assert_eq!(engine2.get_timer_start_subscriptions().len(), 1);

    time_source.advance_time(2 * 60 * 60 * 1000);

    let dispatched = engine2.run_due_timers();
    assert_eq!(dispatched.len(), 2);

    let resumed_store = engine2.get_runtime_store();
    let mut resumed_session = resumed_store.create_session().unwrap();
    let resumed_waiting_instance = resumed_store
        .find_process_instance(&waiting_instance.id, &mut resumed_session)
        .unwrap();
    assert!(resumed_waiting_instance.is_ended);
    drop(resumed_session);

    let timer_started_tasks = engine2
        .get_task_service()
        .create_task_query()
        .task_name("Timer Started Task".to_string())
        .list()
        .unwrap()
        .into_iter()
        .filter(|task| task.task_definition_key == "timerStartTask")
        .collect::<Vec<_>>();
    assert_eq!(timer_started_tasks.len(), 1);

    engine2
        .get_task_service()
        .complete_task_by_id(timer_started_tasks[0].id.clone())
        .unwrap();

    let ended_store = engine2.get_runtime_store();
    let mut ended_session = ended_store.create_session().unwrap();
    let ended_instances = ended_store
        .snapshot_process_instances(&mut ended_session)
        .into_values()
        .filter(|instance| instance.is_ended)
        .count();
    assert_eq!(ended_instances, 2);

    cleanup_snapshot_file(&snapshot_path);
}
