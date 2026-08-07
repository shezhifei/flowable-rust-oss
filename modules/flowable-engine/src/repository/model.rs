use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryModel {
    pub id: String,
    pub name: Option<String>,
    pub key: String,
    pub category: Option<String>,
    pub version: i32,
    pub meta_info: Option<String>,
    pub deployment_id: Option<String>,
    pub resource_name: Option<String>,
    pub process_definition_id: Option<String>,
    pub tenant_id: Option<String>,
    pub create_time: i64,
    pub last_update_time: i64,
    pub source_content_type: String,
    pub source_extra_content_type: String,
}

#[derive(Debug, Clone)]
pub struct RepositoryModelBytes {
    pub content_type: String,
    pub bytes: Vec<u8>,
}
