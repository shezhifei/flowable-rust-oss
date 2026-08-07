use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub actor_id: String,
    pub subject: String,
    pub issuer: String,
    pub tenant_id: Option<String>,
    pub roles: HashSet<String>,
    pub profile_id: Option<String>,
}

impl Principal {
    pub fn new(actor_id: &str, subject: &str, issuer: &str, tenant_id: Option<String>) -> Self {
        Self {
            actor_id: actor_id.to_string(),
            subject: subject.to_string(),
            issuer: issuer.to_string(),
            tenant_id,
            roles: HashSet::new(),
            profile_id: None,
        }
    }

    pub fn with_role(mut self, role: &str) -> Self {
        self.roles.insert(role.to_string());
        self
    }

    pub fn with_profile_id(mut self, profile_id: &str) -> Self {
        self.profile_id = Some(profile_id.to_string());
        self
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.contains(role)
    }
}

pub trait AuthProvider: Send + Sync {
    fn authenticate(&self, token: Option<&str>) -> Option<Principal>;
}
