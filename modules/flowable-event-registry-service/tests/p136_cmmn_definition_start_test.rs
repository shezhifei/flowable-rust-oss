//! P136 1c + 2 (CMMN): definition-level start path + tenant key dedup.
//!
//! Java: CmmnEventRegistryEventConsumer.java:138-278,
//! BaseEventRegistryEventConsumer.java:177-268.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine, CmmnModel,
    REFERENCE_TYPE_EVENT_CASE, START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_event_registry_service::{
    EventInstanceStatus, EventRegistryConfiguration, EventRegistryDeploymentRequest,
    EventRegistryDeploymentResource, FlowableEventRegistryService, InboundRawEvent,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

fn deploy_event(service: &FlowableEventRegistryService, event_key: &str, channel_key: &str) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("p136-{event_key}"),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{event_key}.event"),
                    resource: json!({
                        "key": event_key,
                        "name": event_key,
                        "eventType": event_key,
                        "channelKey": channel_key,
                        "resourceName": format!("{event_key}.event"),
                        "payload": [
                            { "name": "orderId", "type": "string" },
                            { "name": "customerId", "type": "string" }
                        ],
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
                        "type": "in-memory",
                        "destination": channel_key,
                        "payloadExtractor": "json",
                        "filter": "default",
                        "tenantDetector": "default",
                        "transformer": "default",
                        "keyDetector": "default",
                        "consumer": "default",
                        "fixedEventKey": event_key,
                    })
                    .to_string(),
                },
            ],
        })
        .expect("event deploy");
}

fn start_case_model(key: &str, event_type: &str, unique: bool) -> CmmnModel {
    let mut case = CmmnCase::new(
        key,
        key,
        format!("{key} case"),
        CmmnCasePlanModel::new("pm", "Plan"),
    );
    case.start_event_type = Some(event_type.to_string());
    if unique {
        // Java testCaseStartOnlyOneInstance.cmmn: storeAsUniqueReferenceId with no
        // eventCorrelationParameter → broadcast subscription (configuration null);
        // uniqueness uses full inbound correlation key as referenceId
        // (CmmnEventRegistryEventConsumer.java:147).
        case.start_correlation_configuration =
            Some(START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID.to_string());
    }
    CmmnModel::new(vec![case])
}

fn service_with_cmmn(
    process_engine: Arc<ProcessEngine>,
    cmmn: Arc<CmmnEngine>,
    fallback: bool,
) -> FlowableEventRegistryService {
    let config = EventRegistryConfiguration::builder()
        .fallback_to_default_tenant(fallback)
        .build();
    FlowableEventRegistryService::with_cmmn_consumer_config(process_engine, cmmn, config)
}

fn deliver(
    service: &FlowableEventRegistryService,
    channel: &str,
    body: serde_json::Value,
    tenant: Option<&str>,
) {
    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: channel.to_string(),
            body,
            headers: BTreeMap::new(),
            tenant_hint: tenant.map(str::to_string),
        })
        .expect("deliver");
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
}

#[test]
fn broadcast_definition_start_creates_case_instance() {
    let process_engine = Arc::new(ProcessEngine::new("p136-cmmn-broadcast".into()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let service = service_with_cmmn(Arc::clone(&process_engine), Arc::clone(&cmmn), false);
    deploy_event(&service, "broadcastStart", "chBroadcast");
    cmmn.deploy(
        CmmnDeploymentRequest::new("bc").with_resource(
            "case.cmmn",
            start_case_model("broadcastCase", "broadcastStart", false),
        ),
    )
    .expect("deploy case");

    assert!(cmmn
        .runtime_service()
        .create_case_instance_query()
        .list()
        .unwrap()
        .is_empty());

    deliver(&service, "chBroadcast", json!({ "orderId": "x" }), None);

    let instances = cmmn
        .runtime_service()
        .create_case_instance_query()
        .list()
        .unwrap();
    assert_eq!(instances.len(), 1, "broadcast start must create one case");
    assert_eq!(instances[0].case_definition_key, "broadcastCase");
    assert!(instances[0].reference_id.is_none());
}

#[test]
fn store_as_unique_reference_id_first_starts_second_skips() {
    let process_engine = Arc::new(ProcessEngine::new("p136-cmmn-unique".into()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let service = service_with_cmmn(Arc::clone(&process_engine), Arc::clone(&cmmn), false);
    deploy_event(&service, "uniqueStart", "chUnique");
    cmmn.deploy(
        CmmnDeploymentRequest::new("uq").with_resource(
            "case.cmmn",
            start_case_model("uniqueCase", "uniqueStart", true),
        ),
    )
    .expect("deploy");

    let body = json!({ "orderId": "ORD-42", "customerId": "c1" });
    deliver(&service, "chUnique", body.clone(), None);
    deliver(&service, "chUnique", body, None);

    let instances = cmmn
        .runtime_service()
        .create_case_instance_query()
        .list()
        .unwrap();
    assert_eq!(
        instances.len(),
        1,
        "storeAsUniqueReferenceId must not start a second instance for same correlation"
    );
    assert!(instances[0].reference_id.is_some());
    assert_eq!(
        instances[0].reference_type.as_deref(),
        Some(REFERENCE_TYPE_EVENT_CASE)
    );
}

#[test]
fn tenant_override_on_started_case() {
    let process_engine = Arc::new(ProcessEngine::new("p136-cmmn-tenant".into()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let service = service_with_cmmn(Arc::clone(&process_engine), Arc::clone(&cmmn), true);
    deploy_event(&service, "tenantStart", "chTenant");
    cmmn.deploy(
        CmmnDeploymentRequest::new("tn").with_resource(
            "case.cmmn",
            start_case_model("tenantCase", "tenantStart", false),
        ),
    )
    .expect("deploy");

    deliver(
        &service,
        "chTenant",
        json!({ "orderId": "t1" }),
        Some("tenantA"),
    );

    let instances = cmmn
        .runtime_service()
        .create_case_instance_query()
        .list()
        .unwrap();
    assert_eq!(instances.len(), 1);
    assert_eq!(
        instances[0].tenant_id.as_deref(),
        Some("tenantA"),
        "event tenant must override case instance tenant"
    );
}

#[test]
fn same_key_tenant_and_tenantless_dedup_starts_once() {
    let process_engine = Arc::new(ProcessEngine::new("p136-cmmn-dedup".into()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let service = service_with_cmmn(Arc::clone(&process_engine), Arc::clone(&cmmn), true);
    deploy_event(&service, "dedupStart", "chDedup");

    cmmn.deploy(
        CmmnDeploymentRequest::new("global").with_resource(
            "case.cmmn",
            start_case_model("sharedKey", "dedupStart", false),
        ),
    )
    .expect("deploy global");
    cmmn.deploy(
        CmmnDeploymentRequest::new("tenant")
            .with_tenant_id("T1")
            .with_resource(
                "case.cmmn",
                start_case_model("sharedKey", "dedupStart", false),
            ),
    )
    .expect("deploy tenant");

    deliver(
        &service,
        "chDedup",
        json!({ "orderId": "d1" }),
        Some("T1"),
    );

    let instances = cmmn
        .runtime_service()
        .create_case_instance_query()
        .list()
        .unwrap();
    assert_eq!(
        instances.len(),
        1,
        "same-key tenant + tenantless must dedup to one start"
    );
    assert_eq!(instances[0].tenant_id.as_deref(), Some("T1"));
}
