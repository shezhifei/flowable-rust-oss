use crate::persistence::db_session::DbSession;
use crate::persistence::entity_manager::EntityManager;
use crate::persistence::runtime_store::RuntimeStore;
use crate::task::Task;
use std::collections::HashMap;

pub trait TaskEntityManager: EntityManager<Task> {
    fn find_task_by_id(&mut self, id: &str, session: &mut DbSession) -> Option<Task>;
    fn find_by_process_instance_id(
        &mut self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<Task>;
    fn find_by_execution_id(&mut self, execution_id: &str, session: &mut DbSession)
    -> Option<Task>;
    fn find_by_parent_task_id(
        &mut self,
        parent_task_id: &str,
        session: &mut DbSession,
    ) -> Vec<Task>;
    fn snapshot_tasks(&mut self, session: &mut DbSession) -> HashMap<String, Task>;
}

pub struct DefaultTaskEntityManager {
    runtime_store: RuntimeStore,
}

impl DefaultTaskEntityManager {
    pub fn new(runtime_store: RuntimeStore) -> Self {
        Self { runtime_store }
    }
}

impl EntityManager<Task> for DefaultTaskEntityManager {
    fn insert(&mut self, entity: &Task, session: &mut DbSession) {
        self.runtime_store.insert_task(entity, session);
    }

    fn update(&mut self, entity: &Task, session: &mut DbSession) {
        self.runtime_store.update_task(entity, session);
    }

    fn delete(&mut self, id: &str, session: &mut DbSession) {
        self.runtime_store.delete_task(id, session);
    }

    fn find_by_id(&mut self, _id: &str, _session: &mut DbSession) -> Option<Task> {
        None
    }
}

impl TaskEntityManager for DefaultTaskEntityManager {
    fn find_task_by_id(&mut self, id: &str, session: &mut DbSession) -> Option<Task> {
        self.runtime_store.find_task(id, session)
    }

    fn find_by_process_instance_id(
        &mut self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<Task> {
        self.runtime_store
            .find_tasks_by_process_instance_id(process_instance_id, session)
    }

    fn find_by_execution_id(
        &mut self,
        execution_id: &str,
        session: &mut DbSession,
    ) -> Option<Task> {
        self.runtime_store
            .find_task_by_execution_id(execution_id, session)
    }

    fn find_by_parent_task_id(
        &mut self,
        parent_task_id: &str,
        session: &mut DbSession,
    ) -> Vec<Task> {
        self.runtime_store
            .find_tasks_by_parent_task_id(parent_task_id, session)
    }

    fn snapshot_tasks(&mut self, session: &mut DbSession) -> HashMap<String, Task> {
        self.runtime_store.snapshot_tasks(session)
    }
}
