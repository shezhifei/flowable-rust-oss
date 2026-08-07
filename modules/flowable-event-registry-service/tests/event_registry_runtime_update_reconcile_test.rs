//! Report #8 contract: runtime definition resolution goes through the
//! cache-backed resolver (bounded reconcile → cache lookup → store rehydrate),
//! and `update_*` publishes a change record that other instances sharing the
//! store pick up automatically on their next runtime request — without any
//! explicit `detect_and_reconcile_changes()` call.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_event_registry_service::{
    ChannelDefinitionUpdateRequest, EventDefinitionUpdateRequest, EventInstanceStatus,
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, FlowableEventRegistryService,
    InboundEventRequest, OutboundEventRequest,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

fn shared_services(
    label: &str,
) -> (
    FlowableEventRegistryService,
    FlowableEventRegistryService,
    PathBuf,
) {
    let path = std::env::temp_dir().join(format!(
        "event-registry-runtime-update-{}-{}.sqlite",
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
    channel_key: &str,
    channel_type: &str,
    event_key: &str,
    payload: Value,
) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("deploy-{event_key}"),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{event_key}.event"),
                    resource: json!({
                        "key": event_key,
                        "name": event_key,
                        "eventType": format!("{event_key}.type"),
                        "channelKey": channel_key,
                        "resourceName": format!("{event_key}.event"),
                        "payload": payload
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{channel_key}.channel"),
                    resource: json!({
                        "key": channel_key,
                        "name": channel_key,
                        "channelType": channel_type,
                        "resourceName": format!("{channel_key}.channel"),
                        "type": "in-memory",
                        "destination": format!("dest-{channel_key}")
                    })
                    .to_string(),
                },
            ],
        })
        .unwrap();
}

#[test]
fn inbound_runtime_picks_up_cross_instance_event_definition_update() {
    let (service_a, service_b, path) = shared_services("inbound");
    deploy_channel_event(&service_a, "ordersIn", "inbound", "orderReceived", json!([]));

    // B never reconciles explicitly; the runtime request itself must.
    let delivery = service_b
        .receive_inbound_event(InboundEventRequest {
            event_type: "orderReceived.type".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);

    // Runtime resolution populated B's engine-local cache.
    assert!(service_b.cached_latest_event("orderReceived", None).is_some());
    assert!(service_b.cached_latest_channel("ordersIn", None).is_some());

    // A tightens the payload contract via update.
    let definition_id = service_a
        .cached_latest_event("orderReceived", None)
        .unwrap()
        .id;
    service_a
        .update_event_definition(
            &definition_id,
            EventDefinitionUpdateRequest {
                name: None,
                payload: Some(json!([
                    { "name": "amount", "type": "integer", "required": true }
                ])),
            },
        )
        .unwrap();

    // B rejects the now-invalid payload without any explicit reconcile call.
    let error = service_b
        .receive_inbound_event(InboundEventRequest {
            event_type: "orderReceived.type".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("Required field 'amount'"),
        "unexpected error: {error}"
    );
    assert_eq!(
        service_b
            .cached_latest_event("orderReceived", None)
            .unwrap()
            .payload,
        json!([{ "name": "amount", "type": "integer", "required": true }])
    );

    // A payload matching the updated contract flows end to end on B.
    let delivery = service_b
        .receive_inbound_event(InboundEventRequest {
            event_type: "orderReceived.type".to_string(),
            event_payload: json!({ "amount": 5 }),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);

    let _ = std::fs::remove_file(path);
}

#[test]
fn outbound_runtime_picks_up_cross_instance_event_definition_update() {
    let (service_a, service_b, path) = shared_services("outbound");
    deploy_channel_event(
        &service_a,
        "ordersOut",
        "outbound",
        "orderPublished",
        json!([{ "name": "orderId", "type": "string", "required": true }]),
    );

    let delivery = service_b
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "A-1" }),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Published);
    assert!(service_b.cached_latest_event("orderPublished", None).is_some());

    let definition_id = service_a
        .cached_latest_event("orderPublished", None)
        .unwrap()
        .id;
    service_a
        .update_event_definition(
            &definition_id,
            EventDefinitionUpdateRequest {
                name: None,
                payload: Some(json!([
                    { "name": "orderId", "type": "string", "required": true },
                    { "name": "amount", "type": "integer", "required": true }
                ])),
            },
        )
        .unwrap();

    // B validates against the updated contract without an explicit reconcile.
    let error = service_b
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "A-2" }),
            tenant_id: None,
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("Required field 'amount'"),
        "unexpected error: {error}"
    );

    let delivery = service_b
        .publish_outbound_event(OutboundEventRequest {
            event_definition_key: "orderPublished".to_string(),
            event_payload: json!({ "orderId": "A-3", "amount": 3 }),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Published);

    let _ = std::fs::remove_file(path);
}

/// Watermark regression: A deploys an unrelated definition after B updated a
/// definition A has cached but not yet reconciled. The local deploy must not
/// advance A's watermark past B's change, so A's next runtime resolve still
/// picks up B's update.
#[test]
fn local_deploy_does_not_skip_unreconciled_foreign_update() {
    let (service_a, service_b, path) = shared_services("deploy-watermark");
    deploy_channel_event(&service_a, "ordersIn", "inbound", "orderReceived", json!([]));

    // A caches orders v1 through a runtime resolve.
    let delivery = service_a
        .receive_inbound_event(InboundEventRequest {
            event_type: "orderReceived.type".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);

    // B tightens the contract; A does not reconcile yet.
    service_b.detect_and_reconcile_changes().unwrap();
    let definition_id = service_b
        .cached_latest_event("orderReceived", None)
        .unwrap()
        .id;
    service_b
        .update_event_definition(
            &definition_id,
            EventDefinitionUpdateRequest {
                name: None,
                payload: Some(json!([
                    { "name": "amount", "type": "integer", "required": true }
                ])),
            },
        )
        .unwrap();

    // A deploys an unrelated definition before reconciling B's update.
    deploy_channel_event(&service_a, "shippingIn", "inbound", "shipmentReceived", json!([]));

    // A's next runtime resolve must still apply B's update: the empty payload
    // is now invalid.
    let error = service_a
        .receive_inbound_event(InboundEventRequest {
            event_type: "orderReceived.type".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("Required field 'amount'"),
        "A skipped B's update after a local deploy: {error}"
    );

    let _ = std::fs::remove_file(path);
}

/// Watermark regression for delete: A deletes an unrelated deployment after B
/// updated a definition A has cached but not yet reconciled. The delete must
/// not set A's watermark to the delete revision, which would permanently skip
/// B's earlier-revision update.
#[test]
fn local_delete_does_not_skip_unreconciled_foreign_update() {
    let (service_a, service_b, path) = shared_services("delete-watermark");
    deploy_channel_event(&service_a, "ordersIn", "inbound", "orderReceived", json!([]));
    let unrelated = service_a
        .deploy(EventRegistryDeploymentRequest {
            name: "unrelated".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![EventRegistryDeploymentResource {
                resource_name: "tempChannel.channel".to_string(),
                resource: json!({
                    "key": "tempChannel",
                    "name": "tempChannel",
                    "channelType": "inbound",
                    "resourceName": "tempChannel.channel",
                    "type": "in-memory",
                    "destination": "dest-temp"
                })
                .to_string(),
            }],
        })
        .unwrap();

    // A is fully caught up before the interleaving starts.
    service_a.detect_and_reconcile_changes().unwrap();
    let delivery = service_a
        .receive_inbound_event(InboundEventRequest {
            event_type: "orderReceived.type".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);

    // B tightens the contract; A does not reconcile yet.
    service_b.detect_and_reconcile_changes().unwrap();
    let definition_id = service_b
        .cached_latest_event("orderReceived", None)
        .unwrap()
        .id;
    service_b
        .update_event_definition(
            &definition_id,
            EventDefinitionUpdateRequest {
                name: None,
                payload: Some(json!([
                    { "name": "amount", "type": "integer", "required": true }
                ])),
            },
        )
        .unwrap();

    // A deletes the unrelated deployment; its delete revisions are higher than
    // B's update revision.
    service_a.delete_deployment(&unrelated.id).unwrap();

    // A's next runtime resolve must still apply B's earlier update.
    let error = service_a
        .receive_inbound_event(InboundEventRequest {
            event_type: "orderReceived.type".to_string(),
            event_payload: json!({}),
            tenant_id: None,
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("Required field 'amount'"),
        "A skipped B's update after a local delete: {error}"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn channel_update_publishes_change_record_and_refreshes_caches() {
    let (service_a, service_b, path) = shared_services("channel-update");
    deploy_channel_event(&service_a, "orders", "inbound", "orderEvent", json!([]));
    service_b.detect_and_reconcile_changes().unwrap();
    let baseline = service_b.last_change_revision();

    let channel_id = service_a.cached_latest_channel("orders", None).unwrap().id;
    service_a
        .update_channel_definition(
            &channel_id,
            ChannelDefinitionUpdateRequest {
                name: Some("Orders renamed".to_string()),
                configuration: None,
            },
        )
        .unwrap();

    // A's local cache body is replaced right after commit.
    assert_eq!(
        service_a.cached_latest_channel("orders", None).unwrap().name,
        "Orders renamed"
    );

    // The update is durable in the change log; B reconciles it.
    let result = service_b.detect_and_reconcile_changes().unwrap();
    assert!(result.applied >= 1);
    assert!(service_b.last_change_revision() > baseline);
    assert_eq!(
        service_b.cached_latest_channel("orders", None).unwrap().name,
        "Orders renamed"
    );

    let _ = std::fs::remove_file(path);
}
