use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub id: String,
    pub name: Option<String>,
    pub deployment_time: Option<DateTime<Utc>>,
    pub category: Option<String>,
    pub key: Option<String>,
    pub tenant_id: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub derived_from: Option<String>,
    pub derived_from_root: Option<String>,
    pub engine_version: Option<String>,
    #[serde(default)]
    pub is_new: bool,
    #[serde(skip)]
    pub resources: HashMap<String, Vec<u8>>,
}
