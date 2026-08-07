//! M73 — Async production enablement matrix.
//!
//! Proves AsyncExecutor + AsyncHistory can be enabled safely without changing
//! the production defaults (`enabled = false` for both).

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::{TestTimeSource, TimeSource};
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_engine::service::config::{
    AsyncExecutorConfiguration, AsyncHistoryConfiguration, DatabaseConfiguration,
    EngineDatabaseKind, HttpServiceRuntimeMode, HttpServiceTaskConfiguration,
    ProcessEngineConfiguration, RealHttpClientConfiguration,
};
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

fn now_fixed() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap()
}

fn fast_async_executor_config() -> AsyncExecutorConfiguration {
    AsyncExecutorConfiguration {
        enabled: true,
        auto_activate: false,
        pool_size: 2,
        queue_size: 64,
        async_job_acquire_wait_ms: 50,
        timer_job_acquire_wait_ms: 50,
        queue_full_wait_ms: 50,
        max_jobs_per_acquisition: 16,
        async_job_lock_time_ms: 5_000,
        timer_lock_time_ms: 5_000,
        reset_expired_jobs_interval_ms: 50,
        reset_expired_jobs_page_size: 50,
        async_job_acquisition_enabled: true,
        timer_job_acquisition_enabled: true,
        reset_expired_job_enabled: true,
        ..AsyncExecutorConfiguration::default()
    }
}

struct ExecutorStopGuard<'a> {
    engine: &'a ProcessEngine,
    stopped: bool,
}

impl<'a> ExecutorStopGuard<'a> {
    fn new(engine: &'a ProcessEngine) -> Self {
        Self {
            engine,
            stopped: false,
        }
    }

    fn stop(mut self) {
        self.engine.stop_timer_executor();
        self.stopped = true;
    }

    fn close(mut self) {
        self.engine.close();
        self.stopped = true;
    }
}

impl Drop for ExecutorStopGuard<'_> {
    fn drop(&mut self) {
        if !self.stopped {
            self.engine.stop_timer_executor();
        }
    }
}

struct TempSqliteFile {
    path: PathBuf,
}

impl TempSqliteFile {
    fn new(prefix: &str) -> Self {
        Self {
            path: std::env::temp_dir().join(format!("{prefix}-{}.db", uuid::Uuid::new_v4())),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn related_paths(&self) -> [PathBuf; 3] {
        [
            self.path.clone(),
            PathBuf::from(format!("{}-wal", self.path.display())),
            PathBuf::from(format!("{}-shm", self.path.display())),
        ]
    }

    fn cleanup(&self) {
        for candidate in self.related_paths() {
            let _ = fs::remove_file(candidate);
        }
    }
}

impl Drop for TempSqliteFile {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn deploy(engine: &ProcessEngine, xml: &str, resource_name: &str) {
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(resource_name.to_string())
                .add_string(resource_name.to_string(), xml.to_string()),
        )
        .unwrap();
}

fn process_definition_id(engine: &ProcessEngine) -> String {
    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone()
}

fn start_by_definition(engine: &ProcessEngine, process_definition_id: String) -> String {
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap()
        .id
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    predicate()
}

fn accept_before(listener: &TcpListener, deadline: Instant) -> TcpStream {
    listener
        .set_nonblocking(true)
        .expect("configure gated HTTP listener");
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .expect("configure accepted gated HTTP stream");
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for automatic HTTP acquisition"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept gated HTTP request: {error}"),
        }
    }
}

fn read_request_headers(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("configure gated HTTP request timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).expect("read gated HTTP request");
        assert!(
            read > 0,
            "HTTP client closed before request headers arrived"
        );
        bytes.extend_from_slice(&buffer[..read]);
    }
}

fn spawn_gated_http_server(
    statuses: Vec<u16>,
) -> (
    SocketAddr,
    Receiver<usize>,
    SyncSender<()>,
    thread::JoinHandle<usize>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind gated HTTP server");
    let address = listener.local_addr().expect("read gated HTTP address");
    let (arrived_tx, arrived_rx) = mpsc::sync_channel(statuses.len());
    let (release_tx, release_rx) = mpsc::sync_channel(statuses.len());
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut served = 0;
        for (attempt, status) in statuses.into_iter().enumerate() {
            let mut stream = accept_before(&listener, deadline);
            read_request_headers(&mut stream);
            arrived_tx
                .send(attempt)
                .expect("signal gated HTTP request arrival");
            release_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("wait for gated HTTP response release");
            let reason = if status == 200 {
                "OK"
            } else {
                "Internal Server Error"
            };
            let body = format!(r#"{{"attempt":{}}}"#, attempt + 1);
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("write gated HTTP response");
            served += 1;
        }
        served
    });
    (address, arrived_rx, release_tx, server)
}

fn persisted_job(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Option<RuntimeTimerJobState> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let job = store
        .find_timer_job_states_by_process_instance_id(process_instance_id, &mut session)
        .into_iter()
        .next();
    session.rollback().unwrap();
    job
}

fn async_continuation_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="enablementAsyncContinuation" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="asyncTask" />
        <serviceTask id="asyncTask" flowable:async="true" />
        <sequenceFlow id="flow2" sourceRef="asyncTask" targetRef="afterAsyncTask" />
        <userTask id="afterAsyncTask" name="After Async" />
        <sequenceFlow id="flow3" sourceRef="afterAsyncTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#
}

fn intermediate_timer_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="enablementTimerProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="timerCatch" />
        <intermediateCatchEvent id="timerCatch" name="Timer Catch">
            <timerEventDefinition>
                <timeDuration>PT0S</timeDuration>
            </timerEventDefinition>
        </intermediateCatchEvent>
        <sequenceFlow id="flow2" sourceRef="timerCatch" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#
}

fn async_http_retry_xml(url: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="enablementAsyncHttpRetry" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="httpTask" />
        <serviceTask id="httpTask" flowable:type="http" flowable:async="true">
            <extensionElements>
                <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
                <flowable:field name="requestUrl"><flowable:string>http://{url}/retry</flowable:string></flowable:field>
                <flowable:field name="failStatusCodes"><flowable:string>5XX</flowable:string></flowable:field>
                <flowable:failedJobRetryTimeCycle>R2/PT10S</flowable:failedJobRetryTimeCycle>
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="httpTask" targetRef="review" />
        <userTask id="review" name="Review" />
    </process>
</definitions>"#
    )
}

fn simple_user_task_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="enablementSimpleProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="userTask1" />
        <userTask id="userTask1" name="User Task" />
        <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#
}

/// Defaults must stay disabled so existing embeds are not surprised.
#[test]
fn defaults_keep_async_executor_and_history_disabled() {
    let config = ProcessEngineConfiguration::default();
    assert!(
        !config.async_executor.enabled,
        "async_executor.enabled must default to false"
    );
    assert!(
        !config.async_history.enabled,
        "async_history.enabled must default to false"
    );
    assert!(
        config.async_history.use_shared_executor,
        "shared history executor is the default when history is later enabled"
    );
    assert!(config.async_executor.reset_expired_job_enabled);
    assert!(config.async_executor.async_job_acquisition_enabled);
    assert!(config.async_executor.timer_job_acquisition_enabled);

    let engine = ProcessEngine::build_with_config(
        "enablement-defaults".to_string(),
        Arc::new(TestTimeSource::new(now_fixed())),
        config,
    )
    .unwrap();
    assert!(
        engine.get_async_executor().is_none(),
        "default engine must not construct AsyncExecutor"
    );
    assert!(
        engine.get_async_history_executor().is_none(),
        "default engine must not construct AsyncHistoryExecutor"
    );
}

/// Enabling construction alone preserves the existing manual-start Rust extension.
#[test]
fn async_executor_enabled_without_auto_activate_stays_inactive_until_started() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor = fast_async_executor_config();
    config.async_executor.auto_activate = false;

    let engine = ProcessEngine::build_with_config(
        "enablement-async-continuation".to_string(),
        time_source,
        config,
    )
    .unwrap();
    assert!(engine.get_async_executor().is_some());

    deploy(
        &engine,
        async_continuation_xml(),
        "enablement-async-continuation.bpmn20.xml",
    );
    let pi_id = start_by_definition(&engine, process_definition_id(&engine));

    // Job exists before the executor starts.
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let jobs = store.find_timer_job_states_by_process_instance_id(&pi_id, &mut session);
        session.rollback().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_state.as_deref(), Some("async"));
    }
    let advanced_without_start = wait_until(Duration::from_millis(300), || {
        !engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi_id.clone())
            .unwrap()
            .is_empty()
    });
    assert!(
        !advanced_without_start,
        "enabled=true with auto_activate=false must not start background acquisition"
    );

    engine.start_timer_executor();

    let advanced = wait_until(Duration::from_secs(5), || {
        !engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi_id.clone())
            .unwrap()
            .is_empty()
    });
    engine.stop_timer_executor();
    assert!(!engine.async_executor_is_active());

    assert!(
        advanced,
        "AsyncExecutor should execute the async continuation job"
    );
    let task_keys: Vec<_> = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap()
        .into_iter()
        .map(|t| t.task_definition_key)
        .collect();
    assert_eq!(task_keys, vec!["afterAsyncTask".to_string()]);
}

/// Java-compatible auto activation starts acquisition during engine build.
#[test]
fn async_executor_auto_activate_runs_without_manual_start() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor = fast_async_executor_config();
    config.async_executor.auto_activate = true;
    config.async_executor.reset_expired_job_enabled = false;

    let engine = ProcessEngine::build_with_config(
        "enablement-auto-activate".to_string(),
        time_source,
        config,
    )
    .unwrap();
    let stop_guard = ExecutorStopGuard::new(&engine);
    deploy(
        &engine,
        async_continuation_xml(),
        "enablement-auto-activate.bpmn20.xml",
    );
    let pi_id = start_by_definition(&engine, process_definition_id(&engine));

    let advanced = wait_until(Duration::from_secs(5), || {
        !engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi_id.clone())
            .unwrap()
            .is_empty()
    });
    stop_guard.close();

    assert!(
        advanced,
        "auto_activate=true must acquire the async continuation without an explicit start call"
    );
    assert!(
        !engine.async_executor_is_active(),
        "ProcessEngine::close must stop every auto-activated executor thread"
    );
    let task_keys: Vec<_> = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect();
    assert_eq!(task_keys, vec!["afterAsyncTask".to_string()]);
}

#[test]
fn configured_owner_and_lock_ttls_apply_to_async_and_timer_acquisition() {
    let database_file = TempSqliteFile::new("flowable-configured-locks");
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let now_ms = time_source.now().timestamp_millis();
    let async_lock_time_ms = 1_234_i64;
    let timer_lock_time_ms = 5_678_i64;
    let configured_owner = "configured-enablement-owner";
    let (address, arrived, release, server) = spawn_gated_http_server(vec![500, 200]);
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor = AsyncExecutorConfiguration {
        auto_activate: true,
        lock_owner: Some(configured_owner.to_string()),
        async_job_lock_time_ms: async_lock_time_ms as u64,
        timer_lock_time_ms: timer_lock_time_ms as u64,
        reset_expired_job_enabled: false,
        ..fast_async_executor_config()
    };
    config.http_service = HttpServiceTaskConfiguration {
        enabled: true,
        runtime_mode: HttpServiceRuntimeMode::Real,
        real_client: RealHttpClientConfiguration {
            retry_count: 0,
            allow_private_networks: true,
                    ..Default::default()
        },
        ..Default::default()
    };
    config.database = DatabaseConfiguration {
        kind: EngineDatabaseKind::Sqlite,
        url: database_file.path().to_string_lossy().into_owned(),
        pool_size: 8,
        busy_timeout_ms: 5_000,
        journal_mode: Default::default(),
    };

    let engine = ProcessEngine::build_with_config(
        "enablement-configured-locks".to_string(),
        time_source.clone(),
        config,
    )
    .unwrap();
    let stop_guard = ExecutorStopGuard::new(&engine);
    deploy(
        &engine,
        &async_http_retry_xml(&address.to_string()),
        "enablement-configured-locks.bpmn20.xml",
    );
    let pi_id = start_by_definition(&engine, process_definition_id(&engine));

    assert_eq!(
        arrived.recv_timeout(Duration::from_secs(5)).unwrap(),
        0,
        "the first HTTP request must be issued by automatic async acquisition"
    );
    let first_acquisition = persisted_job(&engine, &pi_id)
        .expect("the acquired async job must remain visible while HTTP is gated");
    assert_eq!(first_acquisition.job_state.as_deref(), Some("async"));
    assert_eq!(
        first_acquisition.lock_owner.as_deref(),
        Some(configured_owner)
    );
    assert_eq!(first_acquisition.lock_time, Some(now_ms));
    assert_eq!(
        first_acquisition.lock_expiration_time,
        Some(now_ms + async_lock_time_ms),
        "the configured async-job lock TTL must be persisted"
    );

    release.send(()).unwrap();
    let retry_timer_visible = wait_until(Duration::from_secs(5), || {
        persisted_job(&engine, &pi_id).is_some_and(|job| {
            job.job_state.as_deref() == Some("timer")
                && job.retries == Some(1)
                && job.lock_owner.is_none()
        })
    });
    assert!(
        retry_timer_visible,
        "the first failed attempt must become an unlocked future timer"
    );
    let retry_timer = persisted_job(&engine, &pi_id).unwrap();
    assert_eq!(retry_timer.due_time, Some(now_ms + 10_000));
    assert_eq!(retry_timer.lock_time, None);
    assert_eq!(retry_timer.lock_expiration_time, None);

    time_source.advance_time(10_001);
    assert_eq!(
        arrived.recv_timeout(Duration::from_secs(5)).unwrap(),
        1,
        "the second HTTP request must be issued by automatic timer acquisition"
    );
    let second_acquisition = persisted_job(&engine, &pi_id)
        .expect("the acquired retry timer must remain visible while HTTP is gated");
    assert_eq!(second_acquisition.job_state.as_deref(), Some("timer"));
    assert_eq!(
        second_acquisition.lock_owner.as_deref(),
        Some(configured_owner)
    );
    assert_eq!(second_acquisition.lock_time, Some(now_ms + 10_001));
    assert_eq!(
        second_acquisition.lock_expiration_time,
        Some(now_ms + 10_001 + timer_lock_time_ms),
        "the configured timer-job lock TTL must replace the legacy five-minute constant"
    );

    release.send(()).unwrap();
    let completed = wait_until(Duration::from_secs(5), || {
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi_id.clone())
            .unwrap()
            .iter()
            .any(|task| task.task_definition_key == "review")
            && persisted_job(&engine, &pi_id).is_none()
    });
    stop_guard.stop();
    let served = server.join().unwrap();

    assert!(completed, "the successful timer retry must reach review");
    assert_eq!(served, 2, "exactly one HTTP side effect per Job attempt");
    assert!(
        !engine.async_executor_is_active(),
        "the configured acquisition executor must be inactive after explicit stop"
    );
    drop(engine);
    database_file.cleanup();
    assert!(
        database_file
            .related_paths()
            .into_iter()
            .all(|path| !path.exists()),
        "temporary SQLite database, WAL, and SHM files must not leak after the engine closes"
    );
}

/// When AsyncExecutor is enabled, due timer jobs fire without manual run_due_timers.
#[test]
fn async_executor_enabled_fires_due_timer() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor = fast_async_executor_config();

    let engine =
        ProcessEngine::build_with_config("enablement-timer-due".to_string(), time_source, config)
            .unwrap();

    deploy(
        &engine,
        intermediate_timer_xml(),
        "enablement-timer.bpmn20.xml",
    );
    let pi_id = start_by_definition(&engine, process_definition_id(&engine));

    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let jobs = store.find_timer_job_states_by_process_instance_id(&pi_id, &mut session);
        session.rollback().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_state.as_deref(), Some("timer"));
        assert!(jobs[0].due_time.is_some());
    }

    engine.start_timer_executor();

    let ended = wait_until(Duration::from_secs(5), || {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let pi = store.find_process_instance(&pi_id, &mut session);
        let done = pi.map(|p| p.is_ended).unwrap_or(false);
        session.rollback().unwrap();
        done
    });
    engine.stop_timer_executor();

    assert!(
        ended,
        "AsyncExecutor timer acquisition should fire the intermediate timer"
    );
}

/// Shared async history + AsyncExecutor: history jobs flush and execute via the
/// post-commit HistoryJobDispatcher on the shared pool.
#[test]
fn async_history_shared_executor_flushes_and_executes() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor = fast_async_executor_config();
    config.async_history = AsyncHistoryConfiguration {
        enabled: true,
        use_shared_executor: true,
        ..AsyncHistoryConfiguration::default()
    };

    let engine = ProcessEngine::build_with_config(
        "enablement-shared-history".to_string(),
        time_source,
        config,
    )
    .unwrap();
    assert!(engine.get_async_executor().is_some());
    assert!(
        engine.get_async_history_executor().is_none(),
        "shared mode must not create an independent history executor"
    );

    // Start executor first so the HistoryJobDispatcher is live for post-commit notify.
    engine.start_timer_executor();

    deploy(
        &engine,
        simple_user_task_xml(),
        "enablement-shared-history.bpmn20.xml",
    );
    let pi_id = start_by_definition(&engine, process_definition_id(&engine));

    let history_appeared = wait_until(Duration::from_secs(5), || {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let hist = store.list_historic_process_instances(&mut session);
        let found = hist.iter().any(|h| h.id == pi_id);
        session.rollback().unwrap();
        found
    });
    engine.stop_timer_executor();

    assert!(
        history_appeared,
        "shared HistoryJobDispatcher should execute history jobs and write historic rows"
    );

    // Job should be gone after successful replay (or never linger).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let history_jobs: Vec<_> = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .filter(|j| j.job_state.as_deref() == Some("history"))
        .collect();
    session.rollback().unwrap();
    assert!(
        history_jobs.is_empty(),
        "history jobs should be deleted after successful shared-executor replay"
    );
}

/// Independent history executor still drains history jobs when use_shared_executor=false.
#[test]
fn async_history_independent_executor_drains_history_jobs() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    // Main async executor off — only the independent history executor runs.
    config.async_executor.enabled = false;
    config.async_history = AsyncHistoryConfiguration {
        enabled: true,
        use_shared_executor: false,
        acquire_interval_ms: 50,
        pool_size: 2,
        queue_size: 64,
        ..AsyncHistoryConfiguration::default()
    };

    let engine = ProcessEngine::build_with_config(
        "enablement-independent-history".to_string(),
        time_source,
        config,
    )
    .unwrap();
    assert!(engine.get_async_history_executor().is_some());

    engine.start_timer_executor();

    deploy(
        &engine,
        simple_user_task_xml(),
        "enablement-independent-history.bpmn20.xml",
    );
    let pi_id = start_by_definition(&engine, process_definition_id(&engine));

    let history_appeared = wait_until(Duration::from_secs(5), || {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let hist = store.list_historic_process_instances(&mut session);
        let found = hist.iter().any(|h| h.id == pi_id);
        session.rollback().unwrap();
        found
    });
    engine.stop_timer_executor();

    assert!(
        history_appeared,
        "independent AsyncHistoryExecutor should drain history jobs"
    );
}

/// Lock expiry: ResetExpiredJobs / reset_expired_timer_job_locks reclaims after
/// lock_expiration_time with a controllable TimeSource.
#[test]
fn lock_expiry_reclaims_job_after_expiration() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    // Keep background executor off; exercise the reclaim API with TestTimeSource.
    config.async_executor.enabled = false;

    let engine = ProcessEngine::build_with_config(
        "enablement-lock-expiry".to_string(),
        time_source.clone(),
        config,
    )
    .unwrap();
    let store = engine.get_runtime_store();
    let now_ms = time_source.now().timestamp_millis();
    let lock_duration_ms = 1_000i64;

    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "locked-async-job".to_string(),
            process_instance_id: "pi-lock".to_string(),
            execution_id: "ex-lock".to_string(),
            activity_id: "asyncTask".to_string(),
            job_state: Some("async".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(now_ms),
            lock_owner: Some("dead-worker".to_string()),
            lock_time: Some(now_ms),
            lock_expiration_time: Some(now_ms + lock_duration_ms),
            retries: Some(3),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    // Before expiry: reset must not reclaim.
    let reset_before = engine
        .get_runtime_service()
        .reset_expired_timer_job_locks(100);
    assert_eq!(
        reset_before, 0,
        "lock must not be reclaimed before expiration"
    );

    {
        let mut session = store.create_session().unwrap();
        let job = store
            .find_timer_job_state("locked-async-job", &mut session)
            .expect("job must still exist");
        session.rollback().unwrap();
        assert_eq!(job.lock_owner.as_deref(), Some("dead-worker"));
        assert!(job.lock_expiration_time.is_some());
    }

    // Advance controllable clock past lock expiration.
    time_source.advance_time(lock_duration_ms + 1);

    let reset_after = engine
        .get_runtime_service()
        .reset_expired_timer_job_locks(100);
    assert_eq!(
        reset_after, 1,
        "expired lock must be reclaimed by reset_expired_timer_job_locks"
    );

    {
        let mut session = store.create_session().unwrap();
        let job = store
            .find_timer_job_state("locked-async-job", &mut session)
            .expect("job must remain executable after reclaim");
        session.rollback().unwrap();
        assert!(job.lock_owner.is_none(), "lock owner cleared after reclaim");
        assert!(job.lock_time.is_none());
        assert!(job.lock_expiration_time.is_none());
        assert_eq!(job.job_state.as_deref(), Some("async"));
        assert_eq!(job.retries, Some(3));
    }

    // Reclaimed job is acquirable again.
    let acquired = engine.get_runtime_service().acquire_async_jobs(5_000, 10);
    assert!(
        acquired
            .iter()
            .any(|j| j.timer_job_id == "locked-async-job"),
        "reclaimed job must be acquirable by a new worker"
    );
}

/// Enabling the executor constructs the coordinator; disabling leaves the legacy path.
#[test]
fn async_executor_construction_follows_enabled_flag() {
    let time_source = Arc::new(TestTimeSource::new(now_fixed()));

    let mut off = ProcessEngineConfiguration::default();
    off.async_executor.enabled = false;
    let engine_off = ProcessEngine::build_with_config(
        "enablement-flag-off".to_string(),
        Arc::clone(&time_source) as Arc<dyn TimeSource>,
        off,
    )
    .unwrap();
    assert!(engine_off.get_async_executor().is_none());

    let mut on = ProcessEngineConfiguration::default();
    on.async_executor = fast_async_executor_config();
    let engine_on =
        ProcessEngine::build_with_config("enablement-flag-on".to_string(), time_source, on)
            .unwrap();
    let exec = engine_on
        .get_async_executor()
        .expect("enabled flag must construct AsyncExecutor");
    assert!(exec.configuration().enabled);
    assert_eq!(exec.configuration().pool_size, 2);
}

#[test]
fn reset_expired_jobs_respects_enabled_job_categories() {
    use flowable_engine::persistence::runtime_store::{ExpiredJobClass, RuntimeTimerJobState};

    let time_source = Arc::new(TestTimeSource::new(now_fixed()));
    let mut config = ProcessEngineConfiguration::default();
    config.async_executor.enabled = false;
    let engine = ProcessEngine::build_with_config(
        "reset-expired-category-scope".to_string(),
        time_source.clone(),
        config,
    )
    .unwrap();
    let store = engine.get_runtime_store();
    let now_ms = time_source.now().timestamp_millis();

    let mut session = store.create_session().unwrap();
    for (id, category, state) in [
        ("async-allowed", Some("batch"), "async"),
        ("async-denied", Some("interactive"), "async"),
        ("async-null-category", None, "async"),
        ("timer-allowed", Some("batch"), "timer"),
        ("history-any-category", Some("interactive"), "history"),
        ("history-null-category", None, "history"),
    ] {
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: id.to_string(),
                process_instance_id: format!("pi-{id}"),
                execution_id: format!("ex-{id}"),
                activity_id: "activity".to_string(),
                job_state: Some(state.to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                due_time: Some(now_ms - 2_000),
                lock_owner: Some("dead-owner".to_string()),
                lock_time: Some(now_ms - 2_000),
                lock_expiration_time: Some(now_ms - 1_000),
                retries: Some(3),
                error_message: None,
                error_details: None,
                category: category.map(str::to_string),
                ..Default::default()
            },
            &mut session,
        );
    }
    session.flush_and_commit().unwrap();

    let categories = vec!["batch".to_string()];
    let async_outcome = engine
        .get_runtime_service()
        .reset_expired_jobs_batch_scoped(ExpiredJobClass::Async, 10, &[], &categories)
        .unwrap();
    assert_eq!(async_outcome.reset, 1, "only allowed async category");

    let timer_outcome = engine
        .get_runtime_service()
        .reset_expired_jobs_batch_scoped(ExpiredJobClass::Timer, 10, &[], &categories)
        .unwrap();
    assert_eq!(timer_outcome.reset, 1, "only allowed timer category");

    // History ignores enabled categories (including None category).
    let history_outcome = engine
        .get_runtime_service()
        .reset_expired_jobs_batch_scoped(ExpiredJobClass::History, 10, &[], &categories)
        .unwrap();
    assert_eq!(
        history_outcome.reset, 2,
        "history reset must ignore category filtering"
    );

    let mut session = store.create_session().unwrap();
    for (id, should_reset) in [
        ("async-allowed", true),
        ("async-denied", false),
        ("async-null-category", false),
        ("timer-allowed", true),
        ("history-any-category", true),
        ("history-null-category", true),
    ] {
        let job = store.find_timer_job_state(id, &mut session).unwrap();
        if should_reset {
            assert!(job.lock_owner.is_none(), "{id} should have been reset");
        } else {
            assert_eq!(
                job.lock_owner.as_deref(),
                Some("dead-owner"),
                "{id} should retain its expired lease"
            );
        }
    }
    session.rollback().unwrap();
}
