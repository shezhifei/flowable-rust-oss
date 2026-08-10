//! Admin UI contract tests (stream B).
//!
//! Endpoint inventory checklist (high-frequency):
//! - GET  /admin-app/rest/health
//! - GET  /admin-app/rest/server-configs
//! - GET  /admin-app/rest/server-configs/default/{code}
//! - PUT  /admin-app/rest/server-configs/{id}
//! - GET  /admin-app/rest/admin/deployments  (proxied)
//! - GET  /admin-app/rest/admin/engine-info/{code}

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Json, Router,
};
use flowable_ui_rest::admin::{
    router_with_state, AdminState, EndpointType, ServerConfigRepresentation, ServerConfigStore,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceExt;

async fn spawn_mock_engine() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route(
            "/repository/deployments",
            get(|| async {
                Json(json!({
                    "data": [{ "id": "dep-1", "name": "demo" }],
                    "total": 1,
                    "start": 0,
                    "size": 1
                }))
            }),
        )
        .route(
            "/management/engine",
            get(|| async {
                Json(json!({
                    "name": "default",
                    "version": "0.1.0-rust"
                }))
            }),
        )
        .route(
            "/query/historic-process-instances",
            axum::routing::post(|| async {
                Json(json!({
                    "data": [],
                    "total": 0,
                    "start": 0,
                    "size": 10
                }))
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

fn admin_state_pointing_at(addr: SocketAddr) -> AdminState {
    // Override defaults via env-free empty store, then save configs.
    let store = Arc::new(ServerConfigStore::empty_for_tests(Default::default()));
    for endpoint in EndpointType::all() {
        let cfg = flowable_ui_rest::admin::ServerConfig {
            id: format!("cfg-{}", endpoint.code()),
            name: format!("ep-{}", endpoint.code()),
            description: "test".into(),
            server_address: format!("http://{}", addr.ip()),
            port: addr.port() as i32,
            context_root: String::new(),
            rest_root: String::new(),
            user_name: "admin".into(),
            password: "test".into(),
            endpoint_type: endpoint.code(),
            tenant_id: None,
        };
        store.save_new(cfg, true).unwrap();
    }
    AdminState::with_store(store)
}

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn health_probe() {
    let app = router_with_state(AdminState::new());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin-app/rest/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["app"], "admin");
}

#[tokio::test]
async fn server_config_list_and_default() {
    let app = router_with_state(AdminState::new());
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin-app/rest/server-configs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    let arr = list.as_array().expect("array");
    assert_eq!(arr.len(), 6);
    // Password must not be present on list representation.
    assert!(arr[0].get("password").is_none() || arr[0]["password"].is_null());

    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin-app/rest/server-configs/default/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let def = body_json(res).await;
    assert_eq!(def["endpointType"], 1);
    assert!(def["name"].as_str().unwrap().contains("Process"));
}

#[tokio::test]
async fn server_config_update_and_password_encrypt_roundtrip() {
    let state = AdminState::new();
    let list = state.configs.list_representations();
    let id = list[0].id.clone().unwrap();
    let before = state.configs.get(&id).unwrap();
    let old_cipher = before.password.clone();

    let app = router_with_state(state.clone());
    let body = json!({
        "name": "renamed",
        "description": "updated",
        "serverAddress": "http://127.0.0.1",
        "serverPort": 9999,
        "contextRoot": "",
        "restRoot": "",
        "userName": "admin",
        "password": "new-secret",
        "endpointType": list[0].endpoint_type
    });
    let res = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/admin-app/rest/server-configs/{id}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let after = state.configs.get(&id).unwrap();
    assert_eq!(after.name, "renamed");
    assert_eq!(after.port, 9999);
    assert_ne!(after.password, "new-secret");
    // Decrypt is the real contract; AES/CBC is deterministic so ciphertext
    // may equal a prior value only if the plaintext did not change.
    assert_eq!(
        state.configs.decrypt_password(&after).unwrap(),
        "new-secret"
    );
    let _ = old_cipher;
}

#[tokio::test]
async fn proxy_forwards_get_deployments_with_basic_auth() {
    let (addr, _handle) = spawn_mock_engine().await;
    let state = admin_state_pointing_at(addr);
    let app = router_with_state(state);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin-app/rest/admin/deployments")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["total"], 1);
    assert_eq!(v["data"][0]["id"], "dep-1");
}

#[tokio::test]
async fn proxy_engine_info_and_process_instance_query() {
    let (addr, _handle) = spawn_mock_engine().await;
    let state = admin_state_pointing_at(addr);
    let app = router_with_state(state);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin-app/rest/admin/engine-info/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["name"], "default");

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin-app/rest/admin/process-instances")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "size": 10, "sort": "startTime" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["total"], 0);
}

#[tokio::test]
async fn proxy_connect_failure_maps_to_bad_request() {
    // Point at a closed port.
    let store = Arc::new(ServerConfigStore::empty_for_tests(Default::default()));
    store
        .save_new(
            flowable_ui_rest::admin::ServerConfig {
                id: "x".into(),
                name: "p".into(),
                description: "d".into(),
                server_address: "http://127.0.0.1".into(),
                port: 1,
                context_root: String::new(),
                rest_root: String::new(),
                user_name: "a".into(),
                password: "b".into(),
                endpoint_type: 1,
                tenant_id: None,
            },
            true,
        )
        .unwrap();
    let app = router_with_state(AdminState::with_store(store));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin-app/rest/admin/deployments")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let v = body_json(res).await;
    let msg = v["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("Unable to connect") || msg.contains("timed out") || msg.contains("error"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
async fn server_config_persists_to_disk() {
    let path = std::env::temp_dir().join(format!("ui-sc-{}.json", uuid::Uuid::new_v4()));
    unsafe {
        std::env::set_var(
            "FLOWABLE_UI_SERVER_CONFIG_PATH",
            path.to_string_lossy().as_ref(),
        );
    }
    let store = Arc::new(ServerConfigStore::with_defaults());
    let list = store.list_representations();
    assert_eq!(list.len(), 6);
    assert!(path.exists());

    // Reload from disk
    let store2 = Arc::new(ServerConfigStore::with_defaults());
    assert_eq!(store2.list_representations().len(), 6);
    let _ = std::fs::remove_file(&path);
    unsafe {
        std::env::remove_var("FLOWABLE_UI_SERVER_CONFIG_PATH");
    }
}

#[tokio::test]
async fn process_definition_model_json_with_engine() {
    use flowable_engine::engine::process_engine::ProcessEngine;
    use tower::ServiceExt;

    let engine = Arc::new(ProcessEngine::new("ui-admin-display".into()));
    // Empty DI → empty object (no definition deployed)
    // Route requires a real definition id; expect bad request / empty
    let state = AdminState::new();
    let app = router_with_state(state).layer(axum::Extension(engine));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin-app/rest/admin/process-definitions/missing/model-json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Not found / bad request from repository
    assert!(
        res.status() == StatusCode::BAD_REQUEST || res.status() == StatusCode::OK,
        "status={}",
        res.status()
    );
}

/// Java flowable-ui-admin `AccountResource.getAccount`: the admin app resolves
/// the session user on startup and only loads the server configs on success,
/// so this endpoint must exist and describe the session user.
#[tokio::test]
async fn account_returns_the_session_user() {
    use flowable_engine::engine::process_engine::ProcessEngine;
    use flowable_engine::identity::entities::User;
    use flowable_ui_rest::auth::{AuthMode, UiAuthConfig};
    use flowable_ui_rest::ui_router_with_config;

    let engine = Arc::new(ProcessEngine::new("ui-admin-account".into()));
    engine.get_identity_service().save_user(User {
        id: "admin".into(),
        first_name: Some("Test".into()),
        last_name: Some("Admin".into()),
        email: Some("admin@example.com".into()),
        password: Some("test".into()),
        tenant_id: None,
    });
    let config = Arc::new(UiAuthConfig {
        mode: AuthMode::Disabled,
        dev_user_id: "admin".to_string(),
        ..UiAuthConfig::default()
    });
    let app = ui_router_with_config(config).layer(axum::Extension(Arc::clone(&engine)));

    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin-app/rest/account")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["id"], "admin");
    assert_eq!(body["fullName"], "Test Admin");
    assert!(body["groups"].is_array());
    assert!(body["privileges"].is_array());
}

// silence unused import if representation used only in types
#[allow(dead_code)]
fn _type_use(_: ServerConfigRepresentation) {}
