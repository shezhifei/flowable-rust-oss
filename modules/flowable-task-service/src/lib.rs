use flowable_engine::cmd::task_variable_cmd::TaskVariableScope;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::task_service::{EventWaitState, TaskQuery, TaskUpdate};
use flowable_engine::error::FlowableError;
use flowable_engine::task::Task;
use std::sync::Arc;

pub struct FlowableTaskService {
    engine: Arc<ProcessEngine>,
}

impl FlowableTaskService {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        Self { engine }
    }

    pub fn create_task_query(&self) -> TaskQuery {
        self.engine.get_task_service().create_task_query()
    }

    pub fn get_tasks_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Result<Vec<Task>, FlowableError> {
        self.engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance_id)
    }

    pub fn get_sub_tasks(&self, parent_task_id: String) -> Result<Vec<Task>, FlowableError> {
        self.engine.get_task_service().get_sub_tasks(parent_task_id)
    }

    pub fn delete_task(
        &self,
        task_id: String,
        delete_reason: Option<String>,
        cascade: bool,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .delete_task(task_id, delete_reason, cascade)
    }

    pub fn complete_task_by_id(&self, task_id: String) -> Result<(), FlowableError> {
        self.engine.get_task_service().complete_task_by_id(task_id)
    }

    pub fn complete_task_by_id_with_variables(
        &self,
        task_id: String,
        variables: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .complete_task_by_id_with_variables(task_id, variables)
    }

    pub fn complete_task_by_id_with_local_variables(
        &self,
        task_id: String,
        variables: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .complete_task_by_id_with_local_variables(task_id, variables)
    }

    pub fn complete_task_by_id_with_variable_maps(
        &self,
        task_id: String,
        variables: std::collections::HashMap<String, serde_json::Value>,
        transient_variables: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .complete_task_by_id_with_variable_maps(task_id, variables, transient_variables)
    }

    pub fn set_task_local_variable(
        &self,
        task_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .set_task_local_variable(task_id, name, value)
    }

    pub fn get_task_local_variable(
        &self,
        task_id: String,
        name: String,
    ) -> Result<Option<serde_json::Value>, FlowableError> {
        self.engine
            .get_task_service()
            .get_task_local_variable(task_id, name)
    }

    pub fn get_task_local_variables(
        &self,
        task_id: String,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, FlowableError> {
        self.engine
            .get_task_service()
            .get_task_local_variables(task_id)
    }

    pub fn delete_task_local_variable(
        &self,
        task_id: String,
        name: String,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .delete_task_local_variable(task_id, name)
    }

    pub fn set_task_variable(
        &self,
        task_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .set_task_variable(task_id, name, value)
    }

    pub fn set_task_variables(
        &self,
        task_id: String,
        variables: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .set_task_variables(task_id, variables)
    }

    pub fn set_task_variables_local(
        &self,
        task_id: String,
        variables: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .set_task_variables_local(task_id, variables)
    }

    pub fn create_task_variables(
        &self,
        task_id: String,
        scope: TaskVariableScope,
        variables: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .create_task_variables(task_id, scope, variables)
    }

    pub fn update_task_variable(
        &self,
        task_id: String,
        scope: TaskVariableScope,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .update_task_variable(task_id, scope, name, value)
    }

    pub fn remove_task_variable(&self, task_id: String, name: String) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .remove_task_variable(task_id, name)
    }

    pub fn remove_task_variables(
        &self,
        task_id: String,
        names: Vec<String>,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .remove_task_variables(task_id, names)
    }

    pub fn remove_task_variables_local(
        &self,
        task_id: String,
        names: Vec<String>,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .remove_task_variables_local(task_id, names)
    }

    pub fn remove_all_task_local_variables(&self, task_id: String) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .remove_all_task_local_variables(task_id)
    }

    pub fn remove_task_variable_on_scope(
        &self,
        task_id: String,
        scope: TaskVariableScope,
        name: String,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .remove_task_variable_on_scope(task_id, scope, name)
    }

    pub fn get_task_variable(
        &self,
        task_id: String,
        name: String,
    ) -> Result<Option<serde_json::Value>, FlowableError> {
        self.engine
            .get_task_service()
            .get_task_variable(task_id, name)
    }

    pub fn get_task_variables(
        &self,
        task_id: String,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, FlowableError> {
        self.engine.get_task_service().get_task_variables(task_id)
    }

    pub fn claim_task_by_id(&self, task_id: String, assignee: String) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .claim_task_by_id(task_id, assignee)
    }

    pub fn unclaim_task_by_id(&self, task_id: String) -> Result<(), FlowableError> {
        self.engine.get_task_service().unclaim_task_by_id(task_id)
    }

    pub fn update_task_by_id(
        &self,
        task_id: String,
        update: TaskUpdate,
    ) -> Result<Task, FlowableError> {
        self.engine
            .get_task_service()
            .update_task_by_id(task_id, update)
    }

    pub fn delegate_task_by_id(
        &self,
        task_id: String,
        user_id: String,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .delegate_task_by_id(task_id, user_id)
    }

    pub fn resolve_task_by_id(&self, task_id: String) -> Result<(), FlowableError> {
        self.engine.get_task_service().resolve_task_by_id(task_id)
    }

    pub fn add_candidate_user(
        &self,
        task_id: String,
        user_id: String,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .add_candidate_user(task_id, user_id)
    }

    pub fn add_candidate_group(
        &self,
        task_id: String,
        group_id: String,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .add_candidate_group(task_id, group_id)
    }

    pub fn delete_candidate_user(
        &self,
        task_id: String,
        user_id: String,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .delete_candidate_user(task_id, user_id)
    }

    pub fn delete_candidate_group(
        &self,
        task_id: String,
        group_id: String,
    ) -> Result<(), FlowableError> {
        self.engine
            .get_task_service()
            .delete_candidate_group(task_id, group_id)
    }

    pub fn get_identity_links_for_task(
        &self,
        task_id: String,
    ) -> Result<Vec<flowable_engine::identity::entities::IdentityLink>, FlowableError> {
        self.engine
            .get_task_service()
            .get_identity_links_for_task(task_id)
    }

    pub fn get_event_wait_states_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Vec<EventWaitState> {
        self.engine
            .get_task_service()
            .get_event_wait_states_by_process_instance_id(process_instance_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowable_engine::engine::query::Query;
    use std::sync::atomic::{AtomicU64, Ordering};

    static ENGINE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn task_service_with_task(task: Task) -> FlowableTaskService {
        let engine_id = ENGINE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let engine = Arc::new(ProcessEngine::new(format!(
            "flowable-task-service-claim-{}",
            engine_id
        )));
        let runtime_store = engine.get_runtime_store();
        let mut session = runtime_store.create_session().unwrap();
        runtime_store.insert_task(&task, &mut session);
        session.flush_and_commit().unwrap();
        FlowableTaskService::new(engine)
    }

    fn test_task() -> Task {
        Task::new(
            "task-1".to_string(),
            "process-1".to_string(),
            "execution-1".to_string(),
            "reviewTask".to_string(),
            "Review task".to_string(),
        )
    }

    #[test]
    fn claim_task_rejects_different_assignee_when_already_claimed() {
        let service = task_service_with_task(test_task());

        service
            .claim_task_by_id("task-1".to_string(), "kermit".to_string())
            .unwrap();
        service
            .claim_task_by_id("task-1".to_string(), "kermit".to_string())
            .unwrap();

        let error = service
            .claim_task_by_id("task-1".to_string(), "fozzie".to_string())
            .unwrap_err();
        assert!(
            matches!(&error, FlowableError::Conflict(message) if message.contains("already claimed") && message.contains("kermit")),
            "unexpected error: {error}"
        );

        let tasks = service
            .create_task_query()
            .task_assignee("kermit".to_string())
            .list()
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].assignee.as_deref(), Some("kermit"));

        let reassigned = service
            .create_task_query()
            .task_assignee("fozzie".to_string())
            .list()
            .unwrap();
        assert!(reassigned.is_empty());
    }
}
