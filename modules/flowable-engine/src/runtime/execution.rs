use flowable_engine_common::el::VariableContainer;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    pub id: String,
    pub parent_id: Option<String>,
    pub super_execution_id: Option<String>,
    pub root_process_instance_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub process_definition_id: Option<String>,
    pub process_definition_key: Option<String>,
    pub process_definition_name: Option<String>,
    pub process_definition_version: Option<i32>,
    pub activity_id: Option<String>,
    pub activity_name: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub is_suspended: bool,
    pub is_ended: bool,
    pub is_active: bool,
    pub is_concurrent: bool,
    pub is_scope: bool,
    pub is_multi_instance_root: bool,
    pub tenant_id: Option<String>,
    /// Java `ExecutionEntity.referenceId` — e.g. child case instance id for caseServiceTask
    /// (CaseTaskActivityBehavior.java:118).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_id: Option<String>,
    /// Java `ExecutionEntity.referenceType` — e.g. `bpmn-2.0-to-cmmn-1.1-child-case`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<String>,
    #[serde(default)]
    pub variables: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub local_variables: HashMap<String, serde_json::Value>,
    /// Java transient variables (`VariableScopeImpl.transientVariables`): pure
    /// in-memory scope — not projected to the `variables` table and not durable
    /// ACT_RU_VARIABLE rows. Serialized into the execution JSON during a
    /// command so same-command reloads (`find_execution`, call-activity
    /// `inheritVariables`) still see them; stripped on commit by
    /// `RuntimeStore::strip_transient_variables_before_commit` so they never
    /// survive the transaction (P45 / Java parity).
    #[serde(default)]
    pub transient_variables: HashMap<String, serde_json::Value>,
    /// Structural flag for non-interrupting event-subprocess path executions.
    /// Must survive across commands (path may end in a later complete-task) but
    /// must not appear as a process/REST variable. Replaces the P41
    /// `__flowable_non_interrupting_event_subprocess_path` marker that was
    /// stored in `transient_variables` and would be stripped by P45.
    #[serde(default)]
    pub non_interrupting_event_subprocess_path: bool,
}

impl Execution {
    pub fn set_process_variable(&mut self, name: String, value: serde_json::Value) {
        self.variables.insert(name, value);
    }

    pub fn set_process_variables(&mut self, variables: HashMap<String, serde_json::Value>) {
        self.variables = variables;
    }

    pub fn process_variables(&self) -> HashMap<String, serde_json::Value> {
        self.variables.clone()
    }

    pub fn process_variable(&self, name: &str) -> Option<serde_json::Value> {
        self.transient_variables
            .get(name)
            .or_else(|| self.local_variables.get(name))
            .or_else(|| self.variables.get(name))
            .cloned()
    }

    pub fn persistent_process_variable(&self, name: &str) -> Option<serde_json::Value> {
        self.variables.get(name).cloned()
    }

    pub fn set_local_variable(&mut self, name: String, value: serde_json::Value) {
        self.local_variables.insert(name, value);
    }

    pub fn set_transient_variable(&mut self, name: String, value: serde_json::Value) {
        self.transient_variables.insert(name, value);
    }

    pub fn parallel_scope_id(&self) -> String {
        self.parent_id.clone().unwrap_or_else(|| self.id.clone())
    }

    /// Java parity: the process instance itself is an execution
    /// (`ExecutionEntityImpl`), identified by `parent_id == None` and
    /// `id == process_instance_id`. Forks must preserve this row (inactive)
    /// instead of deleting it, so branch executions keep a resolvable parent.
    pub fn is_process_instance_scope_execution(&self) -> bool {
        self.parent_id.is_none() && self.process_instance_id.as_deref() == Some(self.id.as_str())
    }
}

impl VariableContainer for Execution {
    fn get_variable(&self, name: &str) -> Option<Value> {
        self.process_variable(name)
    }

    fn current_tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }

    fn root_object_json(&self) -> Option<Value> {
        serde_json::to_value(self).ok()
    }
}
