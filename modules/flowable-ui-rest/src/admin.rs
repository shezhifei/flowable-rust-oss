//! `/admin-app/rest/**` — the admin app's proxy layer.
//!
//! Placeholder owned by stream B (`docs/plans/2026-08-09-ui-apps-detailed-plan.md`).
//! Authentication and the `access-admin` requirement are already enforced for
//! this prefix by [`crate::auth::auth_middleware`], so handlers added here can
//! take [`crate::auth::UiAuth`] and assume a valid session.

use axum::Router;

pub fn router() -> Router {
    Router::new()
}