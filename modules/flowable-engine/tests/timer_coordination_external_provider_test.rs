use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{
    AuthProviderKind, ExternalAuthProviderConfig, ExternalClaimMappingConfig, ServicePolicyConfig,
};
use flowable_engine::service::issuer_profile::{JwksRefreshPolicy, RolloutState};
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
fn test_external_provider_auth_success() {
    let db_path = "test_external_provider_auth_success.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let mut config = ServicePolicyConfig {
        bind_addr: format!("127.0.0.1:{}", port),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };
    config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "test-issuer".to_string(),
        audience: "flowable-timer".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![
            flowable_engine::service::config::ExternalIssuerProfileConfig {
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
                validation: flowable_engine::service::config::ExternalValidationConfig::default(),
                role_mappings: vec![],
                required_tenant: false,
                rollout_state: RolloutState::Active,
                jwks_uri: Some("test-local".to_string()),
                allowed_algorithms: Some(vec!["RS256".to_string()]),
                jwks_cache_ttl_seconds: 30,
                jwks_refresh_policy: JwksRefreshPolicy::default(),
                version: 0,
            },
        ],
    });

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
    claims.insert("tenant", Value::String("tenant-A".to_string()));
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

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {}",
        response
    );

    let mut bad_claims = HashMap::new();
    bad_claims.insert("iss", Value::String("bad-issuer".to_string()));
    bad_claims.insert("aud", Value::String("flowable-timer".to_string()));
    bad_claims.insert("sub", Value::String("ext-user-1".to_string()));
    bad_claims.insert("tenant", Value::String("tenant-A".to_string()));
    bad_claims.insert("role", Value::String("admin".to_string()));
    bad_claims.insert("exp", Value::Number((now + 3600).into()));
    let bad_token = create_token(&bad_claims);

    let mut stream2 = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
    let req2 = format!(
        "GET /status HTTP/1.1\r\nAuthorization: Bearer {}\r\n\r\n",
        bad_token
    );
    stream2.write_all(req2.as_bytes()).unwrap();

    let mut response2 = String::new();
    stream2.read_to_string(&mut response2).unwrap();

    assert!(
        response2.contains("401 Unauthorized"),
        "Expected 401, got: {}",
        response2
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_external_provider_missing_config_fails_closed() {
    let db_path = "test_external_provider_missing_config.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let config = ServicePolicyConfig {
        bind_addr: format!("127.0.0.1:{}", port),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };

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

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
    let req = "GET /status HTTP/1.1\r\nAuthorization: Bearer local-secret\r\n\r\n";
    stream.write_all(req.as_bytes()).unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    assert!(
        response.contains("401 Unauthorized"),
        "External mode without config must fail closed"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}
