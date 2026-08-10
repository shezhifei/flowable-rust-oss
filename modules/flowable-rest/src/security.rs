use crate::{config::RestAuthConfig, error::ApiError};
use axum::{
    extract::{ConnectInfo, Extension, Request, State},
    http::{Method, header},
    middleware::Next,
    response::Response,
};
use base64::Engine;
use dashmap::DashMap;
use flowable_engine::engine::process_engine::ProcessEngine;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Failed-authentication lockout (M2): after `MAX_FAILURES_PER_WINDOW` failed
/// Basic-auth attempts from one client IP within `FAILURE_WINDOW`, further
/// attempts from that IP are refused with HTTP 429 until the window rolls.
/// This turns online password brute-force (now against argon2id hashes, see
/// C1) into an offline-impractical path while leaving legit clients untouched.
const FAILURE_WINDOW: Duration = Duration::from_secs(300);
const MAX_FAILURES_PER_WINDOW: u32 = 30;
/// Opportunistic cleanup threshold: sweep expired entries once the map grows
/// past this (unique client IPs with recorded failures).
const SWEEP_THRESHOLD: usize = 2048;

struct AuthFailureWindow {
    failures: DashMap<String, (Instant, u32)>,
    window: Duration,
}

impl AuthFailureWindow {
    fn new(window: Duration) -> Self {
        Self {
            failures: DashMap::new(),
            window,
        }
    }

    /// Record a failed attempt for `key`; returns true when the client is now
    /// over the limit and must be refused.
    fn record_failure(&self, key: &str) -> bool {
        if self.failures.len() > SWEEP_THRESHOLD {
            self.failures
                .retain(|_, (started_at, _)| started_at.elapsed() <= self.window);
        }
        let now = Instant::now();
        let mut entry = self.failures.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) > self.window {
            *entry = (now, 1);
        } else {
            entry.1 += 1;
        }
        let over_limit = entry.1 > MAX_FAILURES_PER_WINDOW;
        drop(entry);
        over_limit
    }
}

static AUTH_FAILURE_WINDOW: OnceLock<AuthFailureWindow> = OnceLock::new();

fn auth_failure_window() -> &'static AuthFailureWindow {
    AUTH_FAILURE_WINDOW.get_or_init(|| AuthFailureWindow::new(FAILURE_WINDOW))
}

/// Client key for the failure window: the peer IP from ConnectInfo. Falls back
/// to "unknown" only when the server was started without connect-info (the
/// production/test paths all use `into_make_service_with_connect_info`).
fn client_failure_key(req: &Request) -> String {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Middleware state: the engine API's own auth config plus the UI auth config
/// needed to resolve remember-me cookies.
#[derive(Debug, Clone)]
pub struct RestSecurityState {
    pub auth: RestAuthConfig,
    pub ui_auth: Arc<flowable_ui_rest::auth::UiAuthConfig>,
}

impl RestSecurityState {
    pub fn from_auth_config(auth: RestAuthConfig) -> Self {
        Self {
            auth,
            ui_auth: Arc::new(flowable_ui_rest::auth::UiAuthConfig::from_env()),
        }
    }
}

pub async fn auth_middleware(
    State(state): State<Arc<RestSecurityState>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let auth = &state.auth;
    if !auth.mode.is_enforced() {
        return Ok(next.run(req).await);
    }

    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    if !auth_header.starts_with("Basic ") {
        // SSO with the UI surface: the static bundles and the engine API share
        // one origin in this stack, and the first-party modeler's repository
        // page calls engine endpoints with the session cookie instead of Basic
        // credentials. A presented UI session authenticates the request with
        // the same strength as a password check (the cookie was issued by one).
        if let Some(scope) =
            flowable_ui_rest::auth::scope_from_cookie_headers(&engine, &state.ui_auth, req.headers())
        {
            if requires_admin(req.method(), req.uri().path()) && !auth.is_admin_user(&scope.user_id)
            {
                return Err(ApiError::Forbidden(
                    "Admin privileges required for this operation".to_string(),
                ));
            }
            return Ok(next.run(req).await);
        }
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
        // Count only real credential-verification failures (the expensive
        // argon2id path), not malformed-header rejections above.
        let client_key = client_failure_key(&req);
        if auth_failure_window().record_failure(&client_key) {
            return Err(ApiError::RateLimited(
                "Too many failed authentication attempts; try again later".to_string(),
            ));
        }
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
/// - Management writes (under `/management`, `/cmmn-management`,
///   `/event-registry-management`, and the read-only management families
///   `/app-management`, `/dmn-management`, `/idm-management` — the latter
///   three are gated pre-emptively so future write routes under them never
///   silently lose the gate)
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

    const ADMIN_PREFIXES: [&str; 7] = [
        "/idm",
        "/management",
        "/cmmn-management",
        "/event-registry-management",
        "/app-management",
        "/dmn-management",
        "/idm-management",
    ];
    ADMIN_PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn failure_window_locks_out_after_limit() {
        let window = AuthFailureWindow::new(FAILURE_WINDOW);
        for _ in 0..(MAX_FAILURES_PER_WINDOW as usize) {
            assert!(!window.record_failure("10.0.0.1"));
        }
        assert!(window.record_failure("10.0.0.1"));
        // Other clients are unaffected.
        assert!(!window.record_failure("10.0.0.2"));
    }

    #[test]
    fn failure_window_expires_and_resets() {
        let window = AuthFailureWindow::new(Duration::from_millis(20));
        for _ in 0..(MAX_FAILURES_PER_WINDOW as usize) {
            window.record_failure("10.0.0.1");
        }
        assert!(window.record_failure("10.0.0.1"));
        thread::sleep(Duration::from_millis(40));
        assert!(!window.record_failure("10.0.0.1"));
    }

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

    #[test]
    fn event_registry_management_writes_require_admin() {
        assert!(requires_admin(
            &Method::POST,
            "/event-registry-management/event-deliveries/d-1/retry"
        ));
        assert!(requires_admin(
            &Method::DELETE,
            "/event-registry-management/event-deliveries/d-1"
        ));
        assert!(requires_admin(
            &Method::DELETE,
            "/event-registry-management/event-instance-deliveries/d-1"
        ));
    }

    #[test]
    fn event_registry_management_reads_do_not_require_admin() {
        assert!(!requires_admin(
            &Method::GET,
            "/event-registry-management/event-instance-deliveries"
        ));
        assert!(!requires_admin(
            &Method::GET,
            "/event-registry-management/engine"
        ));
    }

    #[test]
    fn read_only_management_families_are_gated_preemptively() {
        assert!(requires_admin(&Method::POST, "/app-management/engine"));
        assert!(requires_admin(&Method::POST, "/dmn-management/engine"));
        assert!(requires_admin(&Method::POST, "/idm-management/engine"));
        // Reads remain open (beyond auth) — only writes require admin.
        assert!(!requires_admin(&Method::GET, "/idm-management/engine"));
    }
}
