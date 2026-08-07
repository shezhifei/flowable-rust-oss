mod differential;

use chrono::{TimeZone, Utc};
use differential::{
    ContractCase, ContractFixture, ContractResponse, DEFAULT_WALL_TIMEOUT,
    java_compatible_error_message, load_fixture, normalize_rust_tasks, normalize_rust_variables,
    run_java_contract_runner, run_rust_operations_case, workspace_root,
};
use flowable_engine::bpmn::http_handler::{
    HttpHandlerRegistry, HttpResponseHandler, HttpResponseHandlerContext,
};
use flowable_engine::engine::event_dispatcher::{
    EngineEvent, EngineEventDispatcher, EngineEventListener, EngineEventType, TransactionState,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::runtime_service::RuntimeService;
use flowable_engine::engine::time_source::{SystemTimeSource, TestTimeSource, TimeSource};
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_engine::service::config::{
    AsyncExecutorConfiguration, AsyncExecutorTenantScope, DatabaseConfiguration,
    EngineDatabaseKind, HttpServiceRuntimeMode, HttpServiceTaskConfiguration,
    ProcessEngineConfiguration, RealHttpClientConfiguration,
};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const AUTOMATIC_WALL_TIMEOUT: Duration = DEFAULT_WALL_TIMEOUT;

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    body: String,
}

struct UnrecoverableResponseHandler;

impl HttpResponseHandler for UnrecoverableResponseHandler {
    fn handle_response(
        &self,
        _context: &mut HttpResponseHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        Err(FlowableError::UnrecoverableJobError(
            "response payload cannot be safely processed".to_string(),
        ))
    }
}

struct NestedUnrecoverableResponseHandler;

impl HttpResponseHandler for NestedUnrecoverableResponseHandler {
    fn handle_response(
        &self,
        _context: &mut HttpResponseHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        Err(
            FlowableError::ExecutionError("response handler wrapper failed".to_string()).caused_by(
                FlowableError::UnrecoverableJobError(
                    "response payload cannot be safely processed".to_string(),
                ),
            ),
        )
    }
}

struct RecordingJobEventListener {
    events: Arc<Mutex<Vec<String>>>,
}

struct TransactionRecordingJobEventListener {
    label: &'static str,
    state: Option<TransactionState>,
    fatal: bool,
    phases: Arc<Mutex<Vec<String>>>,
}

impl EngineEventListener for TransactionRecordingJobEventListener {
    fn on_event(&self, _event: &EngineEvent) -> Result<(), FlowableError> {
        self.phases.lock().unwrap().push(self.label.to_string());
        if self.fatal {
            return Err(FlowableError::ExecutionError(format!(
                "fatal {} job event listener",
                self.label
            )));
        }
        Ok(())
    }

    fn is_fail_on_exception(&self) -> bool {
        self.fatal
    }

    fn is_fire_on_transaction_lifecycle_event(&self) -> bool {
        self.state.is_some()
    }

    fn on_transaction(&self) -> TransactionState {
        self.state.unwrap_or(TransactionState::Committed)
    }
}

impl EngineEventListener for RecordingJobEventListener {
    fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
        // Mirror Java RecordingJobEventListener#getTypes — only job lifecycle events.
        let name = match event.event_type() {
            EngineEventType::EntityUpdated => "ENTITY_UPDATED",
            EngineEventType::JobExecutionFailure => "JOB_EXECUTION_FAILURE",
            EngineEventType::JobExecutionSuccess => "JOB_EXECUTION_SUCCESS",
            EngineEventType::JobMovedToDeadLetter => "JOB_MOVED_TO_DEADLETTER",
            EngineEventType::JobRetriesDecremented => "JOB_RETRIES_DECREMENTED",
            _ => return Ok(()),
        };
        self.events.lock().unwrap().push(name.to_string());
        Ok(())
    }
}

#[test]
#[ignore = "requires the sibling Flowable Java checkout and its Maven wrapper"]
fn flowable_java_and_rust_match_shared_http_contract_fixtures() {
    let root = workspace_root();
    let fixture_directory = root.join("differential/fixtures/http");
    let fixture = load_fixture(&fixture_directory);

    let java_output = run_java_contract_runner(&root, &fixture_directory, "java-http.json");
    assert_eq!(
        java_output["flowableVersion"],
        json!(fixture.flowable_java_version),
        "the Java runner version must remain explicitly pinned by the shared fixture"
    );

    let rust_output = run_rust_contracts(&fixture_directory, &fixture);
    let output_directory = root.join("target/differential");
    fs::create_dir_all(&output_directory).expect("create differential output directory");
    fs::write(
        output_directory.join("rust-http.json"),
        serde_json::to_vec_pretty(&rust_output).expect("serialize Rust normalized output"),
    )
    .expect("write Rust normalized output");

    assert_eq!(
        rust_output["cases"], java_output["cases"],
        "Flowable Rust HTTP behavior diverged from the normalized Flowable Java contract"
    );
}

fn run_rust_contracts(fixture_directory: &Path, fixture: &ContractFixture) -> Value {
    let mut cases = Map::new();
    for contract_case in &fixture.cases {
        cases.insert(
            contract_case.id.clone(),
            run_rust_contract(fixture_directory, fixture, contract_case),
        );
    }
    json!({
        "engine": "flowable-rust",
        "flowableVersion": fixture.flowable_java_version,
        "cases": cases,
    })
}

fn run_rust_contract(
    fixture_directory: &Path,
    fixture: &ContractFixture,
    contract_case: &ContractCase,
) -> Value {
    if contract_case.is_operations_case() {
        return run_rust_operations_case(fixture_directory, fixture, contract_case);
    }
    if contract_case.execution.as_deref() == Some("automaticAsyncRetry") {
        return run_rust_automatic_async_retry_contract(fixture_directory, fixture, contract_case);
    }
    if contract_case.execution.as_deref() == Some("unlockOwnedJobs") {
        return run_rust_unlock_owned_jobs_contract(fixture_directory, fixture, contract_case);
    }
    if contract_case.execution.as_deref() == Some("sharedMultiTenantUnlockOwnedJobs") {
        return run_rust_shared_multi_tenant_unlock_owned_jobs_contract(
            fixture_directory,
            fixture,
            contract_case,
        );
    }
    if contract_case
        .execution
        .as_deref()
        .is_some_and(|execution| execution.ends_with("Cancel"))
    {
        return run_rust_cancel_contract(fixture_directory, contract_case);
    }
    let (endpoint, server) = spawn_contract_server(contract_case.clone());
    let observed_events = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    event_dispatcher.add_event_listener(Arc::new(RecordingJobEventListener {
        events: Arc::clone(&observed_events),
    }));
    let mut handlers = HttpHandlerRegistry::new();
    match contract_case.execution.as_deref() {
        Some("unrecoverable") => {
            handlers.register_response_handler(
                "org.flowable.rust.contract.UnrecoverableResponseHandler",
                Arc::new(UnrecoverableResponseHandler),
            );
        }
        Some("nestedUnrecoverable") => {
            handlers.register_response_handler(
                "org.flowable.rust.contract.NestedUnrecoverableResponseHandler",
                Arc::new(NestedUnrecoverableResponseHandler),
            );
        }
        _ => {}
    }
    let engine = ProcessEngine::build_with_config(
        format!("java-http-differential-{}", contract_case.id),
        Arc::new(SystemTimeSource),
        ProcessEngineConfiguration {
            http_service: HttpServiceTaskConfiguration {
                enabled: true,
                runtime_mode: HttpServiceRuntimeMode::Real,
                real_client: RealHttpClientConfiguration {
                    retry_count: 0,
                    allow_private_networks: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            http_handler_registry: Some(handlers),
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .expect("build Rust engine for differential HTTP fixture");

    let bpmn_name = contract_case
        .bpmn
        .as_deref()
        .expect("HTTP contract case requires bpmn");
    let bpmn = fs::read_to_string(fixture_directory.join(bpmn_name)).expect("read shared BPMN fixture");
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(format!("contract-{}", contract_case.id))
                .add_string(bpmn_name.to_string(), bpmn),
        )
        .expect("deploy shared BPMN fixture in Rust");
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("query deployed Rust process definition")[0]
        .clone();
    let process_instance = engine.get_runtime_service().start_process_instance(
        engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(process_definition_id)
            .variable("endpoint".to_string(), json!(endpoint)),
    );

    let observe_variables = contract_case.resolved_observe_variables(fixture);
    if contract_case.execution.as_deref() == Some("syncObserved") {
        return normalize_rust_observed_sync_case(
            &engine,
            process_instance,
            server,
            &observe_variables,
        );
    }
    let process_instance = process_instance.expect("start shared BPMN fixture in Rust");

    if contract_case.execution.is_some() {
        return run_rust_async_contract(
            &engine,
            &process_instance.id,
            contract_case,
            server,
            observed_events,
            &observe_variables,
        );
    }

    let captured_requests = server.join().expect("join Rust contract HTTP server");
    let captured_request = captured_requests
        .into_iter()
        .next()
        .expect("sync HTTP fixture should issue one request");
    let variables = normalize_rust_variables(&engine, &process_instance.id, &observe_variables);
    let tasks = normalize_rust_tasks(&engine, &process_instance.id);

    json!({
        "request": {
            "method": captured_request.method,
            "path": captured_request.path,
            "body": captured_request.body,
        },
        "variables": variables,
        "tasks": tasks,
        "error": null,
    })
}

fn run_rust_unlock_owned_jobs_contract(
    fixture_directory: &Path,
    fixture: &ContractFixture,
    contract_case: &ContractCase,
) -> Value {
    let lock_owner = fixture.unlock_owned_jobs_lock_owner.as_str();
    json!({
        "defaultUnlockOwnedJobs": AsyncExecutorConfiguration::default().unlock_owned_jobs,
        "defaultPolicy": run_rust_unlock_owned_jobs_policy(
            fixture_directory,
            contract_case,
            true,
            "default",
            lock_owner,
        ),
        "explicitFalse": run_rust_unlock_owned_jobs_policy(
            fixture_directory,
            contract_case,
            false,
            "disabled",
            lock_owner,
        ),
    })
}

fn run_rust_unlock_owned_jobs_policy(
    fixture_directory: &Path,
    contract_case: &ContractCase,
    unlock_owned_jobs: bool,
    policy_name: &str,
    lock_owner: &str,
) -> Value {
    let engine = ProcessEngine::build_with_config(
        format!("rust-unlock-owned-jobs-{policy_name}"),
        Arc::new(SystemTimeSource),
        ProcessEngineConfiguration {
            async_executor: AsyncExecutorConfiguration {
                enabled: true,
                auto_activate: false,
                lock_owner: Some(lock_owner.to_string()),
                unlock_owned_jobs,
                async_job_acquisition_enabled: false,
                timer_job_acquisition_enabled: false,
                reset_expired_job_enabled: false,
                ..AsyncExecutorConfiguration::default()
            },
            ..ProcessEngineConfiguration::default()
        },
    )
    .expect("build Rust unlockOwnedJobs differential engine");

    let bpmn_name = contract_case
        .bpmn
        .as_deref()
        .expect("unlockOwnedJobs case requires bpmn");
    let bpmn = fs::read_to_string(fixture_directory.join(bpmn_name))
        .expect("read unlockOwnedJobs BPMN fixture");
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(format!("contract-{}-{policy_name}", contract_case.id))
                .add_string(bpmn_name.to_string(), bpmn),
        )
        .expect("deploy unlockOwnedJobs BPMN fixture in Rust");
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("query unlockOwnedJobs process definition")[0]
        .clone();

    engine.start_timer_executor();
    let shutdown_process_instance_id =
        start_rust_unlock_owned_jobs_process(&engine, &process_definition_id);
    let shutdown_before = acquire_rust_owned_job(&engine, &shutdown_process_instance_id);
    let shutdown_active_before = engine.async_executor_is_active();
    engine.stop_timer_executor();
    let shutdown_after = require_rust_owned_job(&engine, &shutdown_before.timer_job_id);

    let startup_process_instance_id =
        start_rust_unlock_owned_jobs_process(&engine, &process_definition_id);
    let startup_before = acquire_rust_owned_job(&engine, &startup_process_instance_id);
    let startup_active_before = engine.async_executor_is_active();
    engine.start_timer_executor();
    let startup_after = require_rust_owned_job(&engine, &startup_before.timer_job_id);

    let normalized = json!({
        "configuredUnlockOwnedJobs": engine
            .get_async_executor()
            .expect("unlockOwnedJobs fixture must construct AsyncExecutor")
            .configuration()
            .unlock_owned_jobs,
        "shutdown": normalize_rust_unlock_transition(
            &shutdown_before,
            &shutdown_after,
            shutdown_active_before,
            false,
        ),
        "startup": normalize_rust_unlock_transition(
            &startup_before,
            &startup_after,
            startup_active_before,
            engine.async_executor_is_active(),
        ),
    });
    engine.close();
    normalized
}

fn start_rust_unlock_owned_jobs_process(
    engine: &ProcessEngine,
    process_definition_id: &str,
) -> String {
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.to_string()),
        )
        .expect("start unlockOwnedJobs BPMN fixture in Rust")
        .id
}

fn acquire_rust_owned_job(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> RuntimeTimerJobState {
    engine
        .get_runtime_service()
        .acquire_async_jobs_for_tenants(60_000, 16, &[], &[])
        .into_iter()
        .find(|job| job.process_instance_id == process_instance_id)
        .unwrap_or_else(|| {
            panic!("Rust acquisition did not lock process job {process_instance_id}")
        })
}

fn require_rust_owned_job(engine: &ProcessEngine, job_id: &str) -> RuntimeTimerJobState {
    engine
        .get_management_service()
        .find_job_by_id(job_id)
        .unwrap_or_else(|| panic!("unlockOwnedJobs lifecycle removed executable job {job_id}"))
}

fn normalize_rust_unlock_transition(
    before: &RuntimeTimerJobState,
    after: &RuntimeTimerJobState,
    active_before: bool,
    active_after: bool,
) -> Value {
    json!({
        "activeBefore": active_before,
        "activeAfter": active_after,
        "before": normalize_rust_lock_state(before),
        "after": normalize_rust_lock_state(after),
        "stateUnchanged": before.job_state == after.job_state,
        "retriesUnchanged": before.retries == after.retries,
        "dueDateUnchanged": before.due_time == after.due_time,
        "otherFieldsUnchanged": rust_non_lock_job_fields_match(before, after),
    })
}

fn normalize_rust_lock_state(job: &RuntimeTimerJobState) -> Value {
    json!({
        "state": "executable",
        "retries": job.retries.unwrap_or_default(),
        "lockOwner": job.lock_owner,
        "lockExpirationSet": job.lock_expiration_time.is_some(),
    })
}

fn rust_non_lock_job_fields_match(
    before: &RuntimeTimerJobState,
    after: &RuntimeTimerJobState,
) -> bool {
    before.timer_job_id == after.timer_job_id
        && before.process_instance_id == after.process_instance_id
        && before.execution_id == after.execution_id
        && before.activity_id == after.activity_id
        && before.is_boundary == after.is_boundary
        && before.attached_activity_id == after.attached_activity_id
        && before.cancel_activity == after.cancel_activity
        && before.time_duration == after.time_duration
        && before.time_date == after.time_date
        && before.time_cycle == after.time_cycle
        && before.error_message == after.error_message
        && before.error_details == after.error_details
}

fn run_rust_shared_multi_tenant_unlock_owned_jobs_contract(
    fixture_directory: &Path,
    fixture: &ContractFixture,
    contract_case: &ContractCase,
) -> Value {
    let registered_tenants = vec![
        fixture.shared_tenant_a.clone(),
        fixture.shared_tenant_b.clone(),
    ];
    json!({
        "engineDefaultUnlockOwnedJobs": AsyncExecutorConfiguration::default().unlock_owned_jobs,
        "sharedDefaultUnlockOwnedJobs": AsyncExecutorConfiguration::default()
            .shared_multi_tenant()
            .unlock_owned_jobs,
        "registeredTenants": registered_tenants,
        "defaultFalse": run_rust_shared_multi_tenant_unlock_owned_jobs_policy(
            fixture_directory,
            fixture,
            contract_case,
            false,
            "default-false",
        ),
        "explicitTrue": run_rust_shared_multi_tenant_unlock_owned_jobs_policy(
            fixture_directory,
            fixture,
            contract_case,
            true,
            "explicit-true",
        ),
    })
}

fn run_rust_shared_multi_tenant_unlock_owned_jobs_policy(
    fixture_directory: &Path,
    fixture: &ContractFixture,
    contract_case: &ContractCase,
    unlock_owned_jobs: bool,
    policy_name: &str,
) -> Value {
    let mut async_executor = AsyncExecutorConfiguration::default()
        .shared_multi_tenant()
        .with_tenant_scope(AsyncExecutorTenantScope::Tenants(vec![
            fixture.shared_tenant_a.clone(),
            fixture.shared_tenant_b.clone(),
        ]));
    async_executor.enabled = true;
    async_executor.auto_activate = false;
    async_executor.lock_owner = Some(fixture.unlock_owned_jobs_lock_owner.clone());
    async_executor.unlock_owned_jobs = unlock_owned_jobs;
    async_executor.async_job_acquisition_enabled = false;
    async_executor.timer_job_acquisition_enabled = false;
    async_executor.reset_expired_job_enabled = false;

    let engine = ProcessEngine::build_with_config(
        format!("rust-shared-unlock-owned-jobs-{policy_name}"),
        Arc::new(SystemTimeSource),
        ProcessEngineConfiguration {
            async_executor,
            ..ProcessEngineConfiguration::default()
        },
    )
    .expect("build Rust shared multi-tenant unlockOwnedJobs differential engine");

    let bpmn_name = contract_case
        .bpmn
        .as_deref()
        .expect("shared unlockOwnedJobs case requires bpmn");
    let bpmn = fs::read_to_string(fixture_directory.join(bpmn_name))
        .expect("read shared multi-tenant unlockOwnedJobs BPMN fixture");
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(format!("contract-{}-{policy_name}", contract_case.id))
                .add_string(bpmn_name.to_string(), bpmn),
        )
        .expect("deploy shared multi-tenant unlockOwnedJobs BPMN fixture in Rust");
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("query shared multi-tenant unlockOwnedJobs process definition")[0]
        .clone();

    let process_instances =
        create_and_acquire_rust_shared_unlock_jobs(&engine, fixture, &process_definition_id);
    let before_start = snapshot_rust_shared_unlock_jobs(&engine, &process_instances);
    let startup_active_before = engine.async_executor_is_active();
    engine
        .try_start_timer_executor()
        .expect("start Rust shared multi-tenant executor");
    let after_start = snapshot_rust_shared_unlock_jobs(&engine, &process_instances);

    let shutdown_active_before = engine.async_executor_is_active();
    engine
        .try_stop_timer_executor()
        .expect("shutdown Rust shared multi-tenant executor");
    let after_shutdown = snapshot_rust_shared_unlock_jobs(&engine, &process_instances);

    let normalized = json!({
        "configuredUnlockOwnedJobs": engine
            .get_async_executor()
            .expect("shared multi-tenant fixture must construct AsyncExecutor")
            .configuration()
            .unlock_owned_jobs,
        "startup": normalize_rust_shared_unlock_phase(
            &before_start,
            &after_start,
            startup_active_before,
            true,
        ),
        "shutdown": normalize_rust_shared_unlock_phase(
            &after_start,
            &after_shutdown,
            shutdown_active_before,
            false,
        ),
    });
    engine.close();
    normalized
}

fn create_and_acquire_rust_shared_unlock_jobs(
    engine: &ProcessEngine,
    fixture: &ContractFixture,
    process_definition_id: &str,
) -> SharedUnlockProcessInstances {
    let runtime = engine.get_runtime_service();
    let registered_tenant_a = start_rust_unlock_owned_jobs_process_for_tenant(
        engine,
        process_definition_id,
        &fixture.shared_tenant_a,
    );
    acquire_rust_owned_job_with_runtime(engine, &runtime, &registered_tenant_a);
    let registered_tenant_b = start_rust_unlock_owned_jobs_process_for_tenant(
        engine,
        process_definition_id,
        &fixture.shared_tenant_b,
    );
    acquire_rust_owned_job_with_runtime(engine, &runtime, &registered_tenant_b);
    let unregistered_tenant_c = start_rust_unlock_owned_jobs_process_for_tenant(
        engine,
        process_definition_id,
        &fixture.shared_tenant_c,
    );
    acquire_rust_owned_job_with_runtime(engine, &runtime, &unregistered_tenant_c);

    let other_owner_tenant_a = start_rust_unlock_owned_jobs_process_for_tenant(
        engine,
        process_definition_id,
        &fixture.shared_tenant_a,
    );
    let other_owner_runtime = RuntimeService::new(
        engine.get_command_executor(),
        Arc::<str>::from(fixture.shared_unlock_other_owner.as_str()),
    );
    acquire_rust_owned_job_with_runtime(engine, &other_owner_runtime, &other_owner_tenant_a);

    SharedUnlockProcessInstances {
        registered_tenant_a,
        registered_tenant_b,
        unregistered_tenant_c,
        other_owner_tenant_a,
    }
}

fn start_rust_unlock_owned_jobs_process_for_tenant(
    engine: &ProcessEngine,
    process_definition_id: &str,
    tenant_id: &str,
) -> String {
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.to_string())
                .tenant_id(tenant_id.to_string()),
        )
        .expect("start tenant-scoped unlockOwnedJobs BPMN fixture in Rust")
        .id
}

fn acquire_rust_owned_job_with_runtime(
    engine: &ProcessEngine,
    runtime: &RuntimeService,
    process_instance_id: &str,
) {
    let acquired = runtime.acquire_async_jobs_for_tenants(60_000, 16, &[], &[]);
    assert!(
        acquired
            .iter()
            .any(|job| job.process_instance_id == process_instance_id),
        "Rust acquisition did not lock process job {process_instance_id}"
    );
    require_rust_owned_job(
        engine,
        &acquired
            .into_iter()
            .find(|job| job.process_instance_id == process_instance_id)
            .expect("acquired target Rust job")
            .timer_job_id,
    );
}

fn snapshot_rust_shared_unlock_jobs(
    engine: &ProcessEngine,
    process_instances: &SharedUnlockProcessInstances,
) -> SharedUnlockJobSnapshots {
    SharedUnlockJobSnapshots {
        registered_tenant_a: require_rust_owned_job_for_process(
            engine,
            &process_instances.registered_tenant_a,
        ),
        registered_tenant_b: require_rust_owned_job_for_process(
            engine,
            &process_instances.registered_tenant_b,
        ),
        unregistered_tenant_c: require_rust_owned_job_for_process(
            engine,
            &process_instances.unregistered_tenant_c,
        ),
        other_owner_tenant_a: require_rust_owned_job_for_process(
            engine,
            &process_instances.other_owner_tenant_a,
        ),
    }
}

fn require_rust_owned_job_for_process(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> RuntimeTimerJobState {
    engine
        .get_management_service()
        .list_executable_jobs()
        .into_iter()
        .find(|job| job.process_instance_id == process_instance_id)
        .unwrap_or_else(|| {
            panic!("shared unlockOwnedJobs lifecycle removed process job {process_instance_id}")
        })
}

fn normalize_rust_shared_unlock_phase(
    before: &SharedUnlockJobSnapshots,
    after: &SharedUnlockJobSnapshots,
    active_before: bool,
    active_after: bool,
) -> Value {
    json!({
        "registeredTenantA": normalize_rust_unlock_transition(
            &before.registered_tenant_a,
            &after.registered_tenant_a,
            active_before,
            active_after,
        ),
        "registeredTenantB": normalize_rust_unlock_transition(
            &before.registered_tenant_b,
            &after.registered_tenant_b,
            active_before,
            active_after,
        ),
        "unregisteredTenantC": normalize_rust_unlock_transition(
            &before.unregistered_tenant_c,
            &after.unregistered_tenant_c,
            active_before,
            active_after,
        ),
        "otherOwnerTenantA": normalize_rust_unlock_transition(
            &before.other_owner_tenant_a,
            &after.other_owner_tenant_a,
            active_before,
            active_after,
        ),
    })
}

struct SharedUnlockProcessInstances {
    registered_tenant_a: String,
    registered_tenant_b: String,
    unregistered_tenant_c: String,
    other_owner_tenant_a: String,
}

struct SharedUnlockJobSnapshots {
    registered_tenant_a: RuntimeTimerJobState,
    registered_tenant_b: RuntimeTimerJobState,
    unregistered_tenant_c: RuntimeTimerJobState,
    other_owner_tenant_a: RuntimeTimerJobState,
}

struct AutomaticTempSqliteFile {
    path: PathBuf,
}

impl AutomaticTempSqliteFile {
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!(
                "flowable-java-http-differential-{}.db",
                uuid::Uuid::new_v4()
            )),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AutomaticTempSqliteFile {
    fn drop(&mut self) {
        for path in [
            self.path.clone(),
            PathBuf::from(format!("{}-wal", self.path.display())),
            PathBuf::from(format!("{}-shm", self.path.display())),
            PathBuf::from(format!("{}-journal", self.path.display())),
        ] {
            let _ = fs::remove_file(path);
        }
    }
}

struct AutomaticEngineCloseGuard<'a> {
    engine: &'a ProcessEngine,
    closed: bool,
}

impl<'a> AutomaticEngineCloseGuard<'a> {
    fn new(engine: &'a ProcessEngine) -> Self {
        Self {
            engine,
            closed: false,
        }
    }

    fn close(mut self) {
        self.engine.close();
        self.closed = true;
    }
}

impl Drop for AutomaticEngineCloseGuard<'_> {
    fn drop(&mut self) {
        if !self.closed {
            self.engine.close();
        }
    }
}

struct AutomaticGatedServer {
    endpoint: String,
    request_arrived: Receiver<usize>,
    allow_response: SyncSender<()>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<Result<Vec<CapturedRequest>, String>>>,
}

impl AutomaticGatedServer {
    fn new(contract_case: ContractCase) -> Self {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("bind automatic Rust differential HTTP server");
        let address = listener
            .local_addr()
            .expect("read automatic Rust differential HTTP address");
        let path = contract_case
            .path
            .as_deref()
            .expect("automatic HTTP case requires path");
        let endpoint = format!("http://{address}{path}");
        let (arrived_tx, request_arrived) = mpsc::sync_channel(2);
        let (allow_response, allow_rx) = mpsc::sync_channel(2);
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("configure automatic listener: {error}"))?;
            let mut responses = vec![ContractResponse {
                status: contract_case
                    .response_status
                    .expect("automatic HTTP case requires responseStatus"),
                body: contract_case
                    .response_body
                    .clone()
                    .unwrap_or(Value::Null),
            }];
            responses.extend(contract_case.subsequent_responses);
            let mut captured = Vec::new();
            for (attempt, response) in responses.into_iter().enumerate() {
                let mut stream = accept_automatic_request(&listener, &server_stop)?;
                stream
                    .set_read_timeout(Some(AUTOMATIC_WALL_TIMEOUT))
                    .map_err(|error| format!("configure automatic request timeout: {error}"))?;
                captured.push(read_http_request(&mut stream));
                arrived_tx
                    .send(attempt)
                    .map_err(|error| format!("signal automatic request arrival: {error}"))?;
                wait_for_automatic_response_gate(&allow_rx, &server_stop, attempt)?;

                let response_body = serde_json::to_vec(&response.body)
                    .map_err(|error| format!("serialize automatic response body: {error}"))?;
                let reason = match response.status {
                    200 => "OK",
                    201 => "Created",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "Contract Response",
                };
                write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.status,
                    reason,
                    response_body.len()
                )
                .map_err(|error| format!("write automatic response headers: {error}"))?;
                stream
                    .write_all(&response_body)
                    .map_err(|error| format!("write automatic response body: {error}"))?;
            }
            Ok(captured)
        });
        Self {
            endpoint,
            request_arrived,
            allow_response,
            stop,
            handle: Some(handle),
        }
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn await_request(&self, expected_attempt: usize) {
        let attempt = self
            .request_arrived
            .recv_timeout(AUTOMATIC_WALL_TIMEOUT)
            .unwrap_or_else(|error| {
                panic!(
                    "timed out waiting for automatic Rust HTTP attempt {}: {error}",
                    expected_attempt + 1
                )
            });
        assert_eq!(attempt, expected_attempt);
    }

    fn allow_response(&self) {
        self.allow_response
            .send(())
            .expect("release automatic Rust HTTP response");
    }

    fn release_all_responses(&self) {
        for _ in 0..2 {
            let _ = self.allow_response.try_send(());
        }
    }

    fn finish(mut self) -> Vec<CapturedRequest> {
        self.release_all_responses();
        self.handle
            .take()
            .expect("automatic server thread handle")
            .join()
            .expect("join automatic Rust differential HTTP server")
            .unwrap_or_else(|error| panic!("automatic Rust differential HTTP server: {error}"))
    }
}

impl Drop for AutomaticGatedServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.release_all_responses();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn accept_automatic_request(
    listener: &TcpListener,
    stop: &AtomicBool,
) -> Result<TcpStream, String> {
    let deadline = Instant::now() + AUTOMATIC_WALL_TIMEOUT;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err("automatic HTTP server stopped before request arrival".to_string());
        }
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("timed out accepting automatic HTTP request".to_string());
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("accept automatic HTTP request: {error}")),
        }
    }
}

fn wait_for_automatic_response_gate(
    allow_response: &Receiver<()>,
    stop: &AtomicBool,
    attempt: usize,
) -> Result<(), String> {
    let deadline = Instant::now() + AUTOMATIC_WALL_TIMEOUT;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err(format!(
                "automatic HTTP server stopped while attempt {} was gated",
                attempt + 1
            ));
        }
        match allow_response.recv_timeout(Duration::from_millis(10)) {
            Ok(()) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(format!(
                    "timed out waiting to release automatic HTTP attempt {}",
                    attempt + 1
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("automatic HTTP response gate disconnected".to_string());
            }
        }
    }
}

fn run_rust_automatic_async_retry_contract(
    fixture_directory: &Path,
    fixture: &ContractFixture,
    contract_case: &ContractCase,
) -> Value {
    let database_file = AutomaticTempSqliteFile::new();
    let server = AutomaticGatedServer::new(contract_case.clone());
    let fixed_time = Utc
        .timestamp_millis_opt(fixture.fixed_clock_millis)
        .single()
        .expect("fixed automatic differential time");
    let time_source = Arc::new(TestTimeSource::new(fixed_time));
    let observed_events = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    event_dispatcher.add_event_listener(Arc::new(RecordingJobEventListener {
        events: Arc::clone(&observed_events),
    }));
    let lock_owner = fixture.automatic_lock_owner.clone();
    let engine = ProcessEngine::build_with_config(
        "java-http-differential-auto".to_string(),
        Arc::clone(&time_source) as Arc<dyn TimeSource>,
        ProcessEngineConfiguration {
            http_service: HttpServiceTaskConfiguration {
                enabled: true,
                runtime_mode: HttpServiceRuntimeMode::Real,
                real_client: RealHttpClientConfiguration {
                    retry_count: 0,
                    allow_private_networks: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            async_executor: AsyncExecutorConfiguration {
                auto_activate: true,
                lock_owner: Some(lock_owner.clone()),
                number_of_retries: 3,
                async_job_acquire_wait_ms: 25,
                timer_job_acquire_wait_ms: 25,
                queue_full_wait_ms: 25,
                max_jobs_per_acquisition: 1,
                async_job_lock_time_ms: 5_000,
                timer_lock_time_ms: 5_000,
                ..Default::default()
            },
            database: DatabaseConfiguration {
                kind: EngineDatabaseKind::Sqlite,
                url: database_file.path().to_string_lossy().into_owned(),
                pool_size: 8,
                busy_timeout_ms: 5_000,
                journal_mode: Default::default(),
            },
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .expect("build automatically activated Rust differential engine");
    let close_guard = AutomaticEngineCloseGuard::new(&engine);

    let bpmn_name = contract_case
        .bpmn
        .as_deref()
        .expect("automatic retry case requires bpmn");
    let bpmn = fs::read_to_string(fixture_directory.join(bpmn_name))
        .expect("read automatic retry BPMN fixture");
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(format!("contract-{}", contract_case.id))
                .add_string(bpmn_name.to_string(), bpmn),
        )
        .expect("deploy automatic retry BPMN fixture in Rust");
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("query automatic retry process definition")[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("endpoint".to_string(), json!(server.endpoint())),
        )
        .expect("start automatic retry process in Rust");

    server.await_request(0);
    let first_job = require_automatic_job(
        &engine,
        &process_instance.id,
        "first automatic acquisition",
        |job| job.lock_owner.is_some(),
    );
    let first_acquisition = normalize_automatic_acquisition(
        &first_job,
        time_source.now().timestamp_millis(),
        &lock_owner,
    );
    server.allow_response();

    let retry_timer = require_automatic_job(
        &engine,
        &process_instance.id,
        "automatic retry timer",
        |job| job.job_state.as_deref() == Some("timer") && job.lock_owner.is_none(),
    );
    let retry_observation_time = time_source.now().timestamp_millis();
    let retry_timer_node = normalize_automatic_retry_timer(&retry_timer, retry_observation_time);

    time_source.advance_time(fixture.async_retry_advance_millis);
    server.await_request(1);
    let second_job = require_automatic_job(
        &engine,
        &process_instance.id,
        "second automatic acquisition",
        |job| job.lock_owner.is_some(),
    );
    let second_acquisition = normalize_automatic_acquisition(
        &second_job,
        time_source.now().timestamp_millis(),
        &lock_owner,
    );
    server.allow_response();

    wait_for_automatic_condition("review task after automatic retry", || {
        normalize_rust_tasks(&engine, &process_instance.id) == vec!["review".to_string()]
    });
    wait_for_automatic_condition("automatic retry job consumption", || {
        normalize_automatic_final_job_state(&engine, &process_instance.id) == "consumed"
    });

    let executor_active = engine.async_executor_is_active();
    let final_job_state = normalize_automatic_final_job_state(&engine, &process_instance.id);
    let tasks = normalize_rust_tasks(&engine, &process_instance.id);
    let captured_requests = server.finish();
    close_guard.close();

    json!({
        "executorActive": executor_active,
        "requestCount": captured_requests.len(),
        "firstAcquisition": first_acquisition,
        "retryTimer": retry_timer_node,
        "secondAcquisition": second_acquisition,
        "finalJobState": final_job_state,
        "events": observed_events.lock().unwrap().clone(),
        "tasks": tasks,
    })
}

fn require_automatic_job(
    engine: &ProcessEngine,
    process_instance_id: &str,
    description: &str,
    predicate: impl Fn(&flowable_engine::persistence::runtime_store::RuntimeTimerJobState) -> bool,
) -> flowable_engine::persistence::runtime_store::RuntimeTimerJobState {
    let deadline = Instant::now() + AUTOMATIC_WALL_TIMEOUT;
    let mut last_observed = None;
    while Instant::now() < deadline {
        if let Some(job) = automatic_job(engine, process_instance_id) {
            if predicate(&job) {
                return job;
            }
            last_observed = Some(job);
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {description}; last observed job: {last_observed:?}");
}

fn automatic_job(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Option<flowable_engine::persistence::runtime_store::RuntimeTimerJobState> {
    let store = engine.get_runtime_store();
    let mut session = store
        .create_session()
        .expect("create automatic job observation session");
    let job = store
        .find_timer_job_states_by_process_instance_id(process_instance_id, &mut session)
        .into_iter()
        .next();
    session
        .rollback()
        .expect("roll back automatic job observation session");
    job
}

fn normalize_automatic_acquisition(
    job: &flowable_engine::persistence::runtime_store::RuntimeTimerJobState,
    current_time_millis: i64,
    configured_lock_owner: &str,
) -> Value {
    let lock_duration_millis = job
        .lock_expiration_time
        .map(|expiration| expiration - current_time_millis);
    json!({
        "automatic": true,
        "jobState": "executable",
        "retries": job.retries,
        "lockOwnerSet": job.lock_owner.is_some(),
        "lockOwnerMatchesConfigured": job.lock_owner.as_deref() == Some(configured_lock_owner),
        "lockExpirationSet": job.lock_expiration_time.is_some(),
        "lockDurationMillis": lock_duration_millis,
    })
}

fn normalize_automatic_retry_timer(
    job: &flowable_engine::persistence::runtime_store::RuntimeTimerJobState,
    current_time_millis: i64,
) -> Value {
    let retry_delay_millis = job.due_time.map(|due_time| due_time - current_time_millis);
    json!({
        "visible": true,
        "dueDateSet": job.due_time.is_some(),
        "dueAfterCurrentTime": job.due_time.is_some_and(|due_time| due_time > current_time_millis),
        "retryDelayMillis": retry_delay_millis,
        "retries": job.retries,
        "errorMessage": job.error_message,
        "lockOwnerSet": job.lock_owner.is_some(),
        "lockExpirationSet": job.lock_expiration_time.is_some(),
    })
}

fn normalize_automatic_final_job_state(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> &'static str {
    match automatic_job(engine, process_instance_id) {
        None => "consumed",
        Some(job) if job.job_state.as_deref() == Some("deadletter") => "deadletter",
        Some(job) if job.job_state.as_deref() == Some("timer") => "timer",
        Some(_) => "executable",
    }
}

fn wait_for_automatic_condition(description: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + AUTOMATIC_WALL_TIMEOUT;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {description}");
}

fn normalize_rust_observed_sync_case(
    engine: &ProcessEngine,
    process_instance: Result<
        flowable_engine::runtime::process_instance::ProcessInstance,
        FlowableError,
    >,
    server: thread::JoinHandle<Vec<CapturedRequest>>,
    observe_variables: &[String],
) -> Value {
    let captured_requests = server.join().expect("join observed Rust HTTP server");
    let request = captured_requests.first().map(|captured_request| {
        json!({
            "method": captured_request.method,
            "path": captured_request.path,
            "body": captured_request.body,
        })
    });
    let (variables, tasks, error) = match process_instance {
        Ok(process_instance) => (
            normalize_rust_variables(engine, &process_instance.id, observe_variables),
            normalize_rust_tasks(engine, &process_instance.id),
            Value::Null,
        ),
        Err(error) => (
            Map::new(),
            Vec::new(),
            json!(java_compatible_error_message(error)),
        ),
    };
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store
        .create_session()
        .expect("create session for observed Rust process count");
    let process_instance_count = runtime_store.snapshot_process_instances(&mut session).len();

    json!({
        "requestCount": captured_requests.len(),
        "request": request,
        "processInstanceCount": process_instance_count,
        "variables": variables,
        "tasks": tasks,
        "error": error,
    })
}

fn run_rust_cancel_contract(fixture_directory: &Path, contract_case: &ContractCase) -> Value {
    let phases = Arc::new(Mutex::new(Vec::new()));
    let mut event_dispatcher = EngineEventDispatcher::new();
    event_dispatcher.add_typed_event_listener(
        EngineEventType::JobCanceled,
        Arc::new(TransactionRecordingJobEventListener {
            label: "IMMEDIATE",
            state: None,
            fatal: false,
            phases: Arc::clone(&phases),
        }),
    );
    for (label, state) in [
        ("COMMITTING", TransactionState::Committing),
        ("COMMITTED", TransactionState::Committed),
        ("ROLLINGBACK", TransactionState::RollingBack),
        ("ROLLED_BACK", TransactionState::RolledBack),
    ] {
        event_dispatcher.add_typed_event_listener(
            EngineEventType::JobCanceled,
            Arc::new(TransactionRecordingJobEventListener {
                label,
                state: Some(state),
                fatal: false,
                phases: Arc::clone(&phases),
            }),
        );
    }
    let fatal_state = match contract_case.execution.as_deref() {
        Some("fatalCommittingCancel") => Some(("COMMITTING", TransactionState::Committing)),
        Some("fatalCommittedCancel") => Some(("COMMITTED", TransactionState::Committed)),
        _ => None,
    };
    if let Some((label, state)) = fatal_state {
        event_dispatcher.add_typed_event_listener(
            EngineEventType::JobCanceled,
            Arc::new(TransactionRecordingJobEventListener {
                label,
                state: Some(state),
                fatal: true,
                phases: Arc::clone(&phases),
            }),
        );
    }

    let engine = ProcessEngine::build_with_config(
        format!("java-http-differential-{}", contract_case.id),
        Arc::new(SystemTimeSource),
        ProcessEngineConfiguration {
            http_service: HttpServiceTaskConfiguration {
                enabled: true,
                runtime_mode: HttpServiceRuntimeMode::Real,
                real_client: RealHttpClientConfiguration {
                    retry_count: 0,
                    allow_private_networks: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            engine_event_dispatcher: event_dispatcher,
            ..Default::default()
        },
    )
    .expect("build Rust engine for cancellation differential fixture");
    let bpmn_name = contract_case
        .bpmn
        .as_deref()
        .expect("cancellation case requires bpmn");
    let bpmn = fs::read_to_string(fixture_directory.join(bpmn_name))
        .expect("read shared cancellation BPMN fixture");
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(format!("contract-{}", contract_case.id))
                .add_string(bpmn_name.to_string(), bpmn),
        )
        .expect("deploy shared cancellation BPMN fixture in Rust");
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("query cancellation process definition")[0]
        .clone();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("endpoint".to_string(), json!("http://127.0.0.1:9/unused")),
        )
        .expect("start cancellation BPMN fixture in Rust");
    let job = engine
        .get_management_service()
        .list_executable_jobs()
        .into_iter()
        .find(|job| job.process_instance_id == process_instance.id)
        .expect("cancellation fixture should create one executable job");

    let command_error = engine
        .get_management_service()
        .delete_job(&job.timer_job_id)
        .err()
        .map(java_compatible_error_message);
    let job_state = if engine
        .get_management_service()
        .find_job_by_id(&job.timer_job_id)
        .is_some()
    {
        "executable"
    } else {
        "deleted"
    };
    json!({
        "requestCount": 0,
        "phases": phases.lock().unwrap().clone(),
        "commandError": command_error,
        "jobState": job_state,
        "error": null,
    })
}

fn run_rust_async_contract(
    engine: &ProcessEngine,
    process_instance_id: &str,
    contract_case: &ContractCase,
    server: thread::JoinHandle<Vec<CapturedRequest>>,
    observed_events: Arc<Mutex<Vec<String>>>,
    observe_variables: &[String],
) -> Value {
    let management = engine.get_management_service();
    let initial_job = management
        .list_executable_jobs()
        .into_iter()
        .find(|job| job.process_instance_id == process_instance_id)
        .expect("async Rust fixture should create one executable job");
    let mut attempts = Vec::new();
    attempts.push(execute_rust_job_attempt(
        engine,
        &initial_job.timer_job_id,
        contract_case.execution.as_deref() == Some("nestedUnrecoverable"),
    ));
    if contract_case.execution.as_deref() == Some("asyncRetry") {
        management
            .move_timer_to_executable_job(&initial_job.timer_job_id)
            .expect("move failed Rust timer job back to executable state");
        attempts.push(execute_rust_job_attempt(
            engine,
            &initial_job.timer_job_id,
            false,
        ));
    }

    let captured_requests = server.join().expect("join async Rust contract HTTP server");
    json!({
        "requestCount": captured_requests.len(),
        "attempts": attempts,
        "events": observed_events.lock().unwrap().clone(),
        "variables": normalize_rust_variables(engine, process_instance_id, observe_variables),
        "tasks": normalize_rust_tasks(engine, process_instance_id),
        "error": null,
    })
}

fn execute_rust_job_attempt(
    engine: &ProcessEngine,
    job_id: &str,
    normalize_nested_unrecoverable: bool,
) -> Value {
    let execution_error = engine.get_management_service().execute_job(job_id).err();
    let execution_error_message = execution_error
        .as_ref()
        .map(|error| java_compatible_error_message(error.clone()));
    let persisted = engine.get_management_service().find_job_by_id(job_id);
    let error_details = persisted
        .as_ref()
        .and_then(|job| job.error_details.as_deref());
    let (job_state, retries, error_message, due_date_set) = match persisted.as_ref() {
        None => ("consumed", Value::Null, Value::Null, false),
        Some(job) => {
            let state = if job.job_state.as_deref() == Some("deadletter") {
                "deadletter"
            } else if job.due_time.is_some() {
                "timer"
            } else {
                "executable"
            };
            (
                state,
                json!(job.retries),
                json!(job.error_message),
                job.due_time.is_some(),
            )
        }
    };
    let mut attempt = json!({
        "result": if execution_error_message.is_some() { "failure" } else { "success" },
        "executionError": execution_error_message,
        "jobState": job_state,
        "retries": retries,
        "errorMessage": error_message,
        "dueDateSet": due_date_set,
    });
    if normalize_nested_unrecoverable {
        let execution_error = execution_error
            .as_ref()
            .expect("nested unrecoverable contract must fail job execution");
        assert!(
            matches!(
                execution_error.primary_error(),
                FlowableError::ExecutionError(_)
            ),
            "nested unrecoverable contract must preserve a generic outer execution error"
        );
        let unrecoverable_cause_message = find_unrecoverable_cause_message(execution_error)
            .expect("nested unrecoverable contract must preserve the typed cause");
        let object = attempt
            .as_object_mut()
            .expect("normalized Rust attempt must be an object");
        object.insert("executionErrorKind".to_string(), json!("generic"));
        object.insert(
            "unrecoverableCauseMessage".to_string(),
            json!(unrecoverable_cause_message),
        );
        object.insert(
            "errorDetailsOuterMessagePresent".to_string(),
            json!(
                error_details
                    .is_some_and(|details| { details.contains("response handler wrapper failed") })
            ),
        );
        object.insert(
            "errorDetailsUnrecoverableCausePresent".to_string(),
            json!(error_details.is_some_and(|details| {
                details.contains("response payload cannot be safely processed")
            })),
        );
    }
    attempt
}

fn find_unrecoverable_cause_message(error: &FlowableError) -> Option<&str> {
    let mut current: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(FlowableError::UnrecoverableJobError(message)) =
            current.downcast_ref::<FlowableError>()
        {
            return Some(message);
        }
        current = current.source()?;
    }
}

fn spawn_contract_server(
    contract_case: ContractCase,
) -> (String, thread::JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind Rust contract HTTP server");
    let address = listener
        .local_addr()
        .expect("read Rust contract server address");
    let path = contract_case
        .path
        .as_deref()
        .expect("HTTP contract case requires path");
    let endpoint = format!("http://{address}{path}");
    let server = thread::spawn(move || {
        let mut responses = vec![ContractResponse {
            status: contract_case
                .response_status
                .expect("HTTP contract case requires responseStatus"),
            body: contract_case.response_body.clone().unwrap_or(Value::Null),
        }];
        responses.extend(contract_case.subsequent_responses);
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().expect("accept Rust contract request");
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("set Rust contract request timeout");
            requests.push(read_http_request(&mut stream));
            let response_body =
                serde_json::to_vec(&response.body).expect("serialize Rust contract response body");
            let reason = match response.status {
                200 => "OK",
                201 => "Created",
                404 => "Not Found",
                500 => "Internal Server Error",
                _ => "Contract Response",
            };
            write!(
                stream,
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.status,
                reason,
                response_body.len()
            )
            .expect("write Rust contract response headers");
            stream
                .write_all(&response_body)
                .expect("write Rust contract response body");
        }
        requests
    });
    (endpoint, server)
}

fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 2048];
    let (header_end, content_length) = loop {
        let read = read_http_request_chunk(stream, &mut chunk, "request headers");
        assert!(
            read > 0,
            "HTTP client closed before sending request headers"
        );
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = find_header_end(&bytes) {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .unwrap_or(0);
            break (header_end, content_length);
        }
    };
    let body_start = header_end + 4;
    while bytes.len() < body_start + content_length {
        let read = read_http_request_chunk(stream, &mut chunk, "request body");
        assert!(
            read > 0,
            "HTTP client closed before sending the full request body"
        );
        bytes.extend_from_slice(&chunk[..read]);
    }

    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let request_line = headers.lines().next().expect("HTTP request line");
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .expect("HTTP request method")
        .to_string();
    let path = request_parts.next().expect("HTTP request path").to_string();
    let body = String::from_utf8(bytes[body_start..body_start + content_length].to_vec())
        .expect("UTF-8 HTTP request body");
    CapturedRequest { method, path, body }
}

fn read_http_request_chunk(stream: &mut TcpStream, chunk: &mut [u8], description: &str) -> usize {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match stream.read(chunk) {
            Ok(read) => return read,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("read Rust contract {description}: {error}"),
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}
