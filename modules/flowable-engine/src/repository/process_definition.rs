use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessDefinition {
    pub id: String,
    pub category: Option<String>,
    pub name: Option<String>,
    pub key: String,
    pub description: Option<String>,
    pub version: i32,
    pub resource_name: Option<String>,
    pub deployment_id: Option<String>,
    pub diagram_resource_name: Option<String>,
    pub has_start_form_key: bool,
    pub has_graphical_notation: bool,
    pub is_suspended: bool,
    pub tenant_id: Option<String>,
    pub engine_version: Option<String>,
    pub app_version: Option<i32>,
    /// Per-definition history level key from BPMN `flowable:historyLevel`
    /// extension element (e.g. `"audit"`). `None` means use engine default.
    /// Java reads this from the process extension map at gate time
    /// (`DefaultHistoryConfigurationSettings.getProcessDefinitionHistoryLevel:59-89`);
    /// Rust materializes it at deploy for O(1) lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_level: Option<String>,
}
