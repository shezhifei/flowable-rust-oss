use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{AuthPolicy, ServicePolicyConfig};
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

#[test]
fn test_timer_coordination_auth_boundary() {
    let db_path = format!(
        "file:test_rpc_auth_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "worker_auth_node".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );

    let runtime_service = engine.get_runtime_service();

    let mut config = ServicePolicyConfig::default();
    let random_port = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let actual_addr = format!("127.0.0.1:{}", random_port);
    config.bind_addr = actual_addr.clone();

    config.auth_keys.insert(
        "read-secret".to_string(),
        AuthPolicy {
            actor_id: "reader".to_string(),
            subject: None,
            issuer: None,
            role: "read".to_string(),
            tenant_id: None,
        },
    );
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
    config.auth_keys.insert(
        "tenant-admin-secret".to_string(),
        AuthPolicy {
            actor_id: "tenant-admin".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: Some("tenant-a".to_string()),
        },
    );

    let stop_signal = Arc::new(AtomicBool::new(false));
    let service = TimerCoordinationService::new(Arc::clone(&runtime_service), config);

    let _handle = service.start(Arc::clone(&stop_signal));
    std::thread::sleep(Duration::from_millis(50));

    // 1. Unauthenticated client
    let unauth_client = TimerCoordinationClient::new(actual_addr.clone());
    let status_res = unauth_client.get_status();
    assert!(status_res.is_err());
    assert!(status_res.unwrap_err().contains("UNAUTHORIZED"));

    let cleanup_res = unauth_client.cleanup_expired_nodes();
    assert!(cleanup_res.is_err());
    assert!(cleanup_res.unwrap_err().contains("UNAUTHORIZED"));

    // 2. Read-only client
    let read_client =
        TimerCoordinationClient::new(actual_addr.clone()).with_auth("read-secret".to_string());

    let status_res2 = read_client.get_status();
    assert!(status_res2.is_ok());

    let cleanup_res2 = read_client.cleanup_expired_nodes();
    assert!(cleanup_res2.is_err());
    assert!(cleanup_res2.unwrap_err().contains("UNAUTHORIZED"));

    // 3. Admin client
    let admin_client =
        TimerCoordinationClient::new(actual_addr.clone()).with_auth("admin-secret".to_string());

    let status_res3 = admin_client.get_status();
    assert!(status_res3.is_ok());

    let cleanup_res3 = admin_client.cleanup_expired_nodes();
    assert!(cleanup_res3.is_ok());
    assert_eq!(cleanup_res3.unwrap(), 0);

    // 4. Tenant-scoped admin can operate on tenant-aware timer-node resources,
    // but not on global cluster cleanup or mismatched tenant requests.
    runtime_service.heartbeat_timer_node("embedded").unwrap();

    let mut matching_tenant_stream = TcpStream::connect(&actual_addr).unwrap();
    matching_tenant_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    matching_tenant_stream.write_all(
        b"GET /nodes HTTP/1.1\r\nAuthorization: Bearer tenant-admin-secret\r\nX-Tenant-Id: tenant-a\r\n\r\n"
    ).unwrap();
    let mut matching_buf = vec![0; 4096];
    let matching_n = matching_tenant_stream.read(&mut matching_buf).unwrap();
    let matching_resp = String::from_utf8_lossy(&matching_buf[..matching_n]);
    assert!(matching_resp.contains("200 OK"));

    let mut mismatched_tenant_stream = TcpStream::connect(&actual_addr).unwrap();
    mismatched_tenant_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    mismatched_tenant_stream.write_all(
        b"GET /nodes HTTP/1.1\r\nAuthorization: Bearer tenant-admin-secret\r\nX-Tenant-Id: tenant-b\r\n\r\n"
    ).unwrap();
    let mut mismatched_buf = vec![0; 4096];
    let mismatched_n = mismatched_tenant_stream.read(&mut mismatched_buf).unwrap();
    let mismatched_resp = String::from_utf8_lossy(&mismatched_buf[..mismatched_n]);
    assert!(mismatched_resp.contains("401 Unauthorized"));

    let mut cluster_cleanup_stream = TcpStream::connect(&actual_addr).unwrap();
    cluster_cleanup_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    cluster_cleanup_stream.write_all(
        b"POST /cleanup HTTP/1.1\r\nAuthorization: Bearer tenant-admin-secret\r\nX-Tenant-Id: tenant-a\r\n\r\n"
    ).unwrap();
    let mut cluster_buf = vec![0; 4096];
    let cluster_n = cluster_cleanup_stream.read(&mut cluster_buf).unwrap();
    let cluster_resp = String::from_utf8_lossy(&cluster_buf[..cluster_n]);
    assert!(cluster_resp.contains("401 Unauthorized"));

    // Stop server
    stop_signal.store(true, Ordering::SeqCst);
}
