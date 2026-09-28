use crate::engine::process_engine::ProcessEngine;
use crate::error::FlowableError;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct RestService {
    process_engine: Arc<ProcessEngine>,
}

impl RestService {
    pub fn new(process_engine: Arc<ProcessEngine>) -> Self {
        Self { process_engine }
    }

    pub fn get_process_instance(&self, id: &str) -> Result<Value, FlowableError> {
        let store = self.process_engine.get_runtime_store();
        let mut session = store.create_session()?;
        let instance = store
            .find_process_instance(id, &mut session)?
            .ok_or_else(|| FlowableError::NotFound(format!("Process instance {id} not found")))?;
        Ok(json!(instance))
    }

    pub fn get_tasks(&self, process_instance_id: &str) -> Result<Value, FlowableError> {
        let tasks = self
            .process_engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance_id.to_string())?;
        Ok(json!(tasks))
    }
}
