use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::config::{AuthPolicy, ServicePolicyConfig};
use flowable_engine::service::issuer_profile::{
    ClaimMappingConfig, ClaimValidation, IssuerProfile, JwksRefreshPolicy, RolloutState,
};
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

#[test]
fn test_issuer_profile_optimistic_concurrency_conflict() {
    let db_path = format!(
        "file:test_issuer_profile_concurrency_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "worker_node".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );

    let runtime_service = engine.get_runtime_service();

    let mut config = ServicePolicyConfig::default();
    let random_port = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let actual_addr = format!("127.0.0.1:{}", random_port);
    config.bind_addr = actual_addr.clone();

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
    std::thread::sleep(Duration::from_millis(50));

    let admin_client =
        TimerCoordinationClient::new(actual_addr.clone()).with_auth("admin-secret".to_string());

    // 1. Create a profile (initial version should be 0)
    let profile_id = "concurrency-profile";
    let new_profile = IssuerProfile {
        id: profile_id.to_string(),
        issuer: "https://auth.example.com".to_string(),
        audience: "my-app".to_string(),
        mapping: ClaimMappingConfig::default(),
        validation: ClaimValidation::default(),
        role_mappings: vec![],
        required_tenant: false,
        rollout_state: RolloutState::Active,
        jwks_uri: None,
        allowed_algorithms: vec!["RS256".to_string()],
        jwks_cache_ttl_seconds: 3600,
        jwks_refresh_policy: JwksRefreshPolicy::default(),
        version: 0,
    };
    let created = admin_client.create_issuer_profile(&new_profile).unwrap();
    assert_eq!(created.version, 0);

    // 2. Client A and Client B both read the same version
    let profile_a = admin_client.get_issuer_profile(profile_id).unwrap();
    let profile_b = admin_client.get_issuer_profile(profile_id).unwrap();
    assert_eq!(profile_a.version, 0);
    assert_eq!(profile_b.version, 0);

    // 3. Client A updates successfully, version increments to 1
    let mut update_a = profile_a.clone();
    update_a.audience = "audience-a".to_string();
    let updated_a = admin_client.update_issuer_profile(&update_a).unwrap();
    assert_eq!(updated_a.version, 1);
    assert_eq!(updated_a.audience, "audience-a");

    // 4. Client B tries to update with version 0 — should fail with 409 Conflict
    let mut update_b = profile_b.clone();
    update_b.audience = "audience-b".to_string();
    let result_b = admin_client.update_issuer_profile(&update_b);

    assert!(
        result_b.is_err(),
        "Expected conflict error for stale version update"
    );
    let err_msg = format!("{:?}", result_b.err().unwrap());
    assert!(
        err_msg.contains("409") || err_msg.contains("CONFLICT"),
        "Error should indicate 409 Conflict: {}",
        err_msg
    );

    // 5. Client B fetches the latest version (1) and tries again
    let profile_b_latest = admin_client.get_issuer_profile(profile_id).unwrap();
    assert_eq!(profile_b_latest.version, 1);
    let mut update_b_retry = profile_b_latest.clone();
    update_b_retry.audience = "audience-b-success".to_string();
    let updated_b = admin_client.update_issuer_profile(&update_b_retry).unwrap();
    assert_eq!(updated_b.version, 2);
    assert_eq!(updated_b.audience, "audience-b-success");

    stop_signal.store(true, Ordering::SeqCst);
    let _ = handle.join();
}
