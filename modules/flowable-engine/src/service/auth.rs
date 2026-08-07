use crate::service::principal::{AuthProvider, Principal};
use std::collections::HashMap;

pub struct AuthConfig {
    provider_name: String,
    keys: HashMap<String, Principal>,
}

impl AuthConfig {
    pub fn new(provider_name: impl Into<String>) -> Self {
        Self {
            provider_name: provider_name.into(),
            keys: HashMap::new(),
        }
    }

    pub fn with_key(
        mut self,
        key: &str,
        actor_id: &str,
        subject: Option<&str>,
        issuer: Option<&str>,
        role: &str,
        tenant_id: Option<String>,
    ) -> Self {
        let principal = Principal::new(
            actor_id,
            subject.unwrap_or(actor_id),
            issuer.unwrap_or(self.provider_name.as_str()),
            tenant_id,
        )
        .with_role(role);
        self.keys.insert(key.to_string(), principal);
        self
    }
}

impl AuthProvider for AuthConfig {
    fn authenticate(&self, key: Option<&str>) -> Option<Principal> {
        let key = key?;
        self.keys.get(key).cloned()
    }
}

pub struct RejectAllAuthProvider;

impl AuthProvider for RejectAllAuthProvider {
    fn authenticate(&self, _token: Option<&str>) -> Option<Principal> {
        None
    }
}
