use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tokio::{net::TcpListener, task::JoinHandle};

const ADMIN_USER_ID: &str = "admin";
const ADMIN_PASSWORD: &str = "m33-topology-secret";

fn normalize_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn mysql_raw_dump() -> String {
    let bpmn = r#"<?xml version="1.0" encoding="UTF-8"?><definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples"><process id="historicalImportedProcess" isExecutable="true"><startEvent id="start"/><sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask"/><userTask id="approveTask" name="Approve Imported Request"/><sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end"/><endEvent id="end"/></process></definitions>"#;
    format!(
        r#"
INSERT INTO `ACT_RE_DEPLOYMENT` (`ID_`, `NAME_`, `KEY_`, `ENGINE_VERSION_`, `DEPLOY_TIME_`) VALUES ('deployment-1', 'Imported Deployment', 'deployment-key', '8.0.0', 1710000000000);
INSERT INTO `ACT_GE_BYTEARRAY` (`ID_`, `NAME_`, `DEPLOYMENT_ID_`, `BYTES_`) VALUES ('resource-1', 'historicalImportedProcess.bpmn', 'deployment-1', '{bpmn}');
INSERT INTO `ACT_RE_PROCDEF` (`ID_`, `NAME_`, `KEY_`, `DESCRIPTION_`, `VERSION_`, `RESOURCE_NAME_`, `DEPLOYMENT_ID_`, `HAS_START_FORM_KEY_`, `HAS_GRAPHICAL_NOTATION_`, `SUSPENSION_STATE_`, `ENGINE_VERSION_`) VALUES ('historicalImportedProcess:1:source', 'Historical Imported Process', 'historicalImportedProcess', 'imported from raw dump', 1, 'historicalImportedProcess.bpmn', 'deployment-1', 0, 0, 1, '8.0.0');
INSERT INTO `ACT_RU_EXECUTION` (`ID_`, `ROOT_PROC_INST_ID_`, `PROC_INST_ID_`, `PROC_DEF_ID_`, `ACT_ID_`, `IS_ACTIVE_`, `IS_CONCURRENT_`, `IS_SCOPE_`, `IS_MI_ROOT_`, `SUSPENSION_STATE_`, `NAME_`, `BUSINESS_KEY_`, `START_USER_ID_`, `START_TIME_`) VALUES ('proc-inst-1', 'proc-inst-1', 'proc-inst-1', 'historicalImportedProcess:1:source', 'approveTask', 1, 0, 1, 0, 1, 'Imported Root Execution', 'M33-001', 'kermit', 1710000100000);
INSERT INTO `ACT_RU_TASK` (`ID_`, `PROC_INST_ID_`, `EXECUTION_ID_`, `TASK_DEF_KEY_`, `NAME_`, `CREATE_TIME_`) VALUES ('task-1', 'proc-inst-1', 'proc-inst-1', 'approveTask', 'Approve Imported Request', 1710000100500);
"#,
        bpmn = bpmn
    )
}

fn write_directory_bundle(path: &Path) {
    std::fs::write(
        path,
        r#"
[[users]]
id = "ldap-topology-admin"
first_name = "Topology"
last_name = "Admin"
email = "topology-admin@example.test"

[[groups]]
id = "topology-admins"
name = "Topology Admins"
group_type = "security-role"

[[memberships]]
user_id = "ldap-topology-admin"
group_id = "topology-admins"
"#,
    )
    .expect("directory bundle should be written");
}

struct PlatformConfigSpec<'a> {
    auth_mode: &'a str,
    embedding_mode: &'a str,
    embedding_profile: &'a str,
    adapters: &'a [&'a str],
    directory_provider: &'a str,
    topology_profile: &'a str,
    topology_ingress: Option<&'a str>,
    topology_packaging: Option<&'a str>,
}

fn write_platform_config(
    config_path: &Path,
    process_database_path: &Path,
    directory_bundle_path: &Path,
    spec: PlatformConfigSpec<'_>,
) {
    let adapter_entries = spec
        .adapters
        .iter()
        .map(|adapter| format!("\"{adapter}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "m33-topology"
database_path = "{database_path}"

[security]
auth_mode = "{auth_mode}"

[embedding]
mode = "{embedding_mode}"
profile = "{embedding_profile}"

[enterprise]
adapters = [{adapter_entries}]

[directory]
provider = "{directory_provider}"
sync_on_bootstrap = false
bundle_path = "{bundle_path}"

[operations]
exposure = "jmx-bridge"
management_api_enabled = true

[topology]
profile = "{topology_profile}"
{topology_descriptor}

[bootstrap]
create_default_admin = true
admin_user_id = "{admin_user_id}"
admin_password = "{admin_password}"
"#,
        database_path = normalize_path(process_database_path),
        auth_mode = spec.auth_mode,
        embedding_mode = spec.embedding_mode,
        embedding_profile = spec.embedding_profile,
        adapter_entries = adapter_entries,
        directory_provider = spec.directory_provider,
        bundle_path = normalize_path(directory_bundle_path),
        topology_profile = spec.topology_profile,
        topology_descriptor = match (spec.topology_ingress, spec.topology_packaging) {
            (Some(ingress), Some(packaging)) => {
                format!("ingress = \"{ingress}\"\npackaging = \"{packaging}\"")
            }
            (Some(ingress), None) => format!("ingress = \"{ingress}\""),
            (None, Some(packaging)) => format!("packaging = \"{packaging}\""),
            (None, None) => String::new(),
        },
        admin_user_id = ADMIN_USER_ID,
        admin_password = ADMIN_PASSWORD,
    );
    std::fs::write(config_path, config).expect("platform config should be written");
}

async fn spawn_platform_server(
    config_path: &Path,
) -> (
    Arc<flowable_engine::engine::process_engine::ProcessEngine>,
    String,
    Client,
    JoinHandle<()>,
) {
    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path.to_path_buf()))
        .expect("platform should bootstrap from config");
    let engine = platform.process_engine();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let base_url = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("listener should expose local address")
    );
    let handle = tokio::spawn(async move {
        run_platform_server(platform, listener)
            .await
            .expect("platform server should start");
    });
    (engine, base_url, Client::new(), handle)
}

async fn abort_server(handle: JoinHandle<()>) {
    handle.abort();
    let join_error = handle
        .await
        .expect_err("aborted server should not complete normally");
    assert!(join_error.is_cancelled(), "server task should be cancelled");
}

#[tokio::test]
async fn replacement_contract_external_topology_cert_test_covers_named_topologies() {
    let tempdir = tempfile::tempdir().expect("tempdir should be created");

    let reverse_proxy_bundle = tempdir.path().join("reverse-proxy-directory.toml");
    write_directory_bundle(&reverse_proxy_bundle);
    let reverse_proxy_dump = tempdir.path().join("historical-source-mysql.sql");
    std::fs::write(&reverse_proxy_dump, mysql_raw_dump()).expect("raw dump");
    let reverse_proxy_config = tempdir.path().join("reverse-proxy.toml");
    write_platform_config(
        &reverse_proxy_config,
        &tempdir.path().join("reverse-proxy.sqlite"),
        &reverse_proxy_bundle,
        PlatformConfigSpec {
            auth_mode: "basic",
            embedding_mode: "standalone",
            embedding_profile: "standalone-service",
            adapters: &["camel", "cxf"],
            directory_provider: "internal",
            topology_profile: "reverse-proxy-terminated",
            topology_ingress: None,
            topology_packaging: None,
        },
    );

    let (reverse_proxy_engine, reverse_proxy_base_url, reverse_proxy_client, reverse_proxy_handle) =
        spawn_platform_server(&reverse_proxy_config).await;
    let import_result = reverse_proxy_engine
        .import_historical_migration_from_sql_dump(
            &reverse_proxy_dump,
            flowable_engine::engine::historical_migration::HistoricalMigrationRawDialect::Mysql,
        )
        .expect("reverse proxy raw import should succeed");
    assert_eq!(import_result.imported_process_instances, 1);

    let unauthorized = reverse_proxy_client
        .get(format!(
            "{reverse_proxy_base_url}/management/platform/topology-certification"
        ))
        .send()
        .await
        .expect("unauthorized response");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let reverse_proxy_topology: Value = reverse_proxy_client
        .get(format!(
            "{reverse_proxy_base_url}/management/platform/topology-certification"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("topology cert request should succeed")
        .json()
        .await
        .expect("payload should be json");
    let deprecated_reverse_proxy_topology = reverse_proxy_client
        .get(format!(
            "{reverse_proxy_base_url}/service/management/platform/topology-certification"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("deprecated topology cert request should respond");
    assert_eq!(
        deprecated_reverse_proxy_topology.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        reverse_proxy_topology["profile"],
        "reverse-proxy-terminated"
    );
    assert_eq!(reverse_proxy_topology["cutoverCertified"], true);
    assert_eq!(reverse_proxy_topology["rollbackCertified"], true);

    let runtime_tasks: Value = reverse_proxy_client
        .get(format!(
            "{reverse_proxy_base_url}/runtime/tasks?processInstanceId=proc-inst-1&start=0&size=10"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("runtime task query should succeed")
        .json()
        .await
        .expect("runtime tasks payload should be json");
    assert_eq!(runtime_tasks["total"], 1);
    abort_server(reverse_proxy_handle).await;

    let cdi_bundle = tempdir.path().join("cdi-directory.toml");
    write_directory_bundle(&cdi_bundle);
    let cdi_config = tempdir.path().join("cdi-sidecar.toml");
    write_platform_config(
        &cdi_config,
        &tempdir.path().join("cdi-sidecar.sqlite"),
        &cdi_bundle,
        PlatformConfigSpec {
            auth_mode: "disabled",
            embedding_mode: "embedded",
            embedding_profile: "cdi-compatible",
            adapters: &["camel", "cdi"],
            directory_provider: "ldap-live",
            topology_profile: "cdi-sidecar",
            topology_ingress: None,
            topology_packaging: None,
        },
    );
    let (_cdi_engine, cdi_base_url, cdi_client, cdi_handle) =
        spawn_platform_server(&cdi_config).await;
    let cdi_topology: Value = cdi_client
        .get(format!(
            "{cdi_base_url}/management/platform/topology-certification"
        ))
        .send()
        .await
        .expect("cdi request should succeed")
        .json()
        .await
        .expect("cdi payload should be json");
    assert_eq!(cdi_topology["profile"], "cdi-sidecar");
    assert_eq!(cdi_topology["recoveryCertified"], true);
    assert_eq!(cdi_topology["cutoverCertified"], false);
    abort_server(cdi_handle).await;

    let osgi_bundle = tempdir.path().join("osgi-directory.toml");
    write_directory_bundle(&osgi_bundle);
    let osgi_config = tempdir.path().join("osgi-node.toml");
    write_platform_config(
        &osgi_config,
        &tempdir.path().join("osgi-node.sqlite"),
        &osgi_bundle,
        PlatformConfigSpec {
            auth_mode: "disabled",
            embedding_mode: "embedded",
            embedding_profile: "osgi-managed",
            adapters: &["camel", "cxf", "osgi"],
            directory_provider: "ldap-mirror",
            topology_profile: "osgi-operations-node",
            topology_ingress: None,
            topology_packaging: None,
        },
    );
    let (_osgi_engine, osgi_base_url, osgi_client, osgi_handle) =
        spawn_platform_server(&osgi_config).await;
    let osgi_topology: Value = osgi_client
        .get(format!(
            "{osgi_base_url}/management/platform/topology-certification"
        ))
        .send()
        .await
        .expect("osgi request should succeed")
        .json()
        .await
        .expect("osgi payload should be json");
    assert_eq!(osgi_topology["profile"], "osgi-operations-node");
    assert_eq!(osgi_topology["recoveryCertified"], true);
    let deprecated_osgi_topology = osgi_client
        .get(format!(
            "{osgi_base_url}/service/management/platform/topology-certification"
        ))
        .send()
        .await
        .expect("deprecated osgi request should respond");
    assert_eq!(deprecated_osgi_topology.status(), StatusCode::NOT_FOUND);
    abort_server(osgi_handle).await;
}

#[tokio::test]
async fn replacement_contract_external_topology_cert_test_covers_declared_external_service_mesh() {
    let tempdir = tempfile::tempdir().expect("tempdir should be created");

    let directory_bundle = tempdir.path().join("service-mesh-directory.toml");
    write_directory_bundle(&directory_bundle);
    let raw_dump = tempdir.path().join("historical-source-postgres-copy.sql");
    std::fs::write(
        &raw_dump,
        r#"
COPY ACT_RE_DEPLOYMENT (ID_, NAME_, KEY_, ENGINE_VERSION_, DEPLOY_TIME_) FROM STDIN;
deployment-1	Imported Deployment	deployment-key	8.0.0	1710000000000
\.
COPY ACT_GE_BYTEARRAY (ID_, NAME_, DEPLOYMENT_ID_, BYTES_) FROM STDIN;
resource-1	historicalImportedProcess.bpmn	deployment-1	<?xml version="1.0" encoding="UTF-8"?><definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples"><process id="historicalImportedProcess" isExecutable="true"><startEvent id="start"/><sequenceFlow id="flow1" sourceRef="start" targetRef="approveTask"/><userTask id="approveTask" name="Approve Imported Request"/><sequenceFlow id="flow2" sourceRef="approveTask" targetRef="end"/><endEvent id="end"/></process></definitions>
\.
COPY ACT_RE_PROCDEF (ID_, NAME_, KEY_, DESCRIPTION_, VERSION_, RESOURCE_NAME_, DEPLOYMENT_ID_, HAS_START_FORM_KEY_, HAS_GRAPHICAL_NOTATION_, SUSPENSION_STATE_, ENGINE_VERSION_) FROM STDIN;
historicalImportedProcess:1:source	Historical Imported Process	historicalImportedProcess	imported from source manifest	1	historicalImportedProcess.bpmn	deployment-1	0	0	1	8.0.0
\.
COPY ACT_RU_EXECUTION (ID_, ROOT_PROC_INST_ID_, PROC_INST_ID_, PROC_DEF_ID_, ACT_ID_, IS_ACTIVE_, IS_CONCURRENT_, IS_SCOPE_, IS_MI_ROOT_, SUSPENSION_STATE_, NAME_, BUSINESS_KEY_, START_USER_ID_, START_TIME_) FROM STDIN;
proc-inst-1	proc-inst-1	proc-inst-1	historicalImportedProcess:1:source	approveTask	1	0	1	0	1	Imported Root Execution	M38-001	kermit	1710000100000
\.
COPY ACT_RU_TASK (ID_, PROC_INST_ID_, EXECUTION_ID_, TASK_DEF_KEY_, NAME_, CREATE_TIME_) FROM STDIN;
task-1	proc-inst-1	proc-inst-1	approveTask	Approve Imported Request	1710000100500
\.
"#,
    )
    .expect("raw dump");
    let source_manifest = tempdir.path().join("historical-source-manifest.json");
    std::fs::write(
        &source_manifest,
        format!(
            r#"{{
  "format": "historical-migration-source-manifest/v1",
  "source": {{
    "kind": "postgres-copy-dump",
    "path": "{}"
  }}
}}"#,
            normalize_path(&raw_dump)
        ),
    )
    .expect("manifest");

    let config_path = tempdir.path().join("service-mesh.toml");
    write_platform_config(
        &config_path,
        &tempdir.path().join("service-mesh.sqlite"),
        &directory_bundle,
        PlatformConfigSpec {
            auth_mode: "basic",
            embedding_mode: "standalone",
            embedding_profile: "standalone-service",
            adapters: &["camel", "cxf"],
            directory_provider: "internal",
            topology_profile: "declared-external",
            topology_ingress: Some("service-mesh-terminated"),
            topology_packaging: Some("standalone-service"),
        },
    );

    let (engine, base_url, client, handle) = spawn_platform_server(&config_path).await;
    let import_result = engine
        .import_historical_migration_from_source_manifest(&source_manifest)
        .expect("source manifest import should succeed");
    assert_eq!(import_result.imported_process_instances, 1);

    let topology: Value = client
        .get(format!(
            "{base_url}/management/platform/topology-certification"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("topology request should succeed")
        .json()
        .await
        .expect("topology payload");
    assert_eq!(topology["profile"], "declared-external");
    assert_eq!(topology["ingress"], "service-mesh-terminated");
    assert_eq!(topology["packaging"], "standalone-service");
    assert_eq!(topology["cutoverCertified"], true);
    assert_eq!(topology["rollbackCertified"], true);
    assert_eq!(topology["supportedHistoricalIngress"][0], "source-manifest");

    let tasks: Value = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId=proc-inst-1&start=0&size=10"
        ))
        .basic_auth(ADMIN_USER_ID, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("runtime task query should succeed")
        .json()
        .await
        .expect("runtime tasks payload should be json");
    assert_eq!(tasks["total"], 1);

    abort_server(handle).await;
}
