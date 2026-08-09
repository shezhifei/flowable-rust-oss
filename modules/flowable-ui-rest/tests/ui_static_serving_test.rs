//! Static serving for the copied AngularJS bundles.
//!
//! The load-bearing property is that browser paths match Java's, because
//! `app-cfg.js` derives its REST root from `window.location.pathname` at
//! runtime. Serving `/idm/` from somewhere else silently points the frontend at
//! the wrong REST prefix, so these tests pin the paths rather than just checking
//! that some bytes come back.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::Extension;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_ui_rest::auth::{AuthMode, UiAuthConfig};
use tokio::net::TcpListener;

/// The repository's `ui/legacy`, resolved from the crate directory so the tests
/// do not depend on the working directory.
fn legacy_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|modules| modules.parent())
        .expect("crate sits at modules/flowable-ui-rest")
        .join("ui/legacy")
}

/// Serves the real bundle tree. Auth is disabled: the static paths are gated by
/// privileges in enforced mode, which is covered in the auth tests.
async fn spawn(test_name: &str) -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    // The disabled-mode dev identity is `admin`; the row has to exist or the
    // REST handlers correctly report the user as missing.
    engine
        .get_identity_service()
        .save_user(flowable_engine::identity::entities::User {
            id: "admin".to_string(),
            first_name: Some("Ad".to_string()),
            last_name: Some("Min".to_string()),
            email: Some("admin@example.com".to_string()),
            password: Some("test".to_string()),
            tenant_id: None,
        });
    let config = UiAuthConfig {
        mode: AuthMode::Disabled,
        ..UiAuthConfig::default()
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = flowable_ui_rest::ui_router_with_config_and_static(Arc::new(config), &legacy_root())
        .layer(Extension(engine));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn bundle_entry_points_are_served_at_their_java_paths() {
    let (base_url, client) = spawn("static_entry_points").await;

    for path in ["/idm/", "/admin/", "/"] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path} should serve index.html");
        let body = response.text().await.unwrap();
        assert!(
            body.contains("<html") || body.contains("<!DOCTYPE"),
            "{path} did not return an HTML document"
        );
    }
}

#[tokio::test]
async fn bare_prefix_without_a_trailing_slash_serves_the_app() {
    let (base_url, client) = spawn("static_bare_prefix").await;

    // `nest_service("/idm", …)` does not answer "/idm" on its own; a 404 here
    // means the explicit route for the bare path was lost.
    for path in ["/idm", "/admin"] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path} (no trailing slash) should serve the app");
    }
}

#[tokio::test]
async fn app_cfg_is_served_and_still_derives_the_idm_rest_root() {
    let (base_url, client) = spawn("static_app_cfg").await;

    let body = client
        .get(format!("{base_url}/idm/scripts/app-cfg.js"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // The bundle must be unmodified: it derives contextRoot from the URL, which
    // is what makes serving at /idm/ work without a build step.
    assert!(
        body.contains("contextIdmRestRoot"),
        "app-cfg.js was not served from the idm bundle"
    );
    assert!(
        body.contains("window.location.pathname"),
        "app-cfg.js no longer derives its context root from the URL; serving it \
         under /idm/ would then point the frontend at the wrong REST prefix"
    );
}

#[tokio::test]
async fn a_missing_asset_under_a_prefix_is_404_not_the_shell() {
    let (base_url, client) = spawn("static_missing_asset").await;

    // These are AngularJS 1.x apps: routes live in the fragment (`/idm/#/users`),
    // which never reaches the server. So an unknown path under `/idm/` is a
    // missing asset, and answering it with index.html would turn a broken script
    // reference into an HTML document served as JavaScript.
    let response = client
        .get(format!("{base_url}/idm/no-such-file-here.js"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn rest_routes_are_not_shadowed_by_the_static_fallback() {
    let (base_url, client) = spawn("static_rest_precedence").await;

    // The task bundle owns "/" via fallback_service, so a REST path must still
    // reach its handler rather than being served index.html.
    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/json",
        "a REST route returned the SPA shell, so the static fallback shadowed it"
    );
}

#[tokio::test]
async fn a_missing_static_root_mounts_nothing_and_leaves_rest_working() {
    let engine = Arc::new(ProcessEngine::new("static_absent_root".to_string()));
    engine
        .get_identity_service()
        .save_user(flowable_engine::identity::entities::User {
            id: "admin".to_string(),
            first_name: Some("Ad".to_string()),
            last_name: None,
            email: None,
            password: Some("test".to_string()),
            tenant_id: None,
        });
    let config = UiAuthConfig {
        mode: AuthMode::Disabled,
        ..UiAuthConfig::default()
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = flowable_ui_rest::ui_router_with_config_and_static(
        Arc::new(config),
        &PathBuf::from("no/such/directory"),
    )
    .layer(Extension(engine));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();

    // API-only deployments must still start.
    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let response = client.get(format!("{base_url}/idm/")).send().await.unwrap();
    assert_eq!(response.status(), 404, "no bundle should be mounted");
}

/// The task bundle is mounted entry by entry rather than behind a catch-all
/// fallback, because a fallback would answer every path nothing else claimed and
/// displace the engine API's auth layer. The cost of that choice is a list that
/// can drift: an entry missing from `TASK_ENTRIES` is silently unreachable.
///
/// So walk the real directory and require every top-level name to be served.
#[tokio::test]
async fn every_task_bundle_entry_is_reachable() {
    let bundle = legacy_root().join("task");
    if !bundle.is_dir() {
        eprintln!("skipping: {} not present", bundle.display());
        return;
    }
    let (base_url, client) = spawn("static_task_entry_coverage").await;

    let mut checked = 0;
    for entry in std::fs::read_dir(&bundle).expect("read task bundle") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        // A directory is probed through a real file inside it, not through the
        // directory itself: several of these (`display/`, `display-cmmn/`) have no
        // `index.html`, so a bare directory request correctly 404s and would say
        // nothing about whether the mount exists.
        let path = if entry.path().is_dir() {
            let Some(inner) = first_file_in(&entry.path()) else {
                continue;
            };
            format!("/{name}/{inner}")
        } else {
            format!("/{name}")
        };

        let status = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap()
            .status();
        assert_ne!(
            status, 404,
            "{path} is in ui/legacy/task but not served; add \"{name}\" to TASK_ENTRIES in \
             static_srv.rs"
        );
        checked += 1;
    }
    assert!(checked > 0, "the bundle should not be empty");
}

/// The name of some regular file directly inside `directory`, for probing that a
/// mount resolves. Returns `None` for a directory holding only subdirectories,
/// which the caller skips.
fn first_file_in(directory: &std::path::Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    // Sorted so a failure names the same file every run.
    names.sort();
    names.into_iter().next()
}
