//! The UI surface against a real PostgreSQL backend.
//!
//! The idm endpoints and the remember-me token lifecycle both lean on identity
//! persistence, and the token row in particular is written through
//! `insert_with_extra` with projection columns. SQLite is forgiving about column
//! types and null handling in ways Postgres is not, so the login/roll/logout
//! cycle is worth exercising on the real thing.
//!
//! Requires the `postgres` feature and a reachable server; the URL comes from
//! `FLOWABLE_TEST_POSTGRES_URL`, defaulting to the same value the engine's own
//! Postgres tests use. Tests **skip gracefully** when the database is down so a
//! default `cargo test` without Postgres still passes.
//!
//! ```powershell
//! cargo test -p flowable-ui-rest --features postgres --test ui_postgres_smoke_test
//! ```
//!
//! Unlike the SQLite suites, every test here shares one schema. Rather than
//! serialising on a mutex — which a `#[tokio::test]` cannot hold across an
//! await — each test works under its own id suffix and deletes its rows at the
//! end, so concurrent runs never collide and assertions filter to the test's own
//! data instead of counting the whole table.

#![cfg(feature = "postgres")]

use std::sync::{Arc, OnceLock};

use axum::extract::Extension;
use flowable_engine::engine::identity_service::IdentityService;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::identity::entities::{Privilege, User};
use flowable_engine::service::config::{
    DatabaseConfiguration, EngineDatabaseKind, ProcessEngineConfiguration,
};
use flowable_ui_rest::auth::UiAuthConfig;
use serde_json::Value;
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
            "ui-pg-availability-probe".to_string(),
            Arc::new(SystemTimeSource),
            postgres_config(1),
        ) {
            Ok(_) => true,
            Err(error) => {
                eprintln!(
                    "Skipping UI PostgreSQL smoke tests: database unreachable ({error}). Set \
                     FLOWABLE_TEST_POSTGRES_URL to a live instance to run them."
                );
                false
            }
        }
    })
}

/// A served UI surface over Postgres, plus the ids this test owns.
struct TestApp {
    engine: Arc<ProcessEngine>,
    base_url: String,
    client: reqwest::Client,
    /// The signed-in user, unique per test.
    admin_id: String,
    privilege_id: String,
    /// Distinguishes this test's users in list responses.
    suffix: String,
    /// Rows to remove in [`TestApp::cleanup`].
    users: Vec<String>,
    tokens: Vec<String>,
}

impl TestApp {
    fn identity(&self) -> Arc<IdentityService> {
        self.engine.get_identity_service()
    }

    fn cookie_header(&self, cookie: &str) -> String {
        format!("FLOWABLE_REMEMBER_ME={cookie}")
    }

    /// Logs `admin_id` in and returns the remember-me cookie value, recording the
    /// series so cleanup can drop the row.
    async fn login(&mut self) -> String {
        let response = self
            .client
            .post(format!("{}/app/authentication", self.base_url))
            .header("content-type", "application/x-www-form-urlencoded")
            .header("user-agent", "smoke-test-agent/1.0")
            .body(format!("j_username={}&j_password=test", self.admin_id))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "login should succeed");
        let cookie = remember_me_cookie(&response);
        self.tokens.push(series_of(&cookie));
        cookie
    }

    /// Removes everything this test wrote, so repeated runs against a shared
    /// database do not accumulate rows.
    ///
    /// Called explicitly at the end of each test rather than from `Drop`: a
    /// failing assertion unwinds past it, which is what you want here. The rows
    /// are still there to inspect when something breaks, and the next run's ids
    /// are fresh anyway, so nothing collides in the meantime.
    fn cleanup(&self) {
        let identity = self.identity();
        for series in &self.tokens {
            identity.delete_token(series);
        }
        identity.delete_user_privilege_mapping(&self.privilege_id, &self.admin_id);
        identity.delete_privilege(&self.privilege_id);
        for user in &self.users {
            identity.delete_user(user);
        }
    }
}

/// Spawns the UI router over a Postgres-backed engine, or `None` when the
/// database is unreachable so the suite skips rather than fails.
async fn spawn(test_name: &str) -> Option<TestApp> {
    if !postgres_available() {
        return None;
    }

    let engine = match ProcessEngine::build_with_config(
        format!("{test_name}-{}", Uuid::new_v4().simple()),
        Arc::new(SystemTimeSource),
        postgres_config(4),
    ) {
        Ok(engine) => Arc::new(engine),
        Err(error) => {
            eprintln!("Skipping UI PostgreSQL test '{test_name}': {error}");
            return None;
        }
    };

    let suffix = Uuid::new_v4().simple().to_string();
    let admin_id = format!("admin-{suffix}");
    let privilege_id = format!("priv-idm-{suffix}");

    let identity = engine.get_identity_service();
    identity.save_user(User {
        id: admin_id.clone(),
        first_name: Some("Ad".to_string()),
        last_name: Some(suffix.clone()),
        email: Some(format!("{admin_id}@example.com")),
        password: Some("test".to_string()),
        tenant_id: None,
    });
    identity.save_privilege(Privilege {
        id: privilege_id.clone(),
        // The name is what the privilege check reads, so it cannot be uniquified.
        name: "access-idm".to_string(),
    });
    identity.add_user_privilege_mapping(privilege_id.clone(), admin_id.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = flowable_ui_rest::ui_router_with_config(Arc::new(UiAuthConfig::default()))
        .layer(Extension(Arc::clone(&engine)));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Some(TestApp {
        engine,
        base_url,
        client: reqwest::Client::new(),
        users: vec![admin_id.clone()],
        admin_id,
        privilege_id,
        suffix,
        tokens: Vec::new(),
    })
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
    use base64::Engine as _;
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

#[tokio::test]
async fn login_and_authenticated_request_work_against_postgres() {
    let Some(mut app) = spawn("ui-pg-login").await else {
        return;
    };
    let cookie = app.login().await;

    // Reads the token row back out of Postgres and resolves the scope.
    let response = app
        .client
        .get(format!("{}/idm-app/rest/account", app.base_url))
        .header("cookie", app.cookie_header(&cookie))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], app.admin_id);
    assert_eq!(body["privileges"], serde_json::json!(["access-idm"]));

    // A privilege-gated endpoint, exercising the mapping tables. Filtered to this
    // test's own users because the schema is shared.
    let response = app
        .client
        .get(format!(
            "{}/idm-app/rest/admin/users?filter={}",
            app.base_url, app.suffix
        ))
        .header("cookie", app.cookie_header(&cookie))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], app.admin_id);

    app.cleanup();
}

#[tokio::test]
async fn logout_deletes_the_token_row_in_postgres() {
    let Some(mut app) = spawn("ui-pg-logout").await else {
        return;
    };
    let cookie = app.login().await;
    let series = series_of(&cookie);

    assert!(
        app.identity().find_token_by_id(&series).is_some(),
        "the token row should exist in postgres after login"
    );

    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = no_redirect
        .get(format!("{}/app/logout", app.base_url))
        .header("cookie", app.cookie_header(&cookie))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 302);

    assert!(
        app.identity().find_token_by_id(&series).is_none(),
        "logout must delete the row, not just clear the cookie"
    );

    app.cleanup();
}

/// The projection columns on the token row are what the remember-me fields ride
/// in; a type or nullability mismatch would only show up on a real backend.
#[tokio::test]
async fn token_remember_me_fields_round_trip_through_postgres() {
    let Some(mut app) = spawn("ui-pg-token-fields").await else {
        return;
    };
    let cookie = app.login().await;

    let token = app
        .identity()
        .find_token_by_id(&series_of(&cookie))
        .expect("token row");

    assert_eq!(token.user_id.as_deref(), Some(app.admin_id.as_str()));
    assert!(token.token_date.is_some(), "issue time must persist");
    assert_eq!(token.user_agent.as_deref(), Some("smoke-test-agent/1.0"));

    app.cleanup();
}

#[tokio::test]
async fn user_crud_round_trips_through_postgres() {
    let Some(mut app) = spawn("ui-pg-user-crud").await else {
        return;
    };
    let cookie = app.login().await;
    let dave = format!("dave-{}", app.suffix);
    app.users.push(dave.clone());

    let response = app
        .client
        .post(format!("{}/idm-app/rest/admin/users", app.base_url))
        .header("cookie", app.cookie_header(&cookie))
        .json(&serde_json::json!({
            "id": dave, "firstName": "Dave", "lastName": app.suffix,
            "email": format!("{dave}@example.com"), "password": "secret", "tenantId": "acme"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // The hash survived a real round trip, so the user can authenticate.
    assert!(app.identity().check_password(&dave, "secret"));

    let response = app
        .client
        .put(format!("{}/idm-app/rest/admin/users/{dave}", app.base_url))
        .header("cookie", app.cookie_header(&cookie))
        .json(&serde_json::json!({
            "firstName": "David", "email": format!("d-{dave}@example.com")
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let updated = app.identity().find_user_by_id(&dave).expect("user");
    assert_eq!(updated.first_name.as_deref(), Some("David"));
    // Java calls every setter unconditionally, so an omitted field is written as
    // null rather than left alone. Worth pinning on a real backend: the column
    // has to actually accept the null, not just the in-memory struct.
    assert_eq!(
        updated.tenant_id, None,
        "an omitted field is overwritten with null, matching Java"
    );

    // Regression cover on a real backend: an unrelated update must not clear the
    // password.
    assert!(
        app.identity().check_password(&dave, "secret"),
        "updating a user must not wipe their password"
    );

    let response = app
        .client
        .delete(format!("{}/idm-app/rest/admin/users/{dave}", app.base_url))
        .header("cookie", app.cookie_header(&cookie))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(app.identity().find_user_by_id(&dave).is_none());

    app.cleanup();
}
