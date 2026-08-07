use flowable_engine::service::policy::{
    AuthorizationRequest, PolicyEngine, ResourceAction, ResourceType, TenantAwarePolicyEngine,
};
use flowable_engine::service::principal::Principal;

#[test]
fn test_tenant_aware_policy_engine_enforces_tenant_and_resource_rules() {
    let global_admin =
        Principal::new("global-admin", "global-admin", "local-static", None).with_role("admin");
    let tenant_admin = Principal::new(
        "tenant-admin",
        "tenant-admin",
        "local-static",
        Some("tenant-a".to_string()),
    )
    .with_role("admin");
    let tenant_reader = Principal::new(
        "tenant-reader",
        "tenant-reader",
        "local-static",
        Some("tenant-a".to_string()),
    )
    .with_role("read");

    let engine = TenantAwarePolicyEngine::new();

    assert!(engine.authorize(AuthorizationRequest {
        principal: &global_admin,
        action: ResourceAction::AdminDestructive,
        resource: ResourceType::ClusterNodes,
        tenant_id: None,
    }));

    assert!(engine.authorize(AuthorizationRequest {
        principal: &tenant_admin,
        action: ResourceAction::AdminDestructive,
        resource: ResourceType::TimerNode,
        tenant_id: Some("tenant-a"),
    }));

    assert!(!engine.authorize(AuthorizationRequest {
        principal: &tenant_admin,
        action: ResourceAction::AdminDestructive,
        resource: ResourceType::ClusterNodes,
        tenant_id: Some("tenant-a"),
    }));

    assert!(!engine.authorize(AuthorizationRequest {
        principal: &tenant_reader,
        action: ResourceAction::Read,
        resource: ResourceType::TimerNode,
        tenant_id: Some("tenant-b"),
    }));

    assert!(engine.authorize(AuthorizationRequest {
        principal: &tenant_reader,
        action: ResourceAction::Read,
        resource: ResourceType::TimerCoordinator,
        tenant_id: Some("tenant-a"),
    }));

    assert!(!engine.authorize(AuthorizationRequest {
        principal: &tenant_reader,
        action: ResourceAction::AdminDestructive,
        resource: ResourceType::TimerCoordinator,
        tenant_id: Some("tenant-a"),
    }));
}
