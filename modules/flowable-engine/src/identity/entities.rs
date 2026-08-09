use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    /// Password digest: argon2id PHC string (`$argon2id$…`) when the user was
    /// saved through the engine since the hashing change; legacy plaintext
    /// values from before the change are verifiable but never written back as
    /// plaintext. Never echo this field to clients.
    pub password: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: String,
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPicture {
    pub user_id: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
    #[serde(default)]
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub group_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membership {
    pub user_id: String,
    pub group_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Privilege {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivilegeMapping {
    pub id: String,
    pub privilege_id: String,
    pub user_id: Option<String>,
    pub group_id: Option<String>,
}

/// Persistent authentication token. `id` doubles as the remember-me *series*
/// identifier: the UI cookie carries `series:tokenValue`, the series locates the
/// row and the value is then compared to detect cookie theft.
///
/// `token_date`/`ip_address`/`user_agent` mirror Java's `ACT_ID_TOKEN` columns
/// and are what the remember-me rolling/expiry logic reads. They are
/// `#[serde(default)]` because the store keeps entities as JSON documents: rows
/// written before these fields existed still deserialize, with `token_date`
/// absent meaning "unknown age" (treated as expired by the UI auth layer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub id: String,
    pub token_value: String,
    pub user_id: Option<String>,
    /// Issue time in epoch milliseconds.
    #[serde(default)]
    pub token_date: Option<i64>,
    #[serde(default)]
    pub ip_address: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityLink {
    pub id: String,
    pub link_type: String,
    pub user_id: Option<String>,
    pub group_id: Option<String>,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub process_definition_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityLink {
    pub id: String,
    pub link_type: String,
    pub scope_id: Option<String>,
    pub scope_type: Option<String>,
    pub reference_scope_id: Option<String>,
    pub reference_scope_type: Option<String>,
    pub hierarchy_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchEntity {
    pub id: String,
    pub batch_type: String,
    #[serde(default)]
    pub search_key: Option<String>,
    #[serde(default)]
    pub search_key2: Option<String>,
    pub status: String,
    pub total_items: i64,
    pub items_processed: i64,
    pub create_time: u64,
    pub end_time: Option<u64>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub batch_document_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPartEntity {
    pub id: String,
    pub batch_id: String,
    pub batch_type: String,
    #[serde(default)]
    pub search_key: Option<String>,
    #[serde(default)]
    pub search_key2: Option<String>,
    #[serde(default)]
    pub scope_id: Option<String>,
    #[serde(default)]
    pub sub_scope_id: Option<String>,
    #[serde(default)]
    pub scope_type: Option<String>,
    pub create_time: u64,
    #[serde(default)]
    pub complete_time: Option<u64>,
    pub status: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub batch_part_document_json: Option<String>,
}
