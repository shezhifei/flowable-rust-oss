use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub process_instance_id: String,
    pub execution_id: String,
    pub task_definition_key: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub delegation_state: Option<String>,
    #[serde(default)]
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub due_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub form_key: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    pub is_completed: bool,
    pub created_time: Option<DateTime<Utc>>,
    pub completed_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub claim_time: Option<DateTime<Utc>>,
    #[serde(default = "default_task_state")]
    pub state: String,
    /// Suspension state: 0 = ACTIVE, 1 = SUSPENDED. Default is 0 (active).
    #[serde(default)]
    pub suspension_state: i32,
    #[serde(default)]
    pub local_variables: HashMap<String, serde_json::Value>,
}

fn default_task_state() -> String {
    "created".to_string()
}

impl Task {
    pub fn new(
        id: String,
        process_instance_id: String,
        execution_id: String,
        task_definition_key: String,
        name: String,
    ) -> Self {
        Self {
            id,
            process_instance_id,
            execution_id,
            task_definition_key,
            name,
            description: None,
            assignee: None,
            owner: None,
            delegation_state: None,
            parent_task_id: None,
            priority: None,
            due_date: None,
            category: None,
            form_key: None,
            tenant_id: None,
            is_completed: false,
            created_time: Some(Utc::now()),
            completed_time: None,
            claim_time: None,
            state: "created".to_string(),
            suspension_state: 0,
            local_variables: HashMap::new(),
        }
    }

    pub fn mark_completed(&mut self) {
        self.is_completed = true;
        self.completed_time = Some(Utc::now());
        self.state = "completed".to_string();
    }

    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn process_instance_id(&self) -> &String {
        &self.process_instance_id
    }

    pub fn set_local_variable(&mut self, name: String, value: serde_json::Value) {
        self.local_variables.insert(name, value);
    }

    pub fn is_suspended(&self) -> bool {
        self.suspension_state == 1
    }

    pub fn is_active(&self) -> bool {
        self.suspension_state == 0
    }

    pub fn set_suspension_state(&mut self, suspended: bool) {
        self.suspension_state = if suspended { 1 } else { 0 };
    }

    pub fn local_variable(&self, name: &str) -> Option<serde_json::Value> {
        self.local_variables.get(name).cloned()
    }

    pub fn local_variables(&self) -> HashMap<String, serde_json::Value> {
        self.local_variables.clone()
    }
}
