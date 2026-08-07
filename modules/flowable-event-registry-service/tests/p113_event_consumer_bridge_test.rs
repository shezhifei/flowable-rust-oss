//! P113 e2e: event-registry inbound → CMMN event subscription bridge.
//!
//! Java references (verified against flowable-engine sources):
//! - `CmmnEventRegistryEventConsumer.java:80-106` — eventReceived finds CMMN
//!   subscriptions and handles each independently
//! - `BaseEventRegistryEventConsumer.java:156-175` — match eventType +
//!   withoutConfiguration OR configurations(power-set keys)
//! - `EventRegistryEventListenerActivityBehaviour.java:139-153` — subscription
//!   stores event definition key as eventType and correlation as configuration
//! - `EventInstanceCmmnUtil.java:46-68` — payload → variables via eventOutParameter
//! - No match → silent discard (empty processing info, not an error)

use flowable_cmmn_engine::{
    generate_correlation_key, CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnEngine, CmmnEventCorrelationParameter, CmmnEventListener,
    CmmnEventOutParameter, CmmnHumanTask, CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
    CmmnPlanItemOnPart, CmmnSentry,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_event_registry_service::{
    EventInstanceStatus, EventRegistryDeploymentRequest, EventRegistryDeploymentResource,
    FlowableEventRegistryService, InboundRawEvent, CMMN_EVENT_CONSUMER_KEY,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

fn deploy_event_and_channel(
    service: &FlowableEventRegistryService,
    event_key: &str,
    channel_key: &str,
    payload_fields: &[(&str, &str)],
) {
    let payload: Vec<_> = payload_fields
        .iter()
        .map(|(name, ty)| json!({ "name": name, "type": ty }))
        .collect();
    service
        .deploy(EventRegistryDeploymentRequest {
            name: format!("p113-{event_key}"),
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
                        "payload": payload,
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
        .expect("event registry deploy");
}

fn deliver(
    service: &FlowableEventRegistryService,
    channel_key: &str,
    body: serde_json::Value,
) -> flowable_event_registry_service::EventInstanceDelivery {
    service
        .process_inbound_channel_event(InboundRawEvent {
            channel_key: channel_key.to_string(),
            body,
            headers: BTreeMap::new(),
            tenant_hint: None,
        })
        .expect("inbound delivery")
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

fn subscription_count(cmmn: &CmmnEngine, case_id: &str) -> usize {
    cmmn.runtime_service()
        .create_event_subscription_query()
        .case_instance_id(case_id)
        .list()
        .expect("subscription query")
        .len()
}

/// Case with event-registry listener (eventType = event definition key) and a
/// human task activated via occur sentry — mirrors
/// `CmmnEventRegistryConsumerTest.testGenericEventListenerNoCorrelation.cmmn`.
fn no_correlation_case_model(case_key: &str, event_key: &str) -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        "p113-no-corr",
        case_key,
        "P113 no-correlation listener case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("task-a", "A"))
            .with_plan_item(CmmnPlanItem::new("plan-item-a", "task-a"))
            .with_event_listener(CmmnEventListener::new("event-listener", event_key).with_name(
                "myEventListener",
            ))
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
    )])
}

/// Case with correlation on customerIdVar
/// (`CmmnEventRegistryConsumerTest.testGenericEventListenerWithCorrelation.cmmn`).
fn correlation_case_model(case_key: &str, event_key: &str) -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        "p113-corr",
        case_key,
        "P113 correlation listener case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("task-a", "A"))
            .with_plan_item(CmmnPlanItem::new("plan-item-a", "task-a"))
            .with_event_listener(
                CmmnEventListener::new("event-listener", event_key)
                    .with_name("myEventListener")
                    .with_event_correlation_parameter(CmmnEventCorrelationParameter::new(
                        "customerId",
                        "${customerIdVar}",
                    )),
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
    )])
}

/// Case with eventOutParameter mapping payload → variables
/// (`CmmnEventRegistryConsumerTest.testGenericEventListenerWithPayload.cmmn.xml`).
fn payload_case_model(case_key: &str, event_key: &str) -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        "p113-payload",
        case_key,
        "P113 payload mapping case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_event_listener(
                CmmnEventListener::new("event-listener", event_key)
                    .with_event_out_parameter(CmmnEventOutParameter::new(
                        "customerId",
                        "customerIdVar",
                    ))
                    .with_event_out_parameter(CmmnEventOutParameter::new(
                        "payload1",
                        "payload1Var",
                    )),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-event-listener",
                "event-listener",
            ))
            .with_human_task(CmmnHumanTask::new("task-after", "After"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-after", "task-after")
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
    )])
}

#[test]
fn inbound_event_hits_subscription_and_triggers_sentry_task() {
    // CmmnEventRegistryEventConsumer.java:80-106 + testGenericEventListenerNoCorrelation
    let process_engine = Arc::new(ProcessEngine::new("p113-e2e-hit".to_string()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("ProcessEngine default wires a CmmnEngine");
    let service =
        FlowableEventRegistryService::with_cmmn_consumer(Arc::clone(&process_engine), Arc::clone(&cmmn));

    assert!(
        service
            .configuration()
            .consumer(CMMN_EVENT_CONSUMER_KEY)
            .is_some(),
        "cmmnEventConsumer must be registered"
    );

    deploy_event_and_channel(
        &service,
        "myEvent",
        "p113Inbound",
        &[("customerId", "string"), ("payload1", "string")],
    );

    cmmn.deploy(
        CmmnDeploymentRequest::new("p113-no-corr")
            .with_resource("case.cmmn", no_correlation_case_model("myCase", "myEvent")),
    )
    .expect("cmmn deploy");

    let case_id = cmmn
        .start_case_instance_by_key("myCase", CmmnCaseInstanceStartRequest::new())
        .expect("start case")
        .id;

    assert_eq!(subscription_count(&cmmn, &case_id), 1);
    assert_eq!(active_task_count(&cmmn, &case_id), 1, "only task A active");

    let delivery = deliver(
        &service,
        "p113Inbound",
        json!({ "customerId": "test", "payload1": "Hello World" }),
    );
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(delivery.event_definition_key, "myEvent");

    assert_eq!(
        active_task_count(&cmmn, &case_id),
        2,
        "task B activated after event listener occur"
    );
    assert_eq!(
        subscription_count(&cmmn, &case_id),
        0,
        "subscription deleted on occur (idempotent base)"
    );
}

#[test]
fn correlation_match_triggers_only_matching_case() {
    // CmmnEventRegistryConsumerTest.testGenericEventListenerWithCorrelation
    let process_engine = Arc::new(ProcessEngine::new("p113-e2e-corr".to_string()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let service =
        FlowableEventRegistryService::with_cmmn_consumer(Arc::clone(&process_engine), Arc::clone(&cmmn));

    deploy_event_and_channel(
        &service,
        "myEvent",
        "p113CorrInbound",
        &[("customerId", "string")],
    );

    cmmn.deploy(
        CmmnDeploymentRequest::new("p113-corr").with_resource(
            "case.cmmn",
            correlation_case_model("singleCorrelationCase", "myEvent"),
        ),
    )
    .expect("deploy");

    let kermit = cmmn
        .start_case_instance_by_key(
            "singleCorrelationCase",
            CmmnCaseInstanceStartRequest::new()
                .with_variables(json!({ "customerIdVar": "kermit" })),
        )
        .expect("kermit")
        .id;
    let gonzo = cmmn
        .start_case_instance_by_key(
            "singleCorrelationCase",
            CmmnCaseInstanceStartRequest::new()
                .with_variables(json!({ "customerIdVar": "gonzo" })),
        )
        .expect("gonzo")
        .id;

    // Verify subscription configuration is the evaluated correlation key.
    let mut expected_params = BTreeMap::new();
    expected_params.insert("customerId".to_string(), Some("kermit".to_string()));
    let expected_key = generate_correlation_key(&expected_params);
    let kermit_subs = cmmn
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&kermit)
        .list()
        .expect("subs");
    assert_eq!(kermit_subs.len(), 1);
    assert_eq!(
        kermit_subs[0].configuration.as_deref(),
        Some(expected_key.as_str())
    );

    assert_eq!(active_task_count(&cmmn, &kermit), 1);
    assert_eq!(active_task_count(&cmmn, &gonzo), 1);

    // Match kermit only.
    let delivery = deliver(
        &service,
        "p113CorrInbound",
        json!({ "customerId": "kermit" }),
    );
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
    assert_eq!(active_task_count(&cmmn, &kermit), 2);
    assert_eq!(active_task_count(&cmmn, &gonzo), 1);

    // Match gonzo.
    deliver(
        &service,
        "p113CorrInbound",
        json!({ "customerId": "gonzo" }),
    );
    assert_eq!(active_task_count(&cmmn, &kermit), 2);
    assert_eq!(active_task_count(&cmmn, &gonzo), 2);

    // Mismatch (fozzie) — neither advances.
    deliver(
        &service,
        "p113CorrInbound",
        json!({ "customerId": "fozzie" }),
    );
    assert_eq!(active_task_count(&cmmn, &kermit), 2);
    assert_eq!(active_task_count(&cmmn, &gonzo), 2);
}

#[test]
fn no_subscription_is_silently_discarded() {
    // BaseEventRegistryEventConsumer / CmmnEventRegistryEventConsumer: empty
    // subscription list → empty processing info, not an error.
    let process_engine = Arc::new(ProcessEngine::new("p113-e2e-none".to_string()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let service =
        FlowableEventRegistryService::with_cmmn_consumer(Arc::clone(&process_engine), Arc::clone(&cmmn));

    deploy_event_and_channel(
        &service,
        "orphanEvent",
        "p113OrphanInbound",
        &[("x", "string")],
    );

    let delivery = deliver(
        &service,
        "p113OrphanInbound",
        json!({ "x": "no-one-listening" }),
    );
    assert_eq!(delivery.status, EventInstanceStatus::Processed);
}

#[test]
fn payload_maps_to_case_variables_via_out_parameters() {
    // EventInstanceCmmnUtil.java:46-68 + testGenericEventListenerWithPayload
    let process_engine = Arc::new(ProcessEngine::new("p113-e2e-payload".to_string()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let service =
        FlowableEventRegistryService::with_cmmn_consumer(Arc::clone(&process_engine), Arc::clone(&cmmn));

    deploy_event_and_channel(
        &service,
        "payloadEvent",
        "p113PayloadInbound",
        &[("customerId", "string"), ("payload1", "string")],
    );

    cmmn.deploy(
        CmmnDeploymentRequest::new("p113-payload").with_resource(
            "case.cmmn",
            payload_case_model("payloadCase", "payloadEvent"),
        ),
    )
    .expect("deploy");

    let case_id = cmmn
        .start_case_instance_by_key("payloadCase", CmmnCaseInstanceStartRequest::new())
        .expect("start")
        .id;

    let before = cmmn
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case");
    assert!(
        before.variables.is_empty()
            || (!before.variables.contains_key("customerIdVar")
                && !before.variables.contains_key("payload1Var"))
    );

    let delivery = deliver(
        &service,
        "p113PayloadInbound",
        json!({ "customerId": "payloadCustomer", "payload1": "Hello World" }),
    );
    assert_eq!(delivery.status, EventInstanceStatus::Processed);

    let after = cmmn
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case after");
    assert_eq!(
        after.variables.get("customerIdVar"),
        Some(&json!("payloadCustomer"))
    );
    assert_eq!(
        after.variables.get("payload1Var"),
        Some(&json!("Hello World"))
    );
    assert_eq!(
        active_task_count(&cmmn, &case_id),
        1,
        "after-task activated by occur sentry"
    );
}

#[test]
fn multi_subscription_hit_triggers_all_broadcast_cases() {
    // Single inbound event with no correlation hits every withoutConfiguration
    // subscription for that eventType (BaseEventRegistryEventConsumer:172-174).
    let process_engine = Arc::new(ProcessEngine::new("p113-e2e-multi".to_string()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let service =
        FlowableEventRegistryService::with_cmmn_consumer(Arc::clone(&process_engine), Arc::clone(&cmmn));

    deploy_event_and_channel(
        &service,
        "broadcastEvent",
        "p113BroadcastInbound",
        &[("note", "string")],
    );

    cmmn.deploy(
        CmmnDeploymentRequest::new("p113-multi")
            .with_resource("case.cmmn", no_correlation_case_model("broadcastCase", "broadcastEvent")),
    )
    .expect("deploy");

    let case_a = cmmn
        .start_case_instance_by_key("broadcastCase", CmmnCaseInstanceStartRequest::new())
        .expect("a")
        .id;
    let case_b = cmmn
        .start_case_instance_by_key("broadcastCase", CmmnCaseInstanceStartRequest::new())
        .expect("b")
        .id;

    deliver(
        &service,
        "p113BroadcastInbound",
        json!({ "note": "hello-all" }),
    );

    assert_eq!(active_task_count(&cmmn, &case_a), 2);
    assert_eq!(active_task_count(&cmmn, &case_b), 2);
}

#[test]
fn second_delivery_is_idempotent_after_subscription_consumed() {
    // After occur the subscription is deleted; a second event finds no match
    // and is discarded without error (same silent path as no-subscription).
    let process_engine = Arc::new(ProcessEngine::new("p113-e2e-idem".to_string()));
    let cmmn = process_engine
        .get_config()
        .cmmn_engine
        .clone()
        .expect("cmmn");
    let service =
        FlowableEventRegistryService::with_cmmn_consumer(Arc::clone(&process_engine), Arc::clone(&cmmn));

    deploy_event_and_channel(
        &service,
        "onceEvent",
        "p113OnceInbound",
        &[("id", "string")],
    );

    cmmn.deploy(
        CmmnDeploymentRequest::new("p113-idem")
            .with_resource("case.cmmn", no_correlation_case_model("onceCase", "onceEvent")),
    )
    .expect("deploy");

    let case_id = cmmn
        .start_case_instance_by_key("onceCase", CmmnCaseInstanceStartRequest::new())
        .expect("start")
        .id;

    deliver(&service, "p113OnceInbound", json!({ "id": "1" }));
    assert_eq!(active_task_count(&cmmn, &case_id), 2);
    assert_eq!(subscription_count(&cmmn, &case_id), 0);

    let second = deliver(&service, "p113OnceInbound", json!({ "id": "2" }));
    assert_eq!(second.status, EventInstanceStatus::Processed);
    // No second task-B (no repetition rule on listener in this model).
    assert_eq!(active_task_count(&cmmn, &case_id), 2);
}

#[test]
fn default_service_still_uses_noop_not_cmmn_consumer() {
    let engine = Arc::new(ProcessEngine::new("p113-noop".to_string()));
    let service = FlowableEventRegistryService::new(Arc::clone(&engine));
    assert!(
        service.configuration().consumer("default").is_some(),
        "default consumer remains registered"
    );
    assert!(
        service.configuration().consumer(CMMN_EVENT_CONSUMER_KEY).is_none(),
        "cmmnEventConsumer is opt-in via with_cmmn_consumer"
    );
}
