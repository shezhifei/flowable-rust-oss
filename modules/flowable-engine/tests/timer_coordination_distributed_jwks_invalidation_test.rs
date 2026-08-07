use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{AuthPolicy, AuthProviderKind, ServicePolicyConfig};
use flowable_engine::service::issuer_profile::{
    ClaimMappingConfig, ClaimValidation, IssuerProfile, RolloutState,
};
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const TEST_PRIVATE_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCxmF5i94n5EYUV\nhDmXmyS+m+QGUKabS+t+QfXKNkxofaQABsyV/h3mH5rUNQ3DdGeKrvNd0JjP9+yv\n4xH2GIejAl4TjoClPEWs5EB2zVbPr/tFKAVBhkn6sB4RTH/FHfkgaXqMgPSJsfK6\nlukqlwaxmpL6B4dqlGqAu/Z0O8PRhzKo7k1ybLRsFdi+LrpawLvdhXcpHbqIPeqL\ngoQYPGD9wvbZ/BGTYTaXxkJ//lm2UQPvGYmxKh44BH5tAvBotx2aTAL0OJgIpdTw\nTZ3NmrWaa7pd/nijTKaTkkL06weCQmCFLeM+eyQYkEcYbIWkYT1fKr3tHxbX9Lu7\nkfxrFVu/AgMBAAECggEAOy6C+ajzBhikCFUNWiu9tXU+qioTMzo8ClGRxmaM1N9V\nmRqq76sErKzIjEH3ybQPUyRU/mTmn5tHeR+K2z82aAiAcDTzQt0QfPp9TvnDnadP\n7S5Wfgzxt0QcaPhctcP0wqvTxmGs2/v8Xtiub95vQR05MG/03PwDd83rZbWK3lYI\n03gK7LH3H9F7aya4CpF934H8EEKs62zd6utv+UsV+3uJdkbmL/wyibXaJBkKMr5i\nzeeXLAFwQVyzpqpOwI3GgRgjTlLn6XuSi4naMt8Lj2cmfFCz7L+hbPziDYI1VsKg\nqGs/+8Fsy9o+7K9e10IEQRzjNJVEXO7toLL1NB+4wQKBgQDZ11NFLk5bA3nozDvJ\nRJ3aFx7nRtRNvmc25RSzLjw7ceSxh/bbetWw6qsf8z8K3UPkuwPfEEP/HmyLgiC4\ngl8xqwr4Paa4yL3nzbpccmQ2JQGIAlnyyCQDhKwDdwDsRoNS1OwuPuOGhD1Ddvsl\nxCExOI9x38/rzTjWA9EYnIlgwQKBgQDQtEpsEzpWXQzT86KlBRMd34K0mA+iTDHA\n2Hir+ciL6yk6HE/0z+3nVQ7yKZb8tBXXp9JkzNZLPDVxpPIPuwNgexytBse4ORKB\ndmHKw29kDbJ5oh5GdXiE/v078P8woc2hbjd4F2mExcvFDd9m0db9brZM8XVY0EMG\nxaGU9WlcfwKBgExEkBnbgYFp8SepQZFQ3bc6ew5cBP6HGBnnEF0/ZcUmNfxV7v6e\nvewn7OvNvRevqhKNy2gwiK3sV/JsB8qxkmSQTtHku9dcKOjcZU/ymNVAFY4pzJYs\nrjcxHwxDgOY4NcgtVddHG1/AMrbJFFr/lONnuwkSY/hZrHl5cp6cR5jBAoGBAMhR\nxH0nl379oSpvV1V9IXQy7InaymbFK5wmKu0mu09RUCjus/APBBJemhHlyX6Ue8Ka\n2l7WHXnpOIL0B0MCBaO9hzCsqVYxsYmBzyuHmos2enA1I0oNxrgg5395Ofe71lt0\nJtml3yoJkCR7xEo0b16hvWjs+e1dOHhviUAorhCRAoGAQE7xEsRRIAhON++mVHCx\nmdvOR+/x2q7nPNq64BzuQuI6p6gRDtr0g9fYFLjRxtziNXBPxzuspaEC3F3CgPEP\nK4KOPyoPwmt8gtNypaE0yXwn7qRpYyQQhHsJU/yJBfqxqLMayvPveqAzVKDLQxjJ\nO8a0nSZL9fG9vDFviBtM/pY=\n-----END PRIVATE KEY-----";

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
fn test_distributed_jwks_invalidation() {
    let db_path = "test_distributed_jwks_invalidation.db";
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
    config2.auth_provider = AuthProviderKind::External;
    config2.external_provider = Some(
        flowable_engine::service::config::ExternalAuthProviderConfig {
            issuer: "https://auth.example.com".to_string(),
            audience: "my-app".to_string(),
            mapping: flowable_engine::service::config::ExternalClaimMappingConfig::default(),
            trusted_profiles: vec![
                flowable_engine::service::config::ExternalIssuerProfileConfig {
                    id: "profile-1".to_string(),
                    issuer: "https://auth.example.com".to_string(),
                    audience: "my-app".to_string(),
                    mapping: flowable_engine::service::config::ExternalClaimMappingConfig::default(
                    ),
                    validation: flowable_engine::service::config::ExternalValidationConfig::default(
                    ),
                    role_mappings: vec![],
                    required_tenant: false,
                    rollout_state: RolloutState::Active,
                    jwks_uri: Some("test-local".to_string()),
                    allowed_algorithms: Some(vec!["RS256".to_string()]),
                    jwks_cache_ttl_seconds: 30,
                    jwks_refresh_policy: Default::default(),
                    version: 0,
                },
            ],
        },
    );

    let stop_signal2 = Arc::new(AtomicBool::new(false));
    let service2 = TimerCoordinationService::new(Arc::clone(&runtime_service2), config2);
    let handle2 = service2.start(Arc::clone(&stop_signal2));

    std::thread::sleep(Duration::from_millis(50));

    let admin_client1 =
        TimerCoordinationClient::new(addr1.clone()).with_auth("admin-secret-1".to_string());

    // 1. Create a profile with jwks_uri="test-local" on Node 1
    let profile = IssuerProfile {
        id: "profile-1".to_string(),
        issuer: "https://auth.example.com".to_string(),
        audience: "my-app".to_string(),
        mapping: ClaimMappingConfig::default(),
        validation: ClaimValidation::default(),
        role_mappings: vec![],
        required_tenant: false,
        rollout_state: RolloutState::Active,
        jwks_uri: Some("test-local".to_string()),
        allowed_algorithms: vec!["RS256".to_string()],
        jwks_cache_ttl_seconds: 30,
        jwks_refresh_policy: Default::default(),
        version: 0,
    };
    admin_client1.create_issuer_profile(&profile).unwrap();

    // Allow DB to propagate
    std::thread::sleep(Duration::from_millis(100));

    let token = create_token("https://auth.example.com", "test-kid");

    // 2. Auth on Node 2 succeeds, caches the key.
    let user_client2 = TimerCoordinationClient::new(addr2.clone()).with_auth(token.clone());
    let status_res = user_client2.get_status();
    println!("Status Res: {:?}", status_res);
    assert!(status_res.is_ok());

    // 3. Change the profile's jwks_uri to "invalid-uri" on Node 1
    let mut updated_profile = profile.clone();
    updated_profile.jwks_uri = Some("http://localhost:1/nonexistent".to_string());
    admin_client1
        .update_issuer_profile(&updated_profile)
        .unwrap();

    // Allow DB to propagate
    std::thread::sleep(Duration::from_millis(50));

    // 4. Auth on Node 2 should fail because Node 2 reads the DB, gets the new URI,
    // and since cached key uri doesn't match new uri, it attempts to fetch and fails.
    let status_res = user_client2.get_status();
    assert!(
        status_res.is_err(),
        "Expected auth to fail on Node 2 after jwks_uri update invalidated cache"
    );

    stop_signal1.store(true, Ordering::SeqCst);
    stop_signal2.store(true, Ordering::SeqCst);
    let _ = handle1.join();
    let _ = handle2.join();
    let _ = std::fs::remove_file(db_path);
}
