use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::variable_service::VariableInstanceQuery;
use flowable_engine::error::FlowableError;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub struct FlowableVariableService {
    engine: Arc<ProcessEngine>,
}

impl FlowableVariableService {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        Self { engine }
    }

    pub fn create_variable_instance_query(&self) -> VariableInstanceQuery {
        self.engine
            .get_variable_service()
            .create_variable_instance_query()
    }

    pub fn set_variable(
        &self,
        execution_id: String,
        name: String,
        value: Value,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_variable_service()
            .set_variable(execution_id, name, value)
    }

    pub fn get_variable(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<Option<Value>, FlowableError> {
        self.engine
            .get_variable_service()
            .get_variable(execution_id, name)
    }

    pub fn get_variables(
        &self,
        execution_id: String,
    ) -> Result<HashMap<String, Value>, FlowableError> {
        self.engine
            .get_variable_service()
            .get_variables(execution_id)
    }
}
