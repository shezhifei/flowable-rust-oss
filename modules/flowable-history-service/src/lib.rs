use flowable_engine::engine::history_service::*;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::history::historic_entities::*;
use std::sync::Arc;

pub struct FlowableHistoryService {
    engine: Arc<ProcessEngine>,
}

#[derive(Debug, Default, Clone)]
pub struct HistoricActivityInstanceQueryRequest {
    pub process_instance_id: Option<String>,
    pub execution_id: Option<String>,
    pub activity_id: Option<String>,
}

impl FlowableHistoryService {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        Self { engine }
    }

    pub fn create_historic_process_instance_query(&self) -> HistoricProcessInstanceQuery {
        self.engine
            .get_history_service()
            .create_historic_process_instance_query()
    }

    pub fn create_historic_activity_instance_query(&self) -> HistoricActivityInstanceQuery {
        self.engine
            .get_history_service()
            .create_historic_activity_instance_query()
    }

    pub fn list_historic_activity_instances(
        &self,
        request: HistoricActivityInstanceQueryRequest,
    ) -> Result<Vec<HistoricActivityInstance>, flowable_engine::error::FlowableError> {
        let mut query = self.create_historic_activity_instance_query();
        if let Some(process_instance_id) = request.process_instance_id {
            query = query.process_instance_id(process_instance_id);
        }

        let mut activities = query.list()?;
        if let Some(execution_id) = request.execution_id.as_deref() {
            activities.retain(|activity| activity.execution_id == execution_id);
        }
        if let Some(activity_id) = request.activity_id.as_deref() {
            activities.retain(|activity| activity.activity_id == activity_id);
        }

        Ok(activities)
    }

    pub fn create_historic_task_instance_query(&self) -> HistoricTaskInstanceQuery {
        self.engine
            .get_history_service()
            .create_historic_task_instance_query()
    }

    pub fn create_historic_variable_instance_query(&self) -> HistoricVariableInstanceQuery {
        self.engine
            .get_history_service()
            .create_historic_variable_instance_query()
    }

    pub fn create_process_instance_log_query(
        &self,
        process_instance_id: String,
    ) -> ProcessInstanceLogQuery {
        self.engine
            .get_history_service()
            .create_process_instance_log_query(process_instance_id)
    }

    pub fn get_historic_process_instance(&self, id: &str) -> Option<HistoricProcessInstance> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.get_historic_process_instance(id, &mut session)
    }

    pub fn get_historic_activity_instance(
        &self,
        execution_id: &str,
        activity_id: &str,
    ) -> Option<HistoricActivityInstance> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.get_historic_activity_instance_by_execution_and_activity(
            execution_id,
            activity_id,
            &mut session,
        )
    }

    pub fn get_historic_task_instance(&self, task_id: &str) -> Option<HistoricTaskInstance> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.get_historic_task_instance(task_id, &mut session)
    }

    pub fn get_historic_variable_instance(&self, id: &str) -> Option<HistoricVariableInstance> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.get_historic_variable_instance(id, &mut session)
    }

    pub fn get_historic_audit_log(&self, id: &str) -> Option<HistoricAuditLog> {
        let store = self.engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.get_historic_audit_log(id, &mut session)
    }
}
