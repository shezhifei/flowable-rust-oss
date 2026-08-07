use crate::engine::process_engine::ProcessEngine;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct RestService {
    process_engine: Arc<ProcessEngine>,
}

impl RestService {
    pub fn new(process_engine: Arc<ProcessEngine>) -> Self {
        Self { process_engine }
    }

    pub fn get_process_instance(&self, id: &str) -> Value {
        let store = self.process_engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        match store.find_process_instance(id, &mut session) {
            Some(pi) => json!(pi),
            None => json!({"error": "Not found"}),
        }
    }

    pub fn get_tasks(&self, process_instance_id: &str) -> Value {
        let ts = self.process_engine.get_task_service();
        match ts.get_tasks_by_process_instance_id(process_instance_id.to_string()) {
            Ok(tasks) => json!(tasks),
            Err(e) => json!({"error": format!("{:?}", e)}),
        }
    }
}
