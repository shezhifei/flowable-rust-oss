//! The legacy bundles as served by the real `run_server` composition.
//!
//! Kept in its own integration-test target because it sets
//! `FLOWABLE_UI_STATIC_DIR`, which is process-global: cargo gives each test file
//! its own process, so the variable cannot leak into tests that expect an
//! unmounted static root. Every test here sets it to the same value before
//! spawning, so the parallel threads inside this file agree.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;

fn legacy_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|modules| modules.parent())
        .expect("crate sits at modules/flowable-rest")
        .join("ui/legacy")
}

async fn spawn(test_name: &str) -> (String, reqwest::Client) {
    // Same value in every test in this file, so concurrent sets are benign.
    unsafe {
        std::env::set_var("FLOWABLE_UI_STATIC_DIR", legacy_root());
    }

    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
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

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn task_bundle_assets_are_served_from_the_root() {
    let (base_url, client) = spawn("static_server_task_root").await;

    // The task frontend's tree sits directly under `static/`, so its assets are
    // root-relative. These are reached through the router's fallback service.
    for (path, expected_type) in [
        ("/index.html", "text/html"),
        ("/scripts/app-cfg.js", "text/javascript"),
    ] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path} should be served");
        assert!(
            response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(expected_type),
            "{path} should be {expected_type}"
        );
    }
}

#[tokio::test]
async fn idm_bundle_is_served_under_its_prefix() {
    let (base_url, client) = spawn("static_server_idm_prefix").await;

    let response = client
        .get(format!("{base_url}/idm/"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert!(
        response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
}

/// Regression guard for the composition: mounting a bundle at `/` must not turn
/// unknown paths into the SPA shell. A wrong `fallback_service` arrangement
/// would answer every 404 with `index.html`, which hides broken asset
/// references and makes missing API routes look like successes.
///
/// These requests carry Basic credentials, so they clear the API's auth layer and
/// reach a genuine 404. Without credentials the same paths are 401 — that half is
/// `ui_surface_wiring_test::unknown_paths_stay_behind_the_api_auth_layer`.
#[tokio::test]
async fn unknown_paths_still_404_rather_than_returning_the_shell() {
    let (base_url, client) = spawn("static_server_unknown_404").await;

    for path in [
        "/repository/nonexistent-endpoint",
        "/runtime/bogus",
        "/totally-unknown",
    ] {
        let response = client
            .get(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{path} should be a 404");
    }
}

/// Java lists `.antMatchers("/").authenticated()`, so the root is not public
/// even though a bundle is mounted there. The login UI lives at `/idm/`, which
/// is reachable anonymously — that is how a user gets a cookie in the first
/// place.
#[tokio::test]
async fn the_root_requires_authentication_but_the_login_app_does_not() {
    let (base_url, client) = spawn("static_server_root_auth").await;

    let response = client.get(format!("{base_url}/")).send().await.unwrap();
    assert_eq!(response.status(), 401, "the root must require a session");

    let response = client
        .get(format!("{base_url}/idm/"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        200,
        "the idm app hosts the login page and must be reachable anonymously"
    );
}
