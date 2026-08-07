use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{
    AuthProviderKind, ExternalAuthProviderConfig, ExternalClaimMappingConfig,
    ExternalIssuerProfileConfig, ExternalRoleMappingConfig, ExternalValidationConfig,
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
fn test_trusted_profile_exact_match() {
    let db_path = "test_trusted_profile_exact.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let mut config = ServicePolicyConfig {
        bind_addr: format!("127.0.0.1:{}", port),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };

    let profile = ExternalIssuerProfileConfig {
        id: "profile-1".to_string(),
        issuer: "https://issuer-1.example.com".to_string(),
        audience: "flowable-timer".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        validation: ExternalValidationConfig::default(),
        role_mappings: vec![ExternalRoleMappingConfig {
            external_role: "superadmin".to_string(),
            internal_role: "admin".to_string(),
        }],
        required_tenant: false,
        rollout_state: RolloutState::Active,
        jwks_uri: Some("test-local".to_string()),
        allowed_algorithms: Some(vec!["RS256".to_string()]),
        jwks_cache_ttl_seconds: 30,
        jwks_refresh_policy: Default::default(),
        version: 0,
    };

    config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "".to_string(),
        audience: "".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![profile],
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
    claims.insert(
        "iss",
        Value::String("https://issuer-1.example.com".to_string()),
    );
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("user-1".to_string()));
    claims.insert("role", Value::String("superadmin".to_string()));
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
        "Token matching trusted profile should be accepted"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_same_issuer_second_profile_can_match_after_first_rejects() {
    let db_path = "test_same_issuer_second_profile.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let mut config = ServicePolicyConfig {
        bind_addr: format!("127.0.0.1:{}", port),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };

    let profile1 = ExternalIssuerProfileConfig {
        id: "same-issuer-a".to_string(),
        issuer: "https://same-issuer.example.com".to_string(),
        audience: "wrong-audience".to_string(),
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

    let profile2 = ExternalIssuerProfileConfig {
        id: "same-issuer-b".to_string(),
        issuer: "https://same-issuer.example.com".to_string(),
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

    config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "".to_string(),
        audience: "".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![profile1, profile2],
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
    claims.insert(
        "iss",
        Value::String("https://same-issuer.example.com".to_string()),
    );
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("same-issuer-user".to_string()));
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
        "Later profile with same issuer and correct audience should still authenticate"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_deprecated_profile_rejected() {
    let db_path = "test_deprecated_profile.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let mut config = ServicePolicyConfig {
        bind_addr: format!("127.0.0.1:{}", port),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };

    let profile = ExternalIssuerProfileConfig {
        id: "deprecated-profile".to_string(),
        issuer: "https://deprecated-issuer.example.com".to_string(),
        audience: "flowable-timer".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        validation: ExternalValidationConfig::default(),
        role_mappings: vec![],
        required_tenant: false,
        rollout_state: RolloutState::Deprecated,
        jwks_uri: Some("test-local".to_string()),
        allowed_algorithms: Some(vec!["RS256".to_string()]),
        jwks_cache_ttl_seconds: 30,
        jwks_refresh_policy: Default::default(),
        version: 0,
    };

    config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "".to_string(),
        audience: "".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![profile],
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
    claims.insert(
        "iss",
        Value::String("https://deprecated-issuer.example.com".to_string()),
    );
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("user-1".to_string()));
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
        response.contains("401 Unauthorized"),
        "Token from deprecated profile should be rejected"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_multi_profile_first_active_wins() {
    let db_path = "test_multi_profile.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let mut config = ServicePolicyConfig {
        bind_addr: format!("127.0.0.1:{}", port),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };

    let profile1 = ExternalIssuerProfileConfig {
        id: "profile-a".to_string(),
        issuer: "https://issuer-a.example.com".to_string(),
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

    let profile2 = ExternalIssuerProfileConfig {
        id: "profile-b".to_string(),
        issuer: "https://issuer-b.example.com".to_string(),
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

    config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "".to_string(),
        audience: "".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![profile1, profile2],
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
    claims.insert(
        "iss",
        Value::String("https://issuer-b.example.com".to_string()),
    );
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("user-from-b".to_string()));
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
        "Token matching second trusted profile should be accepted"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_role_mapping_in_trusted_profile() {
    let db_path = "test_role_mapping.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let mut config = ServicePolicyConfig {
        bind_addr: format!("127.0.0.1:{}", port),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };

    let profile = ExternalIssuerProfileConfig {
        id: "role-map-profile".to_string(),
        issuer: "https://role-map-issuer.example.com".to_string(),
        audience: "flowable-timer".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        validation: ExternalValidationConfig::default(),
        role_mappings: vec![
            ExternalRoleMappingConfig {
                external_role: "external-admin".to_string(),
                internal_role: "admin".to_string(),
            },
            ExternalRoleMappingConfig {
                external_role: "external-read".to_string(),
                internal_role: "read".to_string(),
            },
        ],
        required_tenant: false,
        rollout_state: RolloutState::Active,
        jwks_uri: Some("test-local".to_string()),
        allowed_algorithms: Some(vec!["RS256".to_string()]),
        jwks_cache_ttl_seconds: 30,
        jwks_refresh_policy: Default::default(),
        version: 0,
    };

    config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "".to_string(),
        audience: "".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![profile],
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
    claims.insert(
        "iss",
        Value::String("https://role-map-issuer.example.com".to_string()),
    );
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("mapped-user".to_string()));
    claims.insert("role", Value::String("external-admin".to_string()));
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
        "Token with mapped role should be accepted"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_required_tenant_enforcement() {
    let db_path = "test_required_tenant.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let mut config = ServicePolicyConfig {
        bind_addr: format!("127.0.0.1:{}", port),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };

    let mapping = ExternalClaimMappingConfig {
        tenant_id_claim: Some("tenant".to_string()),
        ..Default::default()
    };

    let profile = ExternalIssuerProfileConfig {
        id: "tenant-profile".to_string(),
        issuer: "https://tenant-issuer.example.com".to_string(),
        audience: "flowable-timer".to_string(),
        mapping,
        validation: ExternalValidationConfig::default(),
        role_mappings: vec![],
        required_tenant: true,
        rollout_state: RolloutState::Active,
        jwks_uri: Some("test-local".to_string()),
        allowed_algorithms: Some(vec!["RS256".to_string()]),
        jwks_cache_ttl_seconds: 30,
        jwks_refresh_policy: Default::default(),
        version: 0,
    };

    config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "".to_string(),
        audience: "".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![profile],
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
    claims.insert(
        "iss",
        Value::String("https://tenant-issuer.example.com".to_string()),
    );
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("user-1".to_string()));
    claims.insert("role", Value::String("admin".to_string()));
    claims.insert("tenant", Value::String("tenant-a".to_string()));
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
        "Token with required tenant should be accepted"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_required_tenant_missing_rejected() {
    let db_path = "test_tenant_missing.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let mut config = ServicePolicyConfig {
        bind_addr: format!("127.0.0.1:{}", port),
        auth_provider: AuthProviderKind::External,
        ..Default::default()
    };

    let mapping = ExternalClaimMappingConfig {
        tenant_id_claim: Some("tenant".to_string()),
        ..Default::default()
    };

    let profile = ExternalIssuerProfileConfig {
        id: "tenant-profile-strict".to_string(),
        issuer: "https://tenant-issuer-strict.example.com".to_string(),
        audience: "flowable-timer".to_string(),
        mapping,
        validation: ExternalValidationConfig::default(),
        role_mappings: vec![],
        required_tenant: true,
        rollout_state: RolloutState::Active,
        jwks_uri: Some("test-local".to_string()),
        allowed_algorithms: Some(vec!["RS256".to_string()]),
        jwks_cache_ttl_seconds: 30,
        jwks_refresh_policy: Default::default(),
        version: 0,
    };

    config.external_provider = Some(ExternalAuthProviderConfig {
        issuer: "".to_string(),
        audience: "".to_string(),
        mapping: ExternalClaimMappingConfig::default(),
        trusted_profiles: vec![profile],
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
    claims.insert(
        "iss",
        Value::String("https://tenant-issuer-strict.example.com".to_string()),
    );
    claims.insert("aud", Value::String("flowable-timer".to_string()));
    claims.insert("sub", Value::String("user-1".to_string()));
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
        response.contains("401 Unauthorized"),
        "Token without required tenant should be rejected"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}
