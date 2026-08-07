use flowable_platform_bootstrap::{FlowablePlatform, RuntimeEmbeddingProfile};
use flowable_rest::run_platform_server;
use std::path::Path;
use tokio::net::TcpListener;

fn write_platform_config(config_path: &Path, database_path: &Path) {
    let escaped_database_path = database_path.display().to_string().replace('\\', "\\\\");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "rest-platform-embedding"
database_path = "{database_path}"

[security]
auth_mode = "disabled"

[embedding]
mode = "embedded"
profile = "cdi-compatible"

[enterprise]
adapters = ["camel", "cdi"]

[bootstrap]
create_default_admin = false
admin_user_id = "admin"
admin_password = "ignored"
"#,
        database_path = escaped_database_path,
    );
    std::fs::write(config_path, config).expect("platform config should be written");
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
async fn rest_platform_bootstrap_smoke_test_supports_embedded_enterprise_contract() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_platform_config(&config_path, &tempdir.path().join("process-engine.sqlite"));

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path))
        .expect("platform should bootstrap from config file");
    assert_eq!(
        platform.runtime_embedding_contract().profile,
        RuntimeEmbeddingProfile::CdiCompatible
    );
    assert_eq!(
        platform.enterprise_support_statement(),
        "bounded enterprise runtime embedding contract: mode=Embedded, profile=CdiCompatible, adapters=camel, cdi"
    );

    let base_url = spawn_platform_server(platform).await;
    let client = reqwest::Client::new();

    let health = client
        .get(format!("{base_url}/health"))
        .send()
        .await
        .expect("health request");
    assert!(health.status().is_success());

    let tasks = client
        .get(format!("{base_url}/runtime/tasks"))
        .send()
        .await
        .expect("tasks request");
    assert!(tasks.status().is_success(), "status was {}", tasks.status());
}
