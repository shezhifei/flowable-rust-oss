//! `/app/rest/**` — the task app's aggregation layer.
//!
//! Placeholder owned by stream B (`docs/plans/2026-08-09-ui-apps-detailed-plan.md`).
//! Note that this prefix also hosts the login and logout endpoints
//! ([`crate::auth`]); [`crate::auth::required_access`] tests those exact paths
//! before the `/app/rest/**` privilege rule, so adding routes here does not
//! shadow them. `access-task` is enforced for everything except
//! `/app/rest/account` and `/app/rest/runtime/app-definitions`, which Java
//! leaves at plain `authenticated()`.

use axum::Router;

pub fn router() -> Router {
    Router::new()
}