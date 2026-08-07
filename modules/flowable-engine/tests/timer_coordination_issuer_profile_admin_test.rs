use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{
    AuthPolicy, AuthProviderKind, ExternalAuthProviderConfig, ExternalClaimMappingConfig,
    ExternalIssuerProfileConfig, ExternalValidationConfig, ServicePolicyConfig,
};
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

fn create_token(issuer: &str, kid: &str, aud: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut claims = HashMap::new();
    claims.insert("iss", Value::String(issuer.to_string()));
    claims.insert("aud", Value::String(aud.to_string()));
    claims.insert("sub", Value::String("admin-user".to_string()));
    claims.insert("role", Value::String("admin".to_string()));
    claims.insert("exp", Value::Number((now + 3600).into()));

    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(kid.to_string());
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM).unwrap();
    jsonwebtoken::encode(&header, &claims, &encoding_key).unwrap()
}

#[test]
fn test_timer_coordination_issuer_profile_admin() {
    let db_path = format!(
        "file:test_rpc_issuer_profile_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "worker_admin_node".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );

    let runtime_service = engine.get_runtime_service();

    let mut config = ServicePolicyConfig::default();
    let random_port = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let actual_addr = format!("127.0.0.1:{}", random_port);
    config.bind_addr = actual_addr.clone();

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

    let stop_signal = Arc::new(AtomicBool::new(false));
    let service = TimerCoordinationService::new(Arc::clone(&runtime_service), config);

    let handle = service.start(Arc::clone(&stop_signal));
    std::thread::sleep(Duration::from_millis(50));

    let admin_client =
        TimerCoordinationClient::new(actual_addr.clone()).with_auth("admin-secret".to_string());

    // 1. List (should be empty initially, or contain default if external auth configured)
    let initial_profiles = admin_client.list_issuer_profiles().unwrap();
    let initial_len = initial_profiles.len();

    // 2. Create
    let new_profile = IssuerProfile {
        id: "test-profile-1".to_string(),
        issuer: "https://auth.example.com".to_string(),
        audience: "my-app".to_string(),
        mapping: ClaimMappingConfig::default(),
        validation: ClaimValidation::default(),
        role_mappings: vec![],
        required_tenant: false,
        rollout_state: RolloutState::Active,
        jwks_uri: None,
        allowed_algorithms: vec!["RS256".to_string()],
        jwks_cache_ttl_seconds: 3600,
        jwks_refresh_policy: JwksRefreshPolicy::default(),
        version: 0,
    };

    let created = admin_client.create_issuer_profile(&new_profile).unwrap();
    assert_eq!(created.id, "test-profile-1");

    // 3. Read
    let fetched = admin_client.get_issuer_profile("test-profile-1").unwrap();
    assert_eq!(fetched.issuer, "https://auth.example.com");

    let all_profiles = admin_client.list_issuer_profiles().unwrap();
    assert_eq!(all_profiles.len(), initial_len + 1);

    // 4. Update
    let mut updated_profile = fetched.clone();
    updated_profile.audience = "new-audience".to_string();
    let updated = admin_client
        .update_issuer_profile(&updated_profile)
        .unwrap();
    assert_eq!(updated.audience, "new-audience");

    let fetched_again = admin_client.get_issuer_profile("test-profile-1").unwrap();
    assert_eq!(fetched_again.audience, "new-audience");

    // 5. Delete
    let deleted = admin_client
        .delete_issuer_profile("test-profile-1")
        .unwrap();
    assert!(deleted);

    let fetched_deleted = admin_client.get_issuer_profile("test-profile-1");
    assert!(fetched_deleted.is_err());

    // Stop server
    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
}

#[test]
fn test_bootstrap_from_config_seeds_once_and_does_not_clobber_db_managed_changes() {
    let db_path = format!(
        "file:test_rpc_issuer_profile_bootstrap_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let seed_profile = ExternalIssuerProfileConfig {
        id: "bootstrap-profile".to_string(),
        issuer: "https://bootstrap.example.com".to_string(),
        audience: "seed-audience".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        validation: ExternalValidationConfig::default(),
        role_mappings: vec![],
        required_tenant: false,
        rollout_state: RolloutState::Active,
        jwks_uri: Some("test-local".to_string()),
        allowed_algorithms: Some(vec!["RS256".to_string()]),
        jwks_cache_ttl_seconds: 30,
        jwks_refresh_policy: Default::default(),
        version: 0,
    };

    let make_config = |bind_addr: String| {
        let mut config = ServicePolicyConfig {
            bind_addr,
            auth_provider: AuthProviderKind::External,
            ..Default::default()
        };
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
        config.external_provider = Some(ExternalAuthProviderConfig {
            issuer: String::new(),
            audience: String::new(),
            mapping: ExternalClaimMappingConfig::default(),
            trusted_profiles: vec![seed_profile.clone()],
        });
        config
    };

    let engine1 = ProcessEngine::build(
        "bootstrap-node-1".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );
    let runtime_service1 = engine1.get_runtime_service();
    let port1 = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let addr1 = format!("127.0.0.1:{}", port1);
    let stop_signal1 = Arc::new(AtomicBool::new(false));
    let service1 =
        TimerCoordinationService::new(Arc::clone(&runtime_service1), make_config(addr1.clone()));
    let handle1 = service1.start(Arc::clone(&stop_signal1));
    std::thread::sleep(Duration::from_millis(50));

    let admin_token1 = create_token("https://bootstrap.example.com", "test-kid", "seed-audience");
    let admin_client1 = TimerCoordinationClient::new(addr1.clone()).with_auth(admin_token1);
    let seeded = admin_client1
        .get_issuer_profile("bootstrap-profile")
        .unwrap();
    assert_eq!(seeded.audience, "seed-audience");

    let mut updated = seeded.clone();
    updated.audience = "db-managed-audience".to_string();
    admin_client1.update_issuer_profile(&updated).unwrap();
    let admin_token1_updated = create_token(
        "https://bootstrap.example.com",
        "test-kid",
        "db-managed-audience",
    );
    let admin_client1 = admin_client1.with_auth(admin_token1_updated);
    assert_eq!(
        admin_client1
            .get_issuer_profile("bootstrap-profile")
            .unwrap()
            .audience,
        "db-managed-audience"
    );

    stop_signal1.store(true, Ordering::SeqCst);
    let _ = handle1.join();

    let engine2 = ProcessEngine::build(
        "bootstrap-node-2".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );
    let runtime_service2 = engine2.get_runtime_service();
    let port2 = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let addr2 = format!("127.0.0.1:{}", port2);
    let stop_signal2 = Arc::new(AtomicBool::new(false));
    let service2 =
        TimerCoordinationService::new(Arc::clone(&runtime_service2), make_config(addr2.clone()));
    let handle2 = service2.start(Arc::clone(&stop_signal2));
    std::thread::sleep(Duration::from_millis(50));

    let admin_token2 = create_token(
        "https://bootstrap.example.com",
        "test-kid",
        "db-managed-audience",
    );
    let admin_client2 = TimerCoordinationClient::new(addr2.clone()).with_auth(admin_token2);
    let persisted = admin_client2
        .get_issuer_profile("bootstrap-profile")
        .unwrap();
    assert_eq!(
        persisted.audience, "db-managed-audience",
        "restart should not overwrite DB-managed issuer profile state with config bootstrap values"
    );
    assert_eq!(admin_client2.list_issuer_profiles().unwrap().len(), 1);

    stop_signal2.store(true, Ordering::SeqCst);
    let _ = handle2.join();
}
