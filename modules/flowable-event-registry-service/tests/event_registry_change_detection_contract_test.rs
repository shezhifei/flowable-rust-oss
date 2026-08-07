use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_event_registry_service::{
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, FlowableEventRegistryService,
};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

fn shared_services(label: &str) -> (FlowableEventRegistryService, FlowableEventRegistryService, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "event-registry-change-{}-{}.sqlite",
        label,
        Uuid::new_v4()
    ));
    let engine_a = Arc::new(ProcessEngine::new_with_db_path(
        format!("{label}-a"),
        path.to_str().unwrap(),
    ));
    let engine_b = Arc::new(ProcessEngine::new_with_db_path(
        format!("{label}-b"),
        path.to_str().unwrap(),
    ));
    (
        FlowableEventRegistryService::new(engine_a),
        FlowableEventRegistryService::new(engine_b),
        path,
    )
}

fn deploy_channel_event(
    service: &FlowableEventRegistryService,
    name: &str,
    channel_key: &str,
    event_key: &str,
    tenant_id: Option<&str>,
) -> String {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: name.to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: tenant_id.map(str::to_string),
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{event_key}.event"),
                    resource: json!({
                        "key": event_key,
                        "name": event_key,
                        "eventType": format!("{event_key}.type"),
                        "channelKey": channel_key,
                        "resourceName": format!("{event_key}.event"),
                        "payload": []
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{channel_key}.channel"),
                    resource: json!({
                        "key": channel_key,
                        "name": channel_key,
                        "channelType": "inbound",
                        "resourceName": format!("{channel_key}.channel"),
                        "type": "in-memory"
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap()
        .id
}

#[test]
fn two_instances_reconcile_new_version_via_change_log() {
    let (service_a, service_b, path) = shared_services("new-version");

    deploy_channel_event(&service_a, "v1", "orders", "orderEvent", None);
    assert_eq!(
        service_a
            .cached_latest_channel("orders", None)
            .unwrap()
            .version,
        1
    );
    assert!(service_b.cached_latest_channel("orders", None).is_none());

    let result = service_b.detect_and_reconcile_changes().unwrap();
    assert!(result.applied >= 2);
    assert_eq!(
        service_b
            .cached_latest_channel("orders", None)
            .unwrap()
            .version,
        1
    );

    deploy_channel_event(&service_a, "v2", "orders", "orderEvent", None);
    service_b.detect_and_reconcile_changes().unwrap();
    assert_eq!(
        service_b
            .cached_latest_channel("orders", None)
            .unwrap()
            .version,
        2
    );
    assert_eq!(
        service_b
            .cached_latest_event("orderEvent", None)
            .unwrap()
            .version,
        2
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn rollback_after_latest_deletion_repoints_previous_version() {
    let (service_a, service_b, path) = shared_services("rollback");

    let d1 = deploy_channel_event(&service_a, "v1", "orders", "orderEvent", None);
    let d2 = deploy_channel_event(&service_a, "v2", "orders", "orderEvent", None);
    service_b.detect_and_reconcile_changes().unwrap();
    assert_eq!(
        service_b
            .cached_latest_channel("orders", None)
            .unwrap()
            .version,
        2
    );

    service_a.delete_deployment(&d2).unwrap();
    service_b.detect_and_reconcile_changes().unwrap();
    assert_eq!(
        service_b
            .cached_latest_channel("orders", None)
            .unwrap()
            .version,
        1
    );
    assert_eq!(
        service_b
            .cached_latest_event("orderEvent", None)
            .unwrap()
            .version,
        1
    );

    // d1 still present
    let _ = service_a.get_deployment(&d1).unwrap();
    let _ = std::fs::remove_file(path);
}

#[test]
fn complete_key_deletion_unregisters_from_cache() {
    let (service_a, service_b, path) = shared_services("delete-all");

    let d1 = deploy_channel_event(&service_a, "v1", "orders", "orderEvent", None);
    service_b.detect_and_reconcile_changes().unwrap();
    assert!(service_b.cached_latest_channel("orders", None).is_some());

    service_a.delete_deployment(&d1).unwrap();
    service_b.detect_and_reconcile_changes().unwrap();
    assert!(service_b.cached_latest_channel("orders", None).is_none());
    assert!(service_b.cached_latest_event("orderEvent", None).is_none());

    let _ = std::fs::remove_file(path);
}

#[test]
fn tenant_isolation_is_preserved_in_cache_and_change_detection() {
    let (service_a, service_b, path) = shared_services("tenant");

    deploy_channel_event(&service_a, "global", "orders", "orderEvent", None);
    deploy_channel_event(
        &service_a,
        "tenant",
        "orders",
        "orderEvent",
        Some("tenant-a"),
    );
    service_b.detect_and_reconcile_changes().unwrap();

    assert_eq!(
        service_b.cached_latest_channel("orders", None).unwrap().tenant_id,
        None
    );
    assert_eq!(
        service_b
            .cached_latest_channel("orders", Some("tenant-a"))
            .unwrap()
            .tenant_id
            .as_deref(),
        Some("tenant-a")
    );
    assert!(
        service_b
            .cached_latest_channel("orders", Some("tenant-b"))
            .is_none()
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn repeated_polling_is_idempotent_and_bounded() {
    let (service_a, service_b, path) = shared_services("idempotent");

    deploy_channel_event(&service_a, "v1", "orders", "orderEvent", None);
    let first = service_b.detect_and_reconcile_changes_with_limit(10).unwrap();
    assert!(first.applied > 0);
    let revision = service_b.last_change_revision();

    let second = service_b.detect_and_reconcile_changes_with_limit(10).unwrap();
    assert_eq!(second.applied, 0);
    assert_eq!(second.last_revision, revision);
    assert!(second.exhausted);
    assert_eq!(service_b.last_change_revision(), revision);

    let _ = std::fs::remove_file(path);
}

#[test]
fn failed_deployment_does_not_publish_change_records() {
    let (service_a, service_b, path) = shared_services("failed-deploy");

    let before = service_a.last_change_revision();
    let error = service_a
        .deploy(EventRegistryDeploymentRequest {
            name: "bad".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "bad.channel".to_string(),
                resource: json!({
                    "key": "bad",
                    "name": "bad",
                    "channelType": "inbound",
                    "resourceName": "bad.channel",
                    "type": "not-a-real-adapter"
                })
                .to_string(),
            }],
        })
        .unwrap_err();
    assert!(error.to_string().contains("not-a-real-adapter") || error.to_string().contains("Unknown"));
    assert_eq!(service_a.last_change_revision(), before);

    let result = service_b.detect_and_reconcile_changes().unwrap();
    assert_eq!(result.applied, 0);
    assert!(service_b.cached_latest_channel("bad", None).is_none());

    let _ = std::fs::remove_file(path);
}
