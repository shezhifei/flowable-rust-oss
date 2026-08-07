use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use reqwest::{Response, StatusCode, header::CONTENT_TYPE};
use serde_json::{Value, json};
use std::path::Path;
use tokio::net::TcpListener;

const USER_TASK_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="m36RuntimeLedgerProcess" isExecutable="true">
    <startEvent id="startEvent1" />
    <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="task1" />
    <userTask id="task1" name="M36 Approval Task" />
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="endEvent1" />
    <endEvent id="endEvent1" />
  </process>
</definitions>"#;

const TIMER_START_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="m36TimerLedgerProcess" isExecutable="true">
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
id = "ldap-native-user"
first_name = "Native"
last_name = "Ops"
email = "native@example.test"

[[groups]]
id = "native-admins"
name = "Native Admins"
group_type = "security-role"

[[memberships]]
user_id = "ldap-native-user"
group_id = "native-admins"
"#,
    )
    .expect("directory bundle");
}

fn write_platform_config(
    config_path: &Path,
    database_path: &Path,
    bundle_path: &Path,
    auth_mode: &str,
    create_default_admin: bool,
) {
    let escaped_database_path = database_path.display().to_string().replace('\\', "\\\\");
    let escaped_bundle_path = bundle_path.display().to_string().replace('\\', "\\\\");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "m36-jmx-native"
database_path = "{database_path}"

[security]
auth_mode = "{auth_mode}"

[bootstrap]
create_default_admin = {create_default_admin}
admin_user_id = "admin"
admin_password = "platform-secret"

[directory]
provider = "ldap-live"
sync_on_bootstrap = false
bundle_path = "{bundle_path}"

[operations]
exposure = "jmx-native-compatible"
management_api_enabled = true
"#,
        database_path = escaped_database_path,
        auth_mode = auth_mode,
        create_default_admin = create_default_admin,
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

fn prepare_management_runtime(platform: &FlowablePlatform) {
    let engine = platform.process_engine();

    deploy_process(
        engine.as_ref(),
        "m36-runtime-ledger.bpmn20.xml",
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
        "m36-timer-ledger.bpmn20.xml",
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

async fn capture_json(response: Response) -> (StatusCode, Option<String>, Value) {
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let body = response.json().await.expect("json body");
    (status, content_type, body)
}

#[tokio::test]
async fn native_jmx_surface_exposes_connector_registry_and_operations_bus() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    let database_path = tempdir.path().join("process-engine.sqlite");
    write_directory_bundle(&bundle_path);
    write_platform_config(
        &config_path,
        &database_path,
        &bundle_path,
        "disabled",
        false,
    );

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    prepare_management_runtime(&platform);

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
    assert_eq!(operations_support.0, StatusCode::OK);
    assert_eq!(
        operations_support.2["exposure"],
        json!("jmx-native-compatible")
    );
    assert_eq!(
        operations_support.2["nativeCompatibleConnectorEnabled"],
        json!(true)
    );
    assert_eq!(operations_support.2["mbeanRegistryEnabled"], json!(true));
    assert_eq!(operations_support.2["operationsBusEnabled"], json!(true));
    assert_eq!(
        operations_support.2["objectFamilyBreadth"],
        json!("core-runtime-and-platform-ledgers")
    );

    let connector_descriptor = capture_json(
        client
            .get(format!("{base_url}/management/jmx/connector-descriptor"))
            .send()
            .await
            .expect("connector descriptor request"),
    )
    .await;
    assert_eq!(connector_descriptor.0, StatusCode::OK);
    assert_eq!(
        connector_descriptor.2["connectorFamily"],
        json!("native-http")
    );
    assert_eq!(connector_descriptor.2["transport"], json!("http-json"));
    assert_eq!(
        connector_descriptor.2["simulatedRmiTransport"],
        json!(false)
    );
    assert_eq!(
        connector_descriptor.2["engineName"],
        json!("m36-jmx-native")
    );

    let deprecated_connector_descriptor = client
        .get(format!(
            "{base_url}/service/management/jmx/connector-descriptor"
        ))
        .send()
        .await
        .expect("deprecated connector descriptor request");
    assert_eq!(
        deprecated_connector_descriptor.status(),
        StatusCode::NOT_FOUND
    );

    let mbean_registry = capture_json(
        client
            .get(format!("{base_url}/management/jmx/mbean-registry"))
            .send()
            .await
            .expect("mbean registry request"),
    )
    .await;
    assert_eq!(mbean_registry.0, StatusCode::OK);
    assert_eq!(mbean_registry.2["domain"], json!("org.flowable"));
    assert_eq!(mbean_registry.2["engineName"], json!("m36-jmx-native"));
    assert_eq!(mbean_registry.2["mbeanCount"], json!(5));
    assert_eq!(
        mbean_registry.2["mbeans"][0]["path"],
        json!("/management/jmx/connector-descriptor")
    );
    assert_eq!(
        mbean_registry.2["mbeans"][1]["kind"],
        json!("runtime-ledger")
    );

    let deprecated_mbean_registry = client
        .get(format!("{base_url}/service/management/jmx/mbean-registry"))
        .send()
        .await
        .expect("deprecated mbean registry request");
    assert_eq!(deprecated_mbean_registry.status(), StatusCode::NOT_FOUND);

    let operations_bus = capture_json(
        client
            .get(format!("{base_url}/management/jmx/operations-bus"))
            .send()
            .await
            .expect("operations bus request"),
    )
    .await;
    assert_eq!(operations_bus.0, StatusCode::OK);
    assert_eq!(operations_bus.2["family"], json!("bounded-native-jmx"));
    assert_eq!(
        operations_bus.2["objectFamilyBreadth"],
        json!("core-runtime-and-platform-ledgers")
    );
    assert_eq!(operations_bus.2["engineName"], json!("m36-jmx-native"));
    assert_eq!(
        operations_bus.2["connector"]["path"],
        json!("/management/jmx/connector-descriptor")
    );
    assert_eq!(
        operations_bus.2["runtimeLedger"]["processInstanceCount"],
        json!(1)
    );
    assert_eq!(operations_bus.2["runtimeLedger"]["taskCount"], json!(1));
    assert_eq!(
        operations_bus.2["timerLedger"]["coordinator"]["status"],
        json!("active")
    );
    assert_eq!(
        operations_bus.2["topology"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        operations_bus.2["topology"]["operations"]["exposure"],
        json!("jmx-native-compatible")
    );

    let deprecated_operations_bus = client
        .get(format!("{base_url}/service/management/jmx/operations-bus"))
        .send()
        .await
        .expect("deprecated operations bus request");
    assert_eq!(deprecated_operations_bus.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn native_jmx_surface_enforces_basic_auth_contract() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    let database_path = tempdir.path().join("process-engine.sqlite");
    write_directory_bundle(&bundle_path);
    write_platform_config(&config_path, &database_path, &bundle_path, "basic", true);

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    prepare_management_runtime(&platform);

    let base_url = spawn_platform_server(platform).await;
    let client = reqwest::Client::new();

    let main_unauthorized = capture_json(
        client
            .get(format!("{base_url}/management/jmx/connector-descriptor"))
            .send()
            .await
            .expect("main unauthorized connector request"),
    )
    .await;
    assert_eq!(main_unauthorized.0, StatusCode::UNAUTHORIZED);
    assert_eq!(main_unauthorized.2["code"], json!("UNAUTHORIZED"));

    let deprecated_unauthorized = client
        .get(format!(
            "{base_url}/service/management/jmx/connector-descriptor"
        ))
        .send()
        .await
        .expect("deprecated unauthorized connector request");
    assert_eq!(deprecated_unauthorized.status(), StatusCode::UNAUTHORIZED);

    let main_authorized = capture_json(
        client
            .get(format!("{base_url}/management/jmx/operations-bus"))
            .basic_auth("admin", Some("platform-secret"))
            .send()
            .await
            .expect("main authorized operations bus request"),
    )
    .await;
    assert_eq!(main_authorized.0, StatusCode::OK);
    assert_eq!(main_authorized.2["family"], json!("bounded-native-jmx"));
    assert_eq!(
        main_authorized.2["objectFamilyBreadth"],
        json!("core-runtime-and-platform-ledgers")
    );

    let deprecated_authorized = client
        .get(format!("{base_url}/service/management/jmx/operations-bus"))
        .basic_auth("admin", Some("platform-secret"))
        .send()
        .await
        .expect("deprecated authorized operations bus request");
    assert_eq!(deprecated_authorized.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deprecated_management_aliases_are_not_registered() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    let database_path = tempdir.path().join("process-engine.sqlite");
    write_directory_bundle(&bundle_path);
    write_platform_config(
        &config_path,
        &database_path,
        &bundle_path,
        "disabled",
        false,
    );

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    prepare_management_runtime(&platform);

    let base_url = spawn_platform_server(platform).await;
    let client = reqwest::Client::new();

    let removed_aliases = [
        "/service/management/directory/support",
        "/service/management/directory/reconcile",
        "/service/management/operations/support",
        "/service/management/platform/support",
        "/service/management/platform/topology-certification",
        "/service/management/jmx/runtime",
        "/service/management/jmx/connector-descriptor",
        "/service/management/jmx/mbean-registry",
        "/service/management/jmx/operations-bus",
        "/service/management/jmx/runtime-ledger",
        "/service/management/jmx/timer-ledger",
        "/service/management/operations/topology",
    ];

    for path in removed_aliases {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .expect("deprecated alias request should respond");
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{path} should not be a registered deprecated alias"
        );
    }
}
