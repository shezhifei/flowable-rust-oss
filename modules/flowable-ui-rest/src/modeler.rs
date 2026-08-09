//! `/modeler-app/rest/**` — the modeler's REST surface.
//!
//! Placeholder owned by stream C (`docs/plans/2026-08-09-modeler-detailed-plan.md`).
//! Authentication and the `access-modeler` requirement are already enforced for
//! this prefix by [`crate::auth::auth_middleware`]. The modeler's own frontend
//! lives in `ui/modeler/` and is not served by [`crate::static_srv`], which only
//! mounts the three copied legacy bundles.

use axum::Router;

pub fn router() -> Router {
    Router::new()
}