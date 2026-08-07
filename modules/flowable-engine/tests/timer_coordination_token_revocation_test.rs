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

fn external_config(bind_addr: String) -> ServicePolicyConfig {
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
        issuer: "test-issuer".to_string(),
        audience: "flowable-timer".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![ExternalIssuerProfileConfig {
            id: "profile-1".to_string(),
            issuer: "test-issuer".to_string(),
            audience: "flowable-timer".to_string(),
            mapping: ExternalClaimMappingConfig {
                actor_id_claim: "sub".to_string(),
                subject_claim: "sub".to_string(),
                issuer_claim: "iss".to_string(),
                tenant_id_claim: Some("tenant".to_string()),
                role_claim: "role".to_string(),
            },
            validation: ExternalValidationConfig::default(),
            role_mappings: vec![],
            required_tenant: false,
            rollout_state: RolloutState::Active,
            jwks_uri: Some("test-local".to_string()),
            allowed_algorithms: Some(vec!["RS256".to_string()]),
            jwks_cache_ttl_seconds: 30,
            jwks_refresh_policy: Default::default(),
            version: 0,
        }],
    });
    config
}

#[test]
fn test_revoked_external_token_is_rejected_and_token_without_jti_is_unaffected() {
    let db_path = "test_timer_coordination_token_revocation.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let bind_addr = format!("127.0.0.1:{}", port);
    let config = external_config(bind_addr.clone());

    let db_store = Arc::new(DbStore::new_file(db_path).unwrap());
    let engine = ProcessEngine::build(
        "token-revocation-node".to_string(),
        Arc::new(SystemTimeSource),
        db_store,
    );
    let runtime_service = engine.get_runtime_service();

    let service = TimerCoordinationService::new(runtime_service, config);
    let stop_signal = Arc::new(AtomicBool::new(false));
    let handle = service.start(Arc::clone(&stop_signal));
    std::thread::sleep(Duration::from_millis(100));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut claims = HashMap::new();
    claims.insert("iss", Value::String("test-issuer".to_string()));
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("ext-user-1".to_string()));
    claims.insert("tenant", Value::String("tenant-A".to_string()));
    claims.insert("role", Value::String("admin".to_string()));
    claims.insert("jti", Value::String("revoked-jti".to_string()));
    claims.insert("exp", Value::Number((now + 3600).into()));
    let revoked_token = create_token(&claims);

    let mut no_jti_claims = HashMap::new();
    no_jti_claims.insert("iss", Value::String("test-issuer".to_string()));
    no_jti_claims.insert("aud", Value::String("flowable-timer".to_string()));
    no_jti_claims.insert("sub", Value::String("ext-user-2".to_string()));
    no_jti_claims.insert("tenant", Value::String("tenant-A".to_string()));
    no_jti_claims.insert("role", Value::String("admin".to_string()));
    no_jti_claims.insert("exp", Value::Number((now + 3600).into()));
    let no_jti_token = create_token(&no_jti_claims);

    let revoked_client = TimerCoordinationClient::new(bind_addr.clone()).with_auth(revoked_token);
    let no_jti_client = TimerCoordinationClient::new(bind_addr.clone()).with_auth(no_jti_token);
    let mut admin_claims = HashMap::new();
    admin_claims.insert("iss", Value::String("test-issuer".to_string()));
    admin_claims.insert("aud", Value::String("flowable-timer".to_string()));
    admin_claims.insert("sub", Value::String("admin-user".to_string()));
    admin_claims.insert("role", Value::String("admin".to_string()));
    admin_claims.insert("exp", Value::Number((now + 3600).into()));
    let admin_token = create_token(&admin_claims);
    let admin_client = TimerCoordinationClient::new(bind_addr.clone()).with_auth(admin_token);

    assert!(revoked_client.get_status().is_ok());
    assert!(no_jti_client.get_status().is_ok());

    assert!(
        admin_client
            .revoke_token("revoked-jti", "test-issuer", "compromised")
            .unwrap()
    );

    let revoked_result = revoked_client.get_status();
    assert!(revoked_result.is_err());
    assert!(
        revoked_result.unwrap_err().contains("UNAUTHORIZED"),
        "revoked token should fail authentication"
    );

    assert!(
        no_jti_client.get_status().is_ok(),
        "tokens without jti should remain unaffected by per-token revocation"
    );

    assert!(admin_client.unrevoke_token("revoked-jti").unwrap());
    assert!(
        revoked_client.get_status().is_ok(),
        "token should authenticate again after unrevoke"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}
