//! P2 contract tests for `AppReference::definition_id` pinning and
//! per-reference `tenant_id` resolution.
//!
//! A pinned reference must resolve by exact definition id (never silently
//! fall back to latest-by-key), and an unpinned reference must resolve in the
//! reference tenant when one is declared, only inheriting the App definition
//! tenant otherwise.

use flowable_app_engine::{
    AppDefinition, AppDeploymentRequest, AppEngine, AppModel, AppPage, AppReference,
    InMemoryDefinitionCatalog, TenantResolutionPolicy,
};
use std::sync::Arc;

fn deployment(reference: AppReference) -> AppDeploymentRequest {
    AppDeploymentRequest::new("invoice-apps").with_resource(
        "invoice-app.json",
        AppModel::new().with_app_definition(
            AppDefinition::new("app-invoice", "invoice-portal", "Invoice Portal")
                .with_page(AppPage::new("page-invoice", "Invoices").with_reference(reference)),
        ),
    )
}

/// v1 (tenant "archive") and v2 (tenantless) both exist; latest-by-key would
/// resolve v2, but the pin must bind exactly v1.
#[test]
fn pinned_definition_id_resolves_exact_version_not_latest() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("invoice", "Invoice v1", 1, Some("archive"))
        .with_process_definition("invoice", "Invoice v2", 2, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    engine
        .deploy(deployment(
            AppReference::process("invoice-ref")
                .with_definition_key("invoice")
                .with_definition_id("bpmn-process:invoice:v1"),
        ))
        .unwrap();

    let composition = engine
        .runtime_service()
        .resolve_app_definition_by_key("invoice-portal", None)
        .unwrap();
    assert_eq!(composition.references.len(), 1);
    assert_eq!(
        composition.references[0].resolved_definition_id,
        "bpmn-process:invoice:v1"
    );
    assert_eq!(composition.references[0].resolved_definition_version, 1);
    assert_eq!(
        composition.references[0].tenant_id.as_deref(),
        Some("archive")
    );
}

/// The same catalog without a pin resolves latest-by-key (v2) — proving the
/// pin in the previous test actually changed resolution.
#[test]
fn unpinned_reference_still_resolves_latest_by_key() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("invoice", "Invoice v1", 1, Some("archive"))
        .with_process_definition("invoice", "Invoice v2", 2, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    engine
        .deploy(deployment(
            AppReference::process("invoice-ref").with_definition_key("invoice"),
        ))
        .unwrap();

    let composition = engine
        .runtime_service()
        .resolve_app_definition_by_key("invoice-portal", None)
        .unwrap();
    assert_eq!(
        composition.references[0].resolved_definition_id,
        "bpmn-process:invoice:v2"
    );
    assert_eq!(composition.references[0].resolved_definition_version, 2);
}

/// A pin to a non-existent definition id must fail the deployment instead of
/// silently falling back to latest-by-key.
#[test]
fn deployment_fails_when_pinned_definition_id_is_missing() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("invoice", "Invoice v2", 2, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    let err = engine
        .deploy(deployment(
            AppReference::process("invoice-ref")
                .with_definition_key("invoice")
                .with_definition_id("bpmn-process:invoice:v9"),
        ))
        .unwrap_err();
    assert!(
        err.to_string().contains("pins missing"),
        "unexpected error: {err}"
    );
}

/// A pin whose resolved key differs from the declared reference key must fail.
#[test]
fn deployment_fails_when_pinned_id_key_mismatches_declared_key() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("invoice", "Invoice", 1, None)
        .with_process_definition("billing", "Billing", 1, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    let err = engine
        .deploy(deployment(
            AppReference::process("invoice-ref")
                .with_definition_key("billing")
                .with_definition_id("bpmn-process:invoice:v1"),
        ))
        .unwrap_err();
    assert!(
        err.to_string().contains("does not match the declared key"),
        "unexpected error: {err}"
    );
}

/// A pin combined with a per-reference tenant must fail when the pinned
/// definition belongs to a different tenant.
#[test]
fn deployment_fails_when_pinned_id_tenant_mismatches_reference_tenant() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("invoice", "Invoice", 1, Some("tenant-a"))
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    let err = engine
        .deploy(deployment(
            AppReference::process("invoice-ref")
                .with_definition_key("invoice")
                .with_definition_id("bpmn-process:invoice:v1")
                .with_tenant_id("tenant-b"),
        ))
        .unwrap_err();
    assert!(
        err.to_string().contains("belongs to tenant"),
        "unexpected error: {err}"
    );
}

/// An unpinned reference declaring its own tenant must resolve in that tenant
/// even when the App definition lives in another tenant.
#[test]
fn reference_tenant_overrides_app_tenant() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("invoice", "Invoice tenant-a", 1, Some("tenant-a"))
        .with_process_definition("invoice", "Invoice tenant-b", 5, Some("tenant-b"))
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    engine
        .deploy(
            deployment(
                AppReference::process("invoice-ref")
                    .with_definition_key("invoice")
                    .with_tenant_id("tenant-b"),
            )
            .with_tenant_id("tenant-a"),
        )
        .unwrap();

    let composition = engine
        .runtime_service()
        .resolve_app_definition_by_key("invoice-portal", Some("tenant-a"))
        .unwrap();
    assert_eq!(
        composition.references[0].tenant_id.as_deref(),
        Some("tenant-b")
    );
    assert_eq!(composition.references[0].resolved_definition_version, 5);
}

/// Under the strict policy, a declared reference tenant must not be silently
/// satisfied by a tenantless default definition.
#[test]
fn strict_policy_rejects_default_tenant_substitute_for_reference_tenant() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("invoice", "Invoice default", 1, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog))
        .unwrap()
        .with_tenant_resolution_policy(TenantResolutionPolicy::Strict);

    let err = engine
        .deploy(deployment(
            AppReference::process("invoice-ref")
                .with_definition_key("invoice")
                .with_tenant_id("tenant-b"),
        ))
        .unwrap_err();
    assert!(
        err.to_string().contains("references missing"),
        "unexpected error: {err}"
    );
}
