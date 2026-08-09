//! Proves the UI surface is mounted in the real server, and mounted *outside*
//! the engine API's Basic-auth layer.
//!
//! The UI contract itself is tested in `flowable-ui-rest`; what can only be
//! checked here is the composition in `run_server`.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn(test_name: &str) -> (String, reqwest::Client) {
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

/// The A0 acceptance criterion: 401, not 404. A 404 would mean the router was
/// never merged; a 401 means the route exists and its own auth layer rejected an
/// anonymous request.
#[tokio::test]
async fn idm_account_is_mounted_and_returns_401_not_404() {
    let (base_url, client) = spawn("ui_wiring_account").await;

    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        401,
        "a 404 here means the UI router is not merged into the server"
    );
}

/// The UI surface must not be reachable with engine API Basic credentials — it
/// has its own cookie scheme, and sitting behind the API's auth layer would both
/// break the login flow and let API credentials through.
#[tokio::test]
async fn basic_auth_does_not_authenticate_the_ui_surface() {
    let (base_url, client) = spawn("ui_wiring_basic_auth").await;

    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        401,
        "engine API credentials must not grant access to the UI surface"
    );
}

/// The login endpoint has to be reachable anonymously, or nobody could ever
/// obtain a cookie.
#[tokio::test]
async fn login_endpoint_is_reachable_anonymously() {
    let (base_url, client) = spawn("ui_wiring_login").await;

    let response = client
        .post(format!("{base_url}/app/authentication"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body("j_username=admin&j_password=test")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert!(
        response.headers().get("set-cookie").is_some(),
        "login through the real server must issue the remember-me cookie"
    );
}

/// An unknown URL must still be answered by the engine API's auth layer, not by
/// the UI router.
///
/// `Router::layer` wraps a router's fallback as well as its routes, so the
/// Basic-auth layer on `api_routes` is what turns an unmatched path into a 401
/// instead of a bare 404. `Router::merge` adopts the fallback of whichever router
/// merges later, so merging any router without a fallback of its own after
/// `api_routes` silently drops that — every unknown URL becomes a 404 and
/// authentication failures start looking like missing pages. Merging the UI
/// router *before* `api_routes` is what keeps it; this pins that ordering, since
/// nothing about the call site makes the dependency visible.
#[tokio::test]
async fn unknown_paths_stay_behind_the_api_auth_layer() {
    let (base_url, client) = spawn("ui_wiring_unknown_paths").await;

    for path in [
        "/totally-unknown-xyz",
        // The deprecated management prefix, deliberately unregistered.
        "/service/management/jmx/connector-descriptor",
        "/service",
    ] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            401,
            "{path} should be rejected by the API auth layer, not 404'd by the UI router"
        );
    }
}

/// Merging the UI router must not have shadowed the engine API or the health
/// probes.
#[tokio::test]
async fn engine_api_and_health_still_work() {
    let (base_url, client) = spawn("ui_wiring_no_regression").await;

    let response = client
        .get(format!("{base_url}/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let response = client
        .get(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        200,
        "the engine API must still authenticate with Basic credentials"
    );
}
