//! P136 2 (BPMN): tenant key-based dedup + multi-key dual start (fix under-delivery).
//!
//! Java: BaseEventRegistryEventConsumer.java:177-268.
//! Old Rust short-circuited after first exact-tenant start → under-delivery when a
//! different-key tenantless definition also matched.

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
            name: format!("p136-bpmn-{event_key}"),
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
                        "payload": [{ "name": "orderId", "type": "string" }],
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

fn process_xml(process_id: &str, event_key: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="{process_id}" isExecutable="true">
            <startEvent id="start">
                <extensionElements>
                    <flowable:eventType>{event_key}</flowable:eventType>
                </extensionElements>
            </startEvent>
            <sequenceFlow id="f1" sourceRef="start" targetRef="task" />
            <userTask id="task" name="After start" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

fn deploy_process(engine: &ProcessEngine, name: &str, xml: &str, tenant: Option<&str>) {
    let repository = engine.get_repository_service();
    let mut builder = repository
        .create_deployment()
        .name(name.to_string())
        .add_string(format!("{name}.bpmn20.xml"), xml.to_string());
    if let Some(t) = tenant {
        builder = builder.tenant_id(t.to_string());
    }
    repository.deploy(builder).expect("bpmn deploy");
}

fn service(engine: Arc<ProcessEngine>, fallback: bool) -> FlowableEventRegistryService {
    let config = EventRegistryConfiguration::builder()
        .fallback_to_default_tenant(fallback)
        .build();
    FlowableEventRegistryService::with_bpmn_consumer_config(engine, config)
}

fn deliver(
    service: &FlowableEventRegistryService,
    channel: &str,
    tenant: Option<&str>,
) {
    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: channel.to_string(),
            body: json!({ "orderId": "1" }),
            headers: BTreeMap::new(),
            tenant_hint: tenant.map(str::to_string),
        })
        .expect("deliver");
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
}

fn process_instances(engine: &ProcessEngine) -> Vec<flowable_engine::runtime::process_instance::ProcessInstance> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().expect("session");
    store
        .snapshot_process_instances(&mut session)
        .into_values()
        .collect()
}

fn process_count(engine: &ProcessEngine) -> usize {
    process_instances(engine).len()
}

/// Different keys under tenant T + tenantless: both must start (old Rust only started T).
#[test]
fn different_keys_tenant_and_tenantless_both_start() {
    let engine = Arc::new(ProcessEngine::new("p136-bpmn-dual".into()));
    let service = service(Arc::clone(&engine), true);
    deploy_event(&service, "dualEvt", "chDual");

    deploy_process(
        &engine,
        "procA",
        &process_xml("procA", "dualEvt"),
        Some("T1"),
    );
    deploy_process(
        &engine,
        "procB",
        &process_xml("procB", "dualEvt"),
        None,
    );

    deliver(&service, "chDual", Some("T1"));

    assert_eq!(
        process_count(&engine),
        2,
        "different keys must both start (Java multi-sub; was under-delivery)"
    );
}

/// Same key under tenant T + tenantless: only tenant T starts (dedup).
#[test]
fn same_key_tenant_and_tenantless_dedup_starts_once() {
    let engine = Arc::new(ProcessEngine::new("p136-bpmn-dedup".into()));
    let service = service(Arc::clone(&engine), true);
    deploy_event(&service, "sameKeyEvt", "chSame");

    deploy_process(
        &engine,
        "sharedGlobal",
        &process_xml("sharedProc", "sameKeyEvt"),
        None,
    );
    deploy_process(
        &engine,
        "sharedTenant",
        &process_xml("sharedProc", "sameKeyEvt"),
        Some("T1"),
    );

    deliver(&service, "chSame", Some("T1"));

    let instances = process_instances(&engine);
    assert_eq!(
        instances.len(),
        1,
        "same key must dedup: drop tenantless when tenant-exact exists"
    );
    assert_eq!(instances[0].tenant_id.as_deref(), Some("T1"));
}

/// When defaultTenant is a real tenant (not NO_TENANT_ID), no key dedup — both tenants start.
/// Channel/event definitions must exist under both tenants (GetChannelModelCmd falls back
/// to the configured default tenant, not tenantless).
#[test]
fn real_default_tenant_does_not_dedup() {
    let engine = Arc::new(ProcessEngine::new("p136-bpmn-real-default".into()));
    let config = EventRegistryConfiguration::builder()
        .fallback_to_default_tenant(true)
        .default_tenant("defaultTenant")
        .build();
    let service = FlowableEventRegistryService::with_bpmn_consumer_config(Arc::clone(&engine), config);

    // Deploy event+channel under the default tenant (fallback target for T1 lookups).
    service
        .deploy(EventRegistryDeploymentRequest {
            name: "p136-bpmn-realDefEvt".to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: Some("defaultTenant".to_string()),
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: "realDefEvt.event".to_string(),
                    resource: json!({
                        "key": "realDefEvt",
                        "name": "realDefEvt",
                        "eventType": "realDefEvt",
                        "channelKey": "chReal",
                        "resourceName": "realDefEvt.event",
                        "payload": [{ "name": "orderId", "type": "string" }],
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: "chReal.channel".to_string(),
                    resource: json!({
                        "key": "chReal",
                        "name": "chReal",
                        "channelType": "inbound",
                        "resourceName": "chReal.channel",
                        "type": "in-memory",
                        "destination": "chReal",
                        "payloadExtractor": "json",
                        "filter": "default",
                        "tenantDetector": "default",
                        "transformer": "default",
                        "keyDetector": "default",
                        "consumer": "default",
                        "fixedEventKey": "realDefEvt",
                    })
                    .to_string(),
                },
            ],
        })
        .expect("event deploy defaultTenant");

    deploy_process(
        &engine,
        "realA",
        &process_xml("realA", "realDefEvt"),
        Some("T1"),
    );
    deploy_process(
        &engine,
        "realB",
        &process_xml("realB", "realDefEvt"),
        Some("defaultTenant"),
    );

    deliver(&service, "chReal", Some("T1"));

    assert_eq!(
        process_count(&engine),
        2,
        "real defaultTenant path queries both tenants without key dedup"
    );
}
