use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;

pub fn create_process_engine() -> Arc<ProcessEngine> {
    Arc::new(ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    ))
}
