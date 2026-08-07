use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInstance {
    pub id: String,
    pub name: Option<String>,
    pub process_definition_id: String,
    pub process_definition_key: String,
    pub process_definition_name: Option<String>,
    pub process_definition_version: i32,
    pub business_key: Option<String>,
    pub business_status: Option<String>,
    pub is_suspended: bool,
    pub tenant_id: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
    pub start_user_id: Option<String>,
    pub callback_id: Option<String>,
    pub callback_type: Option<String>,
    pub reference_id: Option<String>,
    pub reference_type: Option<String>,
    pub is_ended: bool,
    pub super_execution_id: Option<String>,
    pub root_process_instance_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProcessInstanceUpdate {
    pub name: Option<Option<String>>,
    pub business_key: Option<Option<String>>,
    pub business_status: Option<Option<String>>,
    pub callback_id: Option<Option<String>>,
    pub callback_type: Option<Option<String>>,
    pub reference_id: Option<Option<String>>,
    pub reference_type: Option<Option<String>>,
}

impl ProcessInstanceUpdate {
    pub fn has_updates(&self) -> bool {
        self.name.is_some()
            || self.business_key.is_some()
            || self.business_status.is_some()
            || self.callback_id.is_some()
            || self.callback_type.is_some()
            || self.reference_id.is_some()
            || self.reference_type.is_some()
    }
}
