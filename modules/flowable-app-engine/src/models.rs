use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DefinitionType {
    BpmnProcess,
    DmnDecision,
    CmmnCase,
    EventRegistry,
}

impl DefinitionType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::BpmnProcess => "BPMN",
            Self::DmnDecision => "DMN",
            Self::CmmnCase => "CMMN",
            Self::EventRegistry => "Event Registry",
        }
    }

    pub(crate) fn slug(&self) -> &'static str {
        match self {
            Self::BpmnProcess => "bpmn-process",
            Self::DmnDecision => "dmn-decision",
            Self::CmmnCase => "cmmn-case",
            Self::EventRegistry => "event-registry",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppReference {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub definition_type: DefinitionType,
    pub definition_key: String,
    /// Canonical `definitionId`: pins an exact definition instead of
    /// latest-by-key. Carried verbatim from canonical bytes, never dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_id: Option<String>,
    /// Canonical `tenantId` of the referenced definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

impl AppReference {
    pub fn new(id: impl Into<String>, definition_type: DefinitionType) -> Self {
        Self {
            id: id.into(),
            name: None,
            description: None,
            definition_type,
            definition_key: String::new(),
            definition_id: None,
            tenant_id: None,
        }
    }

    pub fn process(id: impl Into<String>) -> Self {
        Self::new(id, DefinitionType::BpmnProcess)
    }

    pub fn decision(id: impl Into<String>) -> Self {
        Self::new(id, DefinitionType::DmnDecision)
    }

    pub fn case(id: impl Into<String>) -> Self {
        Self::new(id, DefinitionType::CmmnCase)
    }

    pub fn event(id: impl Into<String>) -> Self {
        Self::new(id, DefinitionType::EventRegistry)
    }

    pub fn with_definition_key(mut self, definition_key: impl Into<String>) -> Self {
        self.definition_key = definition_key.into();
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_definition_id(mut self, definition_id: impl Into<String>) -> Self {
        self.definition_id = Some(definition_id.into());
        self
    }

    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppPage {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub order: Option<i32>,
    pub references: Vec<AppReference>,
}

impl AppPage {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            icon: None,
            order: None,
            references: Vec::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_reference(mut self, reference: AppReference) -> Self {
        self.references.push(reference);
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_order(mut self, order: i32) -> Self {
        self.order = Some(order);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppDefinition {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub theme: Option<String>,
    pub icon: Option<String>,
    pub users_access: Option<String>,
    pub groups_access: Option<String>,
    pub landing_page: Option<String>,
    pub pages: Vec<AppPage>,
}

impl AppDefinition {
    pub fn new(id: impl Into<String>, key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            key: key.into(),
            name: name.into(),
            description: None,
            category: None,
            theme: None,
            icon: None,
            users_access: None,
            groups_access: None,
            landing_page: None,
            pages: Vec::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_page(mut self, page: AppPage) -> Self {
        self.pages.push(page);
        self
    }

    pub fn with_theme(mut self, theme: impl Into<String>) -> Self {
        self.theme = Some(theme.into());
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_users_access(mut self, users_access: impl Into<String>) -> Self {
        self.users_access = Some(users_access.into());
        self
    }

    pub fn with_groups_access(mut self, groups_access: impl Into<String>) -> Self {
        self.groups_access = Some(groups_access.into());
        self
    }

    pub fn with_landing_page(mut self, landing_page: impl Into<String>) -> Self {
        self.landing_page = Some(landing_page.into());
        self
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AppModel {
    pub app_definitions: Vec<AppDefinition>,
}

impl AppModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_app_definition(mut self, app_definition: AppDefinition) -> Self {
        self.app_definitions.push(app_definition);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppDeploymentResource {
    pub resource_name: String,
    pub model: AppModel,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppDeploymentRequest {
    pub name: String,
    pub category: Option<String>,
    pub tenant_id: Option<String>,
    pub resources: Vec<AppDeploymentResource>,
    #[serde(default)]
    pub resource_bytes: BTreeMap<String, Vec<u8>>,
}

impl AppDeploymentRequest {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            category: None,
            tenant_id: None,
            resources: Vec::new(),
            resource_bytes: BTreeMap::new(),
        }
    }

    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_resource(mut self, resource_name: impl Into<String>, model: AppModel) -> Self {
        let resource_name = resource_name.into();
        // Prefer canonical app-model bytes for durable storage; fall back to the
        // engine JSON shape when conversion is not possible (e.g. empty model).
        let bytes = crate::convert::serialize_engine_model_as_durable_bytes(&model)
            .unwrap_or_else(|_| serde_json::to_vec(&model).unwrap_or_default());
        self.resource_bytes.insert(resource_name.clone(), bytes);
        self.resources.push(AppDeploymentResource {
            resource_name,
            model,
        });
        self
    }

    pub fn with_resource_bytes(
        mut self,
        resource_name: impl Into<String>,
        model: AppModel,
        bytes: Vec<u8>,
    ) -> Self {
        let resource_name = resource_name.into();
        self.resource_bytes.insert(resource_name.clone(), bytes);
        self.resources.push(AppDeploymentResource {
            resource_name,
            model,
        });
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppDeployment {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub tenant_id: Option<String>,
    pub resource_names: Vec<String>,
    pub deployed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppDefinitionRecord {
    pub id: String,
    pub app_id: String,
    pub deployment_id: String,
    pub key: String,
    pub name: String,
    pub category: Option<String>,
    pub version: i32,
    pub tenant_id: Option<String>,
    pub resource_name: String,
    pub model: AppDefinition,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppDeploymentResourceData {
    pub deployment_id: String,
    pub resource_name: String,
    pub resource_type: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub created_at: i64,
}

impl AppDeploymentResourceData {
    pub fn new(
        deployment_id: String,
        resource_name: String,
        bytes: Vec<u8>,
        created_at: i64,
    ) -> Self {
        Self {
            deployment_id,
            resource_type: resource_type_for_name(&resource_name).to_string(),
            content_type: content_type_for_name(&resource_name).to_string(),
            resource_name,
            bytes,
            created_at,
        }
    }
}

fn resource_type_for_name(_resource_name: &str) -> &'static str {
    "resource"
}

fn content_type_for_name(resource_name: &str) -> &'static str {
    let lower_name = resource_name.to_ascii_lowercase();
    if lower_name.ends_with(".json") || lower_name.ends_with(".app") {
        "application/json"
    } else if lower_name.ends_with(".xml") {
        "application/xml"
    } else if lower_name.ends_with(".svg") {
        "image/svg+xml"
    } else if lower_name.ends_with(".png") {
        "image/png"
    } else if lower_name.ends_with(".jpg") || lower_name.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower_name.ends_with(".gif") {
        "image/gif"
    } else if lower_name.ends_with(".txt") {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedDefinition {
    pub definition_type: DefinitionType,
    pub definition_id: String,
    pub definition_key: String,
    pub definition_name: String,
    pub deployment_id: String,
    pub version: i32,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResolvedAppReference {
    pub page_id: String,
    pub page_name: String,
    pub reference_id: String,
    pub reference_name: Option<String>,
    pub definition_type: DefinitionType,
    pub requested_definition_key: String,
    pub resolved_definition_id: String,
    pub resolved_definition_key: String,
    pub resolved_definition_name: String,
    pub resolved_definition_version: i32,
    pub resolved_deployment_id: String,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResolvedAppComposition {
    pub id: String,
    pub app_definition_id: String,
    pub app_definition_key: String,
    pub app_definition_name: String,
    pub deployment_id: String,
    pub version: i32,
    pub tenant_id: Option<String>,
    pub references: Vec<ResolvedAppReference>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PagedResult<T> {
    pub start: usize,
    pub size: usize,
    pub total: usize,
    pub data: Vec<T>,
}
