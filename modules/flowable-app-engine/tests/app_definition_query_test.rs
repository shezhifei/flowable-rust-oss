use flowable_app_engine::{
    AppDefinition, AppDeploymentRequest, AppEngine, AppModel, AppPage, AppReference,
    InMemoryDefinitionCatalog,
};
use std::sync::Arc;

#[test]
fn app_definition_query_filters_by_key_version_and_tenant() {
    let catalog = InMemoryDefinitionCatalog::builder()
        .with_process_definition("employee-onboarding", "Employee Onboarding", 3, None)
        .build();
    let engine = AppEngine::new_in_memory_with_catalog(Arc::new(catalog)).unwrap();

    engine
        .deploy(
            AppDeploymentRequest::new("employee-apps-v1")
                .with_tenant_id("tenant-a")
                .with_resource(
                    "employee-app-v1.json",
                    AppModel::new().with_app_definition(app_definition("tenant-a")),
                ),
        )
        .unwrap();

    engine
        .deploy(
            AppDeploymentRequest::new("employee-apps-v2")
                .with_tenant_id("tenant-a")
                .with_resource(
                    "employee-app-v2.json",
                    AppModel::new().with_app_definition(app_definition("tenant-a")),
                ),
        )
        .unwrap();

    engine
        .deploy(
            AppDeploymentRequest::new("employee-apps-tenant-b")
                .with_tenant_id("tenant-b")
                .with_resource(
                    "employee-app-tenant-b.json",
                    AppModel::new().with_app_definition(app_definition("tenant-b")),
                ),
        )
        .unwrap();

    let latest = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .tenant_id("tenant-a")
        .single_result()
        .unwrap()
        .expect("latest definition");

    assert_eq!(latest.version, 2);
    assert_eq!(latest.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(latest.resource_name, "employee-app-v2.json");

    let first_version = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .tenant_id("tenant-a")
        .version(1)
        .single_result()
        .unwrap()
        .expect("version 1");

    assert_eq!(first_version.version, 1);
    assert_eq!(first_version.resource_name, "employee-app-v1.json");

    let tenant_b = engine
        .repository_service()
        .create_app_definition_query()
        .key("employee-portal")
        .tenant_id("tenant-b")
        .single_result()
        .unwrap()
        .expect("tenant-b definition");

    assert_eq!(tenant_b.version, 1);
    assert_eq!(tenant_b.tenant_id.as_deref(), Some("tenant-b"));
}

fn app_definition(tenant: &str) -> AppDefinition {
    AppDefinition::new(
        format!("app-{tenant}"),
        "employee-portal",
        format!("Employee Portal {tenant}"),
    )
    .with_page(
        AppPage::new("page-process", "Process Dashboard").with_reference(
            AppReference::process("start-onboarding").with_definition_key("employee-onboarding"),
        ),
    )
}
