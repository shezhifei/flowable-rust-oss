use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{AuthPolicy, ServicePolicyConfig};
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

#[test]
fn test_timer_coordination_service_lifecycle() {
    let db_path = format!(
        "file:test_rpc_lifecycle_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine1 = ProcessEngine::build(
        "worker_node_1".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );
    let engine2 = ProcessEngine::build(
        "worker_node_2".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );

    // Heartbeat both nodes
    engine1
        .get_runtime_service()
        .heartbeat_timer_node("standalone")
        .unwrap();
    engine2
        .get_runtime_service()
        .heartbeat_timer_node("standalone")
        .unwrap();

    let runtime_service = engine1.get_runtime_service();

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
            actor_id: "admin-cli".to_string(),
            subject: None,
            issuer: None,
            role: "admin".to_string(),
            tenant_id: None,
        },
    );

    let service = TimerCoordinationService::new(Arc::clone(&runtime_service), config);

    let _handle = service.start(Arc::clone(&stop_signal));
    std::thread::sleep(Duration::from_millis(50));

    let client =
        TimerCoordinationClient::new(actual_addr.clone()).with_auth("admin-secret".to_string());

    // Test GET /nodes
    let nodes_res = client.get_nodes().unwrap();
    assert_eq!(nodes_res.len(), 2);

    // Test POST /deregister
    let actual_node_id_1 = nodes_res
        .iter()
        .find(|n| n.node_id.starts_with("worker_node_1"))
        .unwrap()
        .node_id
        .clone();
    let dereg_success = client.deregister_node(&actual_node_id_1).unwrap();
    assert!(dereg_success);

    // Test GET /nodes again
    let nodes_res2 = client.get_nodes().unwrap();
    assert_eq!(nodes_res2.len(), 1);
    assert!(nodes_res2[0].node_id.starts_with("worker_node_2"));

    // Test POST /cleanup
    let cleaned_count = client.cleanup_expired_nodes().unwrap();
    assert_eq!(cleaned_count, 0); // node 2 is still fresh

    // Stop server
    stop_signal.store(true, Ordering::SeqCst);
}
