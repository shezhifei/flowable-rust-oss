use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateContentItemRequest {
    pub name: String,
    pub mime_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attachment_type: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    pub content: Option<String>,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub created_by: Option<String>,
    // M42: Optional TTL in seconds
    #[serde(default)]
    pub expires_in_seconds: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentItem {
    pub id: String,
    pub name: String,
    pub mime_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attachment_type: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    pub content: Option<String>,
    pub content_size: usize,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    /// Form field id that owns this content (Java ContentItem.field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Tenant ownership for multi-tenant association checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    pub created_by: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// M41: 关联的存储对象 ID（新格式），旧格式为 None
    #[serde(default)]
    pub storage_id: Option<String>,
    /// M41: 使用的存储后端名称（新格式），旧格式为 None
    #[serde(default)]
    pub storage_backend: Option<String>,
    // M42: Content versioning support
    #[serde(default)]
    pub version: Option<i32>,
    // M42: TTL expiration timestamp (UNIX timestamp in milliseconds)
    #[serde(default)]
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentItemData {
    pub content_item_id: String,
    pub mime_type: Option<String>,
    pub content: Vec<u8>,
    pub content_size: usize,
}

/// M41: 待存储的内容对象
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentObject {
    /// 唯一标识
    pub id: String,
    /// 关联的 ContentItem ID
    pub content_item_id: String,
    /// 内容字节
    pub data: Vec<u8>,
    /// MIME 类型
    pub mime_type: String,
    /// 原始文件名
    pub file_name: Option<String>,
    /// 内容大小（字节）
    pub size: u64,
}

/// M41: 存储结果元数据
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentObjectStorageMetadata {
    /// 存储后端中的唯一标识
    pub storage_id: String,
    /// 后端名称（如 "local-fs"）
    pub storage_backend: String,
    /// ISO 8601 时间戳
    pub stored_at: String,
    /// 内容大小（字节）
    pub size: u64,
    /// SHA-256 hex 校验和
    pub checksum: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PagedResult<T> {
    pub start: usize,
    pub size: usize,
    pub total: usize,
    pub data: Vec<T>,
}
