//! Cookie-based authentication for the UI surface.
//!
//! Java blueprint (`$J/modules/flowable-ui/flowable-ui-common/.../security/`):
//!
//! * `FlowableUiCustomFormLoginConfigurer` — `POST /app/authentication` with
//!   form parameters `j_username` / `j_password`.
//! * `AjaxAuthenticationSuccessHandler` — success is `200` with an **empty
//!   body**; `AjaxAuthenticationFailureHandler` — failure is `401` via
//!   `sendError(401, "Authentication failed")`.
//! * `CustomPersistentRememberMeServices` + `IdmEnginePersistentTokenService` —
//!   `FLOWABLE_REMEMBER_ME` cookie carrying `series:tokenValue`, rolled once the
//!   row is older than `refreshAge`, rejected past `maxAge`, and treated as
//!   cookie theft when the series resolves but the value does not match.
//! * `FlowableUiSecurityAutoConfiguration` — the URL→privilege table replicated
//!   in [`required_access`].
//! * `DefaultPrivileges` — privilege *names* are lowercase kebab-case
//!   (`access-idm`), not the Java constant identifiers.
//!
//! Deviations from Java, all deliberate:
//!
//! * Series and token values are URL-safe base64 without padding, so they never
//!   contain the `:` delimiter, `/`, `+` or `=`. Java generates standard base64
//!   and then URL-encodes each half before joining. Tokens are not portable
//!   between the two stacks anyway (no shared database), and this removes the
//!   percent-encoding round trip.
//! * The cookie gets `SameSite=Lax`. Java sets no `SameSite` attribute and thus
//!   inherits the browser default; `Lax` is that default in current browsers and
//!   does not affect the same-origin SPAs.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::{FromRequestParts, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine as _;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::Token;
use rand::RngCore;

use crate::error::UiError;

/// Java `CookieConstants.COOKIE_NAME`.
pub const COOKIE_NAME: &str = "FLOWABLE_REMEMBER_ME";

/// Java `DefaultPrivileges`.
pub const ACCESS_IDM: &str = "access-idm";
pub const ACCESS_MODELER: &str = "access-modeler";
pub const ACCESS_ADMIN: &str = "access-admin";
pub const ACCESS_TASK: &str = "access-task";
pub const ACCESS_REST_API: &str = "access-rest-api";

/// All privileges, granted to the synthetic identity used when authentication is
/// switched off for development.
pub const ALL_PRIVILEGES: [&str; 5] = [
    ACCESS_IDM,
    ACCESS_MODELER,
    ACCESS_ADMIN,
    ACCESS_TASK,
    ACCESS_REST_API,
];

const DELIMITER: char = ':';
const SERIES_LENGTH: usize = 16;
const TOKEN_LENGTH: usize = 16;
const BASE64_URL: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Whether the UI surface enforces authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Enforced,
    /// Development only: every request is treated as the configured dev user
    /// holding every privilege. Streams B and C build against this while the
    /// login flow is still settling.
    Disabled,
}

impl AuthMode {
    pub fn is_enforced(self) -> bool {
        matches!(self, AuthMode::Enforced)
    }
}

/// Runtime configuration for the UI auth layer.
///
/// `flowable-rest` owns its own config struct and depends on this crate, so the
/// values are read from the environment here rather than threaded through a
/// shared type (which would invert the dependency).
#[derive(Debug, Clone)]
pub struct UiAuthConfig {
    pub mode: AuthMode,
    /// Java `flowable.common.app.security.cookie.max-age`, default 31 days.
    pub cookie_max_age: Duration,
    /// Java `…cookie.refresh-age`, default 1 day. A token older than this is
    /// replaced (new series) on the next authenticated request.
    pub cookie_refresh_age: Duration,
    /// Java `…cookie.domain`, unset by default.
    pub cookie_domain: Option<String>,
    /// Identity injected when `mode` is [`AuthMode::Disabled`].
    pub dev_user_id: String,
}

impl Default for UiAuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::Enforced,
            cookie_max_age: Duration::from_secs(31 * 24 * 60 * 60),
            cookie_refresh_age: Duration::from_secs(24 * 60 * 60),
            cookie_domain: None,
            dev_user_id: "admin".to_string(),
        }
    }
}

impl UiAuthConfig {
    /// Reads `FLOWABLE_UI_AUTH_MODE` (`disabled` switches enforcement off; any
    /// other value, including absent, keeps it on), `FLOWABLE_UI_DEV_USER`,
    /// `FLOWABLE_UI_COOKIE_MAX_AGE_SECONDS`,
    /// `FLOWABLE_UI_COOKIE_REFRESH_AGE_SECONDS` and `FLOWABLE_UI_COOKIE_DOMAIN`.
    ///
    /// Enforcement is the default in both directions: an unparseable duration
    /// falls back to the Java default rather than to something permissive.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let dev_user_id = std::env::var("FLOWABLE_UI_DEV_USER")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or(defaults.dev_user_id);
        let mode = match std::env::var("FLOWABLE_UI_AUTH_MODE").as_deref() {
            Ok(value) if value.eq_ignore_ascii_case("disabled") => {
                tracing::warn!(
                    "FLOWABLE_UI_AUTH_MODE=disabled: UI requests run as '{dev_user_id}' with all \
                     privileges. Never use this outside development."
                );
                AuthMode::Disabled
            }
            _ => AuthMode::Enforced,
        };
        Self {
            mode,
            cookie_max_age: env_duration_secs(
                "FLOWABLE_UI_COOKIE_MAX_AGE_SECONDS",
                defaults.cookie_max_age,
            ),
            cookie_refresh_age: env_duration_secs(
                "FLOWABLE_UI_COOKIE_REFRESH_AGE_SECONDS",
                defaults.cookie_refresh_age,
            ),
            cookie_domain: std::env::var("FLOWABLE_UI_COOKIE_DOMAIN")
                .ok()
                .filter(|value| !value.is_empty()),
            dev_user_id,
        }
    }
}

fn env_duration_secs(name: &str, fallback: Duration) -> Duration {
    match std::env::var(name) {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(secs) => Duration::from_secs(secs),
            Err(_) => {
                tracing::warn!(
                    "Ignoring {name}='{raw}': not a whole number of seconds; using {fallback:?}"
                );
                fallback
            }
        },
        Err(_) => fallback,
    }
}

/// The authenticated caller. Equivalent to Java's `SecurityScope`
/// (`FlowableAuthenticationSecurityScope`), which derives group and tenant ids
/// from `GROUP_`/`TENANT_`-prefixed authorities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityScope {
    /// Java `Authentication.getName()`; for this stack the user id.
    pub login: String,
    pub user_id: String,
    pub privileges: Vec<String>,
    pub group_ids: Vec<String>,
    /// Empty string when the user has no tenant, matching Java's
    /// `getTenantId()`, which returns `""` rather than null.
    pub tenant_id: String,
}

impl SecurityScope {
    pub fn has_privilege(&self, privilege: &str) -> bool {
        self.privileges.iter().any(|held| held == privilege)
    }

    fn dev_identity(user_id: &str) -> Self {
        Self {
            login: user_id.to_string(),
            user_id: user_id.to_string(),
            privileges: ALL_PRIVILEGES.iter().map(|p| p.to_string()).collect(),
            group_ids: Vec::new(),
            tenant_id: String::new(),
        }
    }
}

/// Resolves a user's groups and effective privileges.
///
/// Mirrors Java `UserServiceImpl.getUserInformation`: privileges are the union
/// of those granted directly to the user and those granted to any group the
/// user belongs to, keyed by privilege **name**. Returns `None` when the user no
/// longer exists, which is how a token outliving its user is rejected.
pub fn load_scope(engine: &Arc<ProcessEngine>, user_id: &str) -> Option<SecurityScope> {
    let identity = engine.get_identity_service();
    let user = identity.find_user_by_id(user_id)?;

    let groups = identity.get_groups_by_user(&user.id);

    // `get_privileges_for_user` already unions the grants reachable through the
    // user's groups, so the group memberships do not have to be walked again
    // here. Deduplication is by **name**, not id: two privilege rows sharing a
    // name are one permission as far as an authorisation check is concerned,
    // which is what Java's `Set<String>` of names encodes.
    let mut privileges: Vec<String> = identity
        .get_privileges_for_user(&user.id)
        .into_iter()
        .map(|privilege| privilege.name)
        .collect();
    // Java collects privileges into a HashSet, so ordering is unspecified there.
    // Sorting keeps our responses deterministic for contract tests.
    privileges.sort();
    privileges.dedup();

    Some(SecurityScope {
        login: user.id.clone(),
        user_id: user.id,
        privileges,
        group_ids: groups.into_iter().map(|group| group.id).collect(),
        tenant_id: user.tenant_id.unwrap_or_default(),
    })
}

/// Resolves the session carried by a request's remember-me cookie, without
/// rolling or clearing it. Returns `None` when no valid session is present.
///
/// Shared with the engine REST surface (`flowable-rest`), which accepts the UI
/// cookie as an alternative to HTTP Basic: in this stack the static bundles and
/// the engine API share one origin, and the first-party modeler's repository
/// page calls the engine endpoints with `credentials: 'same-origin'`. A cookie
/// whose series resolves but whose value does not match is treated as theft by
/// `resolve_token`, which deletes the row — the same side effect the UI
/// middleware has.
pub fn scope_from_cookie_headers(
    engine: &Arc<ProcessEngine>,
    config: &UiAuthConfig,
    headers: &HeaderMap,
) -> Option<SecurityScope> {
    let cookie_raw = cookie_from_headers(headers, COOKIE_NAME)?;
    let token = resolve_token(engine, config, &cookie_raw).ok()?;
    load_scope(engine, token.user_id.as_deref()?)
}

// ── Cookie codec ──

/// Encodes `series:tokenValue` the way Java's `AbstractRememberMeServices`
/// does: join with `:`, then base64 the joined string.
fn encode_cookie_value(series: &str, token_value: &str) -> String {
    BASE64_URL.encode(format!("{series}{DELIMITER}{token_value}"))
}

/// Inverse of [`encode_cookie_value`]. Rejects anything that is not exactly two
/// non-empty halves (Java raises `InvalidCookieException` for the same cases).
fn decode_cookie_value(raw: &str) -> Option<(String, String)> {
    let decoded = BASE64_URL.decode(raw.as_bytes()).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (series, token_value) = decoded.split_once(DELIMITER)?;
    if series.is_empty() || token_value.is_empty() {
        return None;
    }
    Some((series.to_string(), token_value.to_string()))
}

/// Reads a named cookie out of a `Cookie` header.
///
/// Kept deliberately small rather than pulling in a cookie-parsing crate: the
/// only cookie the UI surface reads is its own, and the grammar for that is
/// `name=value` pairs separated by `; `.
fn cookie_from_headers(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_string())
}

fn random_base64(size: usize) -> String {
    let mut bytes = vec![0u8; size];
    // OsRng draws from the OS CSPRNG, matching Java's SecureRandom.
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    BASE64_URL.encode(bytes)
}

/// Builds the `Set-Cookie` value for a freshly issued token.
///
/// `secure` follows Java's `CustomPersistentRememberMeServices.setCookie`: the
/// `X-Forwarded-Proto` header wins when present, otherwise the transport of the
/// request itself decides.
fn build_set_cookie(config: &UiAuthConfig, series: &str, token_value: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{COOKIE_NAME}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
        encode_cookie_value(series, token_value),
        config.cookie_max_age.as_secs()
    );
    if let Some(domain) = &config.cookie_domain {
        cookie.push_str(&format!("; Domain={domain}"));
    }
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// The cookie that clears the session. Java's `ClearFlowableCookieLogoutHandler`
/// writes an empty value with `Max-Age=0` and `Path=/`.
fn build_clear_cookie(config: &UiAuthConfig, secure: bool) -> String {
    let mut cookie = format!("{COOKIE_NAME}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax");
    if let Some(domain) = &config.cookie_domain {
        cookie.push_str(&format!("; Domain={domain}"));
    }
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Whether the response cookie should carry `Secure`.
fn request_is_secure(headers: &HeaderMap, uri_scheme_is_https: bool) -> bool {
    match headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
    {
        Some(proto) => proto.eq_ignore_ascii_case("https") || uri_scheme_is_https,
        None => uri_scheme_is_https,
    }
}

// ── Token lifecycle ──

fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Issues a token row. The generated series becomes `Token::id`, matching Java
/// `IdmEnginePersistentTokenService.createToken`, which passes the series to
/// `newToken(...)`.
fn create_token(
    engine: &Arc<ProcessEngine>,
    user_id: &str,
    ip_address: Option<String>,
    user_agent: Option<String>,
) -> Token {
    let token = Token {
        id: random_base64(SERIES_LENGTH),
        token_value: random_base64(TOKEN_LENGTH),
        user_id: Some(user_id.to_string()),
        token_date: Some(now_millis()),
        ip_address,
        user_agent,
    };
    engine.get_identity_service().save_token(token.clone());
    token
}

/// Why a presented cookie did not yield a session.
#[derive(Debug, PartialEq, Eq)]
enum TokenRejection {
    /// Malformed cookie, or no row for the presented series.
    Invalid,
    /// Past `cookie_max_age`, or a row with no `token_date` to age.
    Expired,
    /// Series resolved but the value did not match: Java's `CookieTheftException`.
    /// The row is deleted before this is returned.
    Theft,
}

/// Validates a presented cookie, mirroring Java's
/// `CustomPersistentRememberMeServices.getPersistentToken`.
///
/// Java consults a 30-second `LoadingCache` and re-reads past it before
/// declaring theft; there is no cache here, so the first read is already
/// authoritative and the value comparison decides directly.
fn resolve_token(
    engine: &Arc<ProcessEngine>,
    config: &UiAuthConfig,
    cookie_raw: &str,
) -> Result<Token, TokenRejection> {
    let (series, presented_value) =
        decode_cookie_value(cookie_raw).ok_or(TokenRejection::Invalid)?;

    let identity = engine.get_identity_service();
    let token = identity
        .find_token_by_id(&series)
        .ok_or(TokenRejection::Invalid)?;

    if token.token_value != presented_value {
        // Java deletes the row and raises CookieTheftException, invalidating
        // every session on that series.
        identity.delete_token(&token.id);
        tracing::warn!(
            series = %series,
            "Remember-me series/token mismatch; deleting token (possible cookie theft)"
        );
        return Err(TokenRejection::Theft);
    }

    // A row without token_date cannot be aged. Java would NPE here; treating it
    // as expired is the conservative reading and forces a fresh login.
    let issued_at = token.token_date.ok_or(TokenRejection::Expired)?;
    if now_millis().saturating_sub(issued_at) > config.cookie_max_age.as_millis() as i64 {
        return Err(TokenRejection::Expired);
    }

    Ok(token)
}

/// Whether the token is old enough that the next authenticated request should
/// replace it (Java compares against `tokenRefreshDurationInMilliseconds`).
fn needs_roll(token: &Token, config: &UiAuthConfig) -> bool {
    match token.token_date {
        Some(issued_at) => {
            now_millis().saturating_sub(issued_at) > config.cookie_refresh_age.as_millis() as i64
        }
        None => true,
    }
}

/// Replaces a token that has passed `refresh_age` with a freshly issued one.
///
/// Deviation from Java: the superseded row is deleted. Java's `createToken` only
/// inserts, so every roll leaves the previous series valid until `max_age` and
/// accumulates rows for the lifetime of the session. Deleting closes the window
/// in which a previously captured cookie still authenticates, and stops the
/// table growing once per refresh interval per session.
fn roll_token(
    engine: &Arc<ProcessEngine>,
    previous: &Token,
    user_id: &str,
    ip_address: Option<String>,
    user_agent: Option<String>,
) -> Token {
    let replacement = create_token(engine, user_id, ip_address, user_agent);
    engine.get_identity_service().delete_token(&previous.id);
    replacement
}

// ── URL → privilege table ──

/// What a path requires of the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reachable without a session.
    Public,
    /// Any valid session, no specific privilege.
    Authenticated,
    /// A valid session holding this privilege name.
    Privilege(&'static str),
}

/// Replicates `FlowableUiSecurityAutoConfiguration.DEFAULT_AUTHORIZE_REQUESTS`.
///
/// Spring evaluates `antMatchers` top to bottom and stops at the first match, so
/// the order here is load-bearing: `/app/rest/account` must be tested before
/// `/app/rest/**`, and `/app/authentication` before either. Paths matching
/// nothing are permitted, because the Java chain has no `anyRequest()` rule —
/// that is what leaves the static bundles (`/idm/scripts/...`) and `/app/logout`
/// reachable without a session.
pub fn required_access(path: &str) -> Access {
    // permitAll entries. Java lists these last, but they are exact paths that
    // cannot collide with the prefix rules below, so hoisting them costs
    // nothing and keeps the login and logout endpoints unambiguous.
    if path == "/app/authentication" || path == "/idm" {
        return Access::Public;
    }
    // Not in the Java table at all: logout is configured through
    // `LogoutConfigurer`, not `authorizeRequests`, so it falls through to the
    // default-permit branch. Named explicitly so the intent is visible.
    if path == "/app/logout" {
        return Access::Public;
    }

    // authenticated() entries, ahead of the privilege rules they would
    // otherwise be captured by.
    if path == "/app/rest/account"
        || path == "/app/rest/runtime/app-definitions"
        || path == "/idm-app/rest/authenticate"
        || path == "/idm-app/rest/account"
        || path == "/"
    {
        return Access::Authenticated;
    }

    if path.starts_with("/app/rest/") || path == "/workflow/" {
        return Access::Privilege(ACCESS_TASK);
    }
    if path.starts_with("/admin-app/") || path == "/admin/" {
        return Access::Privilege(ACCESS_ADMIN);
    }
    if path.starts_with("/idm-app/") {
        return Access::Privilege(ACCESS_IDM);
    }
    if path.starts_with("/modeler-app/") || path == "/modeler/" {
        return Access::Privilege(ACCESS_MODELER);
    }
    // The modeler's `/api/editor/**` endpoints are not in the Java UI security
    // table either (Java mounts them on a separate servlet with basic auth). In
    // this monolith they live inside `ui_router`, so without an explicit rule
    // they would fall through to Public and be callable without a session.
    if path.starts_with("/api/editor/") {
        return Access::Authenticated;
    }

    Access::Public
}

// ── Middleware and extractor ──

/// Set by [`auth_middleware`] on requests that carried a valid session, and read
/// back by the [`UiAuth`] extractor.
#[derive(Clone)]
struct AuthenticatedScope(SecurityScope);

/// Resolves the session, enforces [`required_access`], and rolls the cookie when
/// it is due.
///
/// Applied once by [`crate::ui_router`] across the whole UI surface, so route
/// modules never attach their own auth layer.
pub async fn auth_middleware(
    State(config): State<Arc<UiAuthConfig>>,
    axum::Extension(engine): axum::Extension<Arc<ProcessEngine>>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let access = required_access(&path);

    if !config.mode.is_enforced() {
        // Development backdoor: inject the dev identity so downstream handlers
        // and privilege checks behave as if a full session were present.
        request
            .extensions_mut()
            .insert(AuthenticatedScope(SecurityScope::dev_identity(
                &config.dev_user_id,
            )));
        return next.run(request).await;
    }

    let secure = request_is_secure(request.headers(), request.uri().scheme() == Some(&axum::http::uri::Scheme::HTTPS));
    let cookie_raw = cookie_from_headers(request.headers(), COOKIE_NAME);

    let mut refreshed_cookie: Option<String> = None;
    let mut scope: Option<SecurityScope> = None;

    if let Some(cookie_raw) = cookie_raw {
        match resolve_token(&engine, &config, &cookie_raw) {
            Ok(token) => {
                let user_id = token.user_id.clone().unwrap_or_default();
                match load_scope(&engine, &user_id) {
                    Some(resolved) => {
                        if needs_roll(&token, &config) {
                            let ip_address = client_ip(&request);
                            let user_agent = header_string(request.headers(), header::USER_AGENT);
                            let replacement =
                                roll_token(&engine, &token, &user_id, ip_address, user_agent);
                            refreshed_cookie = Some(build_set_cookie(
                                &config,
                                &replacement.id,
                                &replacement.token_value,
                                secure,
                            ));
                        }
                        scope = Some(resolved);
                    }
                    None => {
                        // Token outlived its user; drop the row so the stale
                        // cookie stops resolving.
                        engine.get_identity_service().delete_token(&token.id);
                        refreshed_cookie = Some(build_clear_cookie(&config, secure));
                    }
                }
            }
            Err(rejection) => {
                tracing::debug!(?rejection, path = %path, "Rejected remember-me cookie");
                refreshed_cookie = Some(build_clear_cookie(&config, secure));
            }
        }
    }

    // Enforce before running the handler, so an unauthorised request never
    // reaches business logic.
    let denial = match (access, &scope) {
        (Access::Public, _) => None,
        (Access::Authenticated, Some(_)) => None,
        (Access::Authenticated, None) => Some(unauthorized_response()),
        (Access::Privilege(_), None) => Some(unauthorized_response()),
        (Access::Privilege(privilege), Some(scope)) => {
            if scope.has_privilege(privilege) {
                None
            } else {
                Some(
                    UiError::Forbidden(format!("Privilege '{privilege}' required"))
                        .into_response(),
                )
            }
        }
    };

    let mut response = match denial {
        Some(response) => response,
        None => {
            if let Some(scope) = scope {
                request.extensions_mut().insert(AuthenticatedScope(scope));
            }
            next.run(request).await
        }
    };

    if let Some(cookie) = refreshed_cookie
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

/// Java's entry point for XHR callers is `HttpStatusEntryPoint(401)`, i.e. a
/// bare status with no body.
fn unauthorized_response() -> Response {
    StatusCode::UNAUTHORIZED.into_response()
}

fn header_string(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Best-effort client address for the token row's audit column. `ConnectInfo` is
/// not wired into this router, so this reads the usual proxy headers and stores
/// nothing when neither is present.
fn client_ip(request: &Request) -> Option<String> {
    let headers = request.headers();
    if let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    {
        if let Some(first) = forwarded.split(',').next() {
            let first = first.trim();
            if !first.is_empty() {
                return Some(first.to_string());
            }
        }
    }
    header_string(headers, header::HeaderName::from_static("x-real-ip"))
}

/// The authenticated caller, for handlers that need it.
///
/// Extraction only succeeds when [`auth_middleware`] has already accepted the
/// request; handlers on `Access::Public` paths must therefore use
/// `Option<UiAuth>` rather than `UiAuth`.
#[derive(Debug, Clone)]
pub struct UiAuth(pub SecurityScope);

impl UiAuth {
    pub fn scope(&self) -> &SecurityScope {
        &self.0
    }

    pub fn user_id(&self) -> &str {
        &self.0.user_id
    }

    /// Handler-level privilege check, for endpoints whose requirement is finer
    /// grained than the path table.
    pub fn require_privilege(&self, privilege: &str) -> Result<(), UiError> {
        if self.0.has_privilege(privilege) {
            Ok(())
        } else {
            Err(UiError::Forbidden(format!(
                "Privilege '{privilege}' required"
            )))
        }
    }
}

#[async_trait::async_trait]
impl<S> FromRequestParts<S> for UiAuth
where
    S: Send + Sync,
{
    type Rejection = UiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedScope>()
            .map(|scope| UiAuth(scope.0.clone()))
            .ok_or_else(|| UiError::Unauthorized("Request did not contain valid authorization".to_string()))
    }
}

// ── Login / logout ──

/// Java's `UsernamePasswordAuthenticationFilter` parameter names.
#[derive(Debug)]
struct LoginForm {
    username: String,
    password: String,
}

fn parse_login_form(body: &str) -> Option<LoginForm> {
    let mut username = None;
    let mut password = None;
    for (key, value) in serde_urlencoded::from_str::<Vec<(String, String)>>(body).ok()? {
        match key.as_str() {
            "j_username" => username = Some(value),
            "j_password" => password = Some(value),
            _ => {}
        }
    }
    Some(LoginForm {
        username: username?,
        password: password?,
    })
}

/// `POST /app/authentication`.
///
/// Success is `200` with an empty body and a `Set-Cookie`
/// (`AjaxAuthenticationSuccessHandler` only calls `setStatus(SC_OK)`); failure is
/// `401` with the body `Authentication failed`, which is what
/// `response.sendError(401, "Authentication failed")` produces.
async fn login(
    State(config): State<Arc<UiAuthConfig>>,
    axum::Extension(engine): axum::Extension<Arc<ProcessEngine>>,
    request: Request,
) -> Response {
    // `Request` consumes the body, so it must be the last extractor and cannot be
    // combined with a `HeaderMap` one; the headers are read off it directly
    // before the body is taken.
    let secure = request_is_secure(
        request.headers(),
        request.uri().scheme() == Some(&axum::http::uri::Scheme::HTTPS),
    );
    let ip_address = client_ip(&request);
    let user_agent = header_string(request.headers(), header::USER_AGENT);

    let body = match axum::body::to_bytes(request.into_body(), 64 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => return authentication_failed(),
    };
    let body = match std::str::from_utf8(&body) {
        Ok(body) => body,
        Err(_) => return authentication_failed(),
    };
    let Some(form) = parse_login_form(body) else {
        return authentication_failed();
    };
    if form.username.is_empty() || form.password.is_empty() {
        return authentication_failed();
    }

    if !engine
        .get_identity_service()
        .check_password(&form.username, &form.password)
    {
        return authentication_failed();
    }
    // A user with no resolvable scope (deleted between check and load) must not
    // get a session.
    if load_scope(&engine, &form.username).is_none() {
        return authentication_failed();
    }

    let token = create_token(&engine, &form.username, ip_address, user_agent);
    let cookie = build_set_cookie(&config, &token.id, &token.token_value, secure);
    match HeaderValue::from_str(&cookie) {
        Ok(value) => {
            let mut response = StatusCode::OK.into_response();
            response.headers_mut().insert(header::SET_COOKIE, value);
            response
        }
        Err(_) => UiError::Internal("Could not encode session cookie".to_string()).into_response(),
    }
}

fn authentication_failed() -> Response {
    (StatusCode::UNAUTHORIZED, "Authentication failed").into_response()
}

/// `GET|POST /app/logout`.
///
/// Java registers the default `logoutUrl("/app/logout")`, deletes the token row
/// (`CustomPersistentRememberMeServices.logout`), clears the cookie
/// (`ClearFlowableCookieLogoutHandler`) and redirects to `logoutSuccessUrl("/")`.
async fn logout(
    State(config): State<Arc<UiAuthConfig>>,
    axum::Extension(engine): axum::Extension<Arc<ProcessEngine>>,
    request: Request,
) -> Response {
    let secure = request_is_secure(
        request.headers(),
        request.uri().scheme() == Some(&axum::http::uri::Scheme::HTTPS),
    );

    if let Some(cookie_raw) = cookie_from_headers(request.headers(), COOKIE_NAME)
        && let Some((series, _)) = decode_cookie_value(&cookie_raw)
    {
        // Java deletes whatever the series resolves to, without requiring the
        // token value to match.
        engine.get_identity_service().delete_token(&series);
    }

    // 302, not 303: Java's `logoutSuccessUrl("/")` goes through
    // `SimpleUrlLogoutSuccessHandler` → `HttpServletResponse.sendRedirect`,
    // which is a 302 Found. axum 0.7 offers only 303/307/308 constructors, so
    // the status and Location are set by hand.
    let mut response = StatusCode::FOUND.into_response();
    response
        .headers_mut()
        .insert(header::LOCATION, HeaderValue::from_static("/"));
    if let Ok(value) = HeaderValue::from_str(&build_clear_cookie(&config, secure)) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

/// Login and logout endpoints. Merged by [`crate::ui_router`].
pub fn router(config: Arc<UiAuthConfig>) -> Router {
    Router::new()
        .route("/app/authentication", post(login))
        .route("/app/logout", get(logout).post(logout))
        .with_state(config)
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The whole `DEFAULT_AUTHORIZE_REQUESTS` table, path by path.
    ///
    /// Spring stops at the first matching `antMatcher`, so the ordering inside
    /// [`required_access`] is what makes the exact-path rules survive the prefix
    /// rules that would otherwise capture them. That ordering is invisible from
    /// the outside and a reordering would silently downgrade access rather than
    /// fail loudly, which is why the pairs that collide are all listed here.
    #[test]
    fn access_table_matches_the_java_configuration() {
        let cases: &[(&str, Access)] = &[
            // permitAll.
            ("/app/authentication", Access::Public),
            ("/idm", Access::Public),
            ("/app/logout", Access::Public),
            // authenticated(), each of which sits under a prefix rule below.
            ("/app/rest/account", Access::Authenticated),
            ("/app/rest/runtime/app-definitions", Access::Authenticated),
            ("/idm-app/rest/authenticate", Access::Authenticated),
            ("/idm-app/rest/account", Access::Authenticated),
            ("/", Access::Authenticated),
            // Privilege prefixes.
            ("/app/rest/tasks", Access::Privilege(ACCESS_TASK)),
            ("/workflow/", Access::Privilege(ACCESS_TASK)),
            ("/admin-app/rest/server-configs", Access::Privilege(ACCESS_ADMIN)),
            ("/admin/", Access::Privilege(ACCESS_ADMIN)),
            ("/idm-app/rest/admin/users", Access::Privilege(ACCESS_IDM)),
            ("/modeler-app/rest/models", Access::Privilege(ACCESS_MODELER)),
            ("/modeler/", Access::Privilege(ACCESS_MODELER)),
            // Modeler `/api/editor/**` servlet surface (see the rule above).
            ("/api/editor/import-process-model", Access::Authenticated),
            // Everything unlisted, which is how Spring's default-permit branch
            // behaves and how the static assets stay reachable before login.
            ("/scripts/app-cfg.js", Access::Public),
            ("/styles/style.css", Access::Public),
            ("/images/logo.png", Access::Public),
            ("/idm/index.html", Access::Public),
            ("/favicon.ico", Access::Public),
            ("/totally-unknown", Access::Public),
        ];

        for (path, expected) in cases {
            assert_eq!(
                required_access(path),
                *expected,
                "access for {path} does not match the Java table"
            );
        }
    }

    /// `/idm` is public so an anonymous user can reach the login screen, while
    /// `/` is not. Both are exact matches that a careless prefix rule would
    /// swallow, and getting them backwards either locks everyone out of login or
    /// exposes the task app, so they are worth stating on their own.
    #[test]
    fn login_app_is_public_but_the_root_is_not() {
        assert_eq!(required_access("/idm"), Access::Public);
        assert_eq!(required_access("/"), Access::Authenticated);
        // The trailing-slash form is not the permitAll entry; it falls through to
        // the idm prefix rules, which do not match `/idm/`, so it is public too.
        // Stated because the pair looks like an oversight otherwise.
        assert_eq!(required_access("/idm/"), Access::Public);
    }
}
