//! P122 — event-registry tenant fallback alignment.
//!
//! Java sources:
//! - Config: `AbstractEngineConfiguration.java:321-329`
//! - Event def: `GetEventModelCmd.java:82-90`
//! - Channel def: `GetChannelModelCmd.java:82-90`
//! - Consumer: `BaseEventRegistryEventConsumer.java:177-265`
//!
//! Covers: exact preferred, fallback to default (tenantless), switch off,
//! empty event-tenant semantics, BPMN + CMMN consumer paths.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnEventListener, CmmnHumanTask, CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
    CmmnPlanItemOnPart, CmmnSentry,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_event_registry_service::{
    EventInstanceRequest, EventInstanceStatus, EventRegistryConfiguration,
    EventRegistryDeploymentRequest, EventRegistryDeploymentResource, FlowableEventRegistryService,
    InboundRawEvent,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

fn service_with_fallback(name: &str, fallback: bool) -> FlowableEventRegistryService {
    let configuration = EventRegistryConfiguration::builder()
        .fallback_to_default_tenant(fallback)
        .build();
    FlowableEventRegistryService::with_configuration(
        Arc::new(ProcessEngine::new(name.to_string())),
        configuration,
    )
}

fn deploy_inbound(
    service: &FlowableEventRegistryService,
    name: &str,
    tenant_id: Option<&str>,
    event_key: &str,
    channel_key: &str,
    required_field: &str,
) {
    service
        .deploy(EventRegistryDeploymentRequest {
            name: name.to_string(),
            category: None,
            parent_deployment_id: None,
            tenant_id: tenant_id.map(str::to_string),
            resources: vec![
                EventRegistryDeploymentResource {
                    resource_name: format!("{name}.event"),
                    resource: json!({
                        "key": event_key,
                        "name": name,
                        "eventType": event_key,
                        "channelKey": channel_key,
                        "resourceName": format!("{name}.event"),
                        "payload": [
                            { "name": required_field, "type": "string", "required": true }
                        ]
                    })
                    .to_string(),
                },
                EventRegistryDeploymentResource {
                    resource_name: format!("{name}.channel"),
                    resource: json!({
                        "key": channel_key,
                        "name": format!("{name} channel"),
                        "channelType": "inbound",
                        "resourceName": format!("{name}.channel"),
                        "type": "in-memory",
                        "destination": name,
                        "deserializerType": "json",
                        "payloadExtractor": "json",
                        "filter": "default",
                        "tenantDetector": "default",
                        "transformer": "default",
                        "keyDetector": "default",
                        "consumer": "default",
                        "fixedEventKey": event_key
                    })
                    .to_string(),
                },
            ],
        })
        .expect("deploy inbound");
}

// ── Definition resolution ──────────────────────────────────────────────────

#[test]
fn event_definition_exact_tenant_preferred_over_fallback() {
    let service = service_with_fallback("p122-event-exact", true);
    deploy_inbound(
        &service,
        "global",
        None,
        "p122Order",
        "p122OrdersIn",
        "globalOnly",
    );
    deploy_inbound(
        &service,
        "tenant-a",
        Some("tenant-a"),
        "p122Order",
        "p122OrdersIn",
        "tenantOnly",
    );

    let delivery = service
        .receive_event_instance(EventInstanceRequest {
            event_definition_id: None,
            event_definition_key: Some("p122Order".to_string()),
            channel_definition_id: None,
            channel_definition_key: Some("p122OrdersIn".to_string()),
            event_payload: json!({ "tenantOnly": "T-1" }),
            tenant_id: Some("tenant-a".to_string()),
        })
        .unwrap();

    let definition = service
        .get_event_definition(&delivery.event_definition_id)
        .unwrap();
    assert_eq!(definition.tenant_id.as_deref(), Some("tenant-a"));
}

#[test]
fn event_definition_falls_back_to_default_tenantless_when_enabled() {
    // GetEventModelCmd.java:84-89 — fallback on, default empty → findLatestByKey.
    let service = service_with_fallback("p122-event-fallback-on", true);
    deploy_inbound(
        &service,
        "global",
        None,
        "p122Order",
        "p122OrdersIn",
        "globalOnly",
    );

    let delivery = service
        .receive_event_instance(EventInstanceRequest {
            event_definition_id: None,
            event_definition_key: Some("p122Order".to_string()),
            channel_definition_id: None,
            channel_definition_key: Some("p122OrdersIn".to_string()),
            event_payload: json!({ "globalOnly": "G-1" }),
            tenant_id: Some("tenant-b".to_string()),
        })
        .unwrap();

    let definition = service
        .get_event_definition(&delivery.event_definition_id)
        .unwrap();
    assert_eq!(definition.tenant_id, None);
}

#[test]
fn event_definition_no_fallback_when_switch_off() {
    // AbstractEngineConfiguration.java:324 default false; GetEventModelCmd.java:84 gate.
    let service = service_with_fallback("p122-event-fallback-off", false);
    deploy_inbound(
        &service,
        "global",
        None,
        "p122Order",
        "p122OrdersIn",
        "globalOnly",
    );

    let err = service
        .receive_event_instance(EventInstanceRequest {
            event_definition_id: None,
            event_definition_key: Some("p122Order".to_string()),
            channel_definition_id: None,
            channel_definition_key: Some("p122OrdersIn".to_string()),
            event_payload: json!({ "globalOnly": "G-1" }),
            tenant_id: Some("tenant-b".to_string()),
        })
        .unwrap_err();

    assert!(
        matches!(err, FlowableError::NotFound(_)),
        "expected NotFound without fallback, got {err:?}"
    );
}

#[test]
fn channel_definition_falls_back_when_enabled() {
    // GetChannelModelCmd.java:84-90
    let service = service_with_fallback("p122-channel-fallback", true);
    deploy_inbound(
        &service,
        "global",
        None,
        "p122Order",
        "p122OrdersIn",
        "globalOnly",
    );

    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "p122OrdersIn".to_string(),
            body: json!({ "globalOnly": "G-2" }),
            headers: BTreeMap::new(),
            tenant_hint: Some("tenant-x".to_string()),
        })
        .unwrap();
    assert_eq!(delivery.channel_key, "p122OrdersIn");
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
}

// ── Empty event-tenant semantics ───────────────────────────────────────────

#[test]
fn empty_event_tenant_does_not_filter_cmmn_subscriptions() {
    // BaseEventRegistryEventConsumer.java:177-178 — empty tenant skips filter.
    let process_engine = Arc::new(ProcessEngine::new("p122-empty-tenant-cmmn".to_string()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let config = EventRegistryConfiguration::builder()
        .fallback_to_default_tenant(false)
        .build();
    let service = FlowableEventRegistryService::with_cmmn_consumer_config(
        Arc::clone(&process_engine),
        Arc::clone(&cmmn),
        config,
    );

    deploy_inbound(
        &service,
        "p122-empty",
        None,
        "p122EmptyEvt",
        "p122EmptyIn",
        "orderId",
    );
    deploy_cmmn_listener(&cmmn, "p122EmptyCase", "p122EmptyEvt", Some("tenant-a"));

    // Resolve tenantless definition, stamp case instance tenant via override
    // (CaseInstanceHelperImpl.java:325-326 / P102).
    let case_id = cmmn
        .start_case_instance_by_key(
            "p122EmptyCase",
            CmmnCaseInstanceStartRequest::new()
                .with_override_definition_tenant_id("tenant-a"),
        )
        .expect("start")
        .id;

    // Delivery with no tenant must still match the tenant-a subscription.
    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "p122EmptyIn".to_string(),
            body: json!({ "orderId": "O-1" }),
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(active_task_count(&cmmn, &case_id), 2);
}

// ── CMMN consumer ──────────────────────────────────────────────────────────

#[test]
fn cmmn_consumer_exact_tenant_hit() {
    // Instance-level exact match (BaseEventRegistryEventConsumer.java:198-201).
    let process_engine = Arc::new(ProcessEngine::new("p122-cmmn-exact".to_string()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let config = EventRegistryConfiguration::builder()
        .fallback_to_default_tenant(true)
        .build();
    let service = FlowableEventRegistryService::with_cmmn_consumer_config(
        Arc::clone(&process_engine),
        Arc::clone(&cmmn),
        config,
    );

    // Event/channel deployed under tenant-a so resolution succeeds without relying
    // solely on fallback for the definition path.
    deploy_inbound(
        &service,
        "p122-cmmn-a",
        Some("tenant-a"),
        "p122CmmnEvt",
        "p122CmmnIn",
        "orderId",
    );
    deploy_cmmn_listener(&cmmn, "p122CmmnCase", "p122CmmnEvt", Some("tenant-a"));
    // Foreign tenant also has a waiting case — must not be triggered.
    deploy_cmmn_listener(&cmmn, "p122CmmnCaseB", "p122CmmnEvt", Some("tenant-b"));

    let case_a = cmmn
        .start_case_instance_by_key(
            "p122CmmnCase",
            CmmnCaseInstanceStartRequest::new()
                .with_override_definition_tenant_id("tenant-a"),
        )
        .expect("start a")
        .id;
    let case_b = cmmn
        .start_case_instance_by_key(
            "p122CmmnCaseB",
            CmmnCaseInstanceStartRequest::new()
                .with_override_definition_tenant_id("tenant-b"),
        )
        .expect("start b")
        .id;

    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "p122CmmnIn".to_string(),
            body: json!({ "orderId": "O-A" }),
            headers: BTreeMap::new(),
            tenant_hint: Some("tenant-a".to_string()),
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(active_task_count(&cmmn, &case_a), 2, "tenant-a case fires");
    assert_eq!(
        active_task_count(&cmmn, &case_b),
        1,
        "tenant-b case must not fire"
    );
}

// ── BPMN consumer ──────────────────────────────────────────────────────────

#[test]
fn bpmn_consumer_exact_tenant_hit() {
    let process_engine = Arc::new(ProcessEngine::new("p122-bpmn-exact".to_string()));
    let config = EventRegistryConfiguration::builder()
        .fallback_to_default_tenant(true)
        .build();
    let service = FlowableEventRegistryService::with_bpmn_consumer_config(
        Arc::clone(&process_engine),
        config,
    );

    deploy_inbound(
        &service,
        "p122-bpmn",
        Some("tenant-a"),
        "p122BpmnEvt",
        "p122BpmnIn",
        "orderId",
    );

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="p122Bridge" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="waitEvent" />
            <intermediateCatchEvent id="waitEvent">
                <extensionElements>
                    <flowable:eventType>p122BpmnEvt</flowable:eventType>
                </extensionElements>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="waitEvent" targetRef="afterTask" />
            <userTask id="afterTask" name="After" />
            <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let repository = process_engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("p122-bpmn".to_string())
                .tenant_id("tenant-a".to_string())
                .add_string("p122.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    // Deploy same process for tenant-b (foreign wait must not fire).
    repository
        .deploy(
            repository
                .create_deployment()
                .name("p122-bpmn-b".to_string())
                .tenant_id("tenant-b".to_string())
                .add_string("p122b.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let runtime = process_engine.get_runtime_service();
    let defs = repository.get_process_definition_ids().unwrap();
    // Start one instance per tenant (definitions are tenant-scoped).
    let mut pi_a = None;
    let mut pi_b = None;
    for def_id in defs {
        let def = repository
            .get_process_definition(&def_id)
            .expect("def");
        let pi = runtime
            .start_process_instance(
                runtime
                    .create_process_instance_builder()
                    .process_definition_id(def_id)
                    .tenant_id(def.tenant_id.clone().unwrap_or_default()),
            )
            .unwrap();
        if def.tenant_id.as_deref() == Some("tenant-a") {
            pi_a = Some(pi.id);
        } else if def.tenant_id.as_deref() == Some("tenant-b") {
            pi_b = Some(pi.id);
        }
    }
    let pi_a = pi_a.expect("tenant-a process");
    let pi_b = pi_b.expect("tenant-b process");

    let delivery = service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: "p122BpmnIn".to_string(),
            body: json!({ "orderId": "ORD-A" }),
            headers: BTreeMap::new(),
            tenant_hint: Some("tenant-a".to_string()),
        })
        .unwrap();
    assert_eq!(delivery.status, EventInstanceStatus::Processed);

    let tasks_a = process_engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_a)
        .unwrap();
    assert_eq!(tasks_a.len(), 1, "tenant-a wait must fire");
    assert_eq!(tasks_a[0].task_definition_key, "afterTask");

    let tasks_b = process_engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_b)
        .unwrap();
    assert!(
        tasks_b.is_empty(),
        "tenant-b wait must not fire on tenant-a event"
    );
}

// ── helpers ────────────────────────────────────────────────────────────────

fn deploy_cmmn_listener(
    cmmn: &CmmnEngine,
    case_key: &str,
    event_key: &str,
    _tenant_id: Option<&str>,
) {
    let model = CmmnModel::new(vec![CmmnCase::new(
        case_key,
        case_key,
        case_key,
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("task-a", "A"))
            .with_plan_item(CmmnPlanItem::new("plan-item-a", "task-a"))
            .with_event_listener(
                CmmnEventListener::new("event-listener", event_key).with_name("listener"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-event-listener",
                "event-listener",
            ))
            .with_human_task(CmmnHumanTask::new("task-b", "B"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-b", "task-b")
                    .with_entry_criterion("sentry-on-event"),
            )
            .with_sentry(CmmnSentry::new(
                "sentry-on-event",
                CmmnPlanItemOnPart::new(
                    "on-event-occur",
                    "plan-item-event-listener",
                    CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
                ),
            )),
    )]);

    cmmn.deploy(
        CmmnDeploymentRequest::new(case_key).with_resource(format!("{case_key}.cmmn"), model),
    )
    .expect("cmmn deploy");
}

fn active_task_count(cmmn: &CmmnEngine, case_id: &str) -> usize {
    cmmn.runtime_service()
        .create_human_task_query()
        .case_instance_id(case_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("task query")
        .len()
}
