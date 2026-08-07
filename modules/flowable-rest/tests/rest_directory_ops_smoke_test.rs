use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use serde_json::Value;
use std::path::Path;
use tokio::net::TcpListener;

fn write_directory_bundle(path: &Path) {
    std::fs::write(
        path,
        r#"
[[users]]
id = "ldap-operator"
first_name = "Ops"
last_name = "User"
email = "ops@example.test"

[[groups]]
id = "ops-team"
name = "Operations"
group_type = "security-role"

[[memberships]]
user_id = "ldap-operator"
group_id = "ops-team"
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
engine_name = "m27-rest-directory-ops"
database_path = "{database_path}"

[security]
auth_mode = "disabled"

[bootstrap]
create_default_admin = false
admin_user_id = "admin"
admin_password = "ignored"

[directory]
provider = "ldap-mirror"
sync_on_bootstrap = true
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

#[tokio::test]
async fn management_routes_expose_directory_and_jmx_bridge_contracts() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_directory_bundle(&bundle_path);
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        &bundle_path,
    );

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    let base_url = spawn_platform_server(platform).await;
    let client = reqwest::Client::new();

    let directory_support: Value = client
        .get(format!("{base_url}/management/directory/support"))
        .send()
        .await
        .expect("directory support request")
        .json()
        .await
        .expect("directory support payload");
    assert_eq!(directory_support["provider"], "ldap-mirror");
    assert_eq!(directory_support["syncOnBootstrap"], true);
    assert_eq!(directory_support["importedUserCount"], 1);

    let jmx_runtime: Value = client
        .get(format!("{base_url}/management/jmx/runtime"))
        .send()
        .await
        .expect("jmx runtime request")
        .json()
        .await
        .expect("jmx runtime payload");
    assert_eq!(jmx_runtime["exposure"], "jmx-bridge");
    assert_eq!(jmx_runtime["managementApiEnabled"], true);
    assert_eq!(jmx_runtime["directoryProvider"], "ldap-mirror");
    assert_eq!(jmx_runtime["identity"]["users"], 1);
    assert_eq!(jmx_runtime["identity"]["groups"], 1);

    let user: Value = client
        .get(format!("{base_url}/identity/users/ldap-operator"))
        .send()
        .await
        .expect("user request")
        .json()
        .await
        .expect("user payload");
    assert_eq!(user["email"], "ops@example.test");
}
