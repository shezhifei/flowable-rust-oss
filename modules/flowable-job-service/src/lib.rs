use flowable_engine::engine::management_service::TimerJobQuery;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use std::sync::Arc;

pub struct FlowableJobService {
    engine: Arc<ProcessEngine>,
}

impl FlowableJobService {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        Self { engine }
    }

    pub fn create_timer_job_query(&self) -> TimerJobQuery {
        self.engine
            .get_management_service()
            .create_timer_job_query()
    }

    pub fn get_timer_jobs_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Result<Vec<RuntimeTimerJobState>, FlowableError> {
        self.engine
            .get_job_service()
            .get_timer_jobs_by_process_instance_id(process_instance_id)
    }

    pub fn move_deadletter_job_to_executable_job(
        &self,
        job_id: String,
        retries: i32,
    ) -> Result<RuntimeTimerJobState, FlowableError> {
        self.engine
            .get_management_service()
            .move_deadletter_job_to_executable_job(&job_id, retries)
    }

    pub fn delete_job(&self, job_id: String) -> Result<(), FlowableError> {
        self.engine.get_runtime_service().delete_job(job_id)
    }
}
