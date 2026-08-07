use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_rest::run_server;
use serde_json::Value;
use std::sync::Arc;
use tokio::net::TcpListener;

fn build_engine(test_name: &str) -> Arc<ProcessEngine> {
    let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap());
    let engine = Arc::new(ProcessEngine::build(
        test_name.to_string(),
        Arc::clone(&time_source) as Arc<_>,
        db_store,
    ));

    engine
        .get_identity_service()
        .save_user(flowable_engine::identity::entities::User {
            id: "admin".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: Some("test".to_string()),
            tenant_id: None,
        });

    engine
}

async fn spawn_server(engine: Arc<ProcessEngine>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

fn assert_engine_info_shape(body: &Value, path: &str) {
    assert!(body.get("name").is_some(), "{path}: missing 'name' field");
    assert!(body["name"].is_string(), "{path}: 'name' must be a string");
    assert!(
        !body["name"].as_str().unwrap().is_empty(),
        "{path}: 'name' must not be empty"
    );

    assert!(
        body.get("version").is_some(),
        "{path}: missing 'version' field"
    );
    assert!(
        body["version"].is_string(),
        "{path}: 'version' must be a string"
    );
    let version = body["version"].as_str().unwrap();
    assert!(
        is_valid_semver(version),
        "{path}: 'version' is not valid semver: {version}"
    );

    assert!(
        body.get("resourceUrl").is_some(),
        "{path}: missing 'resourceUrl' field"
    );

    assert!(
        body.get("exception").is_some(),
        "{path}: missing 'exception' field"
    );
}

fn is_valid_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return false;
    }
    parts.iter().all(|part| part.parse::<u64>().is_ok())
}

#[tokio::test]
async fn bpmn_engine_info_has_required_shape() {
    let engine = build_engine("parity-bpmn-engine-info");
    let (base_url, client) = spawn_server(engine).await;

    let response = client
        .get(format!("{}/management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_engine_info_shape(&body, "/management/engine");
    assert_eq!(body["name"], "parity-bpmn-engine-info");
    assert!(body["resourceUrl"].is_null());
    assert!(body["exception"].is_null());
}

#[tokio::test]
async fn cmmn_engine_info_has_required_shape() {
    let engine = build_engine("parity-cmmn-engine-info");
    let (base_url, client) = spawn_server(engine).await;

    let response = client
        .get(format!("{}/cmmn-management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_engine_info_shape(&body, "/cmmn-management/engine");
    assert_eq!(body["name"], "cmmn-engine");
    assert!(body["resourceUrl"].is_null());
    assert!(body["exception"].is_null());
}

#[tokio::test]
async fn dmn_engine_info_has_required_shape() {
    let engine = build_engine("parity-dmn-engine-info");
    let (base_url, client) = spawn_server(engine).await;

    let response = client
        .get(format!("{}/dmn-management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_engine_info_shape(&body, "/dmn-management/engine");
    assert_eq!(body["name"], "flowable-dmn-engine");
    assert!(body["resourceUrl"].is_null());
    assert!(body["exception"].is_null());
}

#[tokio::test]
async fn app_engine_info_has_required_shape() {
    let engine = build_engine("parity-app-engine-info");
    let (base_url, client) = spawn_server(engine).await;

    let response = client
        .get(format!("{}/app-management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_engine_info_shape(&body, "/app-management/engine");
    assert_eq!(body["name"], "flowable-app-engine");
    assert!(body["resourceUrl"].is_null());
    assert!(body["exception"].is_null());
}

#[tokio::test]
async fn event_registry_engine_info_has_required_shape() {
    let engine = build_engine("parity-event-registry-engine-info");
    let (base_url, client) = spawn_server(engine).await;

    let response = client
        .get(format!("{}/event-registry-management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_engine_info_shape(&body, "/event-registry-management/engine");
    assert!(body["resourceUrl"].is_null());
    assert!(body["exception"].is_null());
}

#[tokio::test]
async fn idm_engine_info_has_required_shape() {
    let engine = build_engine("parity-idm-engine-info");
    let (base_url, client) = spawn_server(engine).await;

    let response = client
        .get(format!("{}/idm-management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_engine_info_shape(&body, "/idm-management/engine");
    assert!(body["resourceUrl"].is_null());
    assert!(body["exception"].is_null());
}

#[tokio::test]
async fn all_engine_info_endpoints_share_same_json_keys() {
    let engine = build_engine("parity-engine-keys");
    let (base_url, client) = spawn_server(engine).await;

    let paths = [
        "/management/engine",
        "/cmmn-management/engine",
        "/dmn-management/engine",
        "/app-management/engine",
        "/event-registry-management/engine",
        "/idm-management/engine",
    ];

    let mut bodies = Vec::new();
    for path in &paths {
        let response = client
            .get(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{path} returned {}",
            response.status()
        );
        let body: Value = response.json().await.unwrap();
        assert_engine_info_shape(&body, path);
        bodies.push((path, body));
    }

    let expected_keys: Vec<&str> = vec!["name", "version", "resourceUrl", "exception"];
    for (path, body) in &bodies {
        let keys: Vec<String> = body.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys.len(),
            expected_keys.len(),
            "{path}: expected {} keys but got {}: {:?}",
            expected_keys.len(),
            keys.len(),
            keys,
        );
        for key in &expected_keys {
            assert!(
                body.get(*key).is_some(),
                "{path}: missing expected key '{key}'"
            );
        }
    }
}
