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
    assert_ne!(after.password, old_cipher);
    assert_eq!(
        state.configs.decrypt_password(&after).unwrap(),
        "new-secret"
    );
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

// silence unused import if representation used only in types
#[allow(dead_code)]
fn _type_use(_: ServerConfigRepresentation) {}
