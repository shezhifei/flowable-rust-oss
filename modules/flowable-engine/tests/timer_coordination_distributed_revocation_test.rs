use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{
    AuthProviderKind, ExternalAuthProviderConfig, ExternalClaimMappingConfig, ServicePolicyConfig,
};
use flowable_engine::service::issuer_profile::{
    ClaimMappingConfig, ClaimValidation, IssuerProfile, RolloutState,
};
use flowable_engine::service::jwks::JwksCache;
use flowable_engine::service::revocation::TokenRevocationRegistry;
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// Helpers
fn get_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

const TEST_PRIVATE_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCxmF5i94n5EYUV\nhDmXmyS+m+QGUKabS+t+QfXKNkxofaQABsyV/h3mH5rUNQ3DdGeKrvNd0JjP9+yv\n4xH2GIejAl4TjoClPEWs5EB2zVbPr/tFKAVBhkn6sB4RTH/FHfkgaXqMgPSJsfK6\nlukqlwaxmpL6B4dqlGqAu/Z0O8PRhzKo7k1ybLRsFdi+LrpawLvdhXcpHbqIPeqL\ngoQYPGD9wvbZ/BGTYTaXxkJ//lm2UQPvGYmxKh44BH5tAvBotx2aTAL0OJgIpdTw\nTZ3NmrWaa7pd/nijTKaTkkL06weCQmCFLeM+eyQYkEcYbIWkYT1fKr3tHxbX9Lu7\nkfxrFVu/AgMBAAECggEAOy6C+ajzBhikCFUNWiu9tXU+qioTMzo8ClGRxmaM1N9V\nmRqq76sErKzIjEH3ybQPUyRU/mTmn5tHeR+K2z82aAiAcDTzQt0QfPp9TvnDnadP\n7S5Wfgzxt0QcaPhctcP0wqvTxmGs2/v8Xtiub95vQR05MG/03PwDd83rZbWK3lYI\n03gK7LH3H9F7aya4CpF934H8EEKs62zd6utv+UsV+3uJdkbmL/wyibXaJBkKMr5i\nzeeXLAFwQVyzpqpOwI3GgRgjTlLn6XuSi4naMt8Lj2cmfFCz7L+hbPziDYI1VsKg\nqGs/+8Fsy9o+7K9e10IEQRzjNJVEXO7toLL1NB+4wQKBgQDZ11NFLk5bA3nozDvJ\nRJ3aFx7nRtRNvmc25RSzLjw7ceSxh/bbetWw6qsf8z8K3UPkuwPfEEP/HmyLgiC4\ngl8xqwr4Paa4yL3nzbpccmQ2JQGIAlnyyCQDhKwDdwDsRoNS1OwuPuOGhD1Ddvsl\nxCExOI9x38/rzTjWA9EYnIlgwQKBgQDQtEpsEzpWXQzT86KlBRMd34K0mA+iTDHA\n2Hir+ciL6yk6HE/0z+3nVQ7yKZb8tBXXp9JkzNZLPDVxpPIPuwNgexytBse4ORKB\ndmHKw29kDbJ5oh5GdXiE/v078P8woc2hbjd4F2mExcvFDd9m0db9brZM8XVY0EMG\nxaGU9WlcfwKBgExEkBnbgYFp8SepQZFQ3bc6ew5cBP6HGBnnEF0/ZcUmNfxV7v6e\nvewn7OvNvRevqhKNy2gwiK3sV/JsB8qxkmSQTtHku9dcKOjcZU/ymNVAFY4pzJYs\nrjcxHwxDgOY4NcgtVddHG1/AMrbJFFr/lONnuwkSY/hZrHl5cp6cR5jBAoGBAMhR\nxH0nl379oSpvV1V9IXQy7InaymbFK5wmKu0mu09RUCjus/APBBJemhHlyX6Ue8Ka\n2l7WHXnpOIL0B0MCBaO9hzCsqVYxsYmBzyuHmos2enA1I0oNxrgg5395Ofe71lt0\nJtml3yoJkCR7xEo0b16hvWjs+e1dOHhviUAorhCRAoGAQE7xEsRRIAhON++mVHCx\nmdvOR+/x2q7nPNq64BzuQuI6p6gRDtr0g9fYFLjRxtziNXBPxzuspaEC3F3CgPEP\nK4KOPyoPwmt8gtNypaE0yXwn7qRpYyQQhHsJU/yJBfqxqLMayvPveqAzVKDLQxjJ\nO8a0nSZL9fG9vDFviBtM/pY=\n-----END PRIVATE KEY-----";

fn get_test_jwk() -> jsonwebtoken::jwk::Jwk {
    serde_json::from_str(r#"{
        "kty": "RSA",
        "kid": "test-kid",
        "n": "sZheYveJ-RGFFYQ5l5skvpvkBlCmm0vrfkH1yjZMaH2kAAbMlf4d5h-a1DUNw3Rniq7zXdCYz_fsr-MR9hiHowJeE46ApTxFrORAds1Wz6_7RSgFQYZJ-rAeEUx_xR35IGl6jID0ibHyupbpKpcGsZqS-geHapRqgLv2dDvD0YcyqO5Ncmy0bBXYvi66WsC73YV3KR26iD3qi4KEGDxg_cL22fwRk2E2l8ZCf_5ZtlED7xmJsSoeOAR-bQLwaLcdmkwC9DiYCKXU8E2dzZq1mmu6Xf54o0ymk5JC9OsHgkJghS3jPnskGJBHGGyFpGE9Xyq97R8W1_S7u5H8axVbvw",
        "e": "AQAB"
    }"#).unwrap()
}

fn create_token(claims: &HashMap<&str, Value>) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-kid".to_string());
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM).unwrap();
    jsonwebtoken::encode(&header, claims, &encoding_key).unwrap()
}

#[test]
fn test_distributed_revocation_coherence() {
    let db_path = "test_distributed_revocation_coherence.db";
    let _ = std::fs::remove_file(db_path);

    // Common DB Store shared by both nodes
    let db_store = Arc::new(DbStore::new_file(db_path).unwrap());

    // Setup Node 1
    let port1 = get_free_port();
    let addr1 = format!("127.0.0.1:{}", port1);

    let mut config1 = ServicePolicyConfig {
        bind_addr: addr1.clone(),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };
    config1.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "https://distributed-test.example.com".to_string(),
        audience: "flowable-timer".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![],
    });

    let engine1 = ProcessEngine::build(
        "node-1".to_string(),
        Arc::new(SystemTimeSource),
        db_store.clone(),
    );
    let runtime_service1 = engine1.get_runtime_service();

    // Inject JWK into JWKS Cache manually via a component build
    let jwks_cache1 = Arc::new(JwksCache::new());
    jwks_cache1.inject_key(
        "https://distributed-test.example.com",
        "test-kid",
        get_test_jwk(),
    );

    let profile = IssuerProfile {
        id: "profile-1".to_string(),
        issuer: "https://distributed-test.example.com".to_string(),
        audience: "flowable-timer".to_string(),
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

    let revocation_registry1 = Arc::new(TokenRevocationRegistry::new(engine1.get_runtime_store()));

    let service1 = TimerCoordinationService::with_identity_components(
        runtime_service1,
        config1,
        vec![profile.clone()],
        jwks_cache1.clone(),
        revocation_registry1.clone(),
    );

    let stop_signal1 = Arc::new(AtomicBool::new(false));
    let handle1 = service1.start(Arc::clone(&stop_signal1));

    // Setup Node 2
    let port2 = get_free_port();
    let addr2 = format!("127.0.0.1:{}", port2);

    let mut config2 = ServicePolicyConfig {
        bind_addr: addr2.clone(),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };
    config2.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "https://distributed-test.example.com".to_string(),
        audience: "flowable-timer".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![],
    });

    let engine2 = ProcessEngine::build(
        "node-2".to_string(),
        Arc::new(SystemTimeSource),
        db_store.clone(),
    );
    let runtime_service2 = engine2.get_runtime_service();

    let jwks_cache2 = Arc::new(JwksCache::new());
    jwks_cache2.inject_key(
        "https://distributed-test.example.com",
        "test-kid",
        get_test_jwk(),
    );

    let revocation_registry2 = Arc::new(TokenRevocationRegistry::new(engine2.get_runtime_store()));

    let service2 = TimerCoordinationService::with_identity_components(
        runtime_service2,
        config2,
        vec![profile.clone()],
        jwks_cache2.clone(),
        revocation_registry2.clone(),
    );

    let stop_signal2 = Arc::new(AtomicBool::new(false));
    let handle2 = service2.start(Arc::clone(&stop_signal2));

    std::thread::sleep(Duration::from_millis(200));

    // Generate Token
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut claims = HashMap::new();
    claims.insert(
        "iss",
        Value::String("https://distributed-test.example.com".to_string()),
    );
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("user-123".to_string()));
    claims.insert("role", Value::String("admin".to_string()));
    claims.insert("jti", Value::String("jti-dist-1".to_string()));
    claims.insert("exp", Value::Number((now + 3600).into()));

    let token = create_token(&claims);

    // Use Node 2 client with token to verify initial auth
    let client2 = TimerCoordinationClient::new(addr2.clone()).with_auth(token.clone());
    let status_res = client2.get_status();
    assert!(status_res.is_ok(), "Initial auth on Node 2 should succeed");

    // Admin Revoke using Node 1 Registry
    revocation_registry1.admin_revoke(
        "jti-dist-1",
        "https://distributed-test.example.com",
        "dist-test",
    );

    // Wait a brief moment for db commit to settle
    std::thread::sleep(Duration::from_millis(50));

    // Authenticate on Node 2 should now fail!
    let status_res2 = client2.get_status();
    assert!(status_res2.is_err());
    assert!(
        status_res2.unwrap_err().contains("UNAUTHORIZED"),
        "Node 2 should reject token revoked by Node 1"
    );

    // Admin Unrevoke on Node 1
    revocation_registry1.admin_unrevoke("jti-dist-1");
    std::thread::sleep(Duration::from_millis(50));

    // Authenticate on Node 2 should succeed again!
    let status_res3 = client2.get_status();
    assert!(
        status_res3.is_ok(),
        "Node 2 should accept token after unrevoke by Node 1"
    );

    stop_signal1.store(true, Ordering::SeqCst);
    stop_signal2.store(true, Ordering::SeqCst);
    let _ = handle1.join();
    let _ = handle2.join();
    let _ = std::fs::remove_file(db_path);
}
