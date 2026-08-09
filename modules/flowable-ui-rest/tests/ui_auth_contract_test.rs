//! Contract tests for the UI auth surface: login, the remember-me cookie,
//! rolling refresh, logout, and privilege enforcement.
//!
//! Java blueprint: `SecurityConfiguration` /
//! `CustomPersistentRememberMeServices` / `AjaxAuthenticationSuccessHandler`.
//!
//! The client deliberately has no cookie jar — every request sets `Cookie` by
//! hand so the assertions are about the wire format, not about what reqwest
//! chose to persist.

use std::sync::Arc;

use axum::extract::Extension;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::{Group, Privilege, User};
use flowable_ui_rest::auth::{AuthMode, UiAuthConfig};
use tokio::net::TcpListener;

/// Spawns the UI router over a fresh in-memory engine holding one user.
///
/// `admin`/`test` exists in every fixture; privileges are added per test.
async fn spawn(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    spawn_with_config(test_name, UiAuthConfig::default()).await
}

async fn spawn_with_config(
    test_name: &str,
    config: UiAuthConfig,
) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    engine.get_identity_service().save_user(User {
        id: "admin".to_string(),
        first_name: Some("Ad".to_string()),
        last_name: Some("Min".to_string()),
        email: Some("admin@example.com".to_string()),
        password: Some("test".to_string()),
        tenant_id: None,
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let app = flowable_ui_rest::ui_router_with_config(Arc::new(config))
        .layer(Extension(Arc::clone(&engine)));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // No cookie jar: the tests drive `Cookie` explicitly.
    (engine, base_url, reqwest::Client::new())
}

/// Grants a privilege directly to a user.
fn grant_user_privilege(engine: &Arc<ProcessEngine>, privilege_id: &str, user_id: &str) {
    let identity = engine.get_identity_service();
    identity.save_privilege(Privilege {
        id: privilege_id.to_string(),
        name: privilege_id.to_string(),
    });
    identity.add_user_privilege_mapping(privilege_id.to_string(), user_id.to_string());
}

async fn login(client: &reqwest::Client, base_url: &str, user: &str, password: &str) -> reqwest::Response {
    client
        .post(format!("{base_url}/app/authentication"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("j_username={user}&j_password={password}"))
        .send()
        .await
        .unwrap()
}

/// Pulls the `FLOWABLE_REMEMBER_ME` value out of a `Set-Cookie` header.
fn remember_me_cookie(response: &reqwest::Response) -> String {
    let header = response
        .headers()
        .get("set-cookie")
        .expect("response carried no Set-Cookie")
        .to_str()
        .unwrap();
    header
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("FLOWABLE_REMEMBER_ME=")
        .expect("Set-Cookie was not the remember-me cookie")
        .to_string()
}

#[tokio::test]
async fn login_success_returns_bare_200_and_sets_cookie() {
    let (_engine, base_url, client) = spawn("ui_auth_login_success").await;

    let response = login(&client, &base_url, "admin", "test").await;

    assert_eq!(response.status(), 200);

    let set_cookie = response
        .headers()
        .get("set-cookie")
        .expect("login did not set a cookie")
        .to_str()
        .unwrap()
        .to_string();

    assert!(set_cookie.starts_with("FLOWABLE_REMEMBER_ME="));
    assert!(set_cookie.contains("Path=/"), "got: {set_cookie}");
    assert!(set_cookie.contains("HttpOnly"), "got: {set_cookie}");
    assert!(set_cookie.contains("SameSite=Lax"), "got: {set_cookie}");
    // 31 days, Java's `TOKEN_VALIDITY_SECONDS` default.
    assert!(set_cookie.contains("Max-Age=2678400"), "got: {set_cookie}");
    // Plain HTTP request: no Secure flag, or the browser would drop the cookie.
    assert!(!set_cookie.contains("Secure"), "got: {set_cookie}");

    // `AjaxAuthenticationSuccessHandler` only calls setStatus(SC_OK) — the body
    // is empty, not JSON.
    assert_eq!(response.text().await.unwrap(), "");
}

#[tokio::test]
async fn login_failure_returns_401_with_java_message() {
    let (_engine, base_url, client) = spawn("ui_auth_login_failure").await;

    let response = login(&client, &base_url, "admin", "wrong-password").await;

    assert_eq!(response.status(), 401);
    assert!(response.headers().get("set-cookie").is_none());
    assert_eq!(response.text().await.unwrap(), "Authentication failed");
}

#[tokio::test]
async fn login_failure_for_unknown_user() {
    let (_engine, base_url, client) = spawn("ui_auth_login_unknown").await;

    let response = login(&client, &base_url, "nobody", "test").await;

    assert_eq!(response.status(), 401);
    assert!(response.headers().get("set-cookie").is_none());
}

#[tokio::test]
async fn missing_and_empty_credentials_are_rejected() {
    let (_engine, base_url, client) = spawn("ui_auth_login_empty").await;

    for body in [
        "j_username=admin",                  // no password field at all
        "j_username=admin&j_password=",      // present but empty
        "j_username=&j_password=test",       // empty username
        "",                                  // nothing
    ] {
        let response = client
            .post(format!("{base_url}/app/authentication"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401, "body {body:?} should not authenticate");
    }
}

#[tokio::test]
async fn protected_endpoint_rejects_missing_cookie_with_bare_401() {
    let (_engine, base_url, client) = spawn("ui_auth_no_cookie").await;

    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .send()
        .await
        .unwrap();

    // Java's `HttpStatusEntryPoint(UNAUTHORIZED)` writes no body.
    assert_eq!(response.status(), 401);
    assert_eq!(response.text().await.unwrap(), "");
}

#[tokio::test]
async fn cookie_from_login_authenticates_subsequent_request() {
    let (engine, base_url, client) = spawn("ui_auth_cookie_roundtrip").await;
    grant_user_privilege(&engine, "access-idm", "admin");

    let cookie = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);

    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={cookie}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["id"], "admin");
}

#[tokio::test]
async fn garbage_and_tampered_cookies_are_rejected() {
    let (engine, base_url, client) = spawn("ui_auth_bad_cookie").await;
    grant_user_privilege(&engine, "access-idm", "admin");

    let valid = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);
    let decoded = String::from_utf8(
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, &valid).unwrap(),
    )
    .unwrap();
    let series = decoded.split(':').next().unwrap();

    // A correct series with a wrong token value is Spring's cookie-theft signal:
    // the row is deleted, so even the real cookie stops working afterwards.
    let forged = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        format!("{series}:not-the-real-token"),
    );

    for candidate in [
        "not-base64-at-all!!".to_string(),
        // Valid base64, but not `series:value`.
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, "onlyonepart"),
        // Unknown series.
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, "nosuchseries:value"),
        forged.clone(),
    ] {
        let response = client
            .get(format!("{base_url}/idm-app/rest/account"))
            .header("cookie", format!("FLOWABLE_REMEMBER_ME={candidate}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401, "cookie {candidate:?} should be rejected");
    }

    // Theft detection invalidated the series, so the originally valid cookie is
    // now dead too.
    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={valid}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        401,
        "a detected theft must invalidate the series for the real client too"
    );
}

/// Rewinds a token's issue date so refresh/expiry boundaries can be crossed
/// without sleeping. Takes the cookie value, not the series, and returns nothing
/// — the row is updated in place.
fn backdate_token(engine: &Arc<ProcessEngine>, cookie: &str, age: std::time::Duration) {
    let decoded = String::from_utf8(
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, cookie).unwrap(),
    )
    .unwrap();
    let series = decoded.split(':').next().unwrap();

    let identity = engine.get_identity_service();
    let token = identity.find_token_by_id(series).expect("token row missing");
    let issued_at = token.token_date.expect("token had no date") - age.as_millis() as i64;
    identity.save_token(flowable_engine::identity::entities::Token {
        token_date: Some(issued_at),
        ..token
    });
}

#[tokio::test]
async fn cookie_is_rolled_once_past_the_refresh_age() {
    let (engine, base_url, client) = spawn("ui_auth_roll").await;

    let first = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);
    // Two days old against the one-day default refresh age.
    backdate_token(&engine, &first, std::time::Duration::from_secs(2 * 24 * 60 * 60));

    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={first}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let rolled = remember_me_cookie(&response);
    assert_ne!(rolled, first, "a request past refresh age must issue a new cookie");

    // The new cookie works.
    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={rolled}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // The superseded one does not: `roll_token` deletes the old row. This is a
    // deliberate deviation — Java leaves the old series valid until maxAge.
    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={first}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn cookie_is_not_rolled_before_the_refresh_age() {
    let (engine, base_url, client) = spawn("ui_auth_no_roll").await;
    grant_user_privilege(&engine, "access-idm", "admin");

    let cookie = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);

    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={cookie}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert!(
        response.headers().get("set-cookie").is_none(),
        "a fresh cookie must not be reissued on every request"
    );
}

#[tokio::test]
async fn expired_cookie_is_rejected() {
    let (engine, base_url, client) = spawn("ui_auth_expired").await;

    let cookie = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);
    // 32 days old against the 31-day default max age.
    backdate_token(&engine, &cookie, std::time::Duration::from_secs(32 * 24 * 60 * 60));

    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={cookie}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn logout_clears_the_cookie_and_invalidates_the_series() {
    let (engine, base_url, client) = spawn("ui_auth_logout").await;
    grant_user_privilege(&engine, "access-idm", "admin");

    let cookie = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);

    // Redirects are not followed: the 302 to "/" is part of the contract.
    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let response = no_redirect
        .get(format!("{base_url}/app/logout"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={cookie}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 302);
    assert_eq!(response.headers().get("location").unwrap(), "/");

    let set_cookie = response
        .headers()
        .get("set-cookie")
        .expect("logout must clear the cookie")
        .to_str()
        .unwrap();
    assert!(set_cookie.contains("Max-Age=0"), "got: {set_cookie}");

    // The series is gone server-side, so replaying the cookie fails.
    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={cookie}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn logout_is_reachable_without_authentication() {
    let (_engine, base_url, client) = spawn("ui_auth_logout_anon").await;

    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let _ = client;

    // Java's chain has no `anyRequest()` rule, so /app/logout is not protected.
    let response = no_redirect
        .get(format!("{base_url}/app/logout"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 302);
}

#[tokio::test]
async fn account_and_authenticate_need_no_privilege() {
    let (_engine, base_url, client) = spawn("ui_auth_account_no_privilege").await;
    // Deliberately no grants. Java lists `/idm-app/rest/authenticate` and
    // `/idm-app/rest/account` as `authenticated()` *ahead* of the
    // `/idm-app/**` privilege rule: the shell has to be able to learn who you
    // are before it can decide which apps you may see.
    let cookie = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);

    for path in ["/idm-app/rest/account", "/idm-app/rest/authenticate"] {
        let response = client
            .get(format!("{base_url}{path}"))
            .header("cookie", format!("FLOWABLE_REMEMBER_ME={cookie}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path} must not require a privilege");
    }
}

#[tokio::test]
async fn admin_endpoints_require_the_access_idm_privilege() {
    let (_engine, base_url, client) = spawn("ui_auth_privilege_denied").await;
    // No privilege granted: authentication succeeds, authorisation does not.

    let cookie = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);

    let response = client
        .get(format!("{base_url}/idm-app/rest/admin/users"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={cookie}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 403);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["messageKey"], "GENERAL.ERROR.FORBIDDEN");
}

#[tokio::test]
async fn privilege_inherited_through_a_group_is_honoured() {
    let (engine, base_url, client) = spawn("ui_auth_group_privilege").await;
    let identity = engine.get_identity_service();

    identity.save_group(Group {
        id: "idm-users".to_string(),
        name: "IDM users".to_string(),
        group_type: Some("security-role".to_string()),
    });
    identity.create_membership("admin".to_string(), "idm-users".to_string());
    identity.save_privilege(Privilege {
        id: "access-idm".to_string(),
        name: "access-idm".to_string(),
    });
    identity.add_group_privilege_mapping("access-idm".to_string(), "idm-users".to_string());

    let cookie = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);

    // A privilege-gated path, so the grant is actually load-bearing here.
    let response = client
        .get(format!("{base_url}/idm-app/rest/admin/users"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={cookie}"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        200,
        "a group-granted privilege must satisfy the path requirement"
    );
}

#[tokio::test]
async fn token_outliving_its_user_is_rejected() {
    let (engine, base_url, client) = spawn("ui_auth_deleted_user").await;
    grant_user_privilege(&engine, "access-idm", "admin");

    let cookie = remember_me_cookie(&login(&client, &base_url, "admin", "test").await);
    engine.get_identity_service().delete_user("admin");

    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .header("cookie", format!("FLOWABLE_REMEMBER_ME={cookie}"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn disabled_mode_injects_a_dev_identity_with_all_privileges() {
    let config = UiAuthConfig {
        mode: AuthMode::Disabled,
        ..UiAuthConfig::default()
    };
    let (_engine, base_url, client) = spawn_with_config("ui_auth_disabled", config).await;

    // No cookie, no privilege grants — streams B and C develop against this.
    let response = client
        .get(format!("{base_url}/idm-app/rest/account"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["id"], "admin");
}

#[tokio::test]
async fn secure_flag_is_set_for_forwarded_https() {
    let (_engine, base_url, client) = spawn("ui_auth_forwarded_https").await;

    let response = client
        .post(format!("{base_url}/app/authentication"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("x-forwarded-proto", "https")
        .body("j_username=admin&j_password=test")
        .send()
        .await
        .unwrap();

    let set_cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        set_cookie.contains("Secure"),
        "a cookie issued behind an HTTPS proxy must be Secure: {set_cookie}"
    );
}
