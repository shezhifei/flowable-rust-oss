//! P27: message/signal start subscription lifecycle on redeploy / undeploy.
//!
//! Java evidence:
//! - BpmnDeploymentHelper.addEventRegistrations (BpmnDeploymentHelper.java:172-173)
//!   → EventSubscriptionManager.removeObsoleteMessageEventSubscriptions
//!   (EventSubscriptionManager.java:55-67) and
//!   removeObsoleteSignalEventSubscription (EventSubscriptionManager.java:122-133):
//!   on redeploy, the previous version's message/signal start subscriptions of the
//!   same process-definition key (+ tenant) are removed.
//! - DeploymentProcessDefinitionDeletionManagerImpl.restorePreviousStartEventsIfNeeded
//!   (DeploymentProcessDefinitionDeletionManagerImpl.java:111-155): undeploying the
//!   latest version re-registers the previous version's signal (:133) and
//!   message (:135) start events.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;

fn message_start_xml(duration_marker: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="startMsg" name="orderMessage" />
        <process id="messageStartLifecycle" isExecutable="true">
            <startEvent id="messageStart" name="{duration_marker}">
                <messageEventDefinition messageRef="startMsg" />
            </startEvent>
            <sequenceFlow id="f1" sourceRef="messageStart" targetRef="task" />
            <userTask id="task" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

fn signal_start_xml(marker: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="startSig" name="alertSignal" />
        <process id="signalStartLifecycle" isExecutable="true">
            <startEvent id="signalStart" name="{marker}">
                <signalEventDefinition signalRef="startSig" />
            </startEvent>
            <sequenceFlow id="f1" sourceRef="signalStart" targetRef="task" />
            <userTask id="task" />
            <sequenceFlow id="f2" sourceRef="task" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

fn deploy(engine: &ProcessEngine, name: &str, resource: &str, xml: String) -> String {
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(name.to_string())
                .add_string(resource.to_string(), xml),
        )
        .unwrap()
        .id
}

#[test]
fn redeploy_cancels_old_version_message_start_subscription() {
    // Java EventSubscriptionManager.removeObsoleteMessageEventSubscriptions
    // (EventSubscriptionManager.java:55-67)
    let engine = ProcessEngine::new("p27-redeploy-message".to_string());
    deploy(&engine, "v1", "v1.bpmn20.xml", message_start_xml("v1"));

    let subs_v1 = engine.get_event_start_subscriptions();
    assert_eq!(subs_v1.len(), 1);
    assert_eq!(subs_v1[0].event_kind, EventSubscriptionKind::Message);
    let v1_def = subs_v1[0].process_definition_id.clone();

    deploy(&engine, "v2", "v2.bpmn20.xml", message_start_xml("v2"));

    let subs = engine.get_event_start_subscriptions();
    assert_eq!(
        subs.len(),
        1,
        "redeploy must remove the obsolete message start subscription; only latest remains"
    );
    assert_ne!(
        subs[0].process_definition_id, v1_def,
        "remaining subscription must belong to the new version"
    );
    assert_eq!(subs[0].start_event_name.as_deref(), Some("v2"));
    assert_eq!(subs[0].event_ref, "orderMessage");
}

#[test]
fn redeploy_cancels_old_version_signal_start_subscription() {
    // Java EventSubscriptionManager.removeObsoleteSignalEventSubscription
    // (EventSubscriptionManager.java:122-133)
    let engine = ProcessEngine::new("p27-redeploy-signal".to_string());
    deploy(&engine, "v1", "v1.bpmn20.xml", signal_start_xml("v1"));

    let subs_v1 = engine.get_event_start_subscriptions();
    assert_eq!(subs_v1.len(), 1);
    assert_eq!(subs_v1[0].event_kind, EventSubscriptionKind::Signal);
    let v1_def = subs_v1[0].process_definition_id.clone();

    deploy(&engine, "v2", "v2.bpmn20.xml", signal_start_xml("v2"));

    let subs = engine.get_event_start_subscriptions();
    assert_eq!(
        subs.len(),
        1,
        "redeploy must remove the obsolete signal start subscription; only latest remains"
    );
    assert_ne!(subs[0].process_definition_id, v1_def);
    assert_eq!(subs[0].start_event_name.as_deref(), Some("v2"));
    assert_eq!(subs[0].event_ref, "startSig");
}

#[test]
fn undeploy_latest_restores_previous_version_message_start_subscription() {
    // Java DeploymentProcessDefinitionDeletionManagerImpl
    // .restorePreviousStartEventsIfNeeded (:111-155, message at :135)
    let engine = ProcessEngine::new("p27-undeploy-restore-message".to_string());
    deploy(&engine, "v1", "v1.bpmn20.xml", message_start_xml("v1"));
    let v1_def = engine.get_event_start_subscriptions()[0]
        .process_definition_id
        .clone();
    let dep2 = deploy(&engine, "v2", "v2.bpmn20.xml", message_start_xml("v2"));

    engine
        .get_repository_service()
        .delete_deployment(&dep2)
        .unwrap();

    let subs = engine.get_event_start_subscriptions();
    assert_eq!(
        subs.len(),
        1,
        "undeploying the latest version must restore the previous message start subscription"
    );
    assert_eq!(subs[0].process_definition_id, v1_def);
    assert_eq!(subs[0].event_kind, EventSubscriptionKind::Message);
    assert_eq!(subs[0].start_event_name.as_deref(), Some("v1"));
}

#[test]
fn undeploy_latest_restores_previous_version_signal_start_subscription() {
    // Java DeploymentProcessDefinitionDeletionManagerImpl
    // .restorePreviousStartEventsIfNeeded (:111-155, signal at :133)
    let engine = ProcessEngine::new("p27-undeploy-restore-signal".to_string());
    deploy(&engine, "v1", "v1.bpmn20.xml", signal_start_xml("v1"));
    let v1_def = engine.get_event_start_subscriptions()[0]
        .process_definition_id
        .clone();
    let dep2 = deploy(&engine, "v2", "v2.bpmn20.xml", signal_start_xml("v2"));

    engine
        .get_repository_service()
        .delete_deployment(&dep2)
        .unwrap();

    let subs = engine.get_event_start_subscriptions();
    assert_eq!(
        subs.len(),
        1,
        "undeploying the latest version must restore the previous signal start subscription"
    );
    assert_eq!(subs[0].process_definition_id, v1_def);
    assert_eq!(subs[0].event_kind, EventSubscriptionKind::Signal);
    assert_eq!(subs[0].start_event_name.as_deref(), Some("v1"));
}

#[test]
fn undeploy_old_version_keeps_latest_event_start_subscription() {
    // Java restorePreviousStartEventsIfNeeded only fires when the deleted
    // definition is the latest version (:111-119); deleting an old version
    // leaves the latest subscription untouched.
    let engine = ProcessEngine::new("p27-undeploy-old-keep".to_string());
    let dep1 = deploy(&engine, "v1", "v1.bpmn20.xml", message_start_xml("v1"));
    deploy(&engine, "v2", "v2.bpmn20.xml", message_start_xml("v2"));

    engine
        .get_repository_service()
        .delete_deployment(&dep1)
        .unwrap();

    let subs = engine.get_event_start_subscriptions();
    assert_eq!(
        subs.len(),
        1,
        "undeploying an old version must keep the latest subscription (no restore, no over-delete)"
    );
    assert_eq!(subs[0].start_event_name.as_deref(), Some("v2"));
}

#[test]
fn redeploy_other_tenant_does_not_cancel_subscription() {
    // Java EventSubscriptionManager filters obsolete subscriptions by tenantId
    // (EventSubscriptionManager.java:60-63,127-130); a deploy in another tenant
    // must not delete this tenant's subscription.
    let engine = ProcessEngine::new("p27-tenant-isolation".to_string());
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("tenantA".to_string())
                .tenant_id("tenantA".to_string())
                .add_string("a.bpmn20.xml".to_string(), message_start_xml("tenantA")),
        )
        .unwrap();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("tenantB".to_string())
                .tenant_id("tenantB".to_string())
                .add_string("b.bpmn20.xml".to_string(), message_start_xml("tenantB")),
        )
        .unwrap();

    let subs = engine.get_event_start_subscriptions();
    assert_eq!(
        subs.len(),
        2,
        "same key in different tenants must keep both subscriptions"
    );
}
