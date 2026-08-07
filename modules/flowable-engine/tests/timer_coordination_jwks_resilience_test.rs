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
fn test_timer_coordination_negative_cache_cleared_on_mutation() {
    let db_path = format!(
        "file:test_jwks_resilience_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "worker_node".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );

    let runtime_service = engine.get_runtime_service();

    let mut config = ServicePolicyConfig::default();
    let random_port = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let actual_addr = format!("127.0.0.1:{}", random_port);
    config.bind_addr = actual_addr.clone();
    config.auth_provider = AuthProviderKind::External;
    config.external_provider = Some(
        flowable_engine::service::config::ExternalAuthProviderConfig {
            issuer: String::new(),
            audience: String::new(),
            mapping: flowable_engine::service::config::ExternalClaimMappingConfig::default(),
            trusted_profiles: vec![
                flowable_engine::service::config::ExternalIssuerProfileConfig {
                    id: "admin-profile".to_string(),
                    issuer: "https://admin.example.com".to_string(),
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
                    jwks_cache_ttl_seconds: 3600,
                    jwks_refresh_policy: Default::default(),
                    version: 0,
                },
            ],
        },
    );

    // Setup an admin secret
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

    let admin_token = create_token("https://admin.example.com", "test-kid");
    let admin_client = TimerCoordinationClient::new(actual_addr.clone()).with_auth(admin_token);

    let issuer = "https://auth.example.com";
    let kid = "test-kid";
    let token = create_token(issuer, kid);
    let user_client = TimerCoordinationClient::new(actual_addr.clone()).with_auth(token.clone());

    // 1. Create a profile with INCORRECT jwks_uri.
    let mut profile = IssuerProfile {
        id: "profile-1".to_string(),
        issuer: issuer.to_string(),
        audience: "my-app".to_string(),
        mapping: ClaimMappingConfig::default(),
        validation: ClaimValidation::default(),
        role_mappings: vec![],
        required_tenant: false,
        rollout_state: RolloutState::Active,
        jwks_uri: Some("http://localhost:1/nonexistent".to_string()),
        allowed_algorithms: vec!["RS256".to_string()],
        jwks_cache_ttl_seconds: 3600,
        jwks_refresh_policy: JwksRefreshPolicy {
            negative_cache_seconds: 600, // 10 minutes of negative cache
            ..Default::default()
        },
        version: 0,
    };
    admin_client.create_issuer_profile(&profile).unwrap();

    // 2. Try to auth. It fails and creates a NEGATIVE CACHE entry for (issuer, kid).
    assert!(user_client.get_status().is_err());

    // 3. Update profile with CORRECT jwks_uri.
    // If we DID NOT invalidate, auth would still fail because of the 10-minute negative cache!
    profile.jwks_uri = Some("test-local".to_string());
    admin_client.update_issuer_profile(&profile).unwrap();

    // 4. Auth should now succeed because update invalidated the negative cache.
    let status_res = user_client.get_status();
    assert!(
        status_res.is_ok(),
        "Expected auth to succeed after fixing jwks_uri because negative cache should be cleared"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
}
