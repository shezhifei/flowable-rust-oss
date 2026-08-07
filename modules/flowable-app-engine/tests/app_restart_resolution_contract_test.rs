use flowable_app_engine::{
    AppDefinition, AppDeploymentRequest, AppEngine, AppModel, AppPage, AppReference,
    InMemoryDefinitionCatalog,
};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_db_path(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir();
    dir.join(format!("flowable-app-restart-{label}-{nanos}.sqlite"))
        .to_string_lossy()
        .into_owned()
}

#[test]
fn restart_rehydrates_persisted_v1_composition_not_newer_catalog_versions() {
    let db_path = temp_db_path("v1-stable");
    let _ = std::fs::remove_file(&db_path);

    let v1_catalog: Arc<dyn flowable_app_engine::DefinitionCatalog> = Arc::new(
        InMemoryDefinitionCatalog::builder()
            .with_process_definition("employee-onboarding", "Employee Onboarding", 1, None)
            .with_decision_definition("benefits-eligibility", "Benefits Eligibility", 1, None)
            .with_case_definition("equipment-case", "Equipment Case", 1, None)
            .with_event_definition("employee-updated", "Employee Updated", 1, None)
            .build(),
    );

    let engine_v1 = AppEngine::new_sqlite_with_catalog(&db_path, Arc::clone(&v1_catalog)).unwrap();
    engine_v1
        .deploy(
            AppDeploymentRequest::new("employee-apps-v1").with_resource(
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

    let definition = engine_v1
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .single_result()
        .unwrap()
        .expect("definition");
    let original = engine_v1
        .runtime_service()
        .get_resolved_composition(&definition.id)
        .unwrap();
    assert_eq!(original.references.len(), 4);
    for reference in &original.references {
        assert_eq!(reference.resolved_definition_version, 1);
    }
    let original_ids: Vec<_> = original
        .references
        .iter()
        .map(|r| r.resolved_definition_id.clone())
        .collect();

    // Simulate a new process: only v2 definitions are available in the catalog.
    let v2_catalog = Arc::new(
        InMemoryDefinitionCatalog::builder()
            .with_process_definition("employee-onboarding", "Employee Onboarding", 2, None)
            .with_decision_definition("benefits-eligibility", "Benefits Eligibility", 2, None)
            .with_case_definition("equipment-case", "Equipment Case", 2, None)
            .with_event_definition("employee-updated", "Employee Updated", 2, None)
            .build(),
    );
    let engine_restart =
        AppEngine::new_sqlite_with_catalog(&db_path, v2_catalog).unwrap();

    // Cold cache: restart engine must rehydrate the durable snapshot.
    assert!(
        !engine_restart
            .deployment_manager()
            .is_cached(&definition.id)
            .unwrap()
    );

    let rehydrated = engine_restart
        .runtime_service()
        .get_resolved_composition(&definition.id)
        .unwrap();
    assert_eq!(rehydrated.references.len(), 4);
    let rehydrated_ids: Vec<_> = rehydrated
        .references
        .iter()
        .map(|r| r.resolved_definition_id.clone())
        .collect();
    assert_eq!(rehydrated_ids, original_ids);
    for reference in &rehydrated.references {
        assert_eq!(
            reference.resolved_definition_version, 1,
            "app v1 must keep persisted dependency versions after restart"
        );
        assert!(
            reference.resolved_definition_id.contains(":v1"),
            "expected v1 definition id, got {}",
            reference.resolved_definition_id
        );
    }

    // Deploying a new app version against v2 catalog must capture v2 ids.
    engine_restart
        .deploy(
            AppDeploymentRequest::new("employee-apps-v2").with_resource(
                "employee-app-v2.json",
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
    let latest = engine_restart
        .runtime_service()
        .resolve_app_definition_by_key("employee-portal", None)
        .unwrap();
    assert_eq!(latest.version, 2);
    assert_eq!(latest.references[0].resolved_definition_version, 2);

    // Original definition id still resolves to the immutable v1 snapshot.
    let still_v1 = engine_restart
        .runtime_service()
        .get_resolved_composition(&definition.id)
        .unwrap();
    assert_eq!(still_v1.version, 1);
    assert_eq!(still_v1.references[0].resolved_definition_version, 1);

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn eviction_rehydrates_from_store_without_catalog_reresolution() {
    let catalog = Arc::new(
        InMemoryDefinitionCatalog::builder()
            .with_process_definition("employee-onboarding", "Employee Onboarding", 1, None)
            .build(),
    );
    let engine = AppEngine::new_in_memory_with_catalog(catalog).unwrap();
    engine
        .deploy(
            AppDeploymentRequest::new("employee-apps").with_resource(
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

    let definition_id = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .single_result()
        .unwrap()
        .expect("definition")
        .id;
    let before = engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    let expected_id = before.composition.references[0]
        .resolved_definition_id
        .clone();

    engine
        .deployment_manager()
        .evict_app_definition(&definition_id);
    let after = engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert_eq!(
        after.composition.references[0].resolved_definition_id,
        expected_id
    );
    assert_eq!(after.composition.references[0].resolved_definition_version, 1);
}
