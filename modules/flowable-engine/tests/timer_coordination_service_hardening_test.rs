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
fn test_timer_coordination_service_hardening() {
    let db_path = format!(
        "file:test_rpc_hardening_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "worker_hardening_node".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );

    let runtime_service = engine.get_runtime_service();

    let stop_signal = Arc::new(AtomicBool::new(false));
    let random_port = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let actual_addr = format!("127.0.0.1:{}", random_port);

    let mut config = ServicePolicyConfig {
        bind_addr: actual_addr.clone(),
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

    let service = TimerCoordinationService::new(Arc::clone(&runtime_service), config);

    let _handle = service.start(Arc::clone(&stop_signal));
    std::thread::sleep(Duration::from_millis(50));

    // Test 1: Malformed request
    let mut stream = TcpStream::connect(&actual_addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(b"GARBAGE\r\n\r\n").unwrap();

    let mut buf = vec![0; 4096];
    let n = stream.read(&mut buf).unwrap();
    let resp_str = String::from_utf8_lossy(&buf[..n]);
    assert!(resp_str.contains("400 Bad Request"));

    // Test 2: Unknown route
    let mut stream2 = TcpStream::connect(&actual_addr).unwrap();
    stream2
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream2
        .write_all(b"GET /unknown_route HTTP/1.1\r\nAuthorization: Bearer admin-secret\r\n\r\n")
        .unwrap();

    let mut buf2 = vec![0; 4096];
    let n2 = stream2.read(&mut buf2).unwrap();
    let resp_str2 = String::from_utf8_lossy(&buf2[..n2]);
    assert!(resp_str2.contains("404 Not Found"));
    assert!(resp_str2.contains("NOT_FOUND"));

    // Test 3: Invalid body for release
    let mut stream3 = TcpStream::connect(&actual_addr).unwrap();
    stream3
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream3.write_all(b"POST /release HTTP/1.1\r\nAuthorization: Bearer admin-secret\r\nContent-Length: 5\r\n\r\n{bad}").unwrap();

    let mut buf3 = vec![0; 4096];
    let n3 = stream3.read(&mut buf3).unwrap();
    let resp_str3 = String::from_utf8_lossy(&buf3[..n3]);
    assert!(resp_str3.contains("400 Bad Request"));
    assert!(resp_str3.contains("BAD_REQUEST"));

    // Stop server
    stop_signal.store(true, Ordering::SeqCst);
}
