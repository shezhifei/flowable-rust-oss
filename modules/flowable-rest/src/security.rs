use crate::{config::RestAuthConfig, error::ApiError};
use axum::{
    extract::{Extension, Request, State},
    http::{Method, header},
    middleware::Next,
    response::Response,
};
use base64::Engine;
use flowable_engine::engine::process_engine::ProcessEngine;
use std::sync::Arc;

pub async fn auth_middleware(
    State(auth): State<Arc<RestAuthConfig>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if !auth.mode.is_enforced() {
        return Ok(next.run(req).await);
    }

    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    if !auth_header.starts_with("Basic ") {
        return Err(ApiError::Unauthorized);
    }

    let encoded = auth_header.trim_start_matches("Basic ");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::Unauthorized)?;
    let decoded = String::from_utf8(decoded).map_err(|_| ApiError::Unauthorized)?;

    let mut parts = decoded.splitn(2, ':');
    let user_id = parts.next().unwrap_or_default();
    let password = parts.next().unwrap_or_default();
    if user_id.is_empty() || password.is_empty() {
        return Err(ApiError::Unauthorized);
    }

    if !engine
        .get_identity_service()
        .check_password(user_id, password)
    {
        return Err(ApiError::Unauthorized);
    }

    // Privileged write paths require an admin user from the configured list.
    if requires_admin(req.method(), req.uri().path()) && !auth.is_admin_user(user_id) {
        return Err(ApiError::Forbidden(
            "Admin privileges required for this operation".to_string(),
        ));
    }

    Ok(next.run(req).await)
}

/// Paths that only admins may write to. GET/HEAD reads are unrestricted (beyond auth).
///
/// Covered:
/// - Deployment writes (POST/PUT/DELETE/PATCH at or under every
///   `*-repository/deployments` base — includes DELETE `…/deployments/{id}`)
/// - IDM writes (under `/idm`)
/// - Management writes (under `/management` and `/cmmn-management`)
pub fn requires_admin(method: &Method, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    let is_write = matches!(
        *method,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    );
    if !is_write {
        return false;
    }

    const DEPLOYMENT_BASES: [&str; 5] = [
        "/repository/deployments",
        "/cmmn-repository/deployments",
        "/dmn-repository/deployments",
        "/event-registry-repository/deployments",
        "/app-repository/deployments",
    ];
    if DEPLOYMENT_BASES
        .iter()
        .any(|base| path == *base || path.starts_with(&format!("{base}/")))
    {
        return true;
    }

    const ADMIN_PREFIXES: [&str; 3] = ["/idm", "/management", "/cmmn-management"];
    ADMIN_PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_post_requires_admin() {
        assert!(requires_admin(&Method::POST, "/repository/deployments"));
        assert!(requires_admin(
            &Method::POST,
            "/cmmn-repository/deployments"
        ));
        assert!(requires_admin(&Method::POST, "/dmn-repository/deployments"));
        assert!(requires_admin(
            &Method::POST,
            "/event-registry-repository/deployments"
        ));
    }

    #[test]
    fn deployment_get_does_not_require_admin() {
        assert!(!requires_admin(&Method::GET, "/repository/deployments"));
        assert!(!requires_admin(
            &Method::GET,
            "/cmmn-repository/deployments"
        ));
    }

    #[test]
    fn idm_and_management_writes_require_admin() {
        assert!(requires_admin(&Method::POST, "/idm/users"));
        assert!(requires_admin(&Method::PUT, "/idm/users/u1"));
        assert!(requires_admin(&Method::DELETE, "/idm/groups/g1"));
        assert!(requires_admin(
            &Method::POST,
            "/management/jobs/job-1"
        ));
        assert!(requires_admin(
            &Method::DELETE,
            "/management/jobs/job-1"
        ));
    }

    #[test]
    fn idm_and_management_gets_do_not_require_admin() {
        assert!(!requires_admin(&Method::GET, "/idm/users"));
        assert!(!requires_admin(&Method::GET, "/management/jobs"));
        assert!(!requires_admin(&Method::GET, "/management/engine"));
    }

    #[test]
    fn ordinary_runtime_writes_do_not_require_admin() {
        assert!(!requires_admin(
            &Method::POST,
            "/runtime/process-instances"
        ));
        assert!(!requires_admin(&Method::POST, "/runtime/tasks/t1"));
    }

    #[test]
    fn deployment_writes_below_the_base_require_admin() {
        assert!(requires_admin(
            &Method::DELETE,
            "/repository/deployments/dep-1"
        ));
        assert!(requires_admin(&Method::POST, "/app-repository/deployments"));
        assert!(requires_admin(
            &Method::DELETE,
            "/app-repository/deployments/dep-1"
        ));
        assert!(requires_admin(
            &Method::DELETE,
            "/cmmn-repository/deployments/dep-1"
        ));
    }

    #[test]
    fn deployment_reads_below_the_base_do_not_require_admin() {
        assert!(!requires_admin(
            &Method::GET,
            "/repository/deployments/dep-1"
        ));
        assert!(!requires_admin(
            &Method::GET,
            "/app-repository/deployments/dep-1/resources"
        ));
    }

    #[test]
    fn cmmn_management_writes_require_admin() {
        assert!(requires_admin(&Method::POST, "/cmmn-management/jobs"));
        assert!(requires_admin(
            &Method::DELETE,
            "/cmmn-management/jobs/job-1"
        ));
        assert!(requires_admin(
            &Method::PUT,
            "/cmmn-management/timer-jobs/job-1"
        ));
    }

    #[test]
    fn cmmn_management_reads_do_not_require_admin() {
        assert!(!requires_admin(&Method::GET, "/cmmn-management/jobs"));
        assert!(!requires_admin(
            &Method::GET,
            "/cmmn-management/jobs/job-1"
        ));
    }
}
