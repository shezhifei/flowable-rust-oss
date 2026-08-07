use crate::engine::query::{Direction, Query, QueryState};
use crate::error::FlowableError;
use crate::identity::entities::{BatchEntity, BatchPartEntity};
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use std::sync::Arc;

pub struct BatchService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl BatchService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    fn get_store(&self) -> crate::persistence::runtime_store::RuntimeStore {
        self.command_executor.runtime_store().clone()
    }

    pub fn create_batch(&self, batch: BatchEntity) {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.get_store().insert_batch(batch, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn find_batch_by_id(&self, batch_id: &str) -> Option<BatchEntity> {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.get_store().find_batch(batch_id, &mut session)
    }

    pub fn delete_batch(&self, batch_id: &str) {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.get_store().delete_batch(batch_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn create_batch_part(&self, batch_part: BatchPartEntity) {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.get_store().insert_batch_part(batch_part, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn find_batch_part_by_id(&self, batch_part_id: &str) -> Option<BatchPartEntity> {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.get_store()
            .find_batch_part(batch_part_id, &mut session)
    }

    pub fn find_batch_parts_by_batch_id(&self, batch_id: &str) -> Vec<BatchPartEntity> {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.get_store()
            .find_batch_parts_by_batch_id(batch_id, &mut session)
    }

    pub fn find_batch_parts_by_batch_id_and_status(
        &self,
        batch_id: &str,
        status: &str,
    ) -> Vec<BatchPartEntity> {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.get_store()
            .find_batch_parts_by_batch_id_and_status(batch_id, status, &mut session)
    }

    pub fn create_batch_query(&self) -> BatchQuery {
        BatchQuery::new(Arc::clone(&self.command_executor))
    }
}

pub struct BatchQuery {
    state: QueryState<BatchEntity>,
    batch_type: Option<String>,
    status: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
}

impl BatchQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            batch_type: None,
            status: None,
            tenant_id: None,
            tenant_id_like: None,
            without_tenant_id: false,
        }
    }

    pub fn batch_type(mut self, batch_type: String) -> Self {
        self.batch_type = Some(batch_type);
        self
    }

    pub fn status(mut self, status: String) -> Self {
        self.status = Some(status);
        self
    }

    pub fn tenant_id(mut self, tenant_id: String) -> Self {
        self.tenant_id = Some(tenant_id);
        self.without_tenant_id = false;
        self
    }

    pub fn tenant_id_like(mut self, tenant_id_like: String) -> Self {
        self.tenant_id_like = Some(tenant_id_like);
        self.without_tenant_id = false;
        self
    }

    pub fn without_tenant_id(mut self) -> Self {
        self.without_tenant_id = true;
        self.tenant_id = None;
        self.tenant_id_like = None;
        self
    }
}

pub struct BatchQueryCmd {
    query: BatchQuery,
}

impl BatchQueryCmd {
    pub fn new(query: BatchQuery) -> Self {
        Self { query }
    }
}

impl Command<Vec<BatchEntity>> for BatchQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<BatchEntity>, FlowableError> {
        let (store, session) = command_context.store_and_session();
        let mut batches = if let Some(tenant_id) = &self.query.tenant_id {
            store.find_batches_by_tenant_id(tenant_id, session)
        } else if let Some(status) = &self.query.status {
            store.find_batches_by_status(status, session)
        } else if let Some(batch_type) = &self.query.batch_type {
            store.find_batches_by_type(batch_type, session)
        } else {
            store.list_batches(session)
        };
        if let Some(batch_type) = &self.query.batch_type {
            batches.retain(|b| &b.batch_type == batch_type);
        }
        if let Some(status) = &self.query.status {
            batches.retain(|b| &b.status == status);
        }
        if let Some(tenant_id) = &self.query.tenant_id {
            batches.retain(|b| b.tenant_id.as_deref() == Some(tenant_id));
        }
        if let Some(tenant_id_like) = &self.query.tenant_id_like {
            batches.retain(|b| {
                b.tenant_id
                    .as_deref()
                    .is_some_and(|tenant_id| tenant_id.contains(tenant_id_like))
            });
        }
        if self.query.without_tenant_id {
            batches.retain(|b| b.tenant_id.as_deref().is_none_or(str::is_empty));
        }
        Ok(batches)
    }
}

impl Query<BatchEntity, BatchQuery> for BatchQuery {
    fn list(&self) -> Result<Vec<BatchEntity>, FlowableError> {
        let query_clone = BatchQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            batch_type: self.batch_type.clone(),
            status: self.status.clone(),
            tenant_id: self.tenant_id.clone(),
            tenant_id_like: self.tenant_id_like.clone(),
            without_tenant_id: self.without_tenant_id,
        };
        let cmd = BatchQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<BatchEntity>, FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}
