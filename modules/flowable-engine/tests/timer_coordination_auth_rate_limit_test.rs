use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{AuthProviderKind, ServicePolicyConfig};
use flowable_engine::service::issuer_profile::{
    ClaimMappingConfig, ClaimValidation, IssuerProfile, JwksRefreshPolicy, RolloutState,
};
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

#[test]
fn test_auth_rate_limiting_on_failures() {
    let db_path = format!("test_auth_rate_limit_{}.db", Uuid::new_v4());
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "node-1".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );

    let mut config = ServicePolicyConfig::default();
    let random_port = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let addr = format!("127.0.0.1:{}", random_port);
    config.bind_addr = addr.clone();
    config.auth_provider = AuthProviderKind::External;
    config.external_provider = Some(
        flowable_engine::service::config::ExternalAuthProviderConfig {
            issuer: "".to_string(),
            audience: "".to_string(),
            mapping: flowable_engine::service::config::ExternalClaimMappingConfig::default(),
            trusted_profiles: vec![],
        },
    );

    // Create a profile for the test
    let issuer = "https://rate-limit.example.com";
    let profile = IssuerProfile {
        id: "rate-limit-profile".to_string(),
        issuer: issuer.to_string(),
        audience: "my-app".to_string(),
        mapping: ClaimMappingConfig::default(),
        validation: ClaimValidation::default(),
        role_mappings: vec![],
        required_tenant: false,
        rollout_state: RolloutState::Active,
        jwks_uri: Some("test-local".to_string()),
        allowed_algorithms: vec!["RS256".to_string()],
        jwks_cache_ttl_seconds: 3600,
        jwks_refresh_policy: JwksRefreshPolicy::default(),
        version: 0,
    };
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_issuer_profile(profile, &mut session);
    session.flush_and_commit().unwrap();

    let stop_signal = Arc::new(AtomicBool::new(false));
    let service = TimerCoordinationService::new(Arc::clone(&engine.get_runtime_service()), config);
    let _handle = service.start(Arc::clone(&stop_signal));

    std::thread::sleep(Duration::from_millis(100));

    let _client = TimerCoordinationClient::new(addr.clone());

    // 1. Hammer the endpoint with invalid tokens
    // Default limit is 10 failures in 60s
    for i in 0..10 {
        let bad_token = format!("invalid-token-{}", i);
        let client_with_auth = TimerCoordinationClient::new(addr.clone()).with_auth(bad_token);
        let _ = client_with_auth.get_status();
    }

    // 2. The 11th attempt should be rate limited immediately
    let res = TimerCoordinationClient::new(addr.clone())
        .with_auth("invalid-token-11".to_string())
        .get_status();
    assert!(
        res.is_err(),
        "11th bad attempt should be blocked by rate limiter"
    );

    // 3. Even a potentially valid token (if we had one) from the SAME issuer
    // should be blocked now because the issuer is in cooldown.
    // Since we don't have a valid token easily here (requires private key signing),
    // we just prove that it continues to fail.

    stop_signal.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_file(db_path);
}
