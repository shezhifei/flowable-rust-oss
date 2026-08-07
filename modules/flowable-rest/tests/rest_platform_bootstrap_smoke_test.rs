use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use std::path::Path;
use tokio::net::TcpListener;

fn write_platform_config(
    config_path: &Path,
    database_path: &Path,
    auth_mode: &str,
    create_default_admin: bool,
    admin_user_id: &str,
    admin_password: &str,
) {
    let escaped_database_path = database_path.display().to_string().replace('\\', "\\\\");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "rest-platform-bootstrap"
database_path = "{database_path}"

[security]
auth_mode = "{auth_mode}"

[bootstrap]
create_default_admin = {create_default_admin}
admin_user_id = "{admin_user_id}"
admin_password = "{admin_password}"
"#,
        database_path = escaped_database_path,
    );
    std::fs::write(config_path, config).expect("platform config should be written");
}

async fn spawn_platform_server_from_config(config_path: &Path) -> String {
    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path.to_path_buf()))
        .expect("platform should bootstrap from config file");
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
async fn rest_platform_bootstrap_smoke_test() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        "basic",
        true,
        "admin",
        "platform-secret",
    );
    let base_url = spawn_platform_server_from_config(&config_path).await;

    let client = reqwest::Client::new();

    let health = client
        .get(format!("{base_url}/health"))
        .send()
        .await
        .expect("health request");
    assert!(health.status().is_success());

    let tasks = client
        .get(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("platform-secret"))
        .send()
        .await
        .expect("tasks request");
    assert!(tasks.status().is_success(), "status was {}", tasks.status());
}

#[tokio::test]
async fn rest_platform_bootstrap_config_path_controls_auth_mode() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        "disabled",
        false,
        "admin",
        "ignored-secret",
    );
    let base_url = spawn_platform_server_from_config(&config_path).await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base_url}/history/historic-process-instances"))
        .send()
        .await
        .expect("history request");

    assert!(
        response.status().is_success(),
        "expected auth to be disabled, got {}",
        response.status()
    );
}
