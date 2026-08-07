use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::service::audit::TimerAdminAuditInput;
use flowable_engine::service::config::{AuthPolicy, ServicePolicyConfig};
use flowable_engine::service::timer_coordination_client::TimerCoordinationClient;
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

#[test]
fn test_timer_coordination_audit() {
    let db_path = format!(
        "file:test_rpc_audit_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let db_store = Arc::new(DbStore::new_file(&db_path).unwrap());

    let engine = ProcessEngine::build(
        "worker_audit_node".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        Arc::clone(&db_store),
    );

    let runtime_service = engine.get_runtime_service();

    let random_port = 20000 + (uuid::Uuid::new_v4().as_u128() % 10000) as u16;
    let actual_addr = format!("127.0.0.1:{}", random_port);
    let mut config = ServicePolicyConfig {
        bind_addr: actual_addr.clone(),
        ..Default::default()
    };

    config.auth_keys.insert(
        "admin-secret".to_string(),
        AuthPolicy {
            actor_id: "admin-actor-1".to_string(),
            subject: Some("subject-1".to_string()),
            issuer: Some("issuer-1".to_string()),
            role: "admin".to_string(),
            tenant_id: None,
        },
    );

    let stop_signal = Arc::new(AtomicBool::new(false));
    let service = TimerCoordinationService::new(Arc::clone(&runtime_service), config);
    let _handle = service.start(Arc::clone(&stop_signal));
    std::thread::sleep(Duration::from_millis(50));

    let admin_client =
        TimerCoordinationClient::new(actual_addr.clone()).with_auth("admin-secret".to_string());

    // Trigger some admin actions
    admin_client.release_leadership(10).unwrap();
    let _ = admin_client.admin_step_down().unwrap();
    runtime_service
        .audit_admin_action(TimerAdminAuditInput {
            request_id: "req-123".to_string(),
            tenant_id: Some("tenant-a".to_string()),
            issuer: "issuer-1".to_string(),
            subject: "subject-1".to_string(),
            actor: "admin-actor-1".to_string(),
            action: "cleanup".to_string(),
            target: "cluster-nodes".to_string(),
            outcome: "success: 0".to_string(),
            profile_id: None,
        })
        .unwrap();

    // Check database for queryable audit columns and contract snapshot.
    let mut session = db_store.create_session().unwrap();
    let rows = session.raw_query(
        "SELECT request_id, tenant_id, issuer, subject, actor, action, target, outcome, data FROM timer_admin_audit_logs ORDER BY timestamp",
        flowable_engine::persistence::DbParams::new(),
    ).unwrap();

    let records: Vec<_> = rows
        .into_iter()
        .filter_map(|row| {
            let request_id = row.get_text("request_id");
            let tenant_id = row.get_text("tenant_id");
            let issuer = row.get_text("issuer")?;
            let subject = row.get_text("subject")?;
            let actor = row.get_text("actor")?;
            let action = row.get_text("action")?;
            let target = row.get_text("target")?;
            let outcome = row.get_text("outcome")?;
            let json = row.get_text("data")?;
            let record: flowable_engine::service::audit::TimerAdminAuditRecord =
                serde_json::from_str(&json).unwrap();
            Some((
                request_id, tenant_id, issuer, subject, actor, action, target, outcome, record,
            ))
        })
        .collect();
    assert_eq!(records.len(), 3);

    assert!(records.iter().any(
        |(_, tenant_id, issuer, subject, actor, action, _, _, record)| {
            action == "release"
                && actor == "admin-actor-1"
                && tenant_id.is_none()
                && issuer == "issuer-1"
                && subject == "subject-1"
                && record.action == "release"
        }
    ));
    assert!(
        records
            .iter()
            .any(|(_, _, _, _, actor, action, _, _, record)| {
                action == "step-down" && actor == "admin-actor-1" && record.action == "step-down"
            })
    );
    assert!(records.iter().any(
        |(request_id, _, _, _, actor, action, target, outcome, record)| {
            request_id.as_deref() == Some("req-123")
                && actor == "admin-actor-1"
                && action == "cleanup"
                && target == "cluster-nodes"
                && outcome.starts_with("success")
                && record.request_id == "req-123"
        }
    ));

    let mut count_params = flowable_engine::persistence::DbParams::new();
    count_params.push("req-123");
    count_params.push("admin-actor-1");
    let matching_request_count: i64 = session
        .raw_query_one(
            "SELECT COUNT(*) AS RES_ FROM timer_admin_audit_logs WHERE request_id = ?1 AND actor = ?2",
            count_params,
        )
        .unwrap()
        .and_then(|r| r.get_integer("RES_"))
        .unwrap_or(0);
    assert_eq!(matching_request_count, 1);

    let _ = session.rollback();

    stop_signal.store(true, Ordering::SeqCst);
}
