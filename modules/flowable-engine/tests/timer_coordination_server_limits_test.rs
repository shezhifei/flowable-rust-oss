use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{AuthPolicy, ServicePolicyConfig};
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

#[test]
fn test_timer_coordination_server_limits() {
    let db_path = format!(
        "file:test_rpc_limits_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "worker_limits_node".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );

    let runtime_service = engine.get_runtime_service();

    let mut config = ServicePolicyConfig::default();
    let random_port = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let actual_addr = format!("127.0.0.1:{}", random_port);
    config.bind_addr = actual_addr.clone();
    config.max_request_size = 50; // VERY SMALL for testing
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
    let _handle = service.start(Arc::clone(&stop_signal));
    std::thread::sleep(Duration::from_millis(50));

    // Send a payload that exceeds max_request_size
    let mut stream = TcpStream::connect(&actual_addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let big_body = "x".repeat(100);
    stream.write_all(format!("POST /release HTTP/1.1\r\nAuthorization: Bearer admin-secret\r\nContent-Length: {}\r\n\r\n{}", big_body.len(), big_body).as_bytes()).unwrap();

    let mut buf = vec![0; 4096];
    let n = stream.read(&mut buf).unwrap();
    let resp_str = String::from_utf8_lossy(&buf[..n]);

    // Check if it was rejected with 413 Payload Too Large
    assert!(resp_str.contains("413 Payload Too Large"));
    assert!(resp_str.contains("PAYLOAD_TOO_LARGE"));

    stop_signal.store(true, Ordering::SeqCst);
}
