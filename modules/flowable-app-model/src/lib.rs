use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppPageType {
    Process,
    Decision,
    Case,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppReferenceType {
    Bpmn,
    Dmn,
    Cmmn,
    EventRegistry,
}

impl From<AppPageType> for AppReferenceType {
    fn from(value: AppPageType) -> Self {
        match value {
            AppPageType::Process => Self::Bpmn,
            AppPageType::Decision => Self::Dmn,
            AppPageType::Case => Self::Cmmn,
            AppPageType::Event => Self::EventRegistry,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppPage {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub page_type: AppPageType,
    pub definition_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppResourceReference {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub reference_type: AppReferenceType,
    pub definition_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users_access: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups_access: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landing_page: Option<String>,
    #[serde(default)]
    pub pages: Vec<AppPage>,
    #[serde(default)]
    pub references: Vec<AppResourceReference>,
}
