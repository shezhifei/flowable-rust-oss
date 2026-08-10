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

// ---------------------------------------------------------------------------
// Gap endpoints (ui-migration-coverage.md §2.2)
// ---------------------------------------------------------------------------

/// Recorded upstream calls: (method, path, body).
#[derive(Clone, Default)]
struct GapMockState {
    calls: Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
}

async fn spawn_gap_mock_engine() -> (SocketAddr, tokio::task::JoinHandle<()>, GapMockState) {
    use axum::extract::{Path as AxumPath, Query as AxumQuery, State as AxumState};
    use std::collections::HashMap;

    async fn record(state: &GapMockState, method: &str, path: &str, body: &[u8]) {
        state.calls.lock().unwrap().push((
            method.to_string(),
            path.to_string(),
            String::from_utf8_lossy(body).to_string(),
        ));
    }

    async fn echo_query(
        AxumState(state): AxumState<GapMockState>,
        AxumQuery(params): AxumQuery<HashMap<String, String>>,
        request: Request<Body>,
    ) -> Json<Value> {
        record(&state, request.method().as_str(), request.uri().path(), &[]).await;
        Json(json!({
            "data": [],
            "total": 0,
            "start": 0,
            "size": 0,
            "query": params,
        }))
    }

    async fn stacktrace(
        AxumState(state): AxumState<GapMockState>,
        request: Request<Body>,
    ) -> &'static str {
        record(&state, request.method().as_str(), request.uri().path(), &[]).await;
        "java.lang.RuntimeException: boom"
    }

    async fn no_content_with_body(
        AxumState(state): AxumState<GapMockState>,
        request: Request<Body>,
    ) -> StatusCode {
        let method = request.method().to_string();
        let path = request.uri().path().to_string();
        let body = request.into_body().collect().await.unwrap().to_bytes();
        record(&state, &method, &path, &body).await;
        StatusCode::NO_CONTENT
    }

    async fn ok_json_with_body(
        AxumState(state): AxumState<GapMockState>,
        AxumPath(id): AxumPath<String>,
        request: Request<Body>,
    ) -> Json<Value> {
        let path = request.uri().path().to_string();
        let body = request.into_body().collect().await.unwrap().to_bytes();
        record(&state, "POST", &path, &body).await;
        Json(json!({ "id": id, "migrated": true }))
    }

    async fn cmmn_job(AxumPath(id): AxumPath<String>) -> Json<Value> {
        Json(json!({ "id": id, "jobType": "cmmn" }))
    }

    async fn child_deployments() -> Json<Value> {
        Json(json!({
            "data": [{ "id": "child-dep-1" }],
            "total": 1,
            "start": 0,
            "size": 1
        }))
    }

    async fn no_child_deployments() -> Json<Value> {
        Json(json!({ "data": [], "total": 0, "start": 0, "size": 0 }))
    }

    async fn related_models(AxumQuery(params): AxumQuery<HashMap<String, String>>) -> Json<Value> {
        Json(json!({
            "data": [{ "id": "model-1", "deploymentId": params.get("deploymentId") }],
            "total": 1,
            "start": 0,
            "size": 1
        }))
    }

    let state = GapMockState::default();
    let app = Router::new()
        .route(
            "/management/jobs/:id/exception-stacktrace",
            get(stacktrace),
        )
        .route(
            "/management/deadletter-jobs/:id",
            axum::routing::post(no_content_with_body),
        )
        .route(
            "/cmmn-management/jobs/:id",
            get(cmmn_job)
                .delete(no_content_with_body)
                .post(no_content_with_body),
        )
        .route(
            "/cmmn-management/jobs/:id/exception-stacktrace",
            get(stacktrace),
        )
        .route(
            "/cmmn-management/timer-jobs/:id",
            axum::routing::post(no_content_with_body),
        )
        .route(
            "/cmmn-runtime/case-instances/:id/change-state",
            axum::routing::post(ok_json_with_body),
        )
        .route(
            "/cmmn-runtime/case-instances/:id/migrate",
            axum::routing::post(ok_json_with_body),
        )
        .route(
            "/dmn-history/historic-decision-executions",
            get(echo_query),
        )
        .route(
            "/repository/process-definitions/:id/decision-tables",
            get(related_models),
        )
        .route(
            "/repository/process-definitions/:id/form-definitions",
            get(related_models),
        )
        .route(
            "/repository/process-definitions/:id/batch-migrate",
            axum::routing::post(ok_json_with_body),
        )
        .route(
            "/cmmn-repository/case-definitions/:id/decision-tables",
            get(related_models),
        )
        .route(
            "/cmmn-repository/case-definitions/:id/form-definitions",
            get(related_models),
        )
        .route("/content-service/content-items", get(echo_query))
        .route("/form/form-instances", get(echo_query))
        .route(
            "/form/form-instances/:id/values",
            get(|| async { Json(json!({ "fieldA": "valueA" })) }),
        )
        .route("/repository/deployments", get(child_deployments))
        .route("/repository/process-definitions", get(related_models))
        .route("/dmn-repository/deployments", get(no_child_deployments))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle, state)
}

async fn get_uri(app: &Router, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn post_json(app: &Router, uri: &str, body: Value) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// §2.2 decision 2 / §7: the stacktrace endpoint moved to the Java path
/// `/jobs/{jobId}/stacktrace`; the old engine-style path stays as a
/// deprecated alias. Both must work.
#[tokio::test]
async fn job_stacktrace_java_path_and_deprecated_alias() {
    let (addr, _handle, _state) = spawn_gap_mock_engine().await;
    let app = router_with_state(admin_state_pointing_at(addr));

    for path in [
        "/admin-app/rest/admin/jobs/job-1/stacktrace",
        "/admin-app/rest/admin/jobs/job-1/exception-stacktrace",
    ] {
        let res = get_uri(&app, path).await;
        assert_eq!(res.status(), StatusCode::OK, "path={path}");
        let text = String::from_utf8(
            res.into_body().collect().await.unwrap().to_bytes().to_vec(),
        )
        .unwrap();
        assert!(text.contains("boom"), "path={path} body={text}");
    }
}

/// `POST /rest/admin/move-jobs/{jobId}` forwards a server-constructed
/// `{"action":"move"}` body to the job collection selected by `jobType`.
#[tokio::test]
async fn move_job_posts_action_body_to_job_type_collection() {
    let (addr, _handle, state) = spawn_gap_mock_engine().await;
    let app = router_with_state(admin_state_pointing_at(addr));

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin-app/rest/admin/move-jobs/job-9?jobType=deadletterJob")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let calls = state.calls.lock().unwrap();
    assert!(
        calls.iter().any(|(_, _, body)| body.contains("\"action\":\"move\"")),
        "recorded calls: {calls:?}"
    );
}

/// CmmnJobClientResource family: get/delete/execute/move/stacktrace.
#[tokio::test]
async fn cmmn_job_endpoints_family() {
    let (addr, _handle, state) = spawn_gap_mock_engine().await;
    let app = router_with_state(admin_state_pointing_at(addr));

    let res = get_uri(&app, "/admin-app/rest/admin/cmmn-jobs/cj-1").await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["id"], "cj-1");

    let res = get_uri(&app, "/admin-app/rest/admin/cmmn-jobs/cj-1/stacktrace").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin-app/rest/admin/cmmn-jobs/cj-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // execute: always targets cmmn-management/jobs with {"action":"execute"}
    let res = post_json(&app, "/admin-app/rest/admin/cmmn-jobs/cj-1?jobType=timerJob", json!({})).await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    // move: honours jobType and posts {"action":"move"}
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin-app/rest/admin/move-cmmn-jobs/cj-1?jobType=timerJob")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let calls = state.calls.lock().unwrap();
    let bodies: Vec<&str> = calls.iter().map(|(_, _, b)| b.as_str()).collect();
    assert!(
        bodies.iter().any(|b| b.contains("\"action\":\"execute\"")),
        "{bodies:?}"
    );
    assert!(
        bodies.iter().any(|b| b.contains("\"action\":\"move\"")),
        "{bodies:?}"
    );
}

/// FormInstance(s)ClientResource family.
#[tokio::test]
async fn form_instances_family_filters() {
    let (addr, _handle, _state) = spawn_gap_mock_engine().await;
    let app = router_with_state(admin_state_pointing_at(addr));

    let res = get_uri(&app, "/admin-app/rest/admin/task-form-instance/task-1").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["query"]["taskId"], "task-1");

    let res = get_uri(&app, "/admin-app/rest/admin/form-instances/fi-1/form-field-values").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["fieldA"], "valueA");

    let res = get_uri(&app, "/admin-app/rest/admin/form-definition-form-instances/fd-1").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["query"]["formDefinitionId"], "fd-1");

    let res = get_uri(&app, "/admin-app/rest/admin/process-form-instances/pi-1").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["query"]["processInstanceId"], "pi-1");

    let res = get_uri(&app, "/admin-app/rest/admin/case-form-instances/ci-1").await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["query"]["scopeId"], "ci-1");
    assert_eq!(v["query"]["scopeType"], "cmmn");
}

/// AppDefinitionClientResource related models: two-step resolution via
/// child deployments, and the empty-child case.
#[tokio::test]
async fn app_definition_related_models_two_step_lookup() {
    let (addr, _handle, _state) = spawn_gap_mock_engine().await;
    let app = router_with_state(admin_state_pointing_at(addr));

    // PROCESS domain: /repository/deployments returns child-dep-1, which must
    // be forwarded as deploymentId to repository/process-definitions.
    let res = get_uri(
        &app,
        "/admin-app/rest/admin/app-definitions/app-1/process-definitions?deploymentId=dep-1",
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["data"][0]["deploymentId"], "child-dep-1");

    // DMN domain: no child deployment → Java returns {"size":0,"data":[]}.
    let res = get_uri(
        &app,
        "/admin-app/rest/admin/app-definitions/app-1/decision-tables?deploymentId=dep-1",
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["size"], 0);
    assert_eq!(v["data"].as_array().unwrap().len(), 0);

    // deploymentId is required (Java BadRequestException otherwise).
    let res = get_uri(&app, "/admin-app/rest/admin/app-definitions/app-1/form-definitions").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Route exists for the CMMN domain as well (mock has no cmmn-repository
    // deployments route, so any response proves routing works).
    let res = get_uri(
        &app,
        "/admin-app/rest/admin/app-definitions/app-1/case-definitions?deploymentId=dep-1",
    )
    .await;
    assert_ne!(res.status(), StatusCode::NOT_FOUND);
}

/// CaseInstanceClientResource: change-state / migrate / decision-executions.
#[tokio::test]
async fn case_instance_change_state_migrate_decision_executions() {
    let (addr, _handle, _state) = spawn_gap_mock_engine().await;
    let app = router_with_state(admin_state_pointing_at(addr));

    let res = post_json(
        &app,
        "/admin-app/rest/admin/case-instances/ci-1/change-state",
        json!({ "changeToState": "active" }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = post_json(
        &app,
        "/admin-app/rest/admin/case-instances/ci-1/migrate",
        json!({ "document": "..." }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get_uri(&app, "/admin-app/rest/admin/case-instances/ci-1/decision-executions").await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["query"]["instanceId"], "ci-1");
    assert_eq!(v["query"]["scopeType"], "cmmn");
}

/// Low-priority proxies (§2.2): decision-tables/form-definitions by
/// definition, dmn history list, batch-migrate, pi decision-executions,
/// process-instance content items.
#[tokio::test]
async fn low_priority_proxy_endpoints() {
    let (addr, _handle, _state) = spawn_gap_mock_engine().await;
    let app = router_with_state(admin_state_pointing_at(addr));

    for uri in [
        "/admin-app/rest/admin/process-definition-decision-tables/pd-1",
        "/admin-app/rest/admin/process-definition-form-definitions/pd-1",
        "/admin-app/rest/admin/case-definition-decision-tables/cd-1",
        "/admin-app/rest/admin/case-definition-form-definitions/cd-1",
    ] {
        let res = get_uri(&app, uri).await;
        assert_eq!(res.status(), StatusCode::OK, "uri={uri}");
    }

    // Static `history` must win over the `:decision_table_id` parameter route.
    let res = get_uri(&app, "/admin-app/rest/admin/decision-tables/history?decisionKey=abc").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["query"]["decisionKey"], "abc");

    let res = post_json(
        &app,
        "/admin-app/rest/admin/process-definitions/pd-1/batch-migrate",
        json!({ "document": "..." }),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get_uri(&app, "/admin-app/rest/admin/process-instances/pi-1/decision-executions").await;
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["query"]["instanceId"], "pi-1");
    // Unlike the case variant, no scopeType is sent for process instances.
    assert!(v["query"].get("scopeType").is_none());

    let res = get_uri(&app, "/admin-app/rest/admin/process-instance-content-items/pi-1").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["query"]["processInstanceId"], "pi-1");
}

/// CmmnDisplayJsonClientResource routes exist; without a CMMN engine on the
/// process engine they surface the same bad-request style as other admin
/// configuration errors.
#[tokio::test]
async fn cmmn_display_model_json_routes() {
    use flowable_engine::engine::process_engine::ProcessEngine;

    let engine = Arc::new(ProcessEngine::new("ui-admin-cmmn-display".into()));
    let app = router_with_state(AdminState::new()).layer(axum::Extension(engine));

    for uri in [
        "/admin-app/rest/admin/case-definitions/cd-1/model-json",
        "/admin-app/rest/admin/case-instances/ci-1/model-json",
    ] {
        let res = get_uri(&app, uri).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "uri={uri}");
    }
}
