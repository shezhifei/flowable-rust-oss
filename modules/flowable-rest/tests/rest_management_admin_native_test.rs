use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use reqwest::{Response, StatusCode, header::CONTENT_TYPE};
use serde_json::{Value, json};
use std::path::Path;
use tokio::net::TcpListener;

fn write_directory_bundle(path: &Path) {
    std::fs::write(
        path,
        r#"
[[users]]
id = "ops-deprecated"
first_name = "Legacy"
last_name = "Ops"
email = "ops-deprecated@example.test"

[[groups]]
id = "admins-deprecated"
name = "Legacy Admins"
group_type = "security-role"

[[memberships]]
user_id = "ops-deprecated"
group_id = "admins-deprecated"
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
engine_name = "m28-management-admin"
database_path = "{database_path}"

[security]
auth_mode = "{auth_mode}"

[bootstrap]
create_default_admin = {create_default_admin}
admin_user_id = "admin"
admin_password = "platform-secret"

[directory]
provider = "ldap-mirror"
sync_on_bootstrap = true
bundle_path = "{bundle_path}"

[operations]
exposure = "jmx-bridge"
management_api_enabled = true
"#,
        database_path = escaped_database_path,
        auth_mode = auth_mode,
        create_default_admin = create_default_admin,
        bundle_path = escaped_bundle_path,
    );
    std::fs::write(config_path, config).expect("config");
}

async fn spawn_platform_server(config_path: &Path) -> String {
    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path.to_path_buf()))
        .expect("platform");
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
async fn native_management_directory_and_operations_routes_are_exposed() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_directory_bundle(&bundle_path);
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        &bundle_path,
        "disabled",
        false,
    );
    let base_url = spawn_platform_server(&config_path).await;
    let client = reqwest::Client::new();

    let directory = capture_json(
        client
            .get(format!("{}/management/directory/support", base_url))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(directory.0, StatusCode::OK);
    assert_eq!(directory.2["provider"], json!("ldap-mirror"));

    let operations = capture_json(
        client
            .get(format!("{}/management/operations/support", base_url))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(operations.0, StatusCode::OK);
    assert_eq!(operations.2["exposure"], json!("jmx-bridge"));

    let jmx = capture_json(
        client
            .get(format!("{}/management/jmx/runtime", base_url))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(jmx.0, StatusCode::OK);
    assert_eq!(jmx.2["engineName"], json!("m28-management-admin"));

    let alias_directory = client
        .get(format!("{}/service/management/directory/support", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(alias_directory.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn native_management_routes_enforce_basic_auth_contract() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_directory_bundle(&bundle_path);
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        &bundle_path,
        "basic",
        true,
    );
    let base_url = spawn_platform_server(&config_path).await;
    let client = reqwest::Client::new();

    let unauthorized = capture_json(
        client
            .get(format!("{}/management/jmx/runtime", base_url))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(unauthorized.0, StatusCode::UNAUTHORIZED);
    assert_eq!(unauthorized.2["code"], json!("UNAUTHORIZED"));

    let authorized = capture_json(
        client
            .get(format!("{}/management/operations/support", base_url))
            .basic_auth("admin", Some("platform-secret"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(authorized.0, StatusCode::OK);
    assert_eq!(authorized.2["exposure"], json!("jmx-bridge"));

    let alias_unauthorized = client
        .get(format!("{}/service/management/jmx/runtime", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(alias_unauthorized.status(), StatusCode::UNAUTHORIZED);
}
