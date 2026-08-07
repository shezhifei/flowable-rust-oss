//! P136 1b: definition-level CMMN event-registry start subscriptions on deploy/delete.
//!
//! Java: CmmnDeployer.java:194-224, CmmnDeploymentEntityManagerImpl.java:57-108.

use flowable_cmmn_engine::{
    generate_correlation_key, CmmnCase, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnEventCorrelationParameter, CmmnModel, START_EVENT_CORRELATION_MANUAL,
    START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID,
};
use std::collections::BTreeMap;

fn engine() -> CmmnEngine {
    CmmnEngine::new_in_memory().expect("engine")
}

fn static_start_case(key: &str, event_type: &str) -> CmmnModel {
    let mut case = CmmnCase::new(key, key, format!("{key} case"), CmmnCasePlanModel::new("pm", "pm"));
    case.start_event_type = Some(event_type.to_string());
    CmmnModel::new(vec![case])
}

fn unique_ref_case(key: &str, event_type: &str) -> CmmnModel {
    let mut case = CmmnCase::new(key, key, format!("{key} case"), CmmnCasePlanModel::new("pm", "pm"));
    case.start_event_type = Some(event_type.to_string());
    case.start_correlation_configuration =
        Some(START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID.to_string());
    case.start_correlation_parameters = vec![CmmnEventCorrelationParameter::new("orderId", "O-1")];
    CmmnModel::new(vec![case])
}

fn manual_start_case(key: &str, event_type: &str) -> CmmnModel {
    let mut case = CmmnCase::new(key, key, format!("{key} case"), CmmnCasePlanModel::new("pm", "pm"));
    case.start_event_type = Some(event_type.to_string());
    case.start_correlation_configuration = Some(START_EVENT_CORRELATION_MANUAL.to_string());
    CmmnModel::new(vec![case])
}

fn deploy(engine: &CmmnEngine, name: &str, model: CmmnModel) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(name).with_resource("case.cmmn", model),
        )
        .expect("deploy")
        .id
}

fn def_level_subs(
    engine: &CmmnEngine,
    event_type: &str,
) -> Vec<flowable_cmmn_engine::CmmnEventSubscription> {
    engine
        .runtime_service()
        .create_event_subscription_query()
        .event_type(event_type)
        .without_scope_id()
        .list()
        .expect("list")
        .into_iter()
        .filter(|s| s.plan_item_instance_id.is_none())
        .collect()
}

#[test]
fn static_deploy_creates_definition_level_subscription() {
    let engine = engine();
    let deployment_id = deploy(&engine, "d1", static_start_case("staticCase", "startEvt"));
    let defs = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(deployment_id)
        .list()
        .expect("defs");
    assert_eq!(defs.len(), 1);

    let subs = def_level_subs(&engine, "startEvt");
    assert_eq!(subs.len(), 1, "static deploy must create one def-level sub");
    assert_eq!(
        subs[0].case_definition_id.as_deref(),
        Some(defs[0].id.as_str())
    );
    assert!(subs[0].case_instance_id.is_none());
    assert!(subs[0].configuration.is_none());
}

#[test]
fn static_deploy_with_correlation_stores_generated_key() {
    let engine = engine();
    deploy(&engine, "d-corr", unique_ref_case("corrCase", "corrEvt"));
    let subs = def_level_subs(&engine, "corrEvt");
    assert_eq!(subs.len(), 1);
    let mut params = BTreeMap::new();
    params.insert("orderId".to_string(), Some("O-1".to_string()));
    assert_eq!(
        subs[0].configuration.as_deref(),
        Some(generate_correlation_key(&params).as_str())
    );
}

#[test]
fn version_upgrade_replaces_static_subscription() {
    let engine = engine();
    let d1 = deploy(&engine, "v1", static_start_case("upgradeCase", "upEvt"));
    let v1 = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(d1)
        .single_result()
        .expect("v1")
        .expect("v1 def");
    let subs_v1 = def_level_subs(&engine, "upEvt");
    assert_eq!(subs_v1.len(), 1);
    assert_eq!(
        subs_v1[0].case_definition_id.as_deref(),
        Some(v1.id.as_str())
    );

    let d2 = deploy(&engine, "v2", static_start_case("upgradeCase", "upEvt"));
    let v2 = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(d2)
        .single_result()
        .expect("v2")
        .expect("v2 def");

    let subs = def_level_subs(&engine, "upEvt");
    assert_eq!(subs.len(), 1, "upgrade must leave exactly one static sub");
    assert_eq!(
        subs[0].case_definition_id.as_deref(),
        Some(v2.id.as_str()),
        "sub must point at new version"
    );
}

#[test]
fn manual_subscription_does_not_auto_create() {
    let engine = engine();
    deploy(
        &engine,
        "manual",
        manual_start_case("manualCase", "manualEvt"),
    );
    let subs = def_level_subs(&engine, "manualEvt");
    assert!(
        subs.is_empty(),
        "manualSubscription must not auto-create a definition-level sub"
    );
}

#[test]
fn delete_latest_restores_previous_version_subscription() {
    let engine = engine();
    let d1 = deploy(
        &engine,
        "del-v1",
        static_start_case("restoreCase", "restoreEvt"),
    );
    let v1 = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(d1)
        .single_result()
        .expect("q")
        .expect("v1");
    let d2 = deploy(
        &engine,
        "del-v2",
        static_start_case("restoreCase", "restoreEvt"),
    );
    let subs_mid = def_level_subs(&engine, "restoreEvt");
    assert_eq!(subs_mid.len(), 1);

    engine
        .repository_service()
        .delete_deployment(&d2, false)
        .expect("delete v2");

    let subs = def_level_subs(&engine, "restoreEvt");
    assert_eq!(
        subs.len(),
        1,
        "delete latest must restore previous start sub"
    );
    assert_eq!(
        subs[0].case_definition_id.as_deref(),
        Some(v1.id.as_str()),
        "restored sub must target previous version"
    );
}

#[test]
fn delete_non_latest_does_not_restore_an_older_subscription() {
    let engine = engine();
    deploy(
        &engine,
        "del-middle-v1",
        static_start_case("middleCase", "middleEvt"),
    );
    let d2 = deploy(
        &engine,
        "del-middle-v2",
        static_start_case("middleCase", "middleEvt"),
    );
    let d3 = deploy(
        &engine,
        "del-middle-v3",
        static_start_case("middleCase", "middleEvt"),
    );
    let v3 = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(d3)
        .single_result()
        .expect("query v3")
        .expect("v3 definition");

    engine
        .repository_service()
        .delete_deployment(&d2, false)
        .expect("delete middle version");

    let subs = def_level_subs(&engine, "middleEvt");
    assert_eq!(
        subs.len(),
        1,
        "deleting a non-latest version must not restore an older subscription"
    );
    assert_eq!(
        subs[0].case_definition_id.as_deref(),
        Some(v3.id.as_str()),
        "the latest version subscription must remain authoritative"
    );
}

#[test]
fn no_start_event_type_creates_no_subscription() {
    let engine = engine();
    let case = CmmnCase::new(
        "plain",
        "plain",
        "plain",
        CmmnCasePlanModel::new("pm", "pm"),
    );
    deploy(&engine, "plain", CmmnModel::new(vec![case]));
    let all = engine
        .runtime_service()
        .create_event_subscription_query()
        .list()
        .expect("all");
    assert!(all.is_empty());
}
