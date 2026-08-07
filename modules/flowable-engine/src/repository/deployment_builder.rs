use crate::repository::deployment::Deployment;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Clone)]
pub struct DeploymentBuilder {
    name: Option<String>,
    category: Option<String>,
    key: Option<String>,
    tenant_id: Option<String>,
    parent_deployment_id: Option<String>,
    enable_duplicate_filtering: bool,
    activate_process_definitions_on: Option<DateTime<Utc>>,
    properties: HashMap<String, String>,
    pub resources: HashMap<String, Vec<u8>>,
}

impl Default for DeploymentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DeploymentBuilder {
    pub fn new() -> Self {
        Self {
            name: None,
            category: None,
            key: None,
            tenant_id: None,
            parent_deployment_id: None,
            enable_duplicate_filtering: false,
            activate_process_definitions_on: None,
            properties: HashMap::new(),
            resources: HashMap::new(),
        }
    }

    pub fn add_string(mut self, resource_name: String, text: String) -> Self {
        self.resources.insert(resource_name, text.into_bytes());
        self
    }

    pub fn add_bytes(mut self, resource_name: String, bytes: Vec<u8>) -> Self {
        self.resources.insert(resource_name, bytes);
        self
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn category(mut self, category: String) -> Self {
        self.category = Some(category);
        self
    }

    pub fn key(mut self, key: String) -> Self {
        self.key = Some(key);
        self
    }

    pub fn tenant_id(mut self, tenant_id: String) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    pub fn parent_deployment_id(mut self, id: String) -> Self {
        self.parent_deployment_id = Some(id);
        self
    }

    pub fn enable_duplicate_filtering(mut self) -> Self {
        self.enable_duplicate_filtering = true;
        self
    }

    pub(crate) fn duplicate_filtering_enabled(&self) -> bool {
        self.enable_duplicate_filtering
    }

    pub fn activate_process_definitions_on(mut self, date: DateTime<Utc>) -> Self {
        self.activate_process_definitions_on = Some(date);
        self
    }

    pub fn deployment_property(mut self, key: String, value: String) -> Self {
        self.properties.insert(key, value);
        self
    }

    pub fn deploy(self) -> Deployment {
        Deployment {
            id: uuid::Uuid::new_v4().to_string(),
            name: self.name,
            deployment_time: Some(Utc::now()),
            category: self.category,
            key: self.key,
            tenant_id: self.tenant_id,
            parent_deployment_id: self.parent_deployment_id,
            derived_from: None,
            derived_from_root: None,
            engine_version: None,
            is_new: true,
            resources: self.resources,
        }
    }
}
