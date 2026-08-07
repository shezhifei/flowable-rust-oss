//! P93: event-registry correlation model — subscription configuration storage,
//! match semantics (`configuration IS NULL OR IN keys`), and
//! `storeAsUniqueReferenceId` referenceId count dedup.
//!
//! Java: CorrelationUtil.java:30-67, DefaultCorrelationKeyGenerator.java:38-57,
//! BaseEventRegistryEventConsumer.java:156-174,
//! BpmnEventRegistryEventConsumer.java:125-225.
//!
//! P92 consumer bridge is on another branch; matching is exercised via the
//! shared helper that P92 will call after merge. P98: event correlation keys
//! are the power set minus empty (BaseEventRegistryEventConsumer.java:76-131);
//! subscription configuration stays the full-parameter single key
//! (CorrelationUtil.java:30-67).

use flowable_engine::bpmn::event_registry_correlation::{
    count_process_instances_for_unique_reference, generate_correlation_key,
    generate_event_correlation_keys, is_store_as_unique_reference_id,
    matches_subscription_configuration, should_skip_unique_start, REFERENCE_TYPE_EVENT_PROCESS,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::process_instance_builder::ProcessInstanceBuilder;
use serde_json::json;
use std::collections::BTreeMap;

fn deploy_correlated_start(engine: &ProcessEngine, deployment_name: &str) {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="corrStartProcess" isExecutable="true">
    <startEvent id="theStart">
      <extensionElements>
        <flowable:eventType>myEvent</flowable:eventType>
        <flowable:eventCorrelationParameter name="customerId" value="testCustomer"/>
        <flowable:startEventCorrelationConfiguration>storeAsUniqueReferenceId</flowable:startEventCorrelationConfiguration>
      </extensionElements>
    </startEvent>
    <sequenceFlow sourceRef="theStart" targetRef="task"/>
    <userTask id="task" name="After Start"/>
    <sequenceFlow sourceRef="task" targetRef="theEnd"/>
    <endEvent id="theEnd"/>
  </process>
</definitions>"#;

    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name(deployment_name.to_string())
                .add_string(
                    "corr_start.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();
}

fn expected_customer_key() -> String {
    let mut params = BTreeMap::new();
    params.insert("customerId".to_string(), Some("testCustomer".to_string()));
    generate_correlation_key(&params)
}

/// Correlation hit: start subscription stores configuration and matches the
/// event's full-parameter key (would trigger start in the consumer).
#[test]
fn correlation_hit_matches_start_subscription() {
    let engine = ProcessEngine::new("p93-corr-hit".to_string());
    deploy_correlated_start(&engine, "corr-hit");

    let subs = engine.get_event_start_subscriptions();
    let sub = subs
        .iter()
        .find(|s| s.event_ref == "myEvent")
        .expect("event-registry start subscription for myEvent");

    let expected = expected_customer_key();
    assert_eq!(
        sub.configuration.as_deref(),
        Some(expected.as_str()),
        "subscription configuration must be CorrelationUtil static key"
    );

    // Event arrives with the same single parameter; a power set of size 1 is
    // just the full key (Java n=1 special case, :83-87) — contract unchanged.
    let mut event_params = BTreeMap::new();
    event_params.insert("customerId".to_string(), Some("testCustomer".to_string()));
    let keys = generate_event_correlation_keys(&event_params);
    assert_eq!(keys.len(), 1);
    assert!(
        matches_subscription_configuration(sub.configuration.as_deref(), &keys),
        "hit: subscription configuration IN keys → consumer would trigger"
    );

    // Also match null-config subscriptions (broadcast) when keys present.
    assert!(matches_subscription_configuration(None, &keys));
}

/// Correlation miss: different event parameter value does not match the
/// subscription configuration (consumer ignores).
#[test]
fn correlation_miss_ignores_non_matching_key() {
    let engine = ProcessEngine::new("p93-corr-miss".to_string());
    deploy_correlated_start(&engine, "corr-miss");

    let subs = engine.get_event_start_subscriptions();
    let sub = subs
        .iter()
        .find(|s| s.event_ref == "myEvent")
        .expect("subscription");

    let mut wrong = BTreeMap::new();
    wrong.insert("customerId".to_string(), Some("otherCustomer".to_string()));
    let keys = generate_event_correlation_keys(&wrong);
    assert!(
        !matches_subscription_configuration(sub.configuration.as_deref(), &keys),
        "miss: configuration not IN keys → consumer ignores"
    );

    // Empty correlation keys only match subscriptions without configuration
    // (BaseEventRegistryEventConsumer.java:172-174).
    assert!(!matches_subscription_configuration(
        sub.configuration.as_deref(),
        &[]
    ));
    assert!(matches_subscription_configuration(None, &[]));
}

/// `storeAsUniqueReferenceId`: count process instances by referenceId /
/// referenceType; skip second start when count > 0 (no distributed lock).
#[test]
fn unique_reference_id_dedup_skips_second_start() {
    let engine = ProcessEngine::new("p93-unique-ref".to_string());
    deploy_correlated_start(&engine, "unique-ref");

    let subs = engine.get_event_start_subscriptions();
    let sub = subs
        .iter()
        .find(|s| s.event_ref == "myEvent")
        .expect("subscription");
    let corr_key = sub.configuration.clone().expect("configuration");

    // Model flag recognized (BpmnEventRegistryEventConsumer.java:125).
    let repository = engine.get_repository_service();
    let def_id = repository.get_process_definition_ids().unwrap()[0].clone();
    let model = engine
        .get_repository_service()
        .get_bpmn_model(&def_id)
        .expect("model");
    let start = model
        .main_process
        .as_ref()
        .and_then(|p| p.flow_element_map.get("theStart"))
        .expect("start");
    if let flowable_bpmn_model::model::FlowElementEnum::StartEvent(se) = start {
        assert!(is_store_as_unique_reference_id(
            &se.event
                .flow_node
                .flow_element
                .base_element
                .extension_elements
        ));
    } else {
        panic!("expected start event");
    }

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert_eq!(
        count_process_instances_for_unique_reference(
            &store,
            &mut session,
            "corrStartProcess",
            &corr_key,
            None,
        ),
        0
    );
    assert!(!should_skip_unique_start(
        &store,
        &mut session,
        "corrStartProcess",
        &corr_key,
        None,
    ));
    session.flush_and_commit().unwrap();

    // First "start": create PI with referenceId = full correlation key
    // (BpmnEventRegistryEventConsumer.startProcessInstance:242-245).
    let runtime = engine.get_runtime_service();
    let mut builder = ProcessInstanceBuilder::new().process_definition_key("corrStartProcess".to_string());
    builder.reference_id = Some(corr_key.clone());
    builder.reference_type = Some(REFERENCE_TYPE_EVENT_PROCESS.to_string());
    let pi = runtime.start_process_instance(builder).unwrap();
    assert_eq!(pi.reference_id.as_deref(), Some(corr_key.as_str()));
    assert_eq!(
        pi.reference_type.as_deref(),
        Some(REFERENCE_TYPE_EVENT_PROCESS)
    );

    let mut session = store.create_session().unwrap();
    assert_eq!(
        count_process_instances_for_unique_reference(
            &store,
            &mut session,
            "corrStartProcess",
            &corr_key,
            None,
        ),
        1
    );
    assert!(
        should_skip_unique_start(
            &store,
            &mut session,
            "corrStartProcess",
            &corr_key,
            None,
        ),
        "second event with same full correlation key must be skipped"
    );

    // Different correlation key still allowed.
    assert!(!should_skip_unique_start(
        &store,
        &mut session,
        "corrStartProcess",
        "different-key",
        None,
    ));
}

/// Intermediate catch with eventType + correlation stores configuration on
/// the wait state (runtime path with variable evaluation).
#[test]
fn intermediate_catch_stores_runtime_correlation_configuration() {
    let engine = ProcessEngine::new("p93-catch-corr".to_string());
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="catchCorrProcess" isExecutable="true">
    <startEvent id="start"/>
    <sequenceFlow sourceRef="start" targetRef="catchEvent"/>
    <intermediateCatchEvent id="catchEvent">
      <extensionElements>
        <flowable:eventType>myEvent</flowable:eventType>
        <flowable:eventCorrelationParameter name="customerId" value="${customerIdVar}"/>
      </extensionElements>
    </intermediateCatchEvent>
    <sequenceFlow sourceRef="catchEvent" targetRef="task"/>
    <userTask id="task"/>
    <sequenceFlow sourceRef="task" targetRef="end"/>
    <endEvent id="end"/>
  </process>
</definitions>"#;

    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("catch-corr".to_string())
                .add_string("catch_corr.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let runtime = engine.get_runtime_service();
    let builder = ProcessInstanceBuilder::new()
        .process_definition_key("catchCorrProcess".to_string())
        .variable("customerIdVar".to_string(), json!("cust-42"));
    let pi = runtime.start_process_instance(builder).unwrap();

    let waits = runtime.get_event_wait_states_by_process_instance_id(pi.id.clone());
    assert_eq!(waits.len(), 1, "one intermediate catch wait state");
    let wait = &waits[0];
    assert_eq!(wait.event_ref.as_deref(), Some("myEvent"));

    let mut params = BTreeMap::new();
    params.insert("customerId".to_string(), Some("cust-42".to_string()));
    let expected = generate_correlation_key(&params);
    assert_eq!(
        wait.configuration.as_deref(),
        Some(expected.as_str()),
        "wait-state configuration from evaluated customerIdVar expression"
    );

    // Hit / miss for this wait state.
    let hit_keys = generate_event_correlation_keys(&params);
    assert!(matches_subscription_configuration(
        wait.configuration.as_deref(),
        &hit_keys
    ));
    let mut miss = BTreeMap::new();
    miss.insert("customerId".to_string(), Some("other".to_string()));
    assert!(!matches_subscription_configuration(
        wait.configuration.as_deref(),
        &generate_event_correlation_keys(&miss)
    ));
}

/// P98: event correlation keys are the power set minus empty. Two parameters →
/// three subset keys (full + both singles), each reproducible from its own
/// parameter map (Java n=2 branch, BaseEventRegistryEventConsumer.java:89-102).
#[test]
fn correlation_two_params_produce_powerset_keys() {
    let mut params = BTreeMap::new();
    params.insert("customerId".to_string(), Some("testCustomer".to_string()));
    params.insert("orderId".to_string(), Some("order-1".to_string()));

    let keys = generate_event_correlation_keys(&params);
    assert_eq!(keys.len(), 3, "2 params → 3 subset keys");

    let mut only_customer = BTreeMap::new();
    only_customer.insert("customerId".to_string(), Some("testCustomer".to_string()));
    let mut only_order = BTreeMap::new();
    only_order.insert("orderId".to_string(), Some("order-1".to_string()));
    assert!(
        keys.contains(&generate_correlation_key(&only_customer)),
        "single-parameter subset key present"
    );
    assert!(
        keys.contains(&generate_correlation_key(&only_order)),
        "single-parameter subset key present"
    );
    assert!(
        keys.contains(&generate_correlation_key(&params)),
        "full-parameter key present"
    );
}

/// P98: three parameters → 2^3 - 1 = 7 subset keys including the full key;
/// empty parameters → no keys (Java :78-80).
#[test]
fn correlation_three_params_produce_seven_powerset_keys() {
    let mut params = BTreeMap::new();
    params.insert("a".to_string(), Some("1".to_string()));
    params.insert("b".to_string(), Some("2".to_string()));
    params.insert("c".to_string(), Some("3".to_string()));

    let keys = generate_event_correlation_keys(&params);
    assert_eq!(keys.len(), 7, "3 params → 7 subset keys");
    assert!(
        keys.contains(&generate_correlation_key(&params)),
        "full-parameter key present"
    );

    assert!(
        generate_event_correlation_keys(&BTreeMap::new()).is_empty(),
        "no parameters → no keys"
    );
}

/// P98 core observable: the subscription is registered with the full key of its
/// own declared parameters (here: only customerId), while the event's
/// correlation instances carry an EXTRA parameter (orderId). Java generates the
/// power set of the event instances, so the customerId-only subset key equals
/// the subscription configuration → hit. P93 first-phase full-key-only code
/// generated only the two-parameter key and missed.
#[test]
fn correlation_subset_key_hits_subscription_with_fewer_params() {
    let engine = ProcessEngine::new("p98-subset-hit".to_string());
    deploy_correlated_start(&engine, "subset-hit");

    let subs = engine.get_event_start_subscriptions();
    let sub = subs
        .iter()
        .find(|s| s.event_ref == "myEvent")
        .expect("event-registry start subscription for myEvent");

    let mut event_params = BTreeMap::new();
    event_params.insert("customerId".to_string(), Some("testCustomer".to_string()));
    event_params.insert("orderId".to_string(), Some("order-1".to_string()));

    let keys = generate_event_correlation_keys(&event_params);
    assert!(
        matches_subscription_configuration(sub.configuration.as_deref(), &keys),
        "customerId-only subset key must hit the customerId subscription"
    );

    // Demonstrate the P93 regression this fixes: full-key-only would miss.
    let full_only = vec![generate_correlation_key(&event_params)];
    assert!(
        !matches_subscription_configuration(sub.configuration.as_deref(), &full_only),
        "full-parameter-only key does not equal the single-parameter configuration"
    );
}

/// P98 miss: event parameter values differ from the subscription's, so none of
/// the subset keys (customerId=other / orderId / both) equals the configuration.
#[test]
fn correlation_value_mismatch_no_subset_key_hits() {
    let engine = ProcessEngine::new("p98-subset-miss".to_string());
    deploy_correlated_start(&engine, "subset-miss");

    let subs = engine.get_event_start_subscriptions();
    let sub = subs
        .iter()
        .find(|s| s.event_ref == "myEvent")
        .expect("event-registry start subscription for myEvent");

    let mut event_params = BTreeMap::new();
    event_params.insert("customerId".to_string(), Some("otherCustomer".to_string()));
    event_params.insert("orderId".to_string(), Some("order-1".to_string()));

    let keys = generate_event_correlation_keys(&event_params);
    assert_eq!(keys.len(), 3);
    assert!(
        !matches_subscription_configuration(sub.configuration.as_deref(), &keys),
        "value mismatch: no subset key equals the configuration"
    );
}
