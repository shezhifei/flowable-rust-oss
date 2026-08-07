use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use reqwest::{Response, StatusCode};
use serde_json::Value;
use std::path::Path;
use tokio::net::TcpListener;

const USER_TASK_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="m31RuntimeLedgerProcess" isExecutable="true">
    <startEvent id="startEvent1" />
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="task1" />
    <userTask id="task1" name="M31 Approval Task" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#;

const TIMER_START_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="m31TimerLedgerProcess" isExecutable="true">
    <startEvent id="timerStartEvent">
      <timerEventDefinition>
        <timeDuration>PT10M</timeDuration>
      </timerEventDefinition>
    </startEvent>
    <sequenceFlow id="flow1" sourceRef="timerStartEvent" targetRef="task1" />
    <userTask id="task1" name="Timer Started Task" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#;

fn write_directory_bundle(path: &Path) {
    std::fs::write(
        path,
        r#"
[[users]]
id = "ldap-runtime-user"
first_name = "Runtime"
last_name = "User"
email = "runtime@example.test"

[[groups]]
id = "ops-observers"
name = "Ops Observers"
group_type = "security-role"

[[memberships]]
user_id = "ldap-runtime-user"
group_id = "ops-observers"
"#,
    )
    .expect("directory bundle");
}

fn write_platform_config(config_path: &Path, database_path: &Path, bundle_path: &Path) {
    let escaped_database_path = database_path.display().to_string().replace('\\', "\\\\");
    let escaped_bundle_path = bundle_path.display().to_string().replace('\\', "\\\\");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "m31-jmx-ops"
database_path = "{database_path}"

[security]
auth_mode = "disabled"

[bootstrap]
create_default_admin = false
admin_user_id = "admin"
admin_password = "ignored"

[directory]
provider = "ldap-live"
sync_on_bootstrap = false
bundle_path = "{bundle_path}"

[operations]
exposure = "jmx-bridge"
management_api_enabled = true
"#,
        database_path = escaped_database_path,
        bundle_path = escaped_bundle_path,
    );
    std::fs::write(config_path, config).expect("config");
}

fn deploy_process(
    engine: &flowable_engine::engine::process_engine::ProcessEngine,
    name: &str,
    xml: &str,
) {
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name(name.to_string())
                .add_string(name.to_string(), xml.to_string()),
        )
        .expect("deployment");
}

fn start_latest_process_instance(
    engine: &flowable_engine::engine::process_engine::ProcessEngine,
    process_definition_id: String,
) {
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .expect("process instance");
}

async fn spawn_platform_server(platform: FlowablePlatform) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("local addr");
    let base_url = format!("http://{address}");

    tokio::spawn(async move {
        run_platform_server(platform, listener)
            .await
            .expect("server should start");
    });

    base_url
}

async fn capture_json(response: Response) -> Value {
    response.json().await.expect("json body")
}

#[tokio::test]
async fn jmx_ledgers_and_topology_routes_expose_bounded_m31_operations_surface() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    let database_path = tempdir.path().join("process-engine.sqlite");
    write_directory_bundle(&bundle_path);
    write_platform_config(&config_path, &database_path, &bundle_path);

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    let engine = platform.process_engine();

    deploy_process(
        engine.as_ref(),
        "m31-runtime-ledger.bpmn20.xml",
        USER_TASK_PROCESS_BPMN,
    );
    let runtime_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .expect("process definitions")
        .into_iter()
        .last()
        .expect("runtime definition id");
    start_latest_process_instance(engine.as_ref(), runtime_definition_id);

    deploy_process(
        engine.as_ref(),
        "m31-timer-ledger.bpmn20.xml",
        TIMER_START_PROCESS_BPMN,
    );

    let runtime_service = engine.get_runtime_service();
    runtime_service
        .heartbeat_timer_node("embedded")
        .expect("timer heartbeat");
    let _fencing_token = runtime_service
        .acquire_coordinator_lease(300_000)
        .expect("coordinator lease request")
        .expect("coordinator lease token");

    let base_url = spawn_platform_server(platform).await;
    let client = reqwest::Client::new();

    let operations_support = capture_json(
        client
            .get(format!("{base_url}/management/operations/support"))
            .send()
            .await
            .expect("operations support request"),
    )
    .await;
    assert_eq!(operations_support["exposure"], "jmx-bridge");
    assert_eq!(operations_support["runtimeLedgerEnabled"], true);
    assert_eq!(operations_support["timerLedgerEnabled"], true);
    assert_eq!(operations_support["topologyLedgerEnabled"], true);

    let runtime_ledger = capture_json(
        client
            .get(format!("{base_url}/management/jmx/runtime-ledger"))
            .send()
            .await
            .expect("runtime ledger request"),
    )
    .await;
    assert_eq!(runtime_ledger["engineName"], "m31-jmx-ops");
    assert_eq!(runtime_ledger["directoryProvider"], "ldap-live");
    assert_eq!(runtime_ledger["processInstanceCount"], 1);
    assert_eq!(runtime_ledger["taskCount"], 1);
    assert_eq!(runtime_ledger["identity"]["users"], 1);
    assert_eq!(runtime_ledger["identity"]["groups"], 1);

    let timer_ledger = capture_json(
        client
            .get(format!("{base_url}/management/jmx/timer-ledger"))
            .send()
            .await
            .expect("timer ledger request"),
    )
    .await;
    assert_eq!(timer_ledger["exposure"], "jmx-bridge");
    assert_eq!(timer_ledger["managementApiEnabled"], true);
    assert!(timer_ledger["totalTimerJobCount"].as_u64().unwrap() >= 1);
    assert_eq!(timer_ledger["coordinator"]["status"], "active");
    assert_eq!(timer_ledger["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(timer_ledger["nodes"][0]["workerType"], "embedded");

    let topology = capture_json(
        client
            .get(format!("{base_url}/management/operations/topology"))
            .send()
            .await
            .expect("topology request"),
    )
    .await;
    assert_eq!(topology["engineName"], "m31-jmx-ops");
    assert_eq!(topology["directoryProvider"], "ldap-live");
    assert_eq!(topology["directoryRuntimeReads"]["user"], true);
    assert_eq!(topology["operations"]["exposure"], "jmx-bridge");
    assert_eq!(topology["coordinator"]["status"], "active");
    assert_eq!(topology["nodes"].as_array().unwrap().len(), 1);

    let deprecated_runtime_ledger = client
        .get(format!("{base_url}/service/management/jmx/runtime-ledger"))
        .send()
        .await
        .expect("deprecated runtime ledger request");
    assert_eq!(deprecated_runtime_ledger.status(), StatusCode::NOT_FOUND);

    let deprecated_timer_ledger = client
        .get(format!("{base_url}/service/management/jmx/timer-ledger"))
        .send()
        .await
        .expect("deprecated timer ledger request");
    assert_eq!(deprecated_timer_ledger.status(), StatusCode::NOT_FOUND);

    let deprecated_topology = client
        .get(format!("{base_url}/service/management/operations/topology"))
        .send()
        .await
        .expect("deprecated topology request");
    assert_eq!(deprecated_topology.status(), StatusCode::NOT_FOUND);
}
