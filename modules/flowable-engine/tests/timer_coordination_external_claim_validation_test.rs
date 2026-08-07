use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{
    AuthProviderKind, ExternalAuthProviderConfig, ExternalClaimMappingConfig, ServicePolicyConfig,
};
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

const TEST_PRIVATE_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCxmF5i94n5EYUV
hDmXmyS+m+QGUKabS+t+QfXKNkxofaQABsyV/h3mH5rUNQ3DdGeKrvNd0JjP9+yv
4xH2GIejAl4TjoClPEWs5EB2zVbPr/tFKAVBhkn6sB4RTH/FHfkgaXqMgPSJsfK6
lukqlwaxmpL6B4dqlGqAu/Z0O8PRhzKo7k1ybLRsFdi+LrpawLvdhXcpHbqIPeqL
goQYPGD9wvbZ/BGTYTaXxkJ//lm2UQPvGYmxKh44BH5tAvBotx2aTAL0OJgIpdTw
TZ3NmrWaa7pd/nijTKaTkkL06weCQmCFLeM+eyQYkEcYbIWkYT1fKr3tHxbX9Lu7
kfxrFVu/AgMBAAECggEAOy6C+ajzBhikCFUNWiu9tXU+qioTMzo8ClGRxmaM1N9V
mRqq76sErKzIjEH3ybQPUyRU/mTmn5tHeR+K2z82aAiAcDTzQt0QfPp9TvnDnadP
7S5Wfgzxt0QcaPhctcP0wqvTxmGs2/v8Xtiub95vQR05MG/03PwDd83rZbWK3lYI
03gK7LH3H9F7aya4CpF934H8EEKs62zd6utv+UsV+3uJdkbmL/wyibXaJBkKMr5i
zeeXLAFwQVyzpqpOwI3GgRgjTlLn6XuSi4naMt8Lj2cmfFCz7L+hbPziDYI1VsKg
qGs/+8Fsy9o+7K9e10IEQRzjNJVEXO7toLL1NB+4wQKBgQDZ11NFLk5bA3nozDvJ
RJ3aFx7nRtRNvmc25RSzLjw7ceSxh/bbetWw6qsf8z8K3UPkuwPfEEP/HmyLgiC4
gl8xqwr4Paa4yL3nzbpccmQ2JQGIAlnyyCQDhKwDdwDsRoNS1OwuPuOGhD1Ddvsl
xCExOI9x38/rzTjWA9EYnIlgwQKBgQDQtEpsEzpWXQzT86KlBRMd34K0mA+iTDHA
2Hir+ciL6yk6HE/0z+3nVQ7yKZb8tBXXp9JkzNZLPDVxpPIPuwNgexytBse4ORKB
dmHKw29kDbJ5oh5GdXiE/v078P8woc2hbjd4F2mExcvFDd9m0db9brZM8XVY0EMG
xaGU9WlcfwKBgExEkBnbgYFp8SepQZFQ3bc6ew5cBP6HGBnnEF0/ZcUmNfxV7v6e
vewn7OvNvRevqhKNy2gwiK3sV/JsB8qxkmSQTtHku9dcKOjcZU/ymNVAFY4pzJYs
rjcxHwxDgOY4NcgtVddHG1/AMrbJFFr/lONnuwkSY/hZrHl5cp6cR5jBAoGBAMhR
xH0nl379oSpvV1V9IXQy7InaymbFK5wmKu0mu09RUCjus/APBBJemhHlyX6Ue8Ka
2l7WHXnpOIL0B0MCBaO9hzCsqVYxsYmBzyuHmos2enA1I0oNxrgg5395Ofe71lt0
Jtml3yoJkCR7xEo0b16hvWjs+e1dOHhviUAorhCRAoGAQE7xEsRRIAhON++mVHCx
mdvOR+/x2q7nPNq64BzuQuI6p6gRDtr0g9fYFLjRxtziNXBPxzuspaEC3F3CgPEP
K4KOPyoPwmt8gtNypaE0yXwn7qRpYyQQhHsJU/yJBfqxqLMayvPveqAzVKDLQxjJ
O8a0nSZL9fG9vDFviBtM/pY=
-----END PRIVATE KEY-----";

fn create_token(claims: &HashMap<&str, Value>) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-kid".to_string());
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(TEST_PRIVATE_KEY_PEM).unwrap();
    jsonwebtoken::encode(&header, claims, &encoding_key).unwrap()
}

fn external_validation_config(bind_addr: String) -> ServicePolicyConfig {
    ServicePolicyConfig {
        bind_addr,
        auth_provider: AuthProviderKind::External,
        external_provider: Some(ExternalAuthProviderConfig {
            issuer: "test-issuer".to_string(),
            audience: "flowable-timer".to_string(),
            mapping: ExternalClaimMappingConfig::default(),
            trusted_profiles: vec![],
        }),
        ..Default::default()
    }
}

#[test]
fn test_expired_token_rejected() {
    let db_path = "test_expired_token.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let config = external_validation_config(format!("127.0.0.1:{}", port));

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
    claims.insert("sub", Value::String("user-1".to_string()));
    claims.insert("role", Value::String("admin".to_string()));
    claims.insert("exp", Value::Number((now - 3600).into()));
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
        "Expired token should be rejected"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_missing_required_claim_rejected() {
    let db_path = "test_missing_claim.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let config = external_validation_config(format!("127.0.0.1:{}", port));

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
    claims.insert("sub", Value::String("user-1".to_string()));
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
        "Token missing role claim should be rejected"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_empty_role_rejected_when_strict() {
    let db_path = "test_empty_role.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let config = external_validation_config(format!("127.0.0.1:{}", port));

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
    claims.insert("sub", Value::String("user-1".to_string()));
    claims.insert("role", Value::String("".to_string()));
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
        "Token with empty role should be rejected when strict"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_unknown_issuer_rejected() {
    let db_path = "test_unknown_issuer.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let config = external_validation_config(format!("127.0.0.1:{}", port));

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
    claims.insert("iss", Value::String("unknown-issuer".to_string()));
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
        "Token with unknown issuer should be rejected"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn test_wrong_audience_rejected() {
    let db_path = "test_wrong_audience.db";
    let _ = std::fs::remove_file(db_path);

    let port = get_free_port();
    let config = external_validation_config(format!("127.0.0.1:{}", port));

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
    claims.insert("aud", Value::String("wrong-audience".to_string()));
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
        "Token with wrong audience should be rejected"
    );

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
    let _ = std::fs::remove_file(db_path);
}
