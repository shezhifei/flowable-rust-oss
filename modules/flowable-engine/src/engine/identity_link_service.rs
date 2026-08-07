use crate::engine::query::{Direction, Query, QueryState};
use crate::error::FlowableError;
use crate::history::historic_entities::{HistoricComment, HistoricTaskEvent};
use crate::history::history_manager::HistoryManager;
use crate::identity::entities::IdentityLink;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::db_session::DbSession;
use chrono::Utc;
use std::sync::Arc;

pub struct IdentityLinkService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl IdentityLinkService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    fn get_store(&self) -> crate::persistence::runtime_store::RuntimeStore {
        self.command_executor.runtime_store().clone()
    }

    pub fn add_identity_link(&self, link: IdentityLink) {
        self.add_identity_link_with_author(link, None);
    }

    /// Like [`add_identity_link`], but records the authenticated user on the
    /// generated history event/comment. Java threads
    /// `Authentication.getAuthenticatedUserId()` into the identity-link comment
    /// (`AbstractHistoryManager.createProcessInstanceIdentityLinkComment`), so
    /// REST callers pass the request principal here.
    pub fn add_identity_link_with_author(&self, link: IdentityLink, author: Option<String>) {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        self.record_identity_link_event(
            &link,
            "AddUserLink",
            "AddGroupLink",
            author.as_deref(),
            &mut session,
        );
        // P77: historic mirror at AUDIT+ (Java DefaultHistoryManager:396-410).
        self.history_manager()
            .record_identity_link_created(&link, &mut session);
        self.get_store().insert_identity_link(link, &mut session);
        session.flush_and_commit().unwrap();
    }

    pub fn remove_identity_link(&self, link_id: &str) {
        self.remove_identity_link_with_author(link_id, None);
    }

    /// Like [`remove_identity_link`], but records the authenticated user on the
    /// generated `DeleteUserLink`/`DeleteGroupLink` history event/comment.
    pub fn remove_identity_link_with_author(&self, link_id: &str, author: Option<String>) {
        let mut session = self
            .command_executor
            .runtime_store()
            .create_session()
            .unwrap();
        let store = self.get_store();
        if let Some(link) = store.find_identity_link(link_id, &mut session) {
            self.record_identity_link_event(
                &link,
                "DeleteUserLink",
                "DeleteGroupLink",
                author.as_deref(),
                &mut session,
            );
            // P77: Java deletes historic row by id (DefaultHistoryManager:414-417).
            self.history_manager()
                .record_identity_link_deleted(&link.id, &mut session);
        }
        store.delete_identity_link(link_id, &mut session);
        session.flush_and_commit().unwrap();
    }

    /// History manager seeded from this service's engine config (AUDIT gate).
    fn history_manager(&self) -> HistoryManager {
        let config = self.command_executor.config();
        HistoryManager::new(self.get_store(), false)
            .with_history_level(config.history_level)
            .with_enable_process_definition_history_level(
                config.enable_process_definition_history_level,
            )
    }

    pub fn create_identity_link_query(&self) -> IdentityLinkQuery {
        IdentityLinkQuery::new(Arc::clone(&self.command_executor))
    }

    /// Records the history side-effect of adding/removing an identity link.
    ///
    /// Java writes a TYPE_EVENT `CommentEntity` in both cases
    /// (`AbstractHistoryManager.createIdentityLinkComment` for task links and
    /// `createProcessInstanceIdentityLinkComment` for process-instance links),
    /// with message parts `[identityId, type]` and the authenticated user as the
    /// comment author. Rust models the two audit trails separately:
    ///
    /// * Task links keep the existing `HistoricTaskEvent` row (surfaced via
    ///   `/runtime/tasks/{id}/events`).
    /// * Process-instance links get a `HistoricComment` carrying the event
    ///   `action` marker (surfaced via the process-instance comments endpoint,
    ///   matching Java where PI identity-link events live in `ACT_HI_COMMENT`).
    fn record_identity_link_event(
        &self,
        link: &IdentityLink,
        user_action: &str,
        group_action: &str,
        author: Option<&str>,
        session: &mut DbSession,
    ) {
        let (action, identity_id) = match (&link.user_id, &link.group_id) {
            (Some(user_id), None) => (user_action, user_id.clone()),
            (None, Some(group_id)) => (group_action, group_id.clone()),
            _ => return,
        };

        if let Some(task_id) = link.task_id.as_ref() {
            // Task links: user actions carry the user id as the event's user_id
            // (Java sets the comment userId to the authenticated principal, but
            // the existing Rust task-event contract records the link user id).
            let user_id = link.user_id.clone();
            self.command_executor
                .runtime_store()
                .insert_historic_task_event(
                    HistoricTaskEvent {
                        id: uuid::Uuid::new_v4().to_string(),
                        task_id: task_id.clone(),
                        action: action.to_string(),
                        message: vec![identity_id, link.link_type.clone()],
                        user_id,
                        time: Utc::now(),
                    },
                    session,
                );
        } else if let Some(process_instance_id) = link.process_instance_id.as_ref() {
            // Process-instance links: Java stores a TYPE_EVENT comment on the
            // process instance. Message mirrors the event's `[identityId, type]`
            // parts joined with Flowable's " | " marker so the comment text is
            // self-describing; the `action` marker carries the event kind.
            self.command_executor
                .runtime_store()
                .insert_historic_comment(
                    HistoricComment {
                        id: uuid::Uuid::new_v4().to_string(),
                        task_id: None,
                        process_instance_id: Some(process_instance_id.clone()),
                        message: format!("{} | {}", identity_id, link.link_type),
                        author: author.map(str::to_string),
                        time: Utc::now(),
                        action: Some(action.to_string()),
                        // Java stores identity-link audit as TYPE_EVENT comments.
                        comment_type: Some(HistoricComment::TYPE_EVENT.to_string()),
                    },
                    session,
                );
        }
    }
}

pub struct IdentityLinkQuery {
    state: QueryState<IdentityLink>,
    task_id: Option<String>,
    process_instance_id: Option<String>,
    process_definition_id: Option<String>,
    user_id: Option<String>,
    group_id: Option<String>,
    link_type: Option<String>,
}

impl IdentityLinkQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            task_id: None,
            process_instance_id: None,
            process_definition_id: None,
            user_id: None,
            group_id: None,
            link_type: None,
        }
    }

    pub fn task_id(mut self, task_id: String) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn process_instance_id(mut self, process_instance_id: String) -> Self {
        self.process_instance_id = Some(process_instance_id);
        self
    }

    pub fn process_definition_id(mut self, process_definition_id: String) -> Self {
        self.process_definition_id = Some(process_definition_id);
        self
    }

    pub fn user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    pub fn group_id(mut self, group_id: String) -> Self {
        self.group_id = Some(group_id);
        self
    }

    pub fn link_type(mut self, link_type: String) -> Self {
        self.link_type = Some(link_type);
        self
    }
}

pub struct IdentityLinkQueryCmd {
    query: IdentityLinkQuery,
}

impl IdentityLinkQueryCmd {
    pub fn new(query: IdentityLinkQuery) -> Self {
        Self { query }
    }
}

impl Command<Vec<IdentityLink>> for IdentityLinkQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<IdentityLink>, FlowableError> {
        let (store, session) = command_context.store_and_session();
        let links = if let Some(task_id) = &self.query.task_id {
            store.find_identity_links_by_task(task_id, session)
        } else if let Some(process_instance_id) = &self.query.process_instance_id {
            store.find_identity_links_by_process_instance(process_instance_id, session)
        } else if let Some(process_definition_id) = &self.query.process_definition_id {
            store.find_identity_links_by_process_definition(process_definition_id, session)
        } else if let Some(user_id) = &self.query.user_id {
            store.find_identity_links_by_user(user_id, session)
        } else if let Some(group_id) = &self.query.group_id {
            store.find_identity_links_by_group(group_id, session)
        } else if let Some(link_type) = &self.query.link_type {
            store.find_identity_links_by_type(link_type, session)
        } else {
            store.list_identity_links(session)
        };
        Ok(links)
    }
}

impl Query<IdentityLink, IdentityLinkQuery> for IdentityLinkQuery {
    fn list(&self) -> Result<Vec<IdentityLink>, FlowableError> {
        let query_clone = IdentityLinkQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            task_id: self.task_id.clone(),
            process_instance_id: self.process_instance_id.clone(),
            process_definition_id: self.process_definition_id.clone(),
            user_id: self.user_id.clone(),
            group_id: self.group_id.clone(),
            link_type: self.link_type.clone(),
        };
        let cmd = IdentityLinkQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<IdentityLink>, FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}
