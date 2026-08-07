use std::collections::HashMap;

#[derive(Clone)]
pub struct ProcessInstanceBuilder {
    pub process_definition_id: Option<String>,
    pub process_definition_key: Option<String>,
    pub message_name: Option<String>,
    pub name: Option<String>,
    pub business_key: Option<String>,
    pub business_status: Option<String>,
    pub tenant_id: Option<String>,
    pub variables: HashMap<String, serde_json::Value>,
    pub transient_variables: HashMap<String, serde_json::Value>,
    pub callback_id: Option<String>,
    pub callback_type: Option<String>,
    pub reference_id: Option<String>,
    pub reference_type: Option<String>,
    pub super_execution_id: Option<String>,
    /// Authenticated user that initiated this start command. Internal starts
    /// leave this unset, matching Java's command-scoped authentication.
    pub start_user_id: Option<String>,
}

impl Default for ProcessInstanceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessInstanceBuilder {
    pub fn new() -> Self {
        Self {
            process_definition_id: None,
            process_definition_key: None,
            message_name: None,
            name: None,
            business_key: None,
            business_status: None,
            tenant_id: None,
            variables: HashMap::new(),
            transient_variables: HashMap::new(),
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
            super_execution_id: None,
            start_user_id: None,
        }
    }

    pub fn super_execution_id(mut self, super_execution_id: String) -> Self {
        self.super_execution_id = Some(super_execution_id);
        self
    }

    pub fn process_definition_id(mut self, id: String) -> Self {
        self.process_definition_id = Some(id);
        self
    }

    pub fn process_definition_key(mut self, key: String) -> Self {
        self.process_definition_key = Some(key);
        self
    }

    pub fn message_name(mut self, name: String) -> Self {
        self.message_name = Some(name);
        self
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn business_key(mut self, key: String) -> Self {
        self.business_key = Some(key);
        self
    }

    pub fn business_status(mut self, status: String) -> Self {
        self.business_status = Some(status);
        self
    }

    pub fn tenant_id(mut self, tenant_id: String) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    pub fn start_user_id(mut self, start_user_id: String) -> Self {
        self.start_user_id = Some(start_user_id);
        self
    }

    pub fn variable(mut self, key: String, value: serde_json::Value) -> Self {
        self.variables.insert(key, value);
        self
    }

    pub fn transient_variable(mut self, key: String, value: serde_json::Value) -> Self {
        self.transient_variables.insert(key, value);
        self
    }
}
