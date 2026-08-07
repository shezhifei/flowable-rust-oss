use serde_json::Value;
use std::collections::HashMap;

/// Bounded execution context for secure script tasks.
///
/// Constrained script context: the script sees only
/// process variables, has no filesystem/network/process-launch access,
/// and can only write back result variables through the controlled API.
#[derive(Debug, Clone)]
pub struct SecureScriptContext {
    /// Read-only snapshot of process variables visible to the script.
    variables: HashMap<String, Value>,
    /// Variables written by the script during execution.
    result_variables: HashMap<String, Value>,
}

impl SecureScriptContext {
    /// Build a context from the current execution's process variables.
    pub fn from_variables(variables: HashMap<String, Value>) -> Self {
        Self {
            variables,
            result_variables: HashMap::new(),
        }
    }

    /// Read a process variable or a previously set result variable.
    pub fn get_variable(&self, name: &str) -> Option<&Value> {
        self.result_variables
            .get(name)
            .or_else(|| self.variables.get(name))
    }

    /// Set a result variable (will be written back to the execution after script completes).
    pub fn set_result_variable(&mut self, name: String, value: Value) {
        self.result_variables.insert(name, value);
    }

    /// Consume the context and return all result variables to be merged back.
    pub fn into_result_variables(self) -> HashMap<String, Value> {
        self.result_variables
    }
}
