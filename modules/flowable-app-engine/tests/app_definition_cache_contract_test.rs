use flowable_app_engine::{
    AppDefinition, AppDeploymentRequest, AppEngine, AppModel, AppPage, AppReference,
    InMemoryDefinitionCatalog,
};
use std::sync::Arc;

fn catalog() -> Arc<dyn flowable_app_engine::DefinitionCatalog> {
    Arc::new(
        InMemoryDefinitionCatalog::builder()
            .with_process_definition("employee-onboarding", "Employee Onboarding", 1, None)
            .build(),
    )
}

fn portal_request(name: &str, resource_name: &str, app_key: &str) -> AppDeploymentRequest {
    AppDeploymentRequest::new(name).with_resource(
        resource_name,
        AppModel::new().with_app_definition(
            AppDefinition::new(format!("app-{app_key}"), app_key, format!("Portal {app_key}"))
                .with_page(
                    AppPage::new("page-process", "Process Dashboard").with_reference(
                        AppReference::process("start-onboarding")
                            .with_definition_key("employee-onboarding"),
                    ),
                ),
        ),
    )
}

fn deploy_portal(engine: &AppEngine, app_key: &str) -> String {
    engine
        .deploy(portal_request(
            &format!("deploy-{app_key}"),
            &format!("{app_key}.json"),
            app_key,
        ))
        .unwrap();
    engine
        .repository_service()
        .create_app_definition_query()
        .key(app_key)
        .single_result()
        .unwrap()
        .expect("definition")
        .id
}

#[test]
fn cache_hit_returns_same_entry_without_reloading_from_store() {
    let engine = AppEngine::new_in_memory_with_catalog(catalog()).unwrap();
    let definition_id = deploy_portal(&engine, "employee-portal");

    let first = engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert!(
        engine
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );

    let second = engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert_eq!(first.definition.id, second.definition.id);
    assert_eq!(
        first.composition.references[0].resolved_definition_id,
        second.composition.references[0].resolved_definition_id
    );
    assert_eq!(engine.deployment_manager().cache_size(), 1);
}

#[test]
fn explicit_eviction_removes_entry_and_miss_rehydrates_persisted_composition() {
    let engine = AppEngine::new_in_memory_with_catalog(catalog()).unwrap();
    let definition_id = deploy_portal(&engine, "employee-portal");

    let original = engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    let original_version = original
        .composition
        .references[0]
        .resolved_definition_version;

    engine
        .deployment_manager()
        .evict_app_definition(&definition_id);
    assert!(
        !engine
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );
    assert_eq!(engine.deployment_manager().cache_size(), 0);

    let rehydrated = engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert!(
        engine
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );
    assert_eq!(
        rehydrated.composition.references[0].resolved_definition_version,
        original_version
    );
    assert_eq!(
        rehydrated.composition.references[0].resolved_definition_id,
        original.composition.references[0].resolved_definition_id
    );
    assert_eq!(rehydrated.definition.model.key, "employee-portal");
    assert_eq!(rehydrated.app_model.app_definitions.len(), 1);
}

#[test]
fn engine_local_cache_does_not_leak_across_engine_instances() {
    let catalog = catalog();
    let engine_a = AppEngine::new_in_memory_with_catalog(Arc::clone(&catalog)).unwrap();
    let engine_b = AppEngine::new_in_memory_with_catalog(catalog).unwrap();

    let definition_id = deploy_portal(&engine_a, "employee-portal");
    engine_a
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert!(
        engine_a
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );
    assert!(
        !engine_b
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );
    assert_eq!(engine_b.deployment_manager().cache_size(), 0);
}

#[test]
fn bounded_cache_evicts_least_recently_used_entries() {
    let engine = AppEngine::new_in_memory_with_catalog_and_cache_limit(catalog(), 2).unwrap();

    let id_a = deploy_portal(&engine, "portal-a");
    let id_b = deploy_portal(&engine, "portal-b");
    let id_c = deploy_portal(&engine, "portal-c");

    engine
        .deployment_manager()
        .resolve_app_definition(&id_a)
        .unwrap();
    engine
        .deployment_manager()
        .resolve_app_definition(&id_b)
        .unwrap();
    // Touch A so B becomes the least-recently used when C arrives.
    engine
        .deployment_manager()
        .resolve_app_definition(&id_a)
        .unwrap();
    engine
        .deployment_manager()
        .resolve_app_definition(&id_c)
        .unwrap();

    assert_eq!(engine.deployment_manager().cache_size(), 2);
    assert!(engine.deployment_manager().is_cached(&id_a).unwrap());
    assert!(!engine.deployment_manager().is_cached(&id_b).unwrap());
    assert!(engine.deployment_manager().is_cached(&id_c).unwrap());
}

#[test]
fn delete_deployment_invalidates_cached_definitions_after_commit() {
    let engine = AppEngine::new_in_memory_with_catalog(catalog()).unwrap();
    let deployment = engine
        .deploy(portal_request(
            "employee-apps",
            "employee-app.json",
            "employee-portal",
        ))
        .unwrap();
    let definition_id = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .single_result()
        .unwrap()
        .expect("definition")
        .id;

    engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert!(
        engine
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );

    engine
        .repository_service()
        .delete_deployment(&deployment.id)
        .unwrap();

    assert!(
        !engine
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );
    assert!(
        engine
            .deployment_manager()
            .resolve_app_definition(&definition_id)
            .is_err()
    );
}

#[test]
fn category_mutation_invalidates_cache_after_successful_update() {
    let engine = AppEngine::new_in_memory_with_catalog(catalog()).unwrap();
    let definition_id = deploy_portal(&engine, "employee-portal");

    engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert!(
        engine
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );

    engine
        .repository_service()
        .set_app_definition_category(&definition_id, Some("HR"))
        .unwrap();

    assert!(
        !engine
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );

    let entry = engine
        .deployment_manager()
        .resolve_app_definition(&definition_id)
        .unwrap();
    assert_eq!(entry.definition.category.as_deref(), Some("HR"));
    assert_eq!(entry.definition.model.category.as_deref(), Some("HR"));
}

#[test]
fn runtime_composition_reads_go_through_deployment_manager_cache() {
    let engine = AppEngine::new_in_memory_with_catalog(catalog()).unwrap();
    let definition_id = deploy_portal(&engine, "employee-portal");

    let composition = engine
        .runtime_service()
        .get_resolved_composition(&definition_id)
        .unwrap();
    assert_eq!(composition.app_definition_key, "employee-portal");
    assert!(
        engine
            .deployment_manager()
            .is_cached(&definition_id)
            .unwrap()
    );

    let by_key = engine
        .runtime_service()
        .resolve_app_definition_by_key("employee-portal", None)
        .unwrap();
    assert_eq!(by_key.app_definition_id, definition_id);
    assert_eq!(engine.deployment_manager().cache_size(), 1);
}
