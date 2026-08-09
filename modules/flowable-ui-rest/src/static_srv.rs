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

/// Bundles to mount: (URL prefix, directory under the static root).
///
/// Each directory's own `index.html` is the entry point, which `ServeDir`
/// resolves for a directory request without needing it named here.
const BUNDLES: [(&str, &str); 3] = [
    ("/idm", "idm/idm"),
    ("/admin", "admin/admin"),
    // The task frontend's static tree has no wrapping directory: index.html and
    // scripts/ sit directly under `static/`.
    ("/", "task"),
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

        if prefix == "/" {
            // The task bundle owns the root, so it also answers anything the
            // other prefixes and the REST routes did not claim.
            router = router.fallback_service(service);
        } else {
            // `nest_service` answers both `/idm` and `/idm/…`; adding an
            // explicit route for the bare prefix conflicts with the nested
            // wildcard and panics at construction.
            router = router.nest_service(prefix, service);
        }
    }
    router
}