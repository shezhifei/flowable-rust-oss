use flowable_app_engine::{
    AppDefinition, AppDeploymentRequest, AppEngine, AppModel, AppPage, AppReference,
    InMemoryDefinitionCatalog, TenantResolutionPolicy, models_semantically_equal,
    parse_resource_bytes_to_engine_model, serialize_engine_model_as_durable_bytes,
};
use std::sync::Arc;

#[test]
fn deploys_app_definition_and_validates_cross_engine_references() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("employee-onboarding", "Employee Onboarding", 3, None)
        .with_decision_definition("benefits-eligibility", "Benefits Eligibility", 2, None)
        .with_case_definition("equipment-case", "Equipment Case", 1, None)
        .with_event_definition("employee-updated", "Employee Updated", 4, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    let deployment = engine
        .deploy(
            AppDeploymentRequest::new("employee-apps").with_resource(
                "employee-app.json",
                AppModel::new().with_app_definition(
                    AppDefinition::new("app-employee", "employee-portal", "Employee Portal")
                        .with_page(
                            AppPage::new("page-process", "Process Dashboard")
                                .with_reference(
                                    AppReference::process("start-onboarding")
                                        .with_definition_key("employee-onboarding"),
                                )
                                .with_reference(
                                    AppReference::decision("benefits-check")
                                        .with_definition_key("benefits-eligibility"),
                                )
                                .with_reference(
                                    AppReference::case("equipment-case-page")
                                        .with_definition_key("equipment-case"),
                                )
                                .with_reference(
                                    AppReference::event("employee-event-page")
                                        .with_definition_key("employee-updated"),
                                ),
                        ),
                ),
            ),
        )
        .unwrap();

    assert_eq!(deployment.name, "employee-apps");
    assert_eq!(deployment.resource_names, vec!["employee-app.json"]);

    let definition = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .single_result()
        .unwrap()
        .expect("definition should exist");

    assert_eq!(definition.key, "employee-portal");
    assert_eq!(definition.version, 1);
    assert_eq!(definition.resource_name, "employee-app.json");
}

#[test]
fn rejects_deployment_when_referenced_definition_is_missing() {
    let engine =
        AppEngine::new_in_memory_with_catalog(Arc::new(InMemoryDefinitionCatalog::new())).unwrap();

    let error = engine
        .deploy(
            AppDeploymentRequest::new("employee-apps").with_resource(
                "employee-app.json",
                AppModel::new().with_app_definition(
                    AppDefinition::new("app-employee", "employee-portal", "Employee Portal")
                        .with_page(
                            AppPage::new("page-process", "Process Dashboard").with_reference(
                                AppReference::process("start-onboarding")
                                    .with_definition_key("missing-process"),
                            ),
                        ),
                ),
            ),
        )
        .expect_err("deployment should fail");

    assert!(
        error
            .to_string()
            .contains("references missing BPMN definition key 'missing-process'"),
        "unexpected error: {error}"
    );
}

#[test]
fn deployment_preserves_app_metadata_reference_names_and_delete_cascades() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("employee-onboarding", "Employee Onboarding", 3, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    let deployment = engine
        .deploy(
            AppDeploymentRequest::new("employee-apps")
                .with_category("HR")
                .with_resource(
                    "employee-app.app",
                    AppModel::new().with_app_definition(
                        AppDefinition::new("app-employee", "employee-portal", "Employee Portal")
                            .with_theme("flowable")
                            .with_icon("employee")
                            .with_users_access("admin,hr")
                            .with_groups_access("hr")
                            .with_landing_page("page-process")
                            .with_page(
                                AppPage::new("page-process", "Process Dashboard")
                                    .with_description("Start and track onboarding")
                                    .with_icon("play")
                                    .with_order(10)
                                    .with_reference(
                                        AppReference::process("start-onboarding")
                                            .with_name("Start onboarding")
                                            .with_description("Starts onboarding process")
                                            .with_definition_key("employee-onboarding"),
                                    ),
                            ),
                    ),
                ),
        )
        .unwrap();
    assert_eq!(deployment.category.as_deref(), Some("HR"));

    let definition = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .single_result()
        .unwrap()
        .expect("definition should exist");
    assert_eq!(definition.model.theme.as_deref(), Some("flowable"));
    assert_eq!(definition.model.icon.as_deref(), Some("employee"));
    assert_eq!(definition.model.users_access.as_deref(), Some("admin,hr"));
    assert_eq!(definition.model.groups_access.as_deref(), Some("hr"));
    assert_eq!(
        definition.model.landing_page.as_deref(),
        Some("page-process")
    );
    assert_eq!(
        definition.model.pages[0].description.as_deref(),
        Some("Start and track onboarding")
    );
    assert_eq!(definition.model.pages[0].icon.as_deref(), Some("play"));
    assert_eq!(definition.model.pages[0].order, Some(10));

    let composition = engine
        .runtime_service()
        .resolve_app_definition_by_key("employee-portal", None)
        .unwrap();
    assert_eq!(
        composition.references[0].reference_name.as_deref(),
        Some("Start onboarding")
    );

    engine
        .repository_service()
        .delete_deployment(&deployment.id)
        .unwrap();

    assert!(
        engine
            .repository_service()
            .get_deployment(&deployment.id)
            .is_err()
    );
    assert!(
        engine
            .repository_service()
            .get_app_definition(&definition.id)
            .is_err()
    );
    assert!(
        engine
            .repository_service()
            .get_deployment_resources(&deployment.id)
            .is_err()
    );
}

#[test]
fn rejects_duplicate_app_keys_and_rolls_back_failed_deployment() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("employee-onboarding", "Employee Onboarding", 1, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    let error = engine
        .deploy(
            AppDeploymentRequest::new("broken").with_resource(
                "dup.app",
                AppModel::new()
                    .with_app_definition(AppDefinition::new("a", "same-key", "A").with_page(
                        AppPage::new("p", "P").with_reference(
                            AppReference::process("r").with_definition_key("employee-onboarding"),
                        ),
                    ))
                    .with_app_definition(AppDefinition::new("b", "same-key", "B").with_page(
                        AppPage::new("p", "P").with_reference(
                            AppReference::process("r").with_definition_key("employee-onboarding"),
                        ),
                    )),
            ),
        )
        .expect_err("duplicate keys must fail");
    assert!(
        error.to_string().contains("Duplicate app definition key"),
        "unexpected error: {error}"
    );
    assert!(
        engine
            .repository_service()
            .create_deployment_query()
            .list()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn rejects_model_and_resource_bytes_mismatch() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("employee-onboarding", "Employee Onboarding", 1, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    let model = AppModel::new().with_app_definition(
        AppDefinition::new("app-employee", "employee-portal", "Employee Portal").with_page(
            AppPage::new("page-process", "Process Dashboard").with_reference(
                AppReference::process("start-onboarding")
                    .with_definition_key("employee-onboarding"),
            ),
        ),
    );
    let mismatched = AppModel::new().with_app_definition(
        AppDefinition::new("app-other", "other-portal", "Other Portal").with_page(
            AppPage::new("page-process", "Process Dashboard").with_reference(
                AppReference::process("start-onboarding")
                    .with_definition_key("employee-onboarding"),
            ),
        ),
    );
    let bytes = serialize_engine_model_as_durable_bytes(&mismatched).unwrap();

    let error = engine
        .deploy(
            AppDeploymentRequest::new("employee-apps").with_resource_bytes(
                "employee-app.json",
                model,
                bytes,
            ),
        )
        .expect_err("mismatch must fail");
    assert!(
        error
            .to_string()
            .contains("model does not match supplied resource bytes"),
        "unexpected error: {error}"
    );
}

#[test]
fn durable_resource_bytes_round_trip_through_canonical_converter() {
    let model = AppModel::new().with_app_definition(
        AppDefinition::new("app-employee", "employee-portal", "Employee Portal")
            .with_theme("flowable")
            .with_page(
                AppPage::new("page-process", "Process Dashboard").with_reference(
                    AppReference::process("start-onboarding")
                        .with_name("Start")
                        .with_definition_key("employee-onboarding"),
                ),
            ),
    );
    let bytes = serialize_engine_model_as_durable_bytes(&model).unwrap();
    let parsed = parse_resource_bytes_to_engine_model(&bytes).unwrap();
    assert!(models_semantically_equal(&model, &parsed));
}

#[test]
fn strict_tenant_resolution_rejects_fallback_to_default_tenant_definitions() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("employee-onboarding", "Employee Onboarding", 1, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog))
        .unwrap()
        .with_tenant_resolution_policy(TenantResolutionPolicy::Strict);

    let error = engine
        .deploy(
            AppDeploymentRequest::new("tenant-app")
                .with_tenant_id("tenant-a")
                .with_resource(
                    "employee-app.json",
                    AppModel::new().with_app_definition(
                        AppDefinition::new("app-employee", "employee-portal", "Employee Portal")
                            .with_page(
                                AppPage::new("page-process", "Process Dashboard").with_reference(
                                    AppReference::process("start-onboarding")
                                        .with_definition_key("employee-onboarding"),
                                ),
                            ),
                    ),
                ),
        )
        .expect_err("strict policy must not fall back to no-tenant definitions");
    assert!(
        error
            .to_string()
            .contains("references missing BPMN definition key 'employee-onboarding'"),
        "unexpected error: {error}"
    );
}

#[test]
fn fallback_tenant_resolution_uses_default_tenant_definitions() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("employee-onboarding", "Employee Onboarding", 1, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    engine
        .deploy(
            AppDeploymentRequest::new("tenant-app")
                .with_tenant_id("tenant-a")
                .with_resource(
                    "employee-app.json",
                    AppModel::new().with_app_definition(
                        AppDefinition::new("app-employee", "employee-portal", "Employee Portal")
                            .with_page(
                                AppPage::new("page-process", "Process Dashboard").with_reference(
                                    AppReference::process("start-onboarding")
                                        .with_definition_key("employee-onboarding"),
                                ),
                            ),
                    ),
                ),
        )
        .unwrap();

    let composition = engine
        .runtime_service()
        .resolve_app_definition_by_key("employee-portal", Some("tenant-a"))
        .unwrap();
    assert_eq!(composition.references[0].resolved_definition_version, 1);
}
