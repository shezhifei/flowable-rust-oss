use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricProcessInstance {
    pub id: String,
    pub process_definition_id: String,
    pub business_key: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub start_user_id: Option<String>,
    pub delete_reason: Option<String>,
}

impl HistoricProcessInstance {
    pub fn id(&self) -> &String {
        &self.id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricActivityInstance {
    pub id: String,
    pub activity_id: String,
    pub activity_name: Option<String>,
    pub activity_type: String,
    pub process_instance_id: String,
    pub execution_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub assignee: Option<String>,
    /// Java `HistoricActivityInstance.getDeleteReason()`. Absent when the
    /// activity completed normally. Legacy JSON rows omit the field.
    #[serde(default)]
    pub delete_reason: Option<String>,
}

impl HistoricActivityInstance {
    pub fn activity_id(&self) -> &String {
        &self.activity_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricTaskInstance {
    pub id: String,
    pub process_instance_id: String,
    #[serde(default)]
    pub process_definition_id: Option<String>,
    pub execution_id: String,
    #[serde(default)]
    pub task_definition_key: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub assignee: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub claim_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub form_key: Option<String>,
    #[serde(default)]
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub due_date: Option<DateTime<Utc>>,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub delete_reason: Option<String>,
}

impl HistoricTaskInstance {
    /// Copies the mutable task fields mirrored by Flowable's historic task row.
    pub fn update_from_runtime_task(&mut self, task: &crate::task::Task) {
        HistoricTaskUpdate::from_runtime_task(task).apply_to(self);
    }
}

/// Serializable task-info projection used by async history updates. Keeping
/// this separate from the runtime Task avoids persisting task-local variables
/// and unrelated runtime state inside history jobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricTaskUpdate {
    pub id: String,
    pub task_definition_key: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub assignee: Option<String>,
    pub owner: Option<String>,
    pub claim_time: Option<DateTime<Utc>>,
    pub tenant_id: Option<String>,
    pub category: Option<String>,
    pub form_key: Option<String>,
    pub parent_task_id: Option<String>,
    pub priority: Option<i32>,
    pub due_date: Option<DateTime<Utc>>,
}

impl HistoricTaskUpdate {
    pub fn from_runtime_task(task: &crate::task::Task) -> Self {
        Self {
            id: task.id.clone(),
            task_definition_key: (!task.task_definition_key.is_empty())
                .then(|| task.task_definition_key.clone()),
            name: task.name.clone(),
            description: task.description.clone(),
            assignee: task.assignee.clone(),
            owner: task.owner.clone(),
            claim_time: task.claim_time,
            tenant_id: task.tenant_id.clone(),
            category: task.category.clone(),
            form_key: task.form_key.clone(),
            parent_task_id: task.parent_task_id.clone(),
            priority: task.priority,
            due_date: task.due_date,
        }
    }

    pub fn apply_to(&self, instance: &mut HistoricTaskInstance) {
        instance.task_definition_key = self.task_definition_key.clone();
        instance.name = Some(self.name.clone());
        instance.description = self.description.clone();
        instance.assignee = self.assignee.clone();
        instance.owner = self.owner.clone();
        instance.claim_time = self.claim_time;
        instance.tenant_id = self.tenant_id.clone();
        instance.category = self.category.clone();
        instance.form_key = self.form_key.clone();
        instance.parent_task_id = self.parent_task_id.clone();
        instance.priority = self.priority;
        instance.due_date = self.due_date;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricVariableInstance {
    pub id: String,
    pub process_instance_id: String,
    pub execution_id: Option<String>,
    pub task_id: Option<String>,
    pub name: String,
    pub variable_type: String,
    pub value: serde_json::Value,
    pub create_time: DateTime<Utc>,
    pub last_updated_time: DateTime<Utc>,
}

impl HistoricVariableInstance {
    pub fn variable_name(&self) -> &String {
        &self.name
    }

    pub fn value(&self) -> &serde_json::Value {
        &self.value
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricDetail {
    pub id: String,
    pub process_instance_id: String,
    pub execution_id: Option<String>,
    pub activity_instance_id: Option<String>,
    pub task_id: Option<String>,
    pub time: DateTime<Utc>,
    pub detail_type: String,
    pub revision: Option<i32>,
    pub variable_name: Option<String>,
    pub variable_type: Option<String>,
    pub value: Option<serde_json::Value>,
    pub property_id: Option<String>,
    pub property_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricAuditLog {
    pub id: String,
    pub event_type: String, // "deploy", "start", "complete", "cancel"
    pub process_instance_id: Option<String>,
    pub process_definition_id: Option<String>,
    pub details: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricComment {
    pub id: String,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub message: String,
    pub author: Option<String>,
    pub time: DateTime<Utc>,
    /// Java `CommentEntity.action`: `null` for user comments, and an event
    /// marker (e.g. `AddUserLink` / `DeleteUserLink`) for TYPE_EVENT comments
    /// such as process-instance identity-link changes. Kept optional and
    /// `serde(default)` so existing user-comment rows deserialize unchanged.
    #[serde(default)]
    pub action: Option<String>,
    /// Java `CommentEntity.type` (`TYPE_COMMENT` = `"comment"`,
    /// `TYPE_EVENT` = `"event"`, or a custom type). Optional with
    /// `serde(default)` so pre-P65 rows deserialize; use [`Self::resolved_type`]
    /// for the effective type. Historic task events stay in
    /// [`HistoricTaskEvent`] and are not migrated into comments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_type: Option<String>,
}

impl HistoricComment {
    /// Java default comment type for user-authored comments.
    pub const TYPE_COMMENT: &'static str = "comment";
    /// Java type for event-style comments (e.g. process identity-link audit).
    /// Distinct from [`HistoricTaskEvent`], which remains a separate table.
    pub const TYPE_EVENT: &'static str = "event";

    /// Effective comment type with backward-compatible resolution:
    /// - explicit `comment_type` wins;
    /// - legacy rows with an `action` but no type resolve as `"event"`;
    /// - legacy user comments (no type, no action) resolve as `"comment"`.
    pub fn resolved_type(&self) -> &str {
        if let Some(ref comment_type) = self.comment_type {
            return comment_type.as_str();
        }
        if self.action.is_some() {
            Self::TYPE_EVENT
        } else {
            Self::TYPE_COMMENT
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricTaskEvent {
    pub id: String,
    pub task_id: String,
    pub action: String,
    pub message: Vec<String>,
    pub user_id: Option<String>,
    pub time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricTaskLogEntry {
    pub id: String,
    pub log_number: i64,
    pub log_type: String,
    pub task_id: String,
    pub timestamp: DateTime<Utc>,
    pub user_id: Option<String>,
    pub data: Option<String>,
    pub execution_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub process_definition_id: Option<String>,
    pub scope_id: Option<String>,
    pub scope_definition_id: Option<String>,
    pub sub_scope_id: Option<String>,
    pub scope_type: Option<String>,
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupLog {
    pub id: String,
    pub cleanup_type: String,
    pub before_date: Option<DateTime<Utc>>,
    pub records_deleted: usize,
    pub duration_ms: u64,
    pub status: String,
    pub error_message: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupStrategyConfig {
    pub retention_days: Option<u32>,
    pub max_records: Option<usize>,
    pub auto_cleanup: bool,
    pub cleanup_schedule: Option<String>,
}

/// Historic identity-link row (`ACT_HI_IDENTITYLINK` / `historic_identity_links`).
///
/// Java: `HistoricIdentityLinkEntityImpl` + create SQL
/// `flowable.postgres.all.create.sql:95-108` (ID_/TYPE_/USER_ID_/GROUP_ID_/
/// TASK_ID_/CREATE_TIME_/PROC_INST_ID_/SCOPE_ID_/SUB_SCOPE_ID_/SCOPE_TYPE_/
/// SCOPE_DEFINITION_ID_). Same id as the runtime link on create
/// (`DefaultHistoryManager.recordIdentityLinkCreated:402-409`); deleted by id
/// on runtime delete (`:414-417`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricIdentityLink {
    pub id: String,
    pub link_type: String,
    pub user_id: Option<String>,
    pub group_id: Option<String>,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    #[serde(default)]
    pub scope_id: Option<String>,
    #[serde(default)]
    pub sub_scope_id: Option<String>,
    #[serde(default)]
    pub scope_type: Option<String>,
    #[serde(default)]
    pub scope_definition_id: Option<String>,
    #[serde(default)]
    pub create_time: Option<DateTime<Utc>>,
}

impl HistoricIdentityLink {
    /// Builds a historic mirror of a runtime identity link (same id).
    /// Java `DefaultHistoryManager.recordIdentityLinkCreated:402-409`.
    pub fn from_runtime(link: &crate::identity::entities::IdentityLink) -> Self {
        Self {
            id: link.id.clone(),
            link_type: link.link_type.clone(),
            user_id: link.user_id.clone(),
            group_id: link.group_id.clone(),
            task_id: link.task_id.clone(),
            process_instance_id: link.process_instance_id.clone(),
            scope_id: None,
            sub_scope_id: None,
            scope_type: None,
            scope_definition_id: None,
            create_time: Some(Utc::now()),
        }
    }
}
