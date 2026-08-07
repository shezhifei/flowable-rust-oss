use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::RuntimeStore;
use flowable_engine::service::config::{AuthPolicy, ServicePolicyConfig};
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

fn build_local_auth_service() -> (
    Arc<flowable_engine::engine::runtime_service::RuntimeService>,
    String,
    Arc<AtomicBool>,
    std::thread::JoinHandle<()>,
    RuntimeStore,
) {
    let db_path = format!(
        "file:test_revocation_admin_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());
    let engine = ProcessEngine::build(
        "revocation-admin-node".to_string(),
        Arc::new(SystemTimeSource),
        db_store,
    );
    let runtime_service = engine.get_runtime_service();
    let runtime_store = engine.get_runtime_store();

    let mut config = ServicePolicyConfig::default();
    let port = 20000 + (Uuid::new_v4().as_u128() % 10000) as u16;
    let bind_addr = format!("127.0.0.1:{}", port);
    config.bind_addr = bind_addr.clone();
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
    let handle = service.start(Arc::clone(&stop_signal));
    std::thread::sleep(Duration::from_millis(100));

    (
        runtime_service,
        bind_addr,
        stop_signal,
        handle,
        runtime_store,
    )
}

#[test]
fn test_revocation_admin_endpoints_share_persisted_source_of_truth() {
    let (_runtime_service, bind_addr, stop_signal, handle, _store) = build_local_auth_service();
    let admin_client =
        TimerCoordinationClient::new(bind_addr).with_auth("admin-secret".to_string());

    let initial_stats = admin_client.get_revocation_stats().unwrap();
    assert_eq!(initial_stats.active_count, 0);

    assert!(
        admin_client
            .revoke_token_with_ttl("admin-jti", "issuer-1", "manual revoke", 3600)
            .unwrap()
    );

    let status = admin_client.get_revocation_status("admin-jti").unwrap();
    assert!(status.is_revoked);
    assert_eq!(status.issuer.as_deref(), Some("issuer-1"));
    assert_eq!(status.reason.as_deref(), Some("manual revoke"));

    let active_stats = admin_client.get_revocation_stats().unwrap();
    assert_eq!(active_stats.active_count, 1);

    assert!(admin_client.unrevoke_token("admin-jti").unwrap());

    let cleared_status = admin_client.get_revocation_status("admin-jti").unwrap();
    assert!(!cleared_status.is_revoked);
    assert!(cleared_status.issuer.is_none());
    assert!(cleared_status.reason.is_none());

    let cleared_stats = admin_client.get_revocation_stats().unwrap();
    assert_eq!(cleared_stats.active_count, 0);

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
}

#[test]
fn test_revocation_status_read_does_not_delete_expired_entries() {
    let (_runtime_service, bind_addr, stop_signal, handle, store) = build_local_auth_service();
    let admin_client =
        TimerCoordinationClient::new(bind_addr).with_auth("admin-secret".to_string());

    assert!(
        admin_client
            .revoke_token_with_ttl("expiring-jti", "issuer-2", "short ttl", 0)
            .unwrap()
    );
    std::thread::sleep(Duration::from_millis(20));

    let mut session = store.create_session().unwrap();
    assert!(
        store
            .find_token_revocation("expiring-jti", &mut session)
            .is_some(),
        "expired entry should exist until explicit cleanup"
    );
    session.rollback().unwrap();

    let status = admin_client.get_revocation_status("expiring-jti").unwrap();
    assert!(!status.is_revoked);

    let mut session = store.create_session().unwrap();
    assert!(
        store
            .find_token_revocation("expiring-jti", &mut session)
            .is_some(),
        "status reads must not lazily delete persisted revocation rows"
    );
    session.rollback().unwrap();
    assert_eq!(admin_client.get_revocation_stats().unwrap().active_count, 0);

    let mut session = store.create_session().unwrap();
    assert_eq!(store.cleanup_expired_token_revocations(&mut session), 1);
    session.flush_and_commit().unwrap();

    let mut session = store.create_session().unwrap();
    assert!(
        store
            .find_token_revocation("expiring-jti", &mut session)
            .is_none()
    );
    session.rollback().unwrap();

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
}
