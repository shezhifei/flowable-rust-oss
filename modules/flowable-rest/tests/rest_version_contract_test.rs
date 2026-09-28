// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

//! Contract tests for dual-version API detection (`middleware::version_detection`).
//!
//! These exercise the middleware inside the real `run_server` stack rather than
//! an isolated router, asserting the externally observable contract:
//!
//! - every `api_routes` response echoes the resolved generation in
//!   `x-flowable-api-version`;
//! - detection runs ahead of Basic auth, so even 401s carry version context;
//! - unknown/missing headers fall back to modern 8.x semantics;
//! - `/health` and `/ready` sit outside the versioned surface and stay bare.

use axum::http::StatusCode;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use std::sync::Arc;
use tokio::net::TcpListener;

const VERSION_HEADER: &str = "X-Flowable-API-Version";
const ECHO_HEADER: &str = "x-flowable-api-version";

async fn spawn_server(test_name: &str) -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()).unwrap());
    engine
        .get_identity_service()
        .save_user(User {
            id: "admin".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: Some("test".to_string()),
            tenant_id: None,
        })
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

fn echo_of(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(ECHO_HEADER)
        .map(|value| value.to_str().unwrap().to_string())
}

async fn get_definitions(
    client: &reqwest::Client,
    base_url: &str,
    version: Option<&str>,
) -> reqwest::Response {
    let mut request = client
        .get(format!(
            "{base_url}/repository/process-definitions?start=0&size=1"
        ))
        .basic_auth("admin", Some("test"));
    if let Some(value) = version {
        request = request.header(VERSION_HEADER, value);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn authenticated_requests_echo_default_modern_version() {
    let (base_url, client) = spawn_server("versionContractDefault").await;

    let response = get_definitions(&client, &base_url, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(echo_of(&response).as_deref(), Some("8.0"));
}

#[tokio::test]
async fn legacy_version_header_is_echoed_on_success() {
    let (base_url, client) = spawn_server("versionContractLegacy").await;

    let response = get_definitions(&client, &base_url, Some("6.8")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(echo_of(&response).as_deref(), Some("6.8"));
}

#[tokio::test]
async fn unknown_versions_fall_back_to_modern_semantics() {
    let (base_url, client) = spawn_server("versionContractFallback").await;

    for requested in ["7.5", "", "not-a-version"] {
        let response = get_definitions(&client, &base_url, Some(requested)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            echo_of(&response).as_deref(),
            Some("8.0"),
            "requested {requested:?} must resolve to modern default"
        );
    }
}

#[tokio::test]
async fn detection_precedes_authentication_on_rejected_requests() {
    let (base_url, client) = spawn_server("versionContractUnauthorized").await;

    let unauthorized = client
        .get(format!("{base_url}/repository/process-definitions"))
        .header(VERSION_HEADER, "6.8")
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let body = unauthorized.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["code"], "UNAUTHORIZED");
}

#[tokio::test]
async fn detection_carries_legacy_context_through_auth_rejection() {
    let (base_url, client) = spawn_server("versionContractLegacyRejection").await;

    let mut unauthorized = client
        .get(format!("{base_url}/repository/process-definitions"))
        .header(VERSION_HEADER, "6.8")
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(echo_of(&unauthorized).as_deref(), Some("6.8"));

    // Sanity: without the header the same rejection carries no stale context.
    unauthorized = client
        .get(format!("{base_url}/repository/process-definitions"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(echo_of(&unauthorized).as_deref(), Some("8.0"));
}

#[tokio::test]
async fn health_endpoints_sit_outside_the_versioned_surface() {
    let (base_url, client) = spawn_server("versionContractHealthBoundary").await;

    for path in ["/health", "/ready"] {
        let response = client
            .get(format!("{base_url}{path}"))
            .header(VERSION_HEADER, "6.8")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            echo_of(&response),
            None,
            "{path} is outside api_routes and must not carry version context"
        );
    }
}
