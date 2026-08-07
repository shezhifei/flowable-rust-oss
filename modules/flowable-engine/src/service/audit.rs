use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TimerAdminAuditRecord {
    pub id: String,
    pub request_id: String,
    pub timestamp: i64,
    pub tenant_id: Option<String>,
    pub issuer: String,
    pub subject: String,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub outcome: String,
    #[serde(default)]
    pub profile_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TimerAdminAuditInput {
    pub request_id: String,
    pub tenant_id: Option<String>,
    pub issuer: String,
    pub subject: String,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub outcome: String,
    pub profile_id: Option<String>,
}
