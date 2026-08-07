use serde_json::Value;
use std::collections::HashMap;

/// Abstraction over process/case variable scopes used by the SimpleExpression
/// evaluator. Keeps EL free of engine-specific `Execution` / CMMN types so it
/// can live in a shared crate without dependency cycles.
pub trait VariableContainer {
    /// Resolve a process/case variable by name (transient/local/root merge is
    /// the implementor's responsibility).
    fn get_variable(&self, name: &str) -> Option<Value>;

    /// Tenant id exposed as the `${currentTenantId}` root object.
    fn current_tenant_id(&self) -> Option<&str> {
        None
    }

    /// JSON root for `${execution}` (and similar). Defaults to `None` so
    /// non-BPMN scopes (CMMN case variables) do not invent an execution object.
    fn root_object_json(&self) -> Option<Value> {
        None
    }
}

impl VariableContainer for HashMap<String, Value> {
    fn get_variable(&self, name: &str) -> Option<Value> {
        self.get(name).cloned()
    }
}

impl VariableContainer for serde_json::Map<String, Value> {
    fn get_variable(&self, name: &str) -> Option<Value> {
        self.get(name).cloned()
    }
}

/// Owned map-backed variable scope used by unit tests and lightweight callers
/// (e.g. CMMN case-variable evaluation) that do not carry a full execution.
#[derive(Clone, Debug, Default)]
pub struct MapVariableContainer {
    variables: HashMap<String, Value>,
    tenant_id: Option<String>,
    root_object_json: Option<Value>,
}

impl MapVariableContainer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_map(variables: HashMap<String, Value>) -> Self {
        Self {
            variables,
            tenant_id: None,
            root_object_json: None,
        }
    }

    pub fn from_json_map(variables: &serde_json::Map<String, Value>) -> Self {
        Self {
            variables: variables.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            tenant_id: None,
            root_object_json: None,
        }
    }

    pub fn with_tenant_id(mut self, tenant_id: Option<String>) -> Self {
        self.tenant_id = tenant_id;
        self
    }

    pub fn with_root_object_json(mut self, root: Option<Value>) -> Self {
        self.root_object_json = root;
        self
    }

    pub fn insert(&mut self, name: impl Into<String>, value: Value) {
        self.variables.insert(name.into(), value);
    }
}

impl VariableContainer for MapVariableContainer {
    fn get_variable(&self, name: &str) -> Option<Value> {
        self.variables.get(name).cloned()
    }

    fn current_tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }

    fn root_object_json(&self) -> Option<Value> {
        self.root_object_json.clone()
    }
}
