use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{AuthPolicy, AuthProviderKind, ServicePolicyConfig};
use flowable_engine::service::issuer_profile::{
    ClaimMappingConfig, ClaimValidation, IssuerProfile, JwksRefreshPolicy, RolloutState,
};
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

const TEST_PRIVATE_KEY_PEM: &[u8] = include_bytes!("test_key.pem");

fn create_token(issuer: &str, kid: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut claims = HashMap::new();
    claims.insert("iss", Value::String(issuer.to_string()));
    claims.insert("aud", Value::String("my-app".to_string()));
    claims.insert("sub", Value::String("user-123".to_string()));
    claims.insert("role", Value::String("admin".to_string()));
    claims.insert("exp", Value::Number((now + 3600).into()));

    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(kid.to_string());
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM).unwrap();
    jsonwebtoken::encode(&header, &claims, &encoding_key).unwrap()
}

#[test]
fn test_distributed_identity_sync_via_polling() {
    // Write the PEM to a temporary file for include_bytes (or just use a raw string without bad escapes)
    // Actually, let's just use a raw string literal to avoid any escape issues.
    let db_path = format!("test_identity_sync_{}.db", Uuid::new_v4());
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    // --- Node 1 Setup ---
    let engine1 = ProcessEngine::build(
        "node-1".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );
    let mut config1 = ServicePolicyConfig::default();
    let port1 = 25000 + (uuid::Uuid::new_v4().as_u128() % 5000) as u16;
    let addr1 = format!("127.0.0.1:{}", port1);
    config1.bind_addr = addr1.clone();
    config1.auth_provider = AuthProviderKind::LocalStatic;
    config1.external_provider = Some(
        flowable_engine::service::config::ExternalAuthProviderConfig {
            issuer: "".to_string(),
            audience: "".to_string(),
            mapping: flowable_engine::service::config::ExternalClaimMappingConfig::default(),
            trusted_profiles: vec![],
        },
    );
    config1.auth_keys.insert(
        "admin-secret".to_string(),
        AuthPolicy {
            actor_id: "admin".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: None,
        },
    );

    let stop_signal1 = Arc::new(AtomicBool::new(false));
    let service1 =
        TimerCoordinationService::new(Arc::clone(&engine1.get_runtime_service()), config1);
    let _handle1 = service1.start(Arc::clone(&stop_signal1));

    // --- Node 2 Setup ---
    let engine2 = ProcessEngine::build(
        "node-2".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );
    let mut config2 = ServicePolicyConfig::default();
    let port2 = 30000 + (uuid::Uuid::new_v4().as_u128() % 5000) as u16;
    let addr2 = format!("127.0.0.1:{}", port2);
    config2.bind_addr = addr2.clone();
    config2.auth_provider = AuthProviderKind::External;
    config2.external_provider = Some(
        flowable_engine::service::config::ExternalAuthProviderConfig {
            issuer: "".to_string(),
            audience: "".to_string(),
            mapping: flowable_engine::service::config::ExternalClaimMappingConfig::default(),
            trusted_profiles: vec![],
        },
    );

    let stop_signal2 = Arc::new(AtomicBool::new(false));
    let service2 =
        TimerCoordinationService::new(Arc::clone(&engine2.get_runtime_service()), config2);
    let _handle2 = service2.start(Arc::clone(&stop_signal2));

    std::thread::sleep(Duration::from_millis(100));

    let admin_client1 =
        TimerCoordinationClient::new(addr1.clone()).with_auth("admin-secret".to_string());
    let user_client2 = TimerCoordinationClient::new(addr2.clone());

    let issuer = "https://sync-test.example.com";
    let token = create_token(issuer, "test-kid");
    let user_client2_auth = user_client2.with_auth(token.clone());

    // 1. Create profile on Node 1 with correct jwks_uri
    let profile = IssuerProfile {
        id: "sync-profile".to_string(),
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
    admin_client1.create_issuer_profile(&profile).unwrap();

    // Node 2 should pick it up from DB eventually, but for first auth it will just read DB.
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        user_client2_auth.get_status().is_ok(),
        "Initial auth on Node 2 should succeed"
    );

    // 2. Change jwks_uri on Node 1 to something invalid.
    // This audits an event that Node 2's poller should see.
    let mut updated_profile = profile.clone();
    updated_profile.jwks_uri = Some("http://localhost:1/nonexistent".to_string());
    admin_client1
        .update_issuer_profile(&updated_profile)
        .unwrap();

    // 3. Auth on Node 2 should fail within the polling window (1s).
    // If Node 2 DID NOT sync, it would keep using the cached key for 1 hour!
    std::thread::sleep(Duration::from_millis(1500));

    let auth_res = user_client2_auth.get_status();
    assert!(
        auth_res.is_err(),
        "Auth on Node 2 should fail after Node 1 updated profile and triggered sync invalidation"
    );

    stop_signal1.store(true, Ordering::SeqCst);
    stop_signal2.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_file(db_path);
}
