use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{
    AuthPolicy, AuthProviderKind, ExternalAuthProviderConfig, ExternalClaimMappingConfig,
    ServicePolicyConfig,
};
use flowable_engine::service::issuer_profile::RolloutState;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
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
fn test_external_auth_rejects_local_static_tokens() {
    let db_path = "test_external_auth_fail_closed.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let mut config = ServicePolicyConfig::default();
    config.bind_addr = format!("127.0.0.1:{}", port);
    config.auth_provider = AuthProviderKind::External;
    config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "test-issuer".to_string(),
        audience: "flowable-timer".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![
            flowable_engine::service::config::ExternalIssuerProfileConfig {
                id: "profile-1".to_string(),
                issuer: "test-issuer".to_string(),
                audience: "flowable-timer".to_string(),
                mapping: ExternalClaimMappingConfig::default(),
                validation: flowable_engine::service::config::ExternalValidationConfig::default(),
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
    });

    config.auth_keys.insert(
        "local-secret".to_string(),
        AuthPolicy {
            actor_id: "local-admin".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: None,
        },
    );

    let db_store = Arc::new(DbStore::new_file(db_path).unwrap());
    let engine = ProcessEngine::build(
        "test-owner".to_string(),
        Arc::new(SystemTimeSource),
        db_store.clone(),
    );
    let runtime_service = engine.get_runtime_service();

    let service = TimerCoordinationService::new(runtime_service.clone(), config);
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
    claims.insert("role", Value::String("admin".to_string()));
    claims.insert("exp", Value::Number((now + 3600).into()));
    let token = create_token(&claims);

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
    let req = format!(
        "GET /status HTTP/1.1\r\nAuthorization: Bearer {}\r\n\r\n",
        token
    );
    stream.write_all(req.as_bytes()).unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    assert!(response.contains("200 OK"), "External token should succeed");

    let mut stream2 = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
    let req2 = "GET /status HTTP/1.1\r\nAuthorization: Bearer local-secret\r\n\r\n".to_string();
    stream2.write_all(req2.as_bytes()).unwrap();

    let mut response2 = String::new();
    stream2.read_to_string(&mut response2).unwrap();

    assert!(
        response2.contains("401 Unauthorized"),
        "Local static token should fail in external auth mode"
    );

    let mut stream3 = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
    let req3 = "GET /status HTTP/1.1\r\nAuthorization: Bearer invalid-token\r\n\r\n".to_string();
    stream3.write_all(req3.as_bytes()).unwrap();

    let mut response3 = String::new();
    stream3.read_to_string(&mut response3).unwrap();

    assert!(
        response3.contains("401 Unauthorized"),
        "Invalid token should fail"
    );

    let mut collision_claims = HashMap::new();
    collision_claims.insert("iss", Value::String("wrong-issuer".to_string()));
    collision_claims.insert("aud", Value::String("flowable-timer".to_string()));
    collision_claims.insert("sub", Value::String("ext-user-2".to_string()));
    collision_claims.insert("role", Value::String("admin".to_string()));
    collision_claims.insert("exp", Value::Number((now + 3600).into()));
    let collision_token = create_token(&collision_claims);

    let mut collision_config = ServicePolicyConfig::default();
    let collision_port = get_free_port();
    collision_config.bind_addr = format!("127.0.0.1:{}", collision_port);
    collision_config.auth_provider = AuthProviderKind::External;
    collision_config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "test-issuer".to_string(),
        audience: "flowable-timer".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![
            flowable_engine::service::config::ExternalIssuerProfileConfig {
                id: "profile-1".to_string(),
                issuer: "test-issuer".to_string(),
                audience: "flowable-timer".to_string(),
                mapping: ExternalClaimMappingConfig::default(),
                validation: flowable_engine::service::config::ExternalValidationConfig::default(),
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
    });
    collision_config.auth_keys.insert(
        collision_token.clone(),
        AuthPolicy {
            actor_id: "colliding-local-admin".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: None,
        },
    );

    let collision_db_path = "test_external_auth_token_collision.db";
    let _ = std::fs::remove_file(collision_db_path);
    let collision_db_store = Arc::new(DbStore::new_file(collision_db_path).unwrap());
    let collision_engine = ProcessEngine::build(
        "test-owner-2".to_string(),
        Arc::new(SystemTimeSource),
        collision_db_store.clone(),
    );
    let collision_runtime_service = collision_engine.get_runtime_service();
    let collision_service =
        TimerCoordinationService::new(collision_runtime_service.clone(), collision_config);
    let collision_stop_signal = Arc::new(AtomicBool::new(false));
    let collision_handle = collision_service.start(Arc::clone(&collision_stop_signal));

    std::thread::sleep(Duration::from_millis(100));

    let mut collision_stream = TcpStream::connect(format!("127.0.0.1:{}", collision_port)).unwrap();
    let collision_req = format!(
        "GET /status HTTP/1.1\r\nAuthorization: Bearer {}\r\n\r\n",
        collision_token
    );
    collision_stream
        .write_all(collision_req.as_bytes())
        .unwrap();

    let mut collision_response = String::new();
    collision_stream
        .read_to_string(&mut collision_response)
        .unwrap();

    assert!(
        collision_response.contains("401 Unauthorized"),
        "External-looking token must not silently fall back to local-static even when a local key collides"
    );

    collision_stop_signal.store(true, Ordering::SeqCst);
    let _ = collision_handle.join();
    let _ = std::fs::remove_file(collision_db_path);

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}
