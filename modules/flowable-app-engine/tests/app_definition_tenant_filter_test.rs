//! P1 tenant contract tests for app definition queries.
//!
//! Covers the three-state tenant filter fix: a tenantless lookup
//! (`tenant_id = None`) must generate `TENANT_ID_ IS NULL` and never fall
//! back to "any tenant", otherwise it could leak another tenant's latest
//! definition.

use flowable_app_engine::{
    AppDefinition, AppDeploymentRequest, AppEngine, AppModel, AppPage, AppReference,
    InMemoryDefinitionCatalog,
};
use std::sync::Arc;

fn engine() -> AppEngine {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("employee-onboarding", "Employee Onboarding", 3, None)
        .build();
    AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap()
}

fn app_definition(label: &str) -> AppDefinition {
    AppDefinition::new(
        format!("app-{label}"),
        "employee-portal",
        format!("Employee Portal {label}"),
    )
    .with_page(
        AppPage::new("page-process", "Process Dashboard").with_reference(
            AppReference::process("start-onboarding").with_definition_key("employee-onboarding"),
        ),
    )
}

fn deploy(engine: &AppEngine, deployment_name: &str, tenant_id: Option<&str>, label: &str) {
    let mut request = AppDeploymentRequest::new(deployment_name).with_resource(
        format!("{deployment_name}.json"),
        AppModel::new().with_app_definition(app_definition(label)),
    );
    if let Some(tenant) = tenant_id {
        request = request.with_tenant_id(tenant);
    }
    engine.deploy(request).unwrap();
}

/// tenant-a is deployed twice (v1, v2) BEFORE the tenantless definition (v1),
/// so the buggy "no tenant condition" query would order tenant-a v2 first.
fn seed_mixed_tenants(engine: &AppEngine) {
    deploy(engine, "portal-tenant-a-v1", Some("tenant-a"), "tenant-a-v1");
    deploy(engine, "portal-tenant-a-v2", Some("tenant-a"), "tenant-a-v2");
    deploy(engine, "portal-tenantless", None, "tenantless");
}

#[test]
fn tenantless_resolve_never_leaks_other_tenant_definition() {
    let engine = engine();
    seed_mixed_tenants(&engine);

    // Previous bug: tenant None dropped the tenant condition entirely, so the
    // "latest" row was tenant-a v2. Now None must mean TENANT_ID_ IS NULL.
    let composition = engine
        .runtime_service()
        .resolve_app_definition_by_key("employee-portal", None)
        .unwrap();
    assert_eq!(composition.tenant_id, None);
    assert_eq!(composition.version, 1);
    assert_eq!(composition.app_definition_name, "Employee Portal tenantless");
}

#[test]
fn tenant_scoped_resolve_still_returns_latest_of_that_tenant() {
    let engine = engine();
    seed_mixed_tenants(&engine);

    let composition = engine
        .runtime_service()
        .resolve_app_definition_by_key("employee-portal", Some("tenant-a"))
        .unwrap();
    assert_eq!(composition.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(composition.version, 2);
}

#[test]
fn without_tenant_query_only_matches_tenantless_definitions() {
    let engine = engine();
    seed_mixed_tenants(&engine);

    let tenantless = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .without_tenant_id()
        .list()
        .unwrap();
    assert_eq!(tenantless.len(), 1);
    assert_eq!(tenantless[0].tenant_id, None);
    assert_eq!(tenantless[0].version, 1);

    // No filter at all still sees every tenant's definitions.
    let all = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .list()
        .unwrap();
    assert_eq!(all.len(), 3);

    let tenant_a = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .tenant_id("tenant-a")
        .list()
        .unwrap();
    assert_eq!(tenant_a.len(), 2);
    assert!(tenant_a
        .iter()
        .all(|item| item.tenant_id.as_deref() == Some("tenant-a")));
}

#[test]
fn tenantless_resolve_fails_when_only_tenant_definitions_exist() {
    let engine = engine();
    deploy(&engine, "portal-only-tenant-a", Some("tenant-a"), "tenant-a-v1");

    // With only tenant-scoped definitions deployed, a tenantless lookup must
    // report "not found" instead of silently borrowing tenant-a's definition.
    let err = engine
        .runtime_service()
        .resolve_app_definition_by_key("employee-portal", None)
        .unwrap_err();
    assert!(
        err.to_string().contains("was not found"),
        "unexpected error: {err}"
    );
}
