use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

fn unique_sqlite_path(prefix: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!(
        "{prefix}-{}-{}.sqlite",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let path = path.to_string_lossy().into_owned();
    remove_sqlite_files(&path);
    path
}

fn remove_sqlite_files(path: &str) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{path}-wal"));
    let _ = std::fs::remove_file(format!("{path}-shm"));
}

#[test]
fn test_concurrent_timer_acquisition() {
    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let mock_time = Arc::new(TestTimeSource::new(now));

    let db_path = unique_sqlite_path("flowable-concurrent-timers");

    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_file(&db_path).unwrap());

    let engine1 = Arc::new(ProcessEngine::build(
        "engine1".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    ));

    let bpmn_xml = r#"
    <bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
      <bpmn2:process id="concurrent_timer_test_process" isExecutable="true">
        <bpmn2:startEvent id="start" />
        <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="timer" />
        <bpmn2:intermediateCatchEvent id="timer">
          <bpmn2:timerEventDefinition>
            <bpmn2:timeDuration>PT5M</bpmn2:timeDuration>
          </bpmn2:timerEventDefinition>
        </bpmn2:intermediateCatchEvent>
        <bpmn2:sequenceFlow id="flow2" sourceRef="timer" targetRef="end" />
        <bpmn2:endEvent id="end" />
      </bpmn2:process>
    </bpmn2:definitions>
    "#;

    let builder = engine1
        .get_repository_service()
        .create_deployment()
        .name("concurrent_timer_test_deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), bpmn_xml.to_string());
    engine1.get_repository_service().deploy(builder).unwrap();

    let engine2 = Arc::new(ProcessEngine::build(
        "engine2".to_string(),
        Arc::clone(&mock_time) as Arc<_>,
        Arc::clone(&db_store),
    ));

    // Start instance using engine1
    let pd_id = engine1
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
                .process_definition_id(pd_id),
        )
        .unwrap();

    let __runtime_store = engine1.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let timer_jobs = __runtime_store.snapshot_timer_job_states(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert_eq!(timer_jobs.len(), 1);

    // Advance time so it's due
    mock_time.advance_time(300_001);

    // Run two separate engines against the same DB to prove acquisition is exclusive.
    let e1 = Arc::clone(&engine1);
    let e2 = Arc::clone(&engine2);

    let handle1 = thread::spawn(move || e1.run_due_timers());

    let handle2 = thread::spawn(move || e2.run_due_timers());

    let res1 = handle1.join().unwrap();
    let res2 = handle2.join().unwrap();

    let total_executed = res1.len() + res2.len();
    assert_eq!(
        total_executed, 1,
        "Timer should only be executed exactly once, but got res1: {:?}, res2: {:?}",
        res1, res2
    );

    let runtime_store = engine1.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let p_is_ended = runtime_store
        .snapshot_process_instances(&mut session)
        .get(&process_instance.id)
        .unwrap()
        .is_ended;
    assert!(p_is_ended);

    remove_sqlite_files(&db_path);
}
