//! Static asset serving for the four AngularJS bundles.
//!
//! Java packages each frontend as a jar whose `src/main/resources/static/` tree
//! is served off the classpath. The trees are copied verbatim into `ui/legacy/`
//! here, keeping the *browser* paths identical:
//!
//! * `flowable-ui-idm-frontend/.../static/idm/…`      → `/idm/…`
//! * `flowable-ui-admin-frontend/.../static/admin/…`  → `/admin/…`
//! * `flowable-ui-task-frontend/.../static/…`         → `/…` (bundle sits at the root)
//!
//! Keeping the prefixes is what lets the bundles ship unmodified: `app-cfg.js`
//! derives its REST root at runtime with
//! `window.location.pathname.replace(/^(\/[^\/]*)(\/.*)?idm\/?$/, '$1')`, which
//! yields `""` when the app is served from `/idm/`, so `contextIdmRestRoot`
//! resolves to `/idm-app` — exactly where [`crate::idm`] mounts.
//!
//! There is no classpath at runtime, so the tree is located on disk:
//! `FLOWABLE_UI_STATIC_DIR` when set, else `ui/legacy` relative to the working
//! directory. A missing directory logs once and mounts nothing, which keeps
//! tests and API-only deployments from failing over absent assets.
//!
//! One cosmetic difference from Java: each frontend jar carries its own
//! `favicon.ico` and `manifest.json` at the *root* of its static tree, because
//! each app is a separate deployment there. Here the three trees share one
//! origin, so only the task bundle's root-level files are reachable at `/`; the
//! idm and admin copies are shadowed and browsers fall back to the root favicon.

use std::path::{Path, PathBuf};

use axum::Router;
use tower_http::services::ServeDir;

/// Prefixed bundles to mount: (URL prefix, directory under the static root).
///
/// Each directory's own `index.html` is the entry point, which `ServeDir`
/// resolves for a directory request without needing it named here.
const BUNDLES: [(&str, &str); 2] = [("/idm", "idm/idm"), ("/admin", "admin/admin")];

/// The task frontend's static tree has no wrapping directory — `index.html` and
/// `scripts/` sit directly under `static/` — so in Java it is served from the
/// root context and its assets are requested as `/scripts/…`, `/styles/…`, and so
/// on.
///
/// Reproducing that with a root `fallback_service` would be shorter, but a
/// fallback answers *every* path nothing else claimed, and `Router::layer` in
/// axum applies to a router's fallback as well as its routes. The engine API's
/// Basic-auth middleware therefore answers unmatched paths with 401 today, and a
/// root fallback here would replace that with a 404 from `ServeDir` — quietly
/// changing the whole server's behaviour for unknown URLs, and hiding
/// authentication failures behind "not found". `rest_jmx_native_contract_test`
/// pins exactly that for the deprecated `/service/**` prefix.
///
/// So the bundle's own top-level entries are mounted explicitly and nothing else
/// is claimed. This list must track `ui/legacy/task/`; an entry that is absent on
/// disk is skipped, so a stale name here is harmless, while a *missing* name
/// means that asset 404s.
const TASK_BUNDLE: &str = "task";
const TASK_ENTRIES: [&str; 15] = [
    "browserconfig.xml",
    "display",
    "display-cmmn",
    "error",
    "favicon.ico",
    "fonts",
    "i18n",
    "images",
    // Reachable as `/` too, but the explicit URL has to work on its own.
    "index.html",
    "libs",
    "manifest.json",
    "scripts",
    "styles",
    "views",
    "workflow",
];

/// Resolves the directory holding the copied `static/` trees.
pub fn static_root() -> PathBuf {
    std::env::var("FLOWABLE_UI_STATIC_DIR")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ui/legacy"))
}

pub fn router() -> Router {
    router_from(&static_root())
}

/// [`router`] against an explicit root, for tests.
pub fn router_from(root: &Path) -> Router {
    if !root.is_dir() {
        tracing::info!(
            root = %root.display(),
            "UI static root not found; serving REST endpoints only. Set FLOWABLE_UI_STATIC_DIR \
             to serve the bundled frontends."
        );
        return Router::new();
    }

    let mut router = Router::new();
    for (prefix, directory) in BUNDLES {
        let bundle = root.join(directory);
        if !bundle.is_dir() {
            tracing::info!(
                bundle = %bundle.display(),
                prefix = prefix,
                "UI bundle missing; skipping"
            );
            continue;
        }

        // No SPA fallback: these are AngularJS 1.x apps using hash routing, so a
        // deep link is `/idm/#/users` and the browser only ever asks the server
        // for `/idm/`. An unknown path under the prefix is a genuinely missing
        // asset and 404s, which is what Spring's resource handler does too.
        //
        // `ServeDir` resolves a directory request to `index.html` itself, so the
        // entry point needs no separate route.
        let service = ServeDir::new(&bundle).append_index_html_on_directories(true);

        // `nest_service` answers both `/idm` and `/idm/…`; adding an explicit
        // route for the bare prefix conflicts with the nested wildcard and panics
        // at construction.
        router = router.nest_service(prefix, service);
    }

    router.merge(task_router(root))
}

/// Mounts the task bundle's own entries at the root, and `/` itself.
fn task_router(root: &Path) -> Router {
    let bundle = root.join(TASK_BUNDLE);
    if !bundle.is_dir() {
        tracing::info!(
            bundle = %bundle.display(),
            "UI task bundle missing; skipping"
        );
        return Router::new();
    }

    let serve = |directory: &Path| ServeDir::new(directory).append_index_html_on_directories(true);
    // `/` alone, so an unknown root-level path is not swallowed. `ServeDir`
    // resolves the directory request to the bundle's `index.html`.
    let mut router = Router::new().route_service("/", serve(&bundle));

    for entry in TASK_ENTRIES {
        let target = bundle.join(entry);
        if !target.exists() {
            tracing::debug!(entry = entry, "task bundle entry absent; skipping");
            continue;
        }
        let path = format!("/{entry}");
        if target.is_dir() {
            // `nest_service` strips the prefix before the service sees the path,
            // so this `ServeDir` is rooted at the subdirectory, not the bundle.
            router = router.nest_service(&path, serve(&target));
        } else {
            // `route_service` does not strip, so a file route serves from the
            // bundle root and `ServeDir` resolves the name itself.
            router = router.route_service(&path, serve(&bundle));
        }
    }
    router
}