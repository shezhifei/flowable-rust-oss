//! The admin, task, and modeler UI surfaces against a real PostgreSQL backend.
//!
//! Companion to `ui_postgres_smoke_test.rs` (idm face); same rules: requires the
//! `postgres` feature plus a reachable server from `FLOWABLE_TEST_POSTGRES_URL`,
//! and every test **skips gracefully** when the database is down. Tests share one
//! schema, so each works under its own id suffix and deletes its rows at the end.
//!
//! ```powershell
//! cargo test -p flowable-ui-rest --features postgres --test ui_postgres_smoke_apps_test
//! ```
//!
//! Face notes:
//!
//! - **admin** — the ServerConfig store is *not* database-backed: it persists to
//!   a JSON file (`FLOWABLE_UI_SERVER_CONFIG_PATH`, see
//!   `src/admin/server_config.rs`), so there is no server-config table to round
//!   trip through Postgres. What does touch the engine is the admin proxy: it
//!   decrypts the stored password, builds Basic auth, and reads engine REST. The
//!   pg case below therefore points the proxy at a minimal engine-REST responder
//!   backed by the *same Postgres engine*, so `list_deployments` is exercised
//!   end to end against real pg rows. (The full `flowable-rest` server also
//!   boots on Postgres — see `flowable-rest`'s `postgres_server_boot_test`.)
//!   Password encryption at rest is covered by a separate case that needs no
//!   database at all.
//! - **task** — the aggregation read path (`POST /app/rest/query/tasks`) over a
//!   deployment + started instance on the pg engine, including the
//!   delete-on-complete transition.
//! - **modeler** — editor document round trip (`GET`/`PUT
//!   /modeler-app/rest/models/:id/editor/bpmn-json`) with the source bytes
//!   persisted in pg via the repository model tables.

#![cfg(feature = "postgres")]

use std::sync::{Arc, OnceLock};

use axum::{
    extract::{Extension, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::identity::entities::{Privilege, User};
use flowable_engine::repository::model::RepositoryModel;
use flowable_engine::service::config::{
    DatabaseConfiguration, EngineDatabaseKind, ProcessEngineConfiguration,
};
use flowable_ui_rest::admin::{self, AdminState, EndpointType, ServerConfig, ServerConfigStore};
use flowable_ui_rest::auth::UiAuthConfig;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

fn postgres_url() -> String {
    std::env::var("FLOWABLE_TEST_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/flowable_test".to_string())
}

fn postgres_config(pool_size: u32) -> ProcessEngineConfiguration {
    ProcessEngineConfiguration {
        database: DatabaseConfiguration {
            kind: EngineDatabaseKind::Postgres,
            url: postgres_url(),
            pool_size,
            busy_timeout_ms: 5000,
            journal_mode: Default::default(),
        },
        ..Default::default()
    }
}

/// Cached availability probe, so a down database costs one connection attempt
/// per process rather than one per test.
static PG_AVAILABLE: OnceLock<bool> = OnceLock::new();

fn postgres_available() -> bool {
    *PG_AVAILABLE.get_or_init(|| {
        match ProcessEngine::build_with_config(
            "ui-pg-apps-availability-probe".to_string(),
            Arc::new(SystemTimeSource),
            postgres_config(1),
        ) {
            Ok(_) => true,
            Err(error) => {
                eprintln!(
                    "Skipping UI PostgreSQL app smoke tests: database unreachable ({error}). Set \
                     FLOWABLE_TEST_POSTGRES_URL to a live instance to run them."
                );
                false
            }
        }
    })
}

/// A Postgres-backed engine, or `None` when the database is unreachable so the
/// suite skips rather than fails.
fn build_engine(test_name: &str) -> Option<Arc<ProcessEngine>> {
    if !postgres_available() {
        return None;
    }
    match ProcessEngine::build_with_config(
        format!("{test_name}-{}", Uuid::new_v4().simple()),
        Arc::new(SystemTimeSource),
        postgres_config(4),
    ) {
        Ok(engine) => Some(Arc::new(engine)),
        Err(error) => {
            eprintln!("Skipping UI PostgreSQL test '{test_name}': {error}");
            None
        }
    }
}

/// A served UI surface over Postgres with a signed-in user holding the given
/// privilege names, plus the ids this test owns for cleanup.
struct UiApp {
    engine: Arc<ProcessEngine>,
    base_url: String,
    client: reqwest::Client,
    user_id: String,
    suffix: String,
    privilege_ids: Vec<String>,
    token_series: Vec<String>,
}

impl UiApp {
    fn cookie_header(&self, cookie: &str) -> String {
        format!("FLOWABLE_REMEMBER_ME={cookie}")
    }

    /// POSTs the task app's task query aggregation with the session cookie.
    async fn query_tasks(&self, cookie: &str, body: Value) -> Value {
        let response = self
            .client
            .post(format!("{}/app/rest/query/tasks", self.base_url))
            .header("cookie", self.cookie_header(cookie))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "task query should succeed");
        response.json().await.unwrap()
    }

    /// Removes everything this test wrote to the shared schema.
    ///
    /// Called explicitly rather than from `Drop`, matching the idm smoke: a
    /// failing assertion leaves the rows for inspection, and the next run's ids
    /// are fresh anyway.
    fn cleanup(&self) {
        let identity = self.engine.get_identity_service();
        for series in &self.token_series {
            identity.delete_token(series);
        }
        for privilege_id in &self.privilege_ids {
            identity.delete_user_privilege_mapping(privilege_id, &self.user_id);
            identity.delete_privilege(privilege_id);
        }
        identity.delete_user(&self.user_id);
    }
}

/// Spawns the full UI router over a Postgres engine, signs the test user in,
/// and returns the app plus the remember-me cookie; `None` when pg is down.
async fn spawn_ui(test_name: &str, privileges: &[&str]) -> Option<(UiApp, String)> {
    let engine = build_engine(test_name)?;

    let suffix = Uuid::new_v4().simple().to_string();
    let user_id = format!("ui-{suffix}");

    let identity = engine.get_identity_service();
    identity.save_user(User {
        id: user_id.clone(),
        first_name: Some("Ui".to_string()),
        last_name: Some(suffix.clone()),
        email: Some(format!("{user_id}@example.com")),
        password: Some("test".to_string()),
        tenant_id: None,
    });
    let mut privilege_ids = Vec::new();
    for name in privileges {
        // The privilege *name* is what the access check reads, so it cannot be
        // uniquified; the id carries the suffix instead.
        let privilege_id = format!("priv-{name}-{suffix}");
        identity.save_privilege(Privilege {
            id: privilege_id.clone(),
            name: (*name).to_string(),
        });
        identity.add_user_privilege_mapping(privilege_id.clone(), user_id.clone());
        privilege_ids.push(privilege_id);
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = flowable_ui_rest::ui_router_with_config(Arc::new(UiAuthConfig::default()))
        .layer(Extension(Arc::clone(&engine)));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base_url}/app/authentication"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("user-agent", "smoke-test-agent/1.0")
        .body(format!("j_username={user_id}&j_password=test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "login should succeed");
    let cookie = remember_me_cookie(&response);

    let app = UiApp {
        engine,
        base_url,
        client,
        user_id,
        suffix,
        privilege_ids,
        token_series: vec![series_of(&cookie)],
    };
    Some((app, cookie))
}

fn remember_me_cookie(response: &reqwest::Response) -> String {
    response
        .headers()
        .get("set-cookie")
        .expect("no Set-Cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("FLOWABLE_REMEMBER_ME=")
        .expect("not the remember-me cookie")
        .to_string()
}

/// The cookie is `base64(series:tokenValue)`; the series is the row's primary key.
fn series_of(cookie: &str) -> String {
    String::from_utf8(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(cookie)
            .expect("cookie should be base64"),
    )
    .unwrap()
    .split(':')
    .next()
    .unwrap()
    .to_string()
}

fn smoke_bpmn(key: &str, assignee: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:flowable="http://flowable.org/bpmn" targetNamespace="https://flowable.org/ui-pg-smoke">
  <process id="{key}" name="UI pg smoke" isExecutable="true">
    <startEvent id="start"/>
    <userTask id="review" name="Review {key}" flowable:assignee="{assignee}"/>
    <endEvent id="end"/>
    <sequenceFlow id="f1" sourceRef="start" targetRef="review"/>
    <sequenceFlow id="f2" sourceRef="review" targetRef="end"/>
  </process>
</definitions>"#
    )
}

/// Deploys the smoke process under a per-test key and returns
/// `(deployment_id, process_definition_id)`.
fn deploy_smoke_process(
    engine: &Arc<ProcessEngine>,
    key: &str,
    assignee: &str,
) -> (String, String) {
    let repository = engine.get_repository_service();
    let builder = repository
        .create_deployment()
        .name(format!("ui-pg-{key}"))
        .add_string(format!("{key}.bpmn20.xml"), smoke_bpmn(key, assignee));
    let deployment = repository.deploy(builder).expect("deploy should succeed");
    let definition = repository
        .latest_process_definition_by_key(key, None)
        .unwrap()
        .expect("the deployed definition should be found by its key");
    (deployment.id, definition.id)
}

fn start_instance(engine: &Arc<ProcessEngine>, definition_id: &str) -> String {
    let runtime = engine.get_runtime_service();
    let builder = runtime
        .create_process_instance_builder()
        .process_definition_id(definition_id.to_string());
    runtime
        .start_process_instance(builder)
        .expect("process instance should start")
        .id
}

// ---------------------------------------------------------------------------
// admin
// ---------------------------------------------------------------------------

const REST_USER: &str = "rest-smoke-user";
const REST_PASSWORD: &str = "rest-smoke-secret";

/// Minimal stand-in for the engine REST API, backed by the same Postgres
/// engine. The full `flowable-rest` server does boot on Postgres
/// (`postgres_server_boot_test`), but standing one up here would pull the whole
/// REST layer into a test about the admin proxy, so this responder implements
/// exactly the one endpoint the proxy reads, with Basic auth checked strictly so
/// a wrong decrypt of the stored password fails the test.
#[derive(Clone)]
struct EngineRestState {
    engine: Arc<ProcessEngine>,
    suffix: String,
    expected_auth: String,
}

async fn list_deployments_endpoint(
    State(state): State<EngineRestState>,
    headers: HeaderMap,
) -> Response {
    let authorized = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some(state.expected_auth.as_str());
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let deployments = state
        .engine
        .get_repository_service()
        .get_deployments()
        .expect("deployments should list from postgres");
    let data: Vec<Value> = deployments
        .into_iter()
        .filter(|d| {
            d.name
                .as_deref()
                .map(|name| name.contains(&state.suffix))
                .unwrap_or(false)
        })
        .map(|d| json!({ "id": d.id, "name": d.name, "category": d.category }))
        .collect();
    let size = data.len();
    Json(json!({
        "data": data, "total": size, "start": 0, "sort": "id", "order": "asc", "size": size
    }))
    .into_response()
}

/// The admin face against Postgres: `GET /admin-app/rest/admin/deployments`
/// decrypts the ServerConfig password, builds the engine URL, sends Basic auth,
/// and returns rows that live in the pg-backed repository tables.
#[tokio::test]
async fn admin_proxy_list_deployments_reads_postgres_engine() {
    let Some(engine) = build_engine("ui-pg-admin-proxy") else {
        return;
    };
    let suffix = Uuid::new_v4().simple().to_string();
    let key = format!("pg-admin-{suffix}");
    let (deployment_id, _) = deploy_smoke_process(&engine, &key, "nobody");

    // Engine REST responder over the same pg engine.
    let expected_auth = format!("Basic {}", B64.encode(format!("{REST_USER}:{REST_PASSWORD}")));
    let responder = Router::new()
        .route("/repository/deployments", get(list_deployments_endpoint))
        .with_state(EngineRestState {
            engine: Arc::clone(&engine),
            suffix: suffix.clone(),
            expected_auth,
        });
    let engine_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let engine_port = engine_listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(engine_listener, responder).await.unwrap();
    });

    // ServerConfig holding the password encrypted at rest, as the real store does.
    let store = Arc::new(ServerConfigStore::empty_for_tests(Default::default()));
    store
        .save_new(
            ServerConfig {
                id: String::new(),
                name: "pg engine".to_string(),
                description: "pg smoke".to_string(),
                server_address: "http://127.0.0.1".to_string(),
                port: engine_port as i32,
                context_root: String::new(),
                rest_root: String::new(),
                user_name: REST_USER.to_string(),
                password: REST_PASSWORD.to_string(),
                endpoint_type: EndpointType::Process.code(),
                tenant_id: None,
            },
            true,
        )
        .expect("server config should save");

    let admin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let admin_base = format!("http://{}", admin_listener.local_addr().unwrap());
    let admin_app = admin::router_with_state(AdminState::with_store(Arc::clone(&store)));
    tokio::spawn(async move {
        axum::serve(admin_listener, admin_app).await.unwrap();
    });

    let response = reqwest::Client::new()
        .get(format!("{admin_base}/admin-app/rest/admin/deployments"))
        .send()
        .await
        .unwrap();
    // Any decrypt/URL/auth failure surfaces as a proxy error here, not a 200.
    assert_eq!(response.status(), 200, "proxy read should succeed");
    let body: Value = response.json().await.unwrap();
    let names: Vec<&str> = body["data"]
        .as_array()
        .expect("data array")
        .iter()
        .filter_map(|d| d["name"].as_str())
        .collect();
    assert!(
        names.contains(&format!("ui-pg-{key}").as_str()),
        "the deployment written to postgres should come back through the proxy: {names:?}"
    );

    engine
        .get_repository_service()
        .delete_deployment_with_cascade(&deployment_id, true)
        .unwrap();
    let _ = std::fs::remove_file(store.path());
}

/// ServerConfig persistence is a JSON file, not a database table (see the file
/// header), so this case runs without Postgres: it pins that the password is
/// stored encrypted on disk and decrypts back to the plaintext.
#[test]
fn server_config_password_is_encrypted_at_rest_and_round_trips() {
    let store = ServerConfigStore::empty_for_tests(Default::default());
    store
        .save_new(
            ServerConfig {
                id: String::new(),
                name: "at-rest".to_string(),
                description: "crypto smoke".to_string(),
                server_address: "http://127.0.0.1".to_string(),
                port: 9999,
                context_root: String::new(),
                rest_root: String::new(),
                user_name: REST_USER.to_string(),
                password: "s3cret-pg-smoke".to_string(),
                endpoint_type: EndpointType::Process.code(),
                tenant_id: None,
            },
            true,
        )
        .expect("server config should save");

    let on_disk = std::fs::read_to_string(store.path()).expect("persisted JSON file");
    assert!(
        !on_disk.contains("s3cret-pg-smoke"),
        "the plaintext password must never hit the disk: {on_disk}"
    );
    let persisted: Vec<Value> = serde_json::from_str(&on_disk).expect("valid JSON");
    let ciphertext = persisted[0]["password"].as_str().expect("password field");
    assert_eq!(
        store.cipher().decrypt(ciphertext).expect("decrypt"),
        "s3cret-pg-smoke",
        "the stored ciphertext must round trip to the plaintext"
    );

    let _ = std::fs::remove_file(store.path());
}

// ---------------------------------------------------------------------------
// task
// ---------------------------------------------------------------------------

/// The aggregation read path on pg: deploy, start an instance, then filter the
/// shared-schema task table down to this test's rows via the same filters the
/// workflow app uses.
#[tokio::test]
async fn task_query_tasks_reads_runtime_aggregation_from_postgres() {
    let Some((app, cookie)) = spawn_ui("ui-pg-task-query", &["access-task"]).await else {
        return;
    };
    let key = format!("pg-task-{}", app.suffix);
    let (deployment_id, definition_id) = deploy_smoke_process(&app.engine, &key, &app.user_id);
    let instance_id = start_instance(&app.engine, &definition_id);

    // Assignee + processInstanceId + text, the app's own filter combination.
    let body = app
        .query_tasks(
            &cookie,
            json!({
                "assignment": "assignee",
                "processInstanceId": instance_id,
                "text": key,
                "size": 25
            }),
        )
        .await;
    assert_eq!(body["total"], 1, "exactly this test's task: {body}");
    let task = &body["data"][0];
    assert_eq!(task["name"], format!("Review {key}"));
    assert_eq!(task["processInstanceId"], instance_id);
    assert_eq!(task["assignee"]["id"], app.user_id);

    // The text filter narrows the same query to nothing.
    let body = app
        .query_tasks(
            &cookie,
            json!({
                "assignment": "assignee",
                "processInstanceId": instance_id,
                "text": "no-such-task-name",
                "size": 25
            }),
        )
        .await;
    assert_eq!(body["total"], 0);

    app.engine
        .get_repository_service()
        .delete_deployment_with_cascade(&deployment_id, true)
        .unwrap();
    app.cleanup();
}

/// Completing through the UI endpoint removes the row from the runtime
/// aggregation on pg — the complete path deletes the runtime task rather than
/// flagging it, so the read side must reflect that.
#[tokio::test]
async fn completed_task_leaves_the_runtime_aggregation_on_postgres() {
    let Some((app, cookie)) = spawn_ui("ui-pg-task-complete", &["access-task"]).await else {
        return;
    };
    let key = format!("pg-done-{}", app.suffix);
    let (deployment_id, definition_id) = deploy_smoke_process(&app.engine, &key, &app.user_id);
    let instance_id = start_instance(&app.engine, &definition_id);

    let body = app
        .query_tasks(
            &cookie,
            json!({ "assignment": "assignee", "processInstanceId": instance_id, "size": 25 }),
        )
        .await;
    assert_eq!(body["total"], 1);
    let task_id = body["data"][0]["id"].as_str().unwrap().to_string();

    let response = app
        .client
        .put(format!("{}/app/rest/tasks/{task_id}/action/complete", app.base_url))
        .header("cookie", app.cookie_header(&cookie))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "complete should succeed");

    let body = app
        .query_tasks(
            &cookie,
            json!({ "assignment": "assignee", "processInstanceId": instance_id, "size": 25 }),
        )
        .await;
    assert_eq!(body["total"], 0, "a completed task leaves the runtime list");

    app.engine
        .get_repository_service()
        .delete_deployment_with_cascade(&deployment_id, true)
        .unwrap();
    app.cleanup();
}

// ---------------------------------------------------------------------------
// modeler
// ---------------------------------------------------------------------------

/// Editor document save round trip: seed a BPMN model into the pg repository
/// tables, read it as an editor document, write a change back, and verify the
/// persisted source bytes — in the database, not just the response.
#[tokio::test]
async fn modeler_bpmn_editor_document_round_trips_through_postgres() {
    let Some((app, cookie)) = spawn_ui("ui-pg-modeler", &["access-modeler"]).await else {
        return;
    };
    let model_id = format!("model-{}", app.suffix);
    let key = format!("pg-modeler-{}", app.suffix);

    let repository = app.engine.get_repository_service();
    repository
        .create_repository_model(RepositoryModel {
            id: model_id.clone(),
            name: Some(key.clone()),
            key: key.clone(),
            category: None,
            version: 1,
            meta_info: None,
            deployment_id: None,
            resource_name: Some(format!("{key}.bpmn20.xml")),
            process_definition_id: None,
            tenant_id: None,
            create_time: 0,
            last_update_time: 0,
            source_content_type: "application/xml".to_string(),
            source_extra_content_type: "application/json".to_string(),
        })
        .unwrap();
    repository
        .update_repository_model_source(
            &model_id,
            "application/xml".to_string(),
            smoke_bpmn(&key, &app.user_id).into_bytes(),
        )
        .unwrap();

    // Read the stored XML back as an editor document.
    let response = app
        .client
        .get(format!(
            "{}/modeler-app/rest/models/{model_id}/editor/bpmn-json",
            app.base_url
        ))
        .header("cookie", app.cookie_header(&cookie))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "editor read should succeed");
    let mut document: Value = response.json().await.unwrap();
    assert_eq!(document["model"]["processes"][0]["id"], key);

    // Save a change through the editor write path.
    document["model"]["processes"][0]["name"] = json!("Renamed via pg smoke");
    let response = app
        .client
        .put(format!(
            "{}/modeler-app/rest/models/{model_id}/editor/bpmn-json",
            app.base_url
        ))
        .header("cookie", app.cookie_header(&cookie))
        .json(&document)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204, "editor save should succeed");

    // The round trip landed in the pg-backed source bytes.
    let stored = repository.get_repository_model_source(&model_id).unwrap();
    let stored = String::from_utf8(stored.bytes).unwrap();
    assert!(
        stored.contains("name=\"Renamed via pg smoke\""),
        "the new name must persist in the stored BPMN source: {stored}"
    );

    // And reads back through the editor endpoint, not just the repository API.
    let response = app
        .client
        .get(format!(
            "{}/modeler-app/rest/models/{model_id}/editor/bpmn-json",
            app.base_url
        ))
        .header("cookie", app.cookie_header(&cookie))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let document: Value = response.json().await.unwrap();
    assert_eq!(
        document["model"]["processes"][0]["name"],
        "Renamed via pg smoke"
    );

    // The stored model still validates after the save.
    let response = app
        .client
        .post(format!(
            "{}/modeler-app/rest/models/{model_id}/validate",
            app.base_url
        ))
        .header("cookie", app.cookie_header(&cookie))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let validation: Value = response.json().await.unwrap();
    assert_eq!(validation, json!({ "valid": true, "errors": [] }));

    repository.delete_repository_model(&model_id).unwrap();
    app.cleanup();
}
