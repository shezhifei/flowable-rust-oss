use super::entity_manager::EntityManager;
use super::runtime_store::RuntimeStore;
use crate::persistence::db_session::DbSession;
use crate::runtime::execution::Execution;

pub trait ExecutionEntityManager: EntityManager<Execution> {
    fn find_child_executions_by_parent_execution_id(
        &mut self,
        parent_id: &str,
        session: &mut DbSession,
    ) -> Vec<Execution>;
    fn find_executions_by_process_instance_id(
        &mut self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<Execution>;
    fn find_executions_by_root_process_instance_id(
        &mut self,
        root_process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<Execution>;
}

pub struct DefaultExecutionEntityManager {
    runtime_store: RuntimeStore,
}

impl DefaultExecutionEntityManager {
    pub fn new(runtime_store: RuntimeStore) -> Self {
        Self { runtime_store }
    }
}

impl EntityManager<Execution> for DefaultExecutionEntityManager {
    fn insert(&mut self, entity: &Execution, session: &mut DbSession) {
        self.runtime_store.insert_execution(entity, session);
    }

    fn update(&mut self, entity: &Execution, session: &mut DbSession) {
        self.runtime_store.update_execution(entity, session);
    }

    fn delete(&mut self, id: &str, session: &mut DbSession) {
        self.runtime_store.delete_execution(id, session);
    }

    fn find_by_id(&mut self, id: &str, session: &mut DbSession) -> Option<Execution> {
        self.runtime_store.find_execution(id, session)
    }
}

impl ExecutionEntityManager for DefaultExecutionEntityManager {
    fn find_child_executions_by_parent_execution_id(
        &mut self,
        parent_id: &str,
        session: &mut DbSession,
    ) -> Vec<Execution> {
        let entities = self.runtime_store.snapshot_executions(session);
        entities
            .values()
            .filter(|e| e.parent_id.as_deref() == Some(parent_id))
            .cloned()
            .collect()
    }

    fn find_executions_by_process_instance_id(
        &mut self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<Execution> {
        let entities = self.runtime_store.snapshot_executions(session);
        entities
            .values()
            .filter(|e| e.process_instance_id.as_deref() == Some(process_instance_id))
            .cloned()
            .collect()
    }

    fn find_executions_by_root_process_instance_id(
        &mut self,
        root_process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<Execution> {
        let entities = self.runtime_store.snapshot_executions(session);
        entities
            .values()
            .filter(|e| e.root_process_instance_id.as_deref() == Some(root_process_instance_id))
            .cloned()
            .collect()
    }
}
