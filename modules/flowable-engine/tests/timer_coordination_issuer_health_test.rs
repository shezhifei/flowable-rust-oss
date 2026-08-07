use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{
    AuthPolicy, AuthProviderKind, ExternalAuthProviderConfig, ExternalClaimMappingConfig,
    ExternalIssuerProfileConfig, ExternalValidationConfig, ServicePolicyConfig,
};
use flowable_engine::service::issuer_profile::RolloutState;
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const TEST_PRIVATE_KEY_PEM: &[u8] = include_bytes!("test_key.pem");

fn create_token(issuer: &str, kid: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut claims = HashMap::new();
    claims.insert("iss", Value::String(issuer.to_string()));
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("health-admin".to_string()));
    claims.insert("role", Value::String("admin".to_string()));
    claims.insert("exp", Value::Number((now + 3600).into()));

    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(kid.to_string());
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM).unwrap();
    jsonwebtoken::encode(&header, &claims, &encoding_key).unwrap()
}

fn get_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn health_config(bind_addr: String) -> ServicePolicyConfig {
    ServicePolicyConfig {
        bind_addr,
        auth_provider: AuthProviderKind::External,
        external_provider: Some(ExternalAuthProviderConfig {
            issuer: "health-issuer".to_string(),
            audience: "flowable-timer".to_string(),
            mapping: ExternalClaimMappingConfig::default(),
            trusted_profiles: vec![ExternalIssuerProfileConfig {
                id: "profile-health".to_string(),
                issuer: "health-issuer".to_string(),
                audience: "flowable-timer".to_string(),
                mapping: ExternalClaimMappingConfig::default(),
                validation: ExternalValidationConfig::default(),
                role_mappings: vec![],
                required_tenant: false,
                rollout_state: RolloutState::Active,
                jwks_uri: Some("test-local".to_string()),
                allowed_algorithms: Some(vec!["RS256".to_string()]),
                jwks_cache_ttl_seconds: 45,
                jwks_refresh_policy: Default::default(),
                version: 0,
            }],
        }),
        ..Default::default()
    }
}

#[test]
fn test_issuer_health_endpoint_reports_configured_profile() {
    let db_path = "test_timer_coordination_issuer_health.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let bind_addr = format!("127.0.0.1:{}", port);

    let mut config = health_config(bind_addr.clone());
    config.auth_keys.insert(
        "admin-secret".to_string(),
        AuthPolicy {
            actor_id: "admin".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: None,
        },
    );
    let db_store = Arc::new(DbStore::new_file(db_path).unwrap());
    let engine = ProcessEngine::build(
        "issuer-health-node".to_string(),
        Arc::new(SystemTimeSource),
        db_store,
    );
    let runtime_service = engine.get_runtime_service();

    let service = TimerCoordinationService::new(runtime_service, config);
    let stop_signal = Arc::new(AtomicBool::new(false));
    let handle = service.start(Arc::clone(&stop_signal));
    std::thread::sleep(Duration::from_millis(100));

    let token = create_token("health-issuer", "test-kid");
    let read_client = TimerCoordinationClient::new(bind_addr.clone()).with_auth(token);

    let health = read_client.get_issuer_health().unwrap();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].profile_id, "profile-health");
    assert_eq!(health[0].issuer, "health-issuer");
    assert_eq!(health[0].jwks_cache_ttl_seconds, 45);
    assert!(health[0].jwks_uri_present);
    assert_eq!(health[0].allowed_algorithms, vec!["RS256".to_string()]);

    let issuer_health = read_client
        .get_issuer_health_for_issuer("health-issuer")
        .unwrap();
    assert_eq!(issuer_health.profile_id, "profile-health");
    assert_eq!(issuer_health.issuer, "health-issuer");

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}
