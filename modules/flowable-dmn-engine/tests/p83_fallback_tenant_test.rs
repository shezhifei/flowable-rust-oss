//! P83 task A — `fallbackToDefaultTenant` on DMN decision resolution.
//!
//! Java reference: `AbstractExecuteDecisionCmd.resolveDefinition`
//! (`:77-163`) — when the key+tenant lookup misses and the flag is set, the
//! engine retries against `DefaultTenantProvider.getDefaultTenant(...)`. The
//! engine default provider yields `NO_TENANT_ID` = `""`
//! (`AbstractEngineConfiguration.java:139,329`), so Java takes the
//! `StringUtils.isNotEmpty(defaultTenant) == false` branch and falls back to
//! `findLatestDecisionByKey(key)` — a lookup with no tenant filter at all.
//! Rust has no `DefaultTenantProvider`, so that empty-default branch is the
//! behaviour ported here (same treatment as
//! `call_activity_behavior.rs` for process definitions).

use flowable_dmn_engine::{
    DecisionService, DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest,
    DmnHitPolicy, DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry,
    DmnRuleOutputEntry, DmnUnaryTest,
};
use serde_json::json;

fn dish_decision(default_dish: &str) -> DmnDecision {
    DmnDecision::new(
        "decision-1",
        "dishDecision",
        "Dish decision",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "dishType")],
        vec![DmnOutputClause::new("output-1", "dish")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!(default_dish))],
        )],
    )
}

/// Deploys `dishDecision` into the default (untenanted) deployment only.
fn engine_with_default_tenant_decision() -> DmnEngine {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("default-tenant")
                .with_resource("dish.dmn", DmnModel::new(vec![dish_decision("default")])),
        )
        .expect("default tenant deployment");
    engine
}

#[test]
fn tenant_miss_without_fallback_reports_not_found() {
    let engine = engine_with_default_tenant_decision();

    let error = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({"dishType": "salad"})).with_tenant_id("tenant-a"),
        )
        .expect_err("tenant-a has no dishDecision and fallback is off");

    assert!(
        error.to_string().contains("was not found"),
        "unexpected error: {error}"
    );
}

#[test]
fn tenant_miss_with_fallback_resolves_default_tenant_decision() {
    let engine = engine_with_default_tenant_decision();

    let result = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({"dishType": "salad"}))
                .with_tenant_id("tenant-a")
                .fallback_to_default_tenant(),
        )
        .expect("fallback should resolve the untenanted decision");

    assert_eq!(result.decision_result.len(), 1);
    assert_eq!(result.decision_result[0]["dish"], json!("default"));
}

#[test]
fn fallback_does_not_override_a_matching_tenant_decision() {
    let engine = engine_with_default_tenant_decision();
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("tenant-a")
                .with_tenant_id("tenant-a")
                .with_resource("dish.dmn", DmnModel::new(vec![dish_decision("tenant-a")])),
        )
        .expect("tenant-a deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({"dishType": "salad"}))
                .with_tenant_id("tenant-a")
                .fallback_to_default_tenant(),
        )
        .expect("tenant-owned decision wins");

    assert_eq!(result.decision_result[0]["dish"], json!("tenant-a"));
}

#[test]
fn fallback_reports_not_found_when_no_decision_exists_anywhere() {
    let engine = DmnEngine::new_in_memory().expect("engine");

    let error = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({"dishType": "salad"}))
                .with_tenant_id("tenant-a")
                .fallback_to_default_tenant(),
        )
        .expect_err("nothing to fall back to");

    assert!(
        error.to_string().contains("was not found"),
        "unexpected error: {error}"
    );
}

/// Java resolves decision *services* through the same command
/// (`AbstractExecuteDecisionCmd` is shared), so the fallback applies there too.
#[test]
fn decision_service_lookup_honours_fallback_to_default_tenant() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let mut model = DmnModel::new(vec![dish_decision("default")]);
    model.decision_services.push(DecisionService {
        id: "dishService".to_string(),
        name: "Dish service".to_string(),
        required_decisions: vec![],
        output_decisions: vec!["dishDecision".to_string()],
    });
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("default-tenant").with_resource("dish-service.dmn", model),
        )
        .expect("default tenant deployment");

    let miss = engine.decision_service().execute_by_key(
        "dishService",
        DmnExecutionRequest::new(json!({"dishType": "salad"})).with_tenant_id("tenant-a"),
    );
    assert!(miss.is_err(), "no fallback → decision service not found");

    let result = engine
        .decision_service()
        .execute_by_key(
            "dishService",
            DmnExecutionRequest::new(json!({"dishType": "salad"}))
                .with_tenant_id("tenant-a")
                .fallback_to_default_tenant(),
        )
        .expect("fallback should resolve the untenanted decision service");

    assert_eq!(result.decision_result[0]["dish"], json!("default"));
}

/// The flag defaults to false and survives a JSON round-trip of a request
/// written before P83 existed.
#[test]
fn request_fallback_flag_defaults_to_false_for_legacy_json() {
    let legacy = json!({
        "variables": {"dishType": "salad"},
        "business_key": null,
        "tenant_id": null,
        "parent_deployment_id": null,
        "disable_history": false
    });
    let request: DmnExecutionRequest =
        serde_json::from_value(legacy).expect("legacy request JSON should deserialize");

    assert!(!request.fallback_to_default_tenant);
    assert_eq!(request.instance_id, None);
    assert_eq!(request.execution_id, None);
    assert_eq!(request.activity_id, None);
    assert_eq!(request.scope_type, None);
}
