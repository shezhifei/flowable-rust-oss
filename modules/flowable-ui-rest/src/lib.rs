//! REST and static-asset surface for the Flowable UI applications.
//!
//! This crate serves the four AngularJS apps that Java ships as separate WARs
//! (`idm`, `task`, `admin`, `modeler`) plus the `/…-app/rest/**` endpoints behind
//! them. It is merged into the `flowable-rest` router as its own surface: the
//! engine REST API authenticates with HTTP Basic, whereas everything here uses
//! the `FLOWABLE_REMEMBER_ME` cookie, so the two do not share a middleware.
//!
//! Path layout, taken from the Java servlet registrations
//! (`ApplicationConfiguration` in each `flowable-ui-*-conf` module) and the
//! static resource trees under `flowable-ui-*-frontend`:
//!
//! | Surface  | REST prefix     | Static bundle |
//! |----------|-----------------|---------------|
//! | idm      | `/idm-app`      | `/idm/`       |
//! | task     | `/app`          | `/` (root)    |
//! | admin    | `/admin-app`    | `/admin/`     |
//! | modeler  | `/modeler-app`  | `/modeler/`   |
//!
//! Login and logout live at `/app/authentication` and `/app/logout`, inside the
//! task app's REST prefix; [`auth::required_access`] orders the exact paths ahead
//! of the `/app/rest/**` rule so they stay reachable without a session.

use std::sync::Arc;

use axum::{Router, middleware};

pub mod admin;
pub mod auth;
pub mod error;
pub mod idm;
pub mod modeler;
pub mod static_srv;
pub mod task;

/// Assembles the UI surface.
///
/// Authentication is attached here, once, for every route below: the per-app
/// modules expose bare `router()` functions and never layer their own auth. The
/// [`auth::UiAuth`] extractor reads the scope this layer resolves, so a route
/// module that is merged outside `ui_router` will see extraction fail rather
/// than silently run unauthenticated.
///
/// The caller must have an `Extension<Arc<ProcessEngine>>` in scope for these
/// routes; `flowable-rest` already applies one to the whole application.
pub fn ui_router() -> Router {
    ui_router_with_config(Arc::new(auth::UiAuthConfig::from_env()))
}

/// [`ui_router`] with an explicit config, for tests that need to drive cookie
/// ages or the disabled mode without touching process environment.
pub fn ui_router_with_config(config: Arc<auth::UiAuthConfig>) -> Router {
    ui_router_from_parts(config, static_srv::router())
}

/// [`ui_router_with_config`] with the static root given explicitly, for tests
/// that must not depend on the process working directory.
pub fn ui_router_with_config_and_static(
    config: Arc<auth::UiAuthConfig>,
    static_root: &std::path::Path,
) -> Router {
    ui_router_from_parts(config, static_srv::router_from(static_root))
}

fn ui_router_from_parts(config: Arc<auth::UiAuthConfig>, static_routes: Router) -> Router {
    let routes = Router::new()
        .merge(idm::router())
        .merge(admin::router())
        .merge(task::router())
        .merge(modeler::router())
        .merge(auth::router(Arc::clone(&config)))
        .merge(static_routes);

    // `route_layer`, not `layer`: the latter also wraps the router's fallback,
    // which would make this router carry a layered catch-all. Merged into the
    // engine API's app that catch-all wins over the API's own, so every unknown
    // URL would be answered here — passing the UI auth check, since unmatched
    // paths map to `Public` — instead of by the API's Basic-auth layer, turning
    // its 401 into a bare 404. `route_layer` runs only for paths this router
    // actually claims, which is all the UI surface needs.
    routes.route_layer(middleware::from_fn_with_state(
        config,
        auth::auth_middleware,
    ))
}
