//! P2 storage fault-injection for the Event Registry change log: revision
//! allocator failures (UPDATE and seed INSERT) and change-record INSERT
//! failures must surface as `Err` from the public deploy/delete APIs instead
//! of panicking, and the definitions written in the same transaction must not
//! be committed.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{
    EventRegistryDeployment, EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
    FlowableEventRegistryService,
};
use serde_json::json;
use std::sync::Arc;

fn fixture(name: &str) -> (Arc<ProcessEngine>, FlowableEventRegistryService) {
    let engine = Arc::new(ProcessEngine::new(name.to_string()));
    let service = FlowableEventRegistryService::new(Arc::clone(&engine));
    (engine, service)
}

fn execute_raw(engine: &ProcessEngine, sql: &str) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    session.execute_raw_sql(sql).unwrap();
    session.flush_and_commit().unwrap();
}

fn try_deploy(
    service: &FlowableEventRegistryService,
) -> Result<EventRegistryDeployment, FlowableError> {
    service.deploy(EventRegistryDeploymentRequest {
        name: "fault-deploy".to_string(),
        category: None,
        parent_deployment_id: None,
        tenant_id: None,
        resources: vec![
            EventRegistryDeploymentResource {
                resource_name: "order-published.event".to_string(),
                resource: json!({
                    "key": "orderPublished",
                    "name": "Order published",
                    "eventType": "order.published",
                    "channelKey": "ordersOutbound",
                    "resourceName": "order-published.event",
                    "payload": []
                })
                .to_string(),
            },
            EventRegistryDeploymentResource {
                resource_name: "orders-outbound.channel".to_string(),
                resource: json!({
                    "key": "ordersOutbound",
                    "name": "Orders outbound",
                    "channelType": "outbound",
                    "resourceName": "orders-outbound.channel",
                    "type": "in-memory",
                    "destination": "orders-outbound",
                    "serializerType": "json"
                })
                .to_string(),
            },
        ],
    })
}

fn assert_nothing_committed(service: &FlowableEventRegistryService) {
    assert!(
        service
            .create_channel_definition_query()
            .list()
            .unwrap()
            .is_empty(),
        "channel definitions must not be committed when the change log write fails"
    );
    assert!(
        service
            .create_event_definition_query()
            .list()
            .unwrap()
            .is_empty(),
        "event definitions must not be committed when the change log write fails"
    );
}

/// The allocator UPDATE fails (table missing): deploy must return `Err`
/// without panicking, and no definition may be committed.
#[test]
fn allocator_update_failure_fails_deploy_without_partial_commit() {
    let (engine, service) = fixture("fault-change-allocator-update");
    execute_raw(&engine, "DROP TABLE event_registry_change_revision_seq");

    let error = try_deploy(&service).expect_err("deploy must fail, not panic");
    assert!(
        error.to_string().to_lowercase().contains("storage")
            || error
                .to_string()
                .contains("event_registry_change_revision_seq"),
        "error must surface the storage failure, got: {error}"
    );
    assert_nothing_committed(&service);
}

/// The allocator row is missing and the seed INSERT fails: deploy must return
/// `Err` instead of panicking on the seed path.
#[test]
fn allocator_seed_insert_failure_fails_deploy() {
    let (engine, service) = fixture("fault-change-allocator-seed");
    execute_raw(&engine, "DELETE FROM event_registry_change_revision_seq");
    execute_raw(
        &engine,
        "CREATE TRIGGER block_allocator_seed \
         BEFORE INSERT ON event_registry_change_revision_seq \
         BEGIN SELECT RAISE(ABORT, 'allocator seed blocked'); END",
    );

    let error = try_deploy(&service).expect_err("deploy must fail, not panic");
    assert!(
        error.to_string().contains("allocator seed blocked"),
        "error must surface the seed INSERT failure, got: {error}"
    );
    assert_nothing_committed(&service);
}

/// The change record INSERT fails (table missing): deploy must return `Err`
/// and roll back the definitions written in the same transaction.
#[test]
fn change_record_insert_failure_rolls_back_deployment() {
    let (engine, service) = fixture("fault-change-record-insert");
    execute_raw(&engine, "DROP TABLE event_registry_change_records");

    let error = try_deploy(&service).expect_err("deploy must fail, not panic");
    assert!(
        error.to_string().to_lowercase().contains("storage")
            || error.to_string().contains("event_registry_change_records"),
        "error must surface the storage failure, got: {error}"
    );
    assert_nothing_committed(&service);
}

/// A change record INSERT failure during delete must return `Err` and keep
/// the deployment (and its definitions) intact.
#[test]
fn change_record_insert_failure_on_delete_keeps_deployment() {
    let (engine, service) = fixture("fault-change-record-delete");
    let deployment = try_deploy(&service).unwrap();
    execute_raw(&engine, "DROP TABLE event_registry_change_records");

    let error = service
        .delete_deployment(&deployment.id)
        .expect_err("delete must fail, not panic");
    assert!(
        error.to_string().to_lowercase().contains("storage")
            || error.to_string().contains("event_registry_change_records"),
        "error must surface the storage failure, got: {error}"
    );
    // The failed delete must not have removed the deployment or definitions.
    assert!(service.get_deployment(&deployment.id).is_ok());
    assert_eq!(
        service.create_channel_definition_query().list().unwrap().len(),
        1
    );
}
