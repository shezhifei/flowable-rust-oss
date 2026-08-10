use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::{
    config::{RestAdminSeedConfig, RestAuthConfig, RestAuthMode, RestConfig, RestSecurityConfig},
    run_server_with_config,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(
    test_name: &str,
    config: RestConfig,
) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server_with_config(engine_clone, listener, config)
            .await
            .unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

fn base_config() -> RestConfig {
    RestConfig {
        bind_address: "127.0.0.1:0".to_string(),
        database_path: ":memory:".to_string(),
        engine_name: "rest-security-config-test".to_string(),
        security: RestSecurityConfig::default(),
    }
}

#[tokio::test]
async fn basic_auth_mode_keeps_unauthorized_contract_stable() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Basic,
        ..Default::default()
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: false,
        ..RestAdminSeedConfig::default()
    };

    let (_engine, base_url, client) = spawn_server("rest-auth-basic-contract", config).await;

    let response = client
        .post(format!("{}/runtime/process-instances", base_url))
        .json(&json!({"processDefinitionId": "missing"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "UNAUTHORIZED");
    assert_eq!(body["message"], "Unauthorized");
    assert!(body["details"].is_null());
}

#[tokio::test]
async fn disabled_auth_mode_allows_requests_without_authorization() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Disabled,
        ..Default::default()
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: false,
        ..RestAdminSeedConfig::default()
    };

    let (_engine, base_url, client) = spawn_server("rest-auth-disabled", config).await;

    let response = client
        .post(format!("{}/runtime/process-instances", base_url))
        .json(&json!({"processDefinitionId": "missing"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn configured_admin_seed_bootstraps_basic_auth_credentials() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Basic,
        ..Default::default()
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: true,
        user_id: "seed-admin".to_string(),
        password: "seed-secret".to_string(),
        first_name: Some("Seed".to_string()),
        last_name: Some("Admin".to_string()),
        email: Some("seed-admin@example.test".to_string()),
    };

    let (engine, base_url, client) = spawn_server("rest-admin-seed-enabled", config).await;

    let response = client
        .get(format!("{}/history/historic-process-instances", base_url))
        .basic_auth("seed-admin", Some("seed-secret"))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"], json!([]));

    let seeded_user = engine
        .get_identity_service()
        .find_user_by_id("seed-admin")
        .expect("seed admin should be present");
    assert_eq!(
        seeded_user.email.as_deref(),
        Some("seed-admin@example.test")
    );
    assert!(
        engine
            .get_identity_service()
            .check_password("seed-admin", "seed-secret")
    );
}

#[tokio::test]
async fn disabled_admin_seed_does_not_create_default_admin_user() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Basic,
        ..Default::default()
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: false,
        ..RestAdminSeedConfig::default()
    };

    let (engine, base_url, client) = spawn_server("rest-admin-seed-disabled", config).await;

    assert!(
        engine
            .get_identity_service()
            .find_user_by_id("admin")
            .is_none(),
        "default admin should not be seeded when disabled"
    );

    let response = client
        .get(format!("{}/history/historic-process-instances", base_url))
        .basic_auth("admin", Some("admin"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "UNAUTHORIZED");
}

#[tokio::test]
async fn admin_seed_with_default_password_fails_startup() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Basic,
        ..Default::default()
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: true,
        user_id: "admin".to_string(),
        password: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
    };

    let engine = Arc::new(ProcessEngine::new(
        "rest-admin-seed-default-password".to_string(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let err = run_server_with_config(engine, listener, config)
        .await
        .expect_err("default password seed must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("admin") && (msg.contains("password") || msg.contains("Refusing")),
        "expected refuse-default-password message, got: {msg}"
    );
}

#[tokio::test]
async fn non_admin_deployment_returns_forbidden() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Basic,
        admin_users: vec!["real-admin".to_string()],
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: true,
        user_id: "real-admin".to_string(),
        password: "real-secret".to_string(),
        first_name: None,
        last_name: None,
        email: None,
    };

    let (engine, base_url, client) = spawn_server("rest-non-admin-deploy", config).await;
    engine.get_identity_service().save_user(flowable_engine::identity::entities::User {
        id: "regular".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("user-secret".to_string()),
        tenant_id: None,
    });

    let response = client
        .post(format!("{}/repository/deployments", base_url))
        .basic_auth("regular", Some("user-secret"))
        .json(&json!({
            "name": "should-fail",
            "resourceName": "p.bpmn20.xml",
            "resource": "<?xml version=\"1.0\"?><definitions xmlns=\"http://www.omg.org/spec/BPMN/20100524/MODEL\" targetNamespace=\"t\"><process id=\"p\" isExecutable=\"true\"><startEvent id=\"s\"/><endEvent id=\"e\"/><sequenceFlow id=\"f\" sourceRef=\"s\" targetRef=\"e\"/></process></definitions>"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "FORBIDDEN");
}

#[tokio::test]
async fn admin_deployment_succeeds() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Basic,
        admin_users: vec!["real-admin".to_string()],
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: true,
        user_id: "real-admin".to_string(),
        password: "real-secret".to_string(),
        first_name: None,
        last_name: None,
        email: None,
    };

    let (_engine, base_url, client) = spawn_server("rest-admin-deploy", config).await;

    let response = client
        .post(format!("{}/repository/deployments", base_url))
        .basic_auth("real-admin", Some("real-secret"))
        .json(&json!({
            "name": "ok-deploy",
            "resourceName": "p.bpmn20.xml",
            "resource": "<?xml version=\"1.0\"?><definitions xmlns=\"http://www.omg.org/spec/BPMN/20100524/MODEL\" targetNamespace=\"t\"><process id=\"p\" isExecutable=\"true\"><startEvent id=\"s\"/><endEvent id=\"e\"/><sequenceFlow id=\"f\" sourceRef=\"s\" targetRef=\"e\"/></process></definitions>"
        }))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "admin deploy should succeed, got {}",
        response.status()
    );
}

#[tokio::test]
async fn get_paths_do_not_require_admin_role() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Basic,
        admin_users: vec!["real-admin".to_string()],
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: true,
        user_id: "real-admin".to_string(),
        password: "real-secret".to_string(),
        first_name: None,
        last_name: None,
        email: None,
    };

    let (engine, base_url, client) = spawn_server("rest-get-no-admin", config).await;
    engine.get_identity_service().save_user(flowable_engine::identity::entities::User {
        id: "regular".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("user-secret".to_string()),
        tenant_id: None,
    });

    let response = client
        .get(format!("{}/history/historic-process-instances", base_url))
        .basic_auth("regular", Some("user-secret"))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "GET must not require admin, got {}",
        response.status()
    );
}

#[tokio::test]
async fn auth_disabled_on_non_loopback_fails_startup() {
    let mut config = base_config();
    config.bind_address = "0.0.0.0:8080".to_string();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Disabled,
        ..Default::default()
    };

    let engine = Arc::new(ProcessEngine::new(
        "rest-auth-disabled-non-loopback".to_string(),
    ));
    // Listener is loopback; validation uses config.bind_address.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let err = run_server_with_config(engine, listener, config)
        .await
        .expect_err("auth disabled on non-loopback must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("non-loopback") || msg.contains("loopback") || msg.contains("disabled"),
        "expected non-loopback refusal, got: {msg}"
    );
}

#[tokio::test]
async fn metrics_requires_authentication_when_auth_enforced() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Basic,
        ..Default::default()
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: true,
        user_id: "metrics-admin".to_string(),
        password: "metrics-secret".to_string(),
        first_name: None,
        last_name: None,
        email: None,
    };

    let (_engine, base_url, client) = spawn_server("rest-metrics-auth", config).await;

    let unauth = client
        .get(format!("{}/metrics", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let auth = client
        .get(format!("{}/metrics", base_url))
        .basic_auth("metrics-admin", Some("metrics-secret"))
        .send()
        .await
        .unwrap();
    assert!(auth.status().is_success());
}

/// SSO between the UI surface and the engine API: a valid UI session cookie
/// (`FLOWABLE_REMEMBER_ME`, issued by `POST /app/authentication`) authenticates
/// engine REST requests, because the static bundles and the engine API share
/// one origin in this stack and the first-party modeler calls the engine
/// endpoints with `credentials: 'same-origin'`.
#[tokio::test]
async fn ui_session_cookie_authenticates_engine_requests() {
    let mut config = base_config();
    config.security.auth = RestAuthConfig {
        mode: RestAuthMode::Basic,
        admin_users: vec!["cookie-admin".to_string()],
    };
    config.security.admin_seed = RestAdminSeedConfig {
        enabled: true,
        user_id: "cookie-admin".to_string(),
        password: "cookie-secret".to_string(),
        first_name: None,
        last_name: None,
        email: None,
    };

    let (engine, base_url, client) = spawn_server("rest-ui-cookie-sso", config).await;
    engine
        .get_identity_service()
        .save_user(flowable_engine::identity::entities::User {
            id: "cookie-worker".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: Some("worker-secret".to_string()),
            tenant_id: None,
        });

    // Sanity: no credentials at all is still rejected.
    let anonymous = client
        .get(format!("{}/repository/models", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), reqwest::StatusCode::UNAUTHORIZED);

    let login = client
        .post(format!("{}/app/authentication", base_url))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("j_username=cookie-admin&j_password=cookie-secret")
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), reqwest::StatusCode::OK);
    let cookie = login
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap().to_string())
        .expect("login must set the session cookie");

    // The cookie alone authenticates engine reads.
    let read = client
        .get(format!("{}/repository/models", base_url))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), reqwest::StatusCode::OK);

    // Admin-gated writes pass for the admin user: a 404 on the missing
    // deployment proves the request cleared the gate and reached the handler.
    let admin_write = client
        .delete(format!("{}/repository/deployments/missing", base_url))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(admin_write.status(), reqwest::StatusCode::NOT_FOUND);

    // A non-admin session is authenticated but still blocked from admin paths.
    let worker_login = client
        .post(format!("{}/app/authentication", base_url))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("j_username=cookie-worker&j_password=worker-secret")
        .send()
        .await
        .unwrap();
    let worker_cookie = worker_login
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap().to_string())
        .expect("login must set the session cookie");
    let worker_write = client
        .delete(format!("{}/repository/deployments/missing", base_url))
        .header("Cookie", &worker_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(worker_write.status(), reqwest::StatusCode::FORBIDDEN);
}
