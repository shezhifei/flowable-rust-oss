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

fn get_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

const TEST_PRIVATE_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCxmF5i94n5EYUV\nhDmXmyS+m+QGUKabS+t+QfXKNkxofaQABsyV/h3mH5rUNQ3DdGeKrvNd0JjP9+yv\n4xH2GIejAl4TjoClPEWs5EB2zVbPr/tFKAVBhkn6sB4RTH/FHfkgaXqMgPSJsfK6\nlukqlwaxmpL6B4dqlGqAu/Z0O8PRhzKo7k1ybLRsFdi+LrpawLvdhXcpHbqIPeqL\ngoQYPGD9wvbZ/BGTYTaXxkJ//lm2UQPvGYmxKh44BH5tAvBotx2aTAL0OJgIpdTw\nTZ3NmrWaa7pd/nijTKaTkkL06weCQmCFLeM+eyQYkEcYbIWkYT1fKr3tHxbX9Lu7\nkfxrFVu/AgMBAAECggEAOy6C+ajzBhikCFUNWiu9tXU+qioTMzo8ClGRxmaM1N9V\nmRqq76sErKzIjEH3ybQPUyRU/mTmn5tHeR+K2z82aAiAcDTzQt0QfPp9TvnDnadP\n7S5Wfgzxt0QcaPhctcP0wqvTxmGs2/v8Xtiub95vQR05MG/03PwDd83rZbWK3lYI\n03gK7LH3H9F7aya4CpF934H8EEKs62zd6utv+UsV+3uJdkbmL/wyibXaJBkKMr5i\nzeeXLAFwQVyzpqpOwI3GgRgjTlLn6XuSi4naMt8Lj2cmfFCz7L+hbPziDYI1VsKg\nqGs/+8Fsy9o+7K9e10IEQRzjNJVEXO7toLL1NB+4wQKBgQDZ11NFLk5bA3nozDvJ\nRJ3aFx7nRtRNvmc25RSzLjw7ceSxh/bbetWw6qsf8z8K3UPkuwPfEEP/HmyLgiC4\ngl8xqwr4Paa4yL3nzbpccmQ2JQGIAlnyyCQDhKwDdwDsRoNS1OwuPuOGhD1Ddvsl\nxCExOI9x38/rzTjWA9EYnIlgwQKBgQDQtEpsEzpWXQzT86KlBRMd34K0mA+iTDHA\n2Hir+ciL6yk6HE/0z+3nVQ7yKZb8tBXXp9JkzNZLPDVxpPIPuwNgexytBse4ORKB\ndmHKw29kDbJ5oh5GdXiE/v078P8woc2hbjd4F2mExcvFDd9m0db9brZM8XVY0EMG\nxaGU9WlcfwKBgExEkBnbgYFp8SepQZFQ3bc6ew5cBP6HGBnnEF0/ZcUmNfxV7v6e\nvewn7OvNvRevqhKNy2gwiK3sV/JsB8qxkmSQTtHku9dcKOjcZU/ymNVAFY4pzJYs\nrjcxHwxDgOY4NcgtVddHG1/AMrbJFFr/lONnuwkSY/hZrHl5cp6cR5jBAoGBAMhR\nxH0nl379oSpvV1V9IXQy7InaymbFK5wmKu0mu09RUCjus/APBBJemhHlyX6Ue8Ka\n2l7WHXnpOIL0B0MCBaO9hzCsqVYxsYmBzyuHmos2enA1I0oNxrgg5395Ofe71lt0\nJtml3yoJkCR7xEo0b16hvWjs+e1dOHhviUAorhCRAoGAQE7xEsRRIAhON++mVHCx\nmdvOR+/x2q7nPNq64BzuQuI6p6gRDtr0g9fYFLjRxtziNXBPxzuspaEC3F3CgPEP\nK4KOPyoPwmt8gtNypaE0yXwn7qRpYyQQhHsJU/yJBfqxqLMayvPveqAzVKDLQxjJ\nO8a0nSZL9fG9vDFviBtM/pY=\n-----END PRIVATE KEY-----";

fn create_token(claims: &HashMap<&str, Value>) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-kid".to_string());
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM).unwrap();
    jsonwebtoken::encode(&header, claims, &encoding_key).unwrap()
}

#[test]
fn test_distributed_issuer_profile_coherence() {
    let db_path = "test_distributed_issuer_profile_coherence.db";
    let _ = std::fs::remove_file(db_path);

    let db_store = Arc::new(DbStore::new_file(db_path).unwrap());

    // --- Node 1 Setup ---
    let engine1 = ProcessEngine::build(
        "node-1".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );
    let runtime_service1 = engine1.get_runtime_service();

    let mut config1 = ServicePolicyConfig::default();
    let port1 = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let addr1 = format!("127.0.0.1:{}", port1);
    config1.bind_addr = addr1.clone();
    config1.auth_keys.insert(
        "admin-secret-1".to_string(),
        AuthPolicy {
            actor_id: "admin1".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: None,
        },
    );

    let stop_signal1 = Arc::new(AtomicBool::new(false));
    let service1 = TimerCoordinationService::new(Arc::clone(&runtime_service1), config1);
    let handle1 = service1.start(Arc::clone(&stop_signal1));

    // --- Node 2 Setup ---
    let engine2 = ProcessEngine::build(
        "node-2".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );
    let runtime_service2 = engine2.get_runtime_service();

    let mut config2 = ServicePolicyConfig::default();
    let port2 = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let addr2 = format!("127.0.0.1:{}", port2);
    config2.bind_addr = addr2.clone();
    config2.auth_keys.insert(
        "admin-secret-2".to_string(),
        AuthPolicy {
            actor_id: "admin2".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: None,
        },
    );

    let stop_signal2 = Arc::new(AtomicBool::new(false));
    let service2 = TimerCoordinationService::new(Arc::clone(&runtime_service2), config2);
    let handle2 = service2.start(Arc::clone(&stop_signal2));

    std::thread::sleep(Duration::from_millis(50));

    let client1 =
        TimerCoordinationClient::new(addr1.clone()).with_auth("admin-secret-1".to_string());
    let client2 =
        TimerCoordinationClient::new(addr2.clone()).with_auth("admin-secret-2".to_string());

    // 1. Create on Node 1
    let new_profile = IssuerProfile {
        id: "shared-profile-1".to_string(),
        issuer: "https://shared.example.com".to_string(),
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

    client1.create_issuer_profile(&new_profile).unwrap();

    // 2. Read from Node 2
    let fetched_on_node2 = client2.get_issuer_profile("shared-profile-1").unwrap();
    assert_eq!(fetched_on_node2.issuer, "https://shared.example.com");

    // 3. Update on Node 2
    let mut update_prof = fetched_on_node2.clone();
    update_prof.audience = "updated-audience".to_string();
    client2.update_issuer_profile(&update_prof).unwrap();

    // 4. Verify on Node 1
    let fetched_on_node1 = client1.get_issuer_profile("shared-profile-1").unwrap();
    assert_eq!(fetched_on_node1.audience, "updated-audience");

    // Stop servers
    stop_signal1.store(true, Ordering::SeqCst);
    stop_signal2.store(true, Ordering::SeqCst);
    let _ = handle1.join();
    let _ = handle2.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_distributed_issuer_profile_auth_coherence() {
    let db_path = "test_distributed_issuer_profile_auth_coherence.db";
    let _ = std::fs::remove_file(db_path);

    let db_store = Arc::new(DbStore::new_file(db_path).unwrap());

    let seed_profile = ExternalIssuerProfileConfig {
        id: "seed-profile".to_string(),
        issuer: "https://seed.example.com".to_string(),
        audience: "flowable-timer".to_string(),
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

    let engine1 = ProcessEngine::build(
        "issuer-auth-node-1".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );
    let runtime_service1 = engine1.get_runtime_service();

    let port1 = get_free_port();
    let addr1 = format!("127.0.0.1:{}", port1);
    let mut config1 = ServicePolicyConfig {
        bind_addr: addr1.clone(),
        auth_provider: AuthProviderKind::LocalStatic,
        ..Default::default()
    };
    config1.auth_keys.insert(
        "admin-secret-1".to_string(),
        AuthPolicy {
            actor_id: "admin1".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: None,
        },
    );
    config1.external_provider = Some(ExternalAuthProviderConfig {
        issuer: String::new(),
        audience: String::new(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![seed_profile.clone()],
    });
    let stop_signal1 = Arc::new(AtomicBool::new(false));
    let service1 = TimerCoordinationService::new(Arc::clone(&runtime_service1), config1);
    let handle1 = service1.start(Arc::clone(&stop_signal1));

    let engine2 = ProcessEngine::build(
        "issuer-auth-node-2".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );
    let runtime_service2 = engine2.get_runtime_service();

    let port2 = get_free_port();
    let addr2 = format!("127.0.0.1:{}", port2);
    let mut config2 = ServicePolicyConfig {
        bind_addr: addr2.clone(),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };
    config2.auth_keys.insert(
        "admin-secret-2".to_string(),
        AuthPolicy {
            actor_id: "admin2".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: None,
        },
    );
    config2.external_provider = Some(ExternalAuthProviderConfig {
        issuer: String::new(),
        audience: String::new(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![seed_profile],
    });
    let stop_signal2 = Arc::new(AtomicBool::new(false));
    let service2 = TimerCoordinationService::new(Arc::clone(&runtime_service2), config2);
    let handle2 = service2.start(Arc::clone(&stop_signal2));

    std::thread::sleep(Duration::from_millis(100));

    let admin_client1 =
        TimerCoordinationClient::new(addr1.clone()).with_auth("admin-secret-1".to_string());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut claims = HashMap::new();
    claims.insert(
        "iss",
        Value::String("https://dynamic.example.com".to_string()),
    );
    claims.insert("aud", Value::String("dynamic-app".to_string()));
    claims.insert("sub", Value::String("issuer-user".to_string()));
    claims.insert("role", Value::String("admin".to_string()));
    claims.insert("exp", Value::Number((now + 3600).into()));
    let dynamic_token = create_token(&claims);

    let dynamic_client_node2 =
        TimerCoordinationClient::new(addr2.clone()).with_auth(dynamic_token.clone());
    let initial_auth = dynamic_client_node2.get_status();
    assert!(initial_auth.is_err());
    assert!(initial_auth.unwrap_err().contains("UNAUTHORIZED"));

    let dynamic_profile = IssuerProfile {
        id: "dynamic-profile".to_string(),
        issuer: "https://dynamic.example.com".to_string(),
        audience: "dynamic-app".to_string(),
        mapping: ClaimMappingConfig::default(),
        validation: ClaimValidation::default(),
        role_mappings: vec![],
        required_tenant: false,
        rollout_state: RolloutState::Active,
        jwks_uri: Some("test-local".to_string()),
        allowed_algorithms: vec!["RS256".to_string()],
        jwks_cache_ttl_seconds: 30,
        jwks_refresh_policy: JwksRefreshPolicy::default(),
        version: 0,
    };

    let created = admin_client1
        .create_issuer_profile(&dynamic_profile)
        .unwrap();
    assert_eq!(created.id, "dynamic-profile");
    assert_eq!(created.version, 0);

    assert!(
        dynamic_client_node2.get_status().is_ok(),
        "node 2 should authenticate against profile created on node 1"
    );

    let mut deprecated_profile = created.clone();
    deprecated_profile.rollout_state = RolloutState::Deprecated;
    let updated_v1 = admin_client1
        .update_issuer_profile(&deprecated_profile)
        .unwrap();
    assert_eq!(updated_v1.version, 1);

    let deprecated_auth = dynamic_client_node2.get_status();
    assert!(deprecated_auth.is_err());
    assert!(deprecated_auth.unwrap_err().contains("UNAUTHORIZED"));

    let mut reactivated_profile = updated_v1.clone();
    reactivated_profile.audience = "dynamic-app-v2".to_string();
    reactivated_profile.rollout_state = RolloutState::Active;
    let updated_v2 = admin_client1
        .update_issuer_profile(&reactivated_profile)
        .unwrap();
    assert_eq!(updated_v2.version, 2);

    let mut updated_claims = HashMap::new();
    updated_claims.insert(
        "iss",
        Value::String("https://dynamic.example.com".to_string()),
    );
    updated_claims.insert("aud", Value::String("dynamic-app-v2".to_string()));
    updated_claims.insert("sub", Value::String("issuer-user".to_string()));
    updated_claims.insert("role", Value::String("admin".to_string()));
    updated_claims.insert("exp", Value::Number((now + 3600).into()));
    let updated_token = create_token(&updated_claims);
    let updated_client_node2 = TimerCoordinationClient::new(addr2.clone()).with_auth(updated_token);

    assert!(
        updated_client_node2.get_status().is_ok(),
        "node 2 should authenticate against updated profile state from node 1"
    );

    assert!(
        admin_client1
            .delete_issuer_profile("dynamic-profile")
            .unwrap()
    );

    let deleted_auth = updated_client_node2.get_status();
    assert!(deleted_auth.is_err());
    assert!(deleted_auth.unwrap_err().contains("UNAUTHORIZED"));

    stop_signal1.store(true, Ordering::SeqCst);
    stop_signal2.store(true, Ordering::SeqCst);
    let _ = handle1.join();
    let _ = handle2.join();
    let _ = std::fs::remove_file(db_path);
}
