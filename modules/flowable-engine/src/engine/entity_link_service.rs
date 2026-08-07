use crate::engine::query::{Direction, Query, QueryState};
use crate::error::FlowableError;
use crate::identity::entities::EntityLink;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use std::sync::Arc;

pub struct EntityLinkService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl EntityLinkService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    fn get_store(&self) -> crate::persistence::runtime_store::RuntimeStore {
        self.command_executor.runtime_store().clone()
    }

    pub fn add_entity_link(&self, link: EntityLink) {
        let store = self.get_store();
        let mut session = store.create_session().unwrap();
        store.insert_entity_link(link, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn remove_entity_link(&self, link_id: &str) {
        let store = self.get_store();
        let mut session = store.create_session().unwrap();
        store.delete_entity_link(link_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn create_entity_link_query(&self) -> EntityLinkQuery {
        EntityLinkQuery::new(Arc::clone(&self.command_executor))
    }
}

pub struct EntityLinkQuery {
    state: QueryState<EntityLink>,
    scope_id: Option<String>,
    scope_type: Option<String>,
    reference_scope_id: Option<String>,
    reference_scope_type: Option<String>,
    link_type: Option<String>,
}

impl EntityLinkQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            scope_id: None,
            scope_type: None,
            reference_scope_id: None,
            reference_scope_type: None,
            link_type: None,
        }
    }

    pub fn scope_id(mut self, scope_id: String) -> Self {
        self.scope_id = Some(scope_id);
        self
    }

    pub fn scope_type(mut self, scope_type: String) -> Self {
        self.scope_type = Some(scope_type);
        self
    }

    pub fn reference_scope_id(mut self, reference_scope_id: String) -> Self {
        self.reference_scope_id = Some(reference_scope_id);
        self
    }

    pub fn reference_scope_type(mut self, reference_scope_type: String) -> Self {
        self.reference_scope_type = Some(reference_scope_type);
        self
    }

    pub fn link_type(mut self, link_type: String) -> Self {
        self.link_type = Some(link_type);
        self
    }
}

pub struct EntityLinkQueryCmd {
    query: EntityLinkQuery,
}

impl EntityLinkQueryCmd {
    pub fn new(query: EntityLinkQuery) -> Self {
        Self { query }
    }
}

impl Command<Vec<EntityLink>> for EntityLinkQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<EntityLink>, FlowableError> {
        let mut links = if let Some(scope_id) = &self.query.scope_id {
            command_context
                .runtime_store
                .find_entity_links_by_scope(scope_id, &mut command_context.session)
        } else if let Some(reference_scope_id) = &self.query.reference_scope_id {
            command_context
                .runtime_store
                .find_entity_links_by_reference_scope(
                    reference_scope_id,
                    &mut command_context.session,
                )
        } else {
            command_context
                .runtime_store
                .list_entity_links(&mut command_context.session)
        };
        if let Some(scope_type) = &self.query.scope_type {
            links.retain(|l| l.scope_type.as_deref() == Some(scope_type));
        }
        if let Some(reference_scope_type) = &self.query.reference_scope_type {
            links.retain(|l| l.reference_scope_type.as_deref() == Some(reference_scope_type));
        }
        if let Some(link_type) = &self.query.link_type {
            links.retain(|l| &l.link_type == link_type);
        }
        Ok(links)
    }
}

impl Query<EntityLink, EntityLinkQuery> for EntityLinkQuery {
    fn list(&self) -> Result<Vec<EntityLink>, FlowableError> {
        let query_clone = EntityLinkQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            scope_id: self.scope_id.clone(),
            scope_type: self.scope_type.clone(),
            reference_scope_id: self.reference_scope_id.clone(),
            reference_scope_type: self.reference_scope_type.clone(),
            link_type: self.link_type.clone(),
        };
        let cmd = EntityLinkQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<EntityLink>, FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}
