//! Shared differential-test harness for Java vs Rust black-box contracts.
//!
//! New domain fixtures only need:
//! 1. a fixture directory with `cases.json` + BPMN files
//! 2. a thin `#[ignore]` integration test that calls [`run_differential_suite`]
//!
//! Complex HTTP/job lifecycle cases still use specialized execution modes in the
//! Java runner and domain-specific Rust tests; generic cases use the scripted
//! `operations` array (deploy / start / completeTask / trigger / signal /
//! setVariable / snapshot / httpStub).

#![allow(dead_code)]

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::error::FlowableError;
use flowable_engine::identity::entities::{Group, User};
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_FIXED_CLOCK_MILLIS: i64 = 1_700_000_000_000;
pub const DEFAULT_ASYNC_RETRY_ADVANCE_MILLIS: i64 = 10_001;
pub const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(15);
pub const DEFAULT_AUTOMATIC_LOCK_OWNER: &str = "java-http-differential";
pub const DEFAULT_UNLOCK_OWNED_JOBS_LOCK_OWNER: &str = "unlock-owned-jobs-differential";
pub const DEFAULT_SHARED_UNLOCK_OTHER_OWNER: &str = "shared-unlock-other-owner";
pub const DEFAULT_SHARED_TENANT_A: &str = "tenant-a";
pub const DEFAULT_SHARED_TENANT_B: &str = "tenant-b";
pub const DEFAULT_SHARED_TENANT_C: &str = "tenant-c";

pub const DEFAULT_OBSERVE_VARIABLES: &[&str] = &[
    "contractRequestMethod",
    "contractRequestBody",
    "contractDisallowRedirects",
    "contractResponseStatusCode",
    "contractErrorMessage",
    "responseBody",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractFixture {
    pub flowable_java_version: String,
    #[serde(default = "default_fixed_clock_millis")]
    pub fixed_clock_millis: i64,
    #[serde(default = "default_async_retry_advance_millis")]
    pub async_retry_advance_millis: i64,
    #[serde(default = "default_automatic_lock_owner")]
    pub automatic_lock_owner: String,
    #[serde(default = "default_unlock_owned_jobs_lock_owner")]
    pub unlock_owned_jobs_lock_owner: String,
    #[serde(default = "default_shared_unlock_other_owner")]
    pub shared_unlock_other_owner: String,
    #[serde(default = "default_shared_tenant_a")]
    pub shared_tenant_a: String,
    #[serde(default = "default_shared_tenant_b")]
    pub shared_tenant_b: String,
    #[serde(default = "default_shared_tenant_c")]
    pub shared_tenant_c: String,
    #[serde(default = "default_observe_variables")]
    pub observe_variables: Vec<String>,
    pub cases: Vec<ContractCase>,
}

fn default_fixed_clock_millis() -> i64 {
    DEFAULT_FIXED_CLOCK_MILLIS
}
fn default_async_retry_advance_millis() -> i64 {
    DEFAULT_ASYNC_RETRY_ADVANCE_MILLIS
}
fn default_automatic_lock_owner() -> String {
    DEFAULT_AUTOMATIC_LOCK_OWNER.to_string()
}
fn default_unlock_owned_jobs_lock_owner() -> String {
    DEFAULT_UNLOCK_OWNED_JOBS_LOCK_OWNER.to_string()
}
fn default_shared_unlock_other_owner() -> String {
    DEFAULT_SHARED_UNLOCK_OTHER_OWNER.to_string()
}
fn default_shared_tenant_a() -> String {
    DEFAULT_SHARED_TENANT_A.to_string()
}
fn default_shared_tenant_b() -> String {
    DEFAULT_SHARED_TENANT_B.to_string()
}
fn default_shared_tenant_c() -> String {
    DEFAULT_SHARED_TENANT_C.to_string()
}
fn default_observe_variables() -> Vec<String> {
    DEFAULT_OBSERVE_VARIABLES
        .iter()
        .map(|name| (*name).to_string())
        .collect()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractCase {
    pub id: String,
    #[serde(default)]
    pub bpmn: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub execution: Option<String>,
    #[serde(default)]
    pub response_status: Option<u16>,
    #[serde(default)]
    pub response_body: Option<Value>,
    #[serde(default)]
    pub subsequent_responses: Vec<ContractResponse>,
    #[serde(default)]
    pub observe_variables: Option<Vec<String>>,
    #[serde(default)]
    pub observe: Option<Vec<String>>,
    #[serde(default)]
    pub operations: Vec<ContractOperation>,
    #[serde(default)]
    pub start_variables: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ContractResponse {
    pub status: u16,
    pub body: Value,
}

/// One term inside a `queryTasks` `or` block (conditions inside the block are OR'd).
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskQueryFilterSpec {
    #[serde(default)]
    pub candidate_user: Option<String>,
    #[serde(default)]
    pub candidate_group: Option<String>,
    #[serde(default)]
    pub candidate_or_assigned: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub task_name: Option<String>,
    #[serde(default)]
    pub task_definition_key: Option<String>,
    #[serde(default)]
    pub ignore_assignee: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractOperation {
    pub op: String,
    #[serde(default)]
    pub bpmn: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub variables: Option<Map<String, Value>>,
    #[serde(default)]
    pub business_key: Option<String>,
    #[serde(default)]
    pub task_definition_key: Option<String>,
    #[serde(default)]
    pub activity_id: Option<String>,
    #[serde(default)]
    pub signal_name: Option<String>,
    #[serde(default)]
    pub message_name: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub group_name: Option<String>,
    #[serde(default)]
    pub candidate_user: Option<String>,
    #[serde(default)]
    pub candidate_group: Option<String>,
    #[serde(default)]
    pub candidate_or_assigned: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub task_name: Option<String>,
    #[serde(default)]
    pub ignore_assignee: Option<bool>,
    /// OR-block terms for `queryTasks` (AND'd with top-level filters).
    #[serde(default)]
    pub or: Vec<TaskQueryFilterSpec>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub response_status: Option<u16>,
    #[serde(default)]
    pub response_body: Option<Value>,
    #[serde(default)]
    pub subsequent_responses: Vec<ContractResponse>,
    #[serde(default)]
    pub millis: Option<i64>,
    #[serde(default)]
    pub local: Option<bool>,
    /// Optional process definition key for `start` (multi-process deployments).
    #[serde(default)]
    pub process_definition_key: Option<String>,
}

impl ContractCase {
    pub fn resolved_observe_variables(&self, fixture: &ContractFixture) -> Vec<String> {
        self.observe_variables
            .clone()
            .unwrap_or_else(|| fixture.observe_variables.clone())
    }

    pub fn resolved_observe_fields(&self) -> Vec<String> {
        self.observe.clone().unwrap_or_else(|| {
            vec![
                "tasks".to_string(),
                "variables".to_string(),
                "processEnded".to_string(),
                "error".to_string(),
            ]
        })
    }

    pub fn is_operations_case(&self) -> bool {
        !self.operations.is_empty()
    }

    pub fn requires_http_stub(&self) -> bool {
        if self.path.is_some() && self.response_status.is_some() {
            return true;
        }
        self.operations
            .iter()
            .any(|operation| operation.op == "httpStub")
    }
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("flowable-engine crate should be inside the workspace modules directory")
        .to_path_buf()
}

pub fn load_fixture(fixture_directory: &Path) -> ContractFixture {
    serde_json::from_str(
        &fs::read_to_string(fixture_directory.join("cases.json"))
            .unwrap_or_else(|error| panic!("read fixture cases.json: {error}")),
    )
    .unwrap_or_else(|error| panic!("parse fixture cases.json: {error}"))
}

/// Run the isolated Maven Java runner against a fixture directory and parse JSON output.
pub fn run_java_contract_runner(
    workspace_root: &Path,
    fixture_directory: &Path,
    output_name: &str,
) -> Value {
    let java_engine_root = std::env::var_os("FLOWABLE_JAVA_ENGINE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            workspace_root
                .parent()
                .expect("Rust workspace should have a parent directory")
                .join("flowable-engine")
        });
    let maven_wrapper = if cfg!(windows) {
        java_engine_root.join("mvnw.cmd")
    } else {
        java_engine_root.join("mvnw")
    };
    assert!(
        maven_wrapper.is_file(),
        "Flowable Java Maven wrapper not found at {}; set FLOWABLE_JAVA_ENGINE_ROOT to the Java checkout",
        maven_wrapper.display()
    );

    let output_directory = workspace_root.join("target/differential");
    fs::create_dir_all(&output_directory).expect("create differential output directory");
    let java_output_path = output_directory.join(output_name);
    let runner_pom = workspace_root.join("differential/java-http-runner/pom.xml");
    let exec_args = format!(
        "{} {}",
        fixture_directory.display(),
        java_output_path.display()
    );

    let status = Command::new(&maven_wrapper)
        .arg("-q")
        .arg("-f")
        .arg(&runner_pom)
        .arg("-DskipTests")
        .arg("compile")
        .arg("exec:java")
        .arg(format!("-Dexec.args={exec_args}"))
        .status()
        .expect("start Flowable Java contract runner");
    assert!(
        status.success(),
        "Flowable Java contract runner failed for {}",
        fixture_directory.display()
    );

    serde_json::from_str(
        &fs::read_to_string(&java_output_path).expect("read Java normalized output"),
    )
    .expect("parse Java normalized output")
}

/// Compare Java and Rust normalized case maps for a fixture directory.
///
/// `run_rust_case` implements domain-specific execution (HTTP special modes,
/// operations scripts, etc.). Generic fixtures can pass
/// [`run_rust_operations_case`] as the runner.
pub fn run_differential_suite<F>(
    fixture_relative_directory: &str,
    output_stem: &str,
    mut run_rust_case: F,
) where
    F: FnMut(&Path, &ContractFixture, &ContractCase) -> Value,
{
    let workspace = workspace_root();
    let fixture_directory = workspace.join(fixture_relative_directory);
    let fixture = load_fixture(&fixture_directory);

    let java_output =
        run_java_contract_runner(&workspace, &fixture_directory, &format!("java-{output_stem}.json"));
    assert_eq!(
        java_output["flowableVersion"],
        json!(fixture.flowable_java_version),
        "the Java runner version must remain explicitly pinned by the shared fixture"
    );

    let mut cases = Map::new();
    for contract_case in &fixture.cases {
        cases.insert(
            contract_case.id.clone(),
            run_rust_case(&fixture_directory, &fixture, contract_case),
        );
    }
    let rust_output = json!({
        "engine": "flowable-rust",
        "flowableVersion": fixture.flowable_java_version,
        "cases": cases,
    });

    let output_directory = workspace.join("target/differential");
    fs::create_dir_all(&output_directory).expect("create differential output directory");
    fs::write(
        output_directory.join(format!("rust-{output_stem}.json")),
        serde_json::to_vec_pretty(&rust_output).expect("serialize Rust normalized output"),
    )
    .expect("write Rust normalized output");

    assert_eq!(
        rust_output["cases"], java_output["cases"],
        "Flowable Rust behavior diverged from the normalized Flowable Java contract ({fixture_relative_directory})"
    );
}

pub fn wait_for_condition(description: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {description}");
}

pub fn java_compatible_error_message(error: FlowableError) -> String {
    let raw = match error {
        FlowableError::ExecutionError(message)
        | FlowableError::UnrecoverableJobError(message)
        | FlowableError::DeploymentValidationError(message)
        | FlowableError::BadRequest(message)
        | FlowableError::Forbidden(message)
        | FlowableError::Conflict(message)
        | FlowableError::NotFound(message)
        | FlowableError::Internal(message)
        | FlowableError::Generic(message) => message,
        FlowableError::InvalidBpmnXml { message, .. } => message,
        FlowableError::UnsupportedElement { activity_id, .. } => activity_id,
        FlowableError::Caused(chain) => java_compatible_error_message(chain.outer().clone()),
    };
    normalize_contract_error_message(&raw)
}

/// Strip engine-specific execution/definition IDs from well-known contract errors.
pub fn normalize_contract_error_message(error: &str) -> String {
    let marker = "could be selected";
    if error.contains("No outgoing sequence flow of the exclusive gateway")
        && error.contains(marker)
    {
        if let Some(end) = error.find(marker) {
            return error[..end + marker.len()].to_string();
        }
    }
    let timer_marker =
        "Timer needs configuration (either timeDate, timeCycle or timeDuration is needed)";
    if error.contains(timer_marker) {
        return timer_marker.to_string();
    }
    error.to_string()
}

pub fn normalize_rust_tasks(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    let mut tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .expect("query Rust active tasks")
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    tasks.sort();
    tasks
}

pub fn normalize_rust_variables(
    engine: &ProcessEngine,
    process_instance_id: &str,
    observe_variables: &[String],
) -> Map<String, Value> {
    let runtime = engine.get_runtime_service();
    let mut variables = Map::new();
    for name in observe_variables {
        if let Some(value) = runtime
            .get_variable(process_instance_id.to_string(), name.clone())
            .expect("read normalized Rust process variable")
        {
            variables.insert(name.clone(), value);
        }
    }
    variables
}

pub fn normalize_rust_job_counts(engine: &ProcessEngine, process_instance_id: &str) -> Value {
    let management = engine.get_management_service();
    let executable = management
        .list_executable_jobs()
        .into_iter()
        .filter(|job| job.process_instance_id == process_instance_id)
        .count();
    let deadletter = management
        .list_deadletter_jobs()
        .into_iter()
        .filter(|job| job.process_instance_id == process_instance_id)
        .count();
    // Timer jobs that are not yet executable surface via the runtime store.
    let store = engine.get_runtime_store();
    let mut session = store
        .create_session()
        .expect("create session for timer job count");
    let timer = store
        .find_timer_job_states_by_process_instance_id(process_instance_id, &mut session)
        .into_iter()
        .filter(|job| job.job_state.as_deref() == Some("timer"))
        .count();
    session
        .rollback()
        .expect("roll back timer job count session");
    json!({
        "executable": executable,
        "timer": timer,
        "deadletter": deadletter,
    })
}

pub fn process_instance_is_active(engine: &ProcessEngine, process_instance_id: &str) -> bool {
    let store = engine.get_runtime_store();
    let mut session = store
        .create_session()
        .expect("create session for process instance check");
    let active = store
        .find_process_instance(process_instance_id, &mut session)
        .is_some();
    session
        .rollback()
        .expect("roll back process instance check session");
    active
}

fn map_to_hashmap(variables: &Map<String, Value>) -> HashMap<String, Value> {
    variables
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Run a generic operations-scripted contract case on Rust.
pub fn run_rust_operations_case(
    fixture_directory: &Path,
    fixture: &ContractFixture,
    contract_case: &ContractCase,
) -> Value {
    assert!(
        contract_case.is_operations_case(),
        "operations case '{}' has empty operations array",
        contract_case.id
    );

    // Fixed logical clock so advanceClock / timer due dates align with Java.
    let clock = Arc::new(TestTimeSource::new(
        chrono::DateTime::from_timestamp_millis(fixture.fixed_clock_millis)
            .expect("fixture fixedClockMillis must be a valid unix epoch millis"),
    ));
    let engine = ProcessEngine::build_with_config(
        format!("differential-ops-{}", contract_case.id),
        clock.clone(),
        ProcessEngineConfiguration::default(),
    )
    .expect("build Rust engine for operations differential fixture");

    let mut process_definition_id: Option<String> = None;
    let mut process_instance_id: Option<String> = None;
    let mut error: Option<String> = None;
    let mut task_query_result: Vec<String> = Vec::new();
    let observe_variables = contract_case.resolved_observe_variables(fixture);
    let observe_fields = contract_case.resolved_observe_fields();

    // Optional top-level httpStub is handled inside operations only for ops cases.
    for operation in &contract_case.operations {
        match operation.op.as_str() {
            "httpStub" => {
                // HTTP stubs for operations-based cases are only needed for real HTTP
                // service tasks. Domain fixtures that do not need HTTP skip this op.
                // Starting a server without consuming it would hang join() later, so
                // we deliberately ignore httpStub unless a later domain wires a server
                // helper. Java side mirrors this: httpStub alone does not block.
            }
            "createUser" => {
                let user_id = operation
                    .user_id
                    .as_deref()
                    .expect("createUser requires userId");
                engine.get_identity_service().save_user(User {
                    id: user_id.to_string(),
                    first_name: None,
                    last_name: None,
                    email: None,
                    password: None,
                    tenant_id: None,
                });
            }
            "createGroup" => {
                let group_id = operation
                    .group_id
                    .as_deref()
                    .expect("createGroup requires groupId");
                let name = operation
                    .group_name
                    .clone()
                    .unwrap_or_else(|| group_id.to_string());
                engine.get_identity_service().save_group(Group {
                    id: group_id.to_string(),
                    name,
                    group_type: None,
                });
            }
            "createMembership" => {
                let user_id = operation
                    .user_id
                    .as_deref()
                    .expect("createMembership requires userId");
                let group_id = operation
                    .group_id
                    .as_deref()
                    .expect("createMembership requires groupId");
                engine
                    .get_identity_service()
                    .create_membership(user_id.to_string(), group_id.to_string());
            }
            "queryTasks" => {
                task_query_result =
                    run_rust_task_query(&engine, process_instance_id.as_deref(), operation);
            }
            "advanceClock" => {
                let millis = operation
                    .millis
                    .expect("advanceClock requires millis");
                clock.advance_time(millis);
            }
            "executeDueTimers" => {
                // Fire every timer that is due under the logical clock.
                let _ = engine.run_due_timers();
            }
            "deploy" => {
                let bpmn_name = operation
                    .bpmn
                    .as_deref()
                    .or(contract_case.bpmn.as_deref())
                    .unwrap_or_else(|| {
                        panic!("deploy op in case '{}' requires bpmn", contract_case.id)
                    });
                let bpmn = fs::read_to_string(fixture_directory.join(bpmn_name))
                    .unwrap_or_else(|err| panic!("read BPMN {bpmn_name}: {err}"));
                let mut deployment = engine
                    .get_repository_service()
                    .create_deployment()
                    .name(format!("contract-{}", contract_case.id))
                    .add_string(bpmn_name.to_string(), bpmn);
                if let Some(tenant_id) = &operation.tenant_id {
                    deployment = deployment.tenant_id(tenant_id.clone());
                }
                match engine.get_repository_service().deploy(deployment) {
                    Ok(_) => {
                        process_definition_id = Some(
                            engine
                                .get_repository_service()
                                .get_process_definition_ids()
                                .expect("query deployed process definition")
                                .last()
                                .cloned()
                                .expect("deployed process definition id"),
                        );
                    }
                    Err(err) => error = Some(java_compatible_error_message(err)),
                }
            }
            "start" => {
                let mut builder = engine.get_runtime_service().create_process_instance_builder();
                if let Some(key) = &operation.process_definition_key {
                    builder = builder.process_definition_key(key.clone());
                } else {
                    let definition_id = process_definition_id
                        .clone()
                        .expect("start op requires a prior deploy or processDefinitionKey");
                    builder = builder.process_definition_id(definition_id);
                }
                if let Some(variables) = &operation.variables {
                    for (name, value) in variables {
                        builder = builder.variable(name.clone(), value.clone());
                    }
                }
                if let Some(business_key) = &operation.business_key {
                    builder = builder.business_key(business_key.clone());
                }
                if let Some(tenant_id) = &operation.tenant_id {
                    builder = builder.tenant_id(tenant_id.clone());
                }
                match engine.get_runtime_service().start_process_instance(builder) {
                    Ok(instance) => process_instance_id = Some(instance.id),
                    Err(err) => error = Some(java_compatible_error_message(err)),
                }
            }
            "completeTask" => {
                let pi = process_instance_id
                    .as_deref()
                    .expect("completeTask requires a started process");
                let key = operation
                    .task_definition_key
                    .as_deref()
                    .expect("completeTask requires taskDefinitionKey");
                let task = engine
                    .get_task_service()
                    .get_tasks_by_process_instance_id(pi.to_string())
                    .expect("list tasks")
                    .into_iter()
                    .find(|task| task.task_definition_key == key)
                    .unwrap_or_else(|| panic!("no active task with key {key}"));
                if let Some(variables) = &operation.variables {
                    engine
                        .get_task_service()
                        .complete_task_by_id_with_variables(
                            task.id,
                            map_to_hashmap(variables),
                        )
                        .expect("complete task with variables");
                } else {
                    engine
                        .get_task_service()
                        .complete_task_by_id(task.id)
                        .expect("complete task");
                }
            }
            "setVariable" => {
                let pi = process_instance_id
                    .as_deref()
                    .expect("setVariable requires a started process");
                let name = operation
                    .name
                    .as_deref()
                    .expect("setVariable requires name");
                let value = operation
                    .value
                    .clone()
                    .expect("setVariable requires value");
                if operation.local.unwrap_or(false) {
                    engine
                        .get_runtime_service()
                        .set_variable_local(pi.to_string(), name.to_string(), value)
                        .expect("setVariableLocal on process instance");
                } else {
                    engine
                        .get_runtime_service()
                        .set_variable(pi.to_string(), name.to_string(), value)
                        .expect("setVariable on process instance");
                }
            }
            "setVariableLocal" => {
                let pi = process_instance_id
                    .as_deref()
                    .expect("setVariableLocal requires a started process");
                let name = operation
                    .name
                    .as_deref()
                    .expect("setVariableLocal requires name");
                let value = operation
                    .value
                    .clone()
                    .expect("setVariableLocal requires value");
                if let Some(task_key) = &operation.task_definition_key {
                    let task = engine
                        .get_task_service()
                        .get_tasks_by_process_instance_id(pi.to_string())
                        .expect("list tasks")
                        .into_iter()
                        .find(|task| task.task_definition_key == *task_key)
                        .unwrap_or_else(|| panic!("no active task with key {task_key}"));
                    engine
                        .get_task_service()
                        .set_task_local_variable(task.id, name.to_string(), value)
                        .expect("set task-local variable");
                } else {
                    engine
                        .get_runtime_service()
                        .set_variable_local(pi.to_string(), name.to_string(), value)
                        .expect("setVariableLocal on process instance");
                }
            }
            "trigger" => {
                let pi = process_instance_id
                    .as_deref()
                    .expect("trigger requires a started process");
                // activityId is accepted for documentation; Rust triggers the
                // waiting intermediate catch for the process instance.
                let _activity_id = operation.activity_id.as_deref();
                engine.trigger_intermediate_catch_event_by_process_instance_id(pi.to_string());
            }
            "signalEvent" => {
                let signal_name = operation
                    .signal_name
                    .as_deref()
                    .expect("signalEvent requires signalName");
                let pi = process_instance_id
                    .as_deref()
                    .expect("signalEvent requires a started process");
                engine.trigger_boundary_event_by_signal_ref(
                    signal_name.to_string(),
                    pi.to_string(),
                );
            }
            "messageEvent" => {
                let message_name = operation
                    .message_name
                    .as_deref()
                    .expect("messageEvent requires messageName");
                let pi = process_instance_id
                    .as_deref()
                    .expect("messageEvent requires a started process");
                engine.trigger_boundary_event_by_message_ref(
                    message_name.to_string(),
                    pi.to_string(),
                );
            }
            "triggerBoundary" => {
                let activity_id = operation
                    .activity_id
                    .as_deref()
                    .expect("triggerBoundary requires activityId");
                let pi = process_instance_id
                    .as_deref()
                    .expect("triggerBoundary requires a started process");
                engine.trigger_boundary_event(activity_id.to_string(), pi.to_string());
            }
            "claimTask" => {
                let pi = process_instance_id
                    .as_deref()
                    .expect("claimTask requires a started process");
                let key = operation
                    .task_definition_key
                    .as_deref()
                    .expect("claimTask requires taskDefinitionKey");
                let user_id = operation
                    .user_id
                    .as_deref()
                    .expect("claimTask requires userId");
                let task = engine
                    .get_task_service()
                    .get_tasks_by_process_instance_id(pi.to_string())
                    .expect("list tasks")
                    .into_iter()
                    .find(|task| task.task_definition_key == key)
                    .unwrap_or_else(|| panic!("no active task with key {key}"));
                engine
                    .get_task_service()
                    .claim_task_by_id(task.id, user_id.to_string())
                    .expect("claim task");
            }
            "delegateTask" => {
                let pi = process_instance_id
                    .as_deref()
                    .expect("delegateTask requires a started process");
                let key = operation
                    .task_definition_key
                    .as_deref()
                    .expect("delegateTask requires taskDefinitionKey");
                let user_id = operation
                    .user_id
                    .as_deref()
                    .expect("delegateTask requires userId");
                let task = engine
                    .get_task_service()
                    .get_tasks_by_process_instance_id(pi.to_string())
                    .expect("list tasks")
                    .into_iter()
                    .find(|task| task.task_definition_key == key)
                    .unwrap_or_else(|| panic!("no active task with key {key}"));
                engine
                    .get_task_service()
                    .delegate_task_by_id(task.id, user_id.to_string())
                    .expect("delegate task");
            }
            "resolveTask" => {
                let pi = process_instance_id
                    .as_deref()
                    .expect("resolveTask requires a started process");
                let key = operation
                    .task_definition_key
                    .as_deref()
                    .expect("resolveTask requires taskDefinitionKey");
                let task = engine
                    .get_task_service()
                    .get_tasks_by_process_instance_id(pi.to_string())
                    .expect("list tasks")
                    .into_iter()
                    .find(|task| task.task_definition_key == key)
                    .unwrap_or_else(|| panic!("no active task with key {key}"));
                engine
                    .get_task_service()
                    .resolve_task_by_id(task.id)
                    .expect("resolve task");
            }
            "executeJobs" => {
                let pi = process_instance_id
                    .as_deref()
                    .expect("executeJobs requires a started process");
                let jobs: Vec<_> = engine
                    .get_management_service()
                    .list_executable_jobs()
                    .into_iter()
                    .filter(|job| job.process_instance_id == pi)
                    .map(|job| job.timer_job_id)
                    .collect();
                for job_id in jobs {
                    let _ = engine.get_management_service().execute_job(&job_id);
                }
            }
            "snapshot" => {
                // Final snapshot is built after the loop.
            }
            other => panic!(
                "unsupported differential operation '{other}' in case '{}'",
                contract_case.id
            ),
        }
    }

    build_operations_snapshot(
        &engine,
        process_instance_id.as_deref(),
        process_definition_id.as_deref(),
        error.as_deref(),
        &observe_variables,
        &observe_fields,
        &task_query_result,
    )
}

fn run_rust_task_query(
    engine: &ProcessEngine,
    process_instance_id: Option<&str>,
    operation: &ContractOperation,
) -> Vec<String> {
    let mut query = engine.get_task_service().create_task_query();
    if let Some(pi) = process_instance_id {
        query = query.process_instance_id(pi.to_string());
    }
    query = apply_rust_task_filters(query, operation);
    if !operation.or.is_empty() {
        query = query.or();
        for term in &operation.or {
            query = apply_rust_task_filter_spec(query, term);
        }
        query = query.end_or();
    }
    let mut keys: Vec<String> = query
        .list()
        .expect("queryTasks list")
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect();
    keys.sort();
    keys
}

fn apply_rust_task_filters(
    mut query: flowable_engine::engine::task_service::TaskQuery,
    operation: &ContractOperation,
) -> flowable_engine::engine::task_service::TaskQuery {
    if let Some(user) = &operation.candidate_user {
        query = query.task_candidate_user(user.clone());
    }
    if let Some(group) = &operation.candidate_group {
        query = query.task_candidate_group(group.clone());
    }
    if let Some(user) = &operation.candidate_or_assigned {
        // Java taskCandidateOrAssigned: assignee match OR candidate match.
        // Emulate via or-block (Rust engine TaskQuery has no dedicated setter).
        // Do not combine with a top-level `or` array — nested or() is rejected.
        query = query
            .or()
            .task_assignee(user.clone())
            .task_candidate_user(user.clone());
        if operation.ignore_assignee.unwrap_or(false) {
            query = query.ignore_assignee_value();
        }
        query = query.end_or();
        return query;
    }
    if let Some(assignee) = &operation.assignee {
        query = query.task_assignee(assignee.clone());
    }
    if let Some(name) = &operation.task_name {
        query = query.task_name(name.clone());
    }
    if let Some(key) = &operation.task_definition_key {
        query = query.task_definition_key(key.clone());
    }
    if operation.ignore_assignee.unwrap_or(false) {
        query = query.ignore_assignee_value();
    }
    query
}

fn apply_rust_task_filter_spec(
    mut query: flowable_engine::engine::task_service::TaskQuery,
    spec: &TaskQueryFilterSpec,
) -> flowable_engine::engine::task_service::TaskQuery {
    if let Some(user) = &spec.candidate_user {
        query = query.task_candidate_user(user.clone());
    }
    if let Some(group) = &spec.candidate_group {
        query = query.task_candidate_group(group.clone());
    }
    if let Some(user) = &spec.candidate_or_assigned {
        // Inside an or-block we expand to assignee | candidate terms.
        query = query
            .task_assignee(user.clone())
            .task_candidate_user(user.clone());
    }
    if let Some(assignee) = &spec.assignee {
        query = query.task_assignee(assignee.clone());
    }
    if let Some(name) = &spec.task_name {
        query = query.task_name(name.clone());
    }
    if let Some(key) = &spec.task_definition_key {
        query = query.task_definition_key(key.clone());
    }
    if spec.ignore_assignee.unwrap_or(false) {
        query = query.ignore_assignee_value();
    }
    query
}

fn build_operations_snapshot(
    engine: &ProcessEngine,
    process_instance_id: Option<&str>,
    _process_definition_id: Option<&str>,
    error: Option<&str>,
    observe_variables: &[String],
    observe_fields: &[String],
    task_query_result: &[String],
) -> Value {
    let mut normalized = Map::new();
    let process_ended = match process_instance_id {
        Some(pi) => !process_instance_is_active(engine, pi),
        None => true,
    };

    for field in observe_fields {
        match field.as_str() {
            "tasks" => {
                let tasks = match process_instance_id {
                    Some(pi) if !process_ended => normalize_rust_tasks(engine, pi),
                    _ => Vec::new(),
                };
                normalized.insert("tasks".to_string(), json!(tasks));
            }
            "variables" => {
                let variables = match process_instance_id {
                    Some(pi) if !process_ended => {
                        normalize_rust_variables(engine, pi, observe_variables)
                    }
                    _ => Map::new(),
                };
                normalized.insert("variables".to_string(), json!(variables));
            }
            "processEnded" => {
                normalized.insert("processEnded".to_string(), json!(process_ended));
            }
            "error" => {
                normalized.insert(
                    "error".to_string(),
                    match error {
                        Some(message) => json!(message),
                        None => Value::Null,
                    },
                );
            }
            "processInstanceCount" => {
                let store = engine.get_runtime_store();
                let mut session = store
                    .create_session()
                    .expect("create session for process instance count");
                let count = store.snapshot_process_instances(&mut session).len();
                session
                    .rollback()
                    .expect("roll back process instance count session");
                normalized.insert("processInstanceCount".to_string(), json!(count));
            }
            "jobs" => {
                let jobs = match process_instance_id {
                    Some(pi) => normalize_rust_job_counts(engine, pi),
                    None => json!({"executable": 0, "timer": 0, "deadletter": 0}),
                };
                normalized.insert("jobs".to_string(), jobs);
            }
            "taskLocalVariables" => {
                let locals = match process_instance_id {
                    Some(pi) if !process_ended => normalize_rust_task_local_variables(engine, pi),
                    _ => Map::new(),
                };
                normalized.insert("taskLocalVariables".to_string(), json!(locals));
            }
            "eventSubprocessTimers" => {
                // Rust stores event-subprocess start timers as dedicated subscriptions
                // (not ordinary timer jobs). Count those for parity with Java's
                // timer-job materialization of the same concept.
                let count = match process_instance_id {
                    Some(pi) => {
                        let store = engine.get_runtime_store();
                        let mut session = store
                            .create_session()
                            .expect("session for event-subprocess timer count");
                        let count = store
                            .find_event_subprocess_timer_subscriptions_by_process_instance_id(
                                pi, &mut session,
                            )
                            .len();
                        session.rollback().expect("rollback esp timer session");
                        count
                    }
                    None => 0,
                };
                normalized.insert("eventSubprocessTimers".to_string(), json!(count));
            }
            "taskDetails" => {
                let details = match process_instance_id {
                    Some(pi) if !process_ended => normalize_rust_task_details(engine, pi),
                    _ => Vec::new(),
                };
                normalized.insert("taskDetails".to_string(), json!(details));
            }
            "taskQuery" => {
                normalized.insert("taskQuery".to_string(), json!(task_query_result));
            }
            "jobHandlerTypes" => {
                let mut types = match process_instance_id {
                    Some(pi) => engine
                        .get_management_service()
                        .list_executable_jobs()
                        .into_iter()
                        .filter(|job| job.process_instance_id == pi)
                        .filter_map(|job| job.handler_type)
                        .collect::<Vec<_>>(),
                    None => Vec::new(),
                };
                types.sort();
                normalized.insert("jobHandlerTypes".to_string(), json!(types));
            }
            "timerCalendars" => {
                let mut names = match process_instance_id {
                    Some(pi) => {
                        let store = engine.get_runtime_store();
                        let mut session = store
                            .create_session()
                            .expect("session for timer calendar names");
                        let names = store
                            .find_timer_job_states_by_process_instance_id(pi, &mut session)
                            .into_iter()
                            .filter(|job| job.job_state.as_deref() == Some("timer"))
                            .map(|job| {
                                job.calendar_name
                                    .unwrap_or_else(|| "".to_string())
                            })
                            .collect::<Vec<_>>();
                        session.rollback().expect("rollback timer calendar session");
                        names
                    }
                    None => Vec::new(),
                };
                names.sort();
                normalized.insert("timerCalendars".to_string(), json!(names));
            }
            other => panic!("unknown observe field '{other}'"),
        }
    }
    Value::Object(normalized)
}

fn normalize_rust_task_details(engine: &ProcessEngine, process_instance_id: &str) -> Vec<Value> {
    let mut tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .expect("list tasks for taskDetails");
    tasks.sort_by(|left, right| {
        left.task_definition_key
            .cmp(&right.task_definition_key)
            .then_with(|| {
                left.assignee
                    .as_deref()
                    .unwrap_or("")
                    .cmp(right.assignee.as_deref().unwrap_or(""))
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    tasks
        .into_iter()
        .map(|task| {
            json!({
                "taskDefinitionKey": task.task_definition_key,
                "assignee": task.assignee,
                "owner": task.owner,
                "delegationState": task.delegation_state.as_deref().map(|state| state.to_ascii_lowercase()),
            })
        })
        .collect()
}

fn normalize_rust_task_local_variables(
    engine: &ProcessEngine,
    process_instance_id: &str,
) -> Map<String, Value> {
    let mut tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .expect("list tasks for local variable snapshot")
        .into_iter()
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| left.task_definition_key.cmp(&right.task_definition_key));
    let mut by_task = Map::new();
    for task in tasks {
        let locals = engine
            .get_task_service()
            .get_task_local_variables(task.id.clone())
            .expect("read task local variables");
        let mut names: Vec<_> = locals.keys().cloned().collect();
        names.sort();
        let mut vars = Map::new();
        for name in names {
            if let Some(value) = locals.get(&name) {
                vars.insert(name, value.clone());
            }
        }
        by_task.insert(task.task_definition_key, Value::Object(vars));
    }
    by_task
}
