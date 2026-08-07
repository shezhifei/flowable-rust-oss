use crate::service::principal::Principal;

#[derive(Clone, Debug)]
pub enum ResourceAction {
    Read,
    AdminDestructive,
    IdentityAdmin,
}

#[derive(Clone, Debug)]
pub enum ResourceType {
    TimerCoordinator,
    TimerNode,
    ClusterNodes,
    IssuerHealth,
    RevocationAdmin,
    IssuerAdmin,
}

pub struct AuthorizationRequest<'a> {
    pub principal: &'a Principal,
    pub action: ResourceAction,
    pub resource: ResourceType,
    pub tenant_id: Option<&'a str>,
}

pub trait PolicyEngine: Send + Sync {
    fn authorize(&self, request: AuthorizationRequest) -> bool;
}

pub struct TenantAwarePolicyEngine {
    // For this tranche, we keep a simple rule:
    // If the principal has the "admin" role, they can do AdminDestructive or Read on anything.
    // If they have "read", they can only do Read.
    // In future this will evaluate tenant_id and more granular rules.
}

impl Default for TenantAwarePolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TenantAwarePolicyEngine {
    pub fn new() -> Self {
        Self {}
    }
}

impl PolicyEngine for TenantAwarePolicyEngine {
    fn authorize(&self, request: AuthorizationRequest) -> bool {
        let is_admin = request.principal.has_role("admin");
        let is_read = request.principal.has_role("read") || is_admin;

        let role_authorized = match request.action {
            ResourceAction::AdminDestructive => is_admin,
            ResourceAction::Read => is_read,
            ResourceAction::IdentityAdmin => is_admin,
        };

        if !role_authorized {
            return false;
        }

        let resource_authorized = match request.resource {
            ResourceType::TimerNode => true,
            ResourceType::TimerCoordinator => true,
            ResourceType::ClusterNodes => is_admin && request.principal.tenant_id.is_none(),
            ResourceType::IssuerHealth => is_admin,
            ResourceType::RevocationAdmin => is_admin,
            ResourceType::IssuerAdmin => is_admin,
        };

        if !resource_authorized {
            return false;
        }

        let Some(principal_tenant) = request.principal.tenant_id.as_deref() else {
            return true;
        };

        if let Some(request_tenant) = request.tenant_id
            && request_tenant != principal_tenant
        {
            return false;
        }

        true
    }
}
