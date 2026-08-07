use crate::cmd::create_task_comment_cmd::{
    CreateProcessInstanceCommentCmd, CreateTaskCommentCmd, SaveCommentCmd,
};
use crate::cmd::delete_task_comment_cmd::DeleteTaskCommentCmd;
use crate::cmd::delete_task_event_cmd::DeleteTaskEventCmd;
use crate::cmd::record_form_property_detail_cmd::RecordFormPropertyDetailCmd;
use crate::cmd::record_task_event_cmd::RecordTaskEventCmd;
use crate::engine::query::{Direction, Query, QueryState};
use crate::history::historic_entities::{
    HistoricActivityInstance, HistoricAuditLog, HistoricComment, HistoricDetail,
    HistoricIdentityLink, HistoricProcessInstance, HistoricTaskEvent, HistoricTaskInstance,
    HistoricTaskLogEntry, HistoricVariableInstance,
};
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::FilterOp;
use crate::persistence::db_session::DbSession;
use std::collections::HashSet;
use std::sync::Arc;

// ── Historic Process Instance Query ──

pub struct HistoricProcessInstanceQuery {
    state: QueryState<HistoricProcessInstance>,
    process_instance_id: Option<String>,
    involved_user: Option<String>,
}

impl HistoricProcessInstanceQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            process_instance_id: None,
            involved_user: None,
        }
    }

    pub fn process_instance_id(mut self, process_instance_id: String) -> Self {
        self.process_instance_id = Some(process_instance_id);
        self
    }

    /// Matches historic process instances having an identity link for this
    /// user, regardless of link type.
    pub fn involved_user(mut self, involved_user: String) -> Self {
        self.involved_user = Some(involved_user);
        self
    }
}

pub struct HistoricProcessInstanceQueryCmd {
    query: HistoricProcessInstanceQuery,
}

impl Command<Vec<HistoricProcessInstance>> for HistoricProcessInstanceQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<HistoricProcessInstance>, crate::error::FlowableError> {
        let mut rows = if let Some(pi_id) = &self.query.process_instance_id {
            command_context
                .session()
                .find("historic_process_instances", pi_id)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            command_context
                .session()
                .find_all::<HistoricProcessInstance>("historic_process_instances")
                .unwrap()
        };
        if let Some(involved_user) = &self.query.involved_user {
            // P77: Java HistoricProcessInstance.xml:903-904 filters via
            // ACT_HI_IDENTITYLINK, not the runtime identity-link table.
            let (store, session) = command_context.store_and_session();
            let involved_process_instance_ids: HashSet<String> = store
                .find_process_instance_ids_by_historic_involved_user(involved_user, session)
                .into_iter()
                .collect();
            rows.retain(|instance| involved_process_instance_ids.contains(&instance.id));
        }
        Ok(rows)
    }
}

impl Query<HistoricProcessInstance, HistoricProcessInstanceQuery> for HistoricProcessInstanceQuery {
    fn list(&self) -> Result<Vec<HistoricProcessInstance>, crate::error::FlowableError> {
        let query_clone = HistoricProcessInstanceQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            process_instance_id: self.process_instance_id.clone(),
            involved_user: self.involved_user.clone(),
        };
        let cmd = HistoricProcessInstanceQueryCmd { query: query_clone };
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(
        &self,
    ) -> Result<Option<HistoricProcessInstance>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

pub struct BulkDeleteHistoricProcessInstancesCmd {
    process_instance_ids: Vec<String>,
}

impl BulkDeleteHistoricProcessInstancesCmd {
    pub fn new(process_instance_ids: Vec<String>) -> Self {
        Self {
            process_instance_ids,
        }
    }
}

impl Command<()> for BulkDeleteHistoricProcessInstancesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        for process_instance_id in &self.process_instance_ids {
            if store
                .get_historic_process_instance(process_instance_id, session)
                .is_none()
            {
                return Err(crate::error::FlowableError::NotFound(format!(
                    "Historic process instance '{}' was not found",
                    process_instance_id
                )));
            }
        }

        for process_instance_id in &self.process_instance_ids {
            store.delete_historic_process_instance_cascade(process_instance_id, session);
        }

        Ok(())
    }
}

pub struct DeleteHistoricTaskInstanceCmd {
    task_id: String,
}

impl DeleteHistoricTaskInstanceCmd {
    pub fn new(task_id: String) -> Self {
        Self { task_id }
    }
}

impl Command<()> for DeleteHistoricTaskInstanceCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        if store
            .get_historic_task_instance(&self.task_id, session)
            .is_none()
        {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Historic task instance '{}' was not found",
                self.task_id
            )));
        }

        store.delete_historic_task_instance_cascade(&self.task_id, session);
        Ok(())
    }
}

/// Java `DeleteCommentCmd` with only a comment id (the path used by
/// `HistoricProcessInstanceCommentResource.deleteComment`): deletes the
/// comment row when it exists. Ownership checks stay in the caller so the
/// REST layer can produce its own 404 message.
pub struct DeleteCommentCmd {
    comment_id: String,
}

impl DeleteCommentCmd {
    pub fn new(comment_id: String) -> Self {
        Self { comment_id }
    }
}

impl Command<Option<HistoricComment>> for DeleteCommentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<HistoricComment>, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        let Some(comment) = store.find_historic_comment(&self.comment_id, session) else {
            return Ok(None);
        };
        store.delete_historic_comment(&self.comment_id, session);
        Ok(Some(comment))
    }
}

// ── Historic Activity Instance Query ──

pub struct HistoricActivityInstanceQuery {
    state: QueryState<HistoricActivityInstance>,
    process_instance_id: Option<String>,
}

impl HistoricActivityInstanceQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            process_instance_id: None,
        }
    }

    pub fn process_instance_id(mut self, process_instance_id: String) -> Self {
        self.process_instance_id = Some(process_instance_id);
        self
    }
}

pub struct HistoricActivityInstanceQueryCmd {
    query: HistoricActivityInstanceQuery,
}

impl Command<Vec<HistoricActivityInstance>> for HistoricActivityInstanceQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<HistoricActivityInstance>, crate::error::FlowableError> {
        let rows = if let Some(pi_id) = &self.query.process_instance_id {
            command_context
                .session()
                .find_by::<HistoricActivityInstance>(
                    "historic_activity_instances",
                    "process_instance_id",
                    pi_id,
                )
                .unwrap()
        } else {
            command_context
                .session()
                .find_all::<HistoricActivityInstance>("historic_activity_instances")
                .unwrap()
        };
        Ok(rows)
    }
}

impl Query<HistoricActivityInstance, HistoricActivityInstanceQuery>
    for HistoricActivityInstanceQuery
{
    fn list(&self) -> Result<Vec<HistoricActivityInstance>, crate::error::FlowableError> {
        let query_clone = HistoricActivityInstanceQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            process_instance_id: self.process_instance_id.clone(),
        };
        let cmd = HistoricActivityInstanceQueryCmd { query: query_clone };
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(
        &self,
    ) -> Result<Option<HistoricActivityInstance>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

// ── Historic Task Instance Query ──

pub struct HistoricTaskInstanceQuery {
    state: QueryState<HistoricTaskInstance>,
    process_instance_id: Option<String>,
    process_definition_id: Option<String>,
    task_definition_key: Option<String>,
    task_definition_key_like: Option<String>,
    assignee: Option<String>,
    owner: Option<String>,
    candidate_user: Option<String>,
    candidate_group: Option<String>,
    /// Java `HistoricTaskInstanceQuery.ignoreAssigneeValue`: when true, candidate
    /// filters keep assigned tasks. Default false matches Java HistoricTaskInstance.xml
    /// (`ASSIGNEE_ is null` unless ignoreAssigneeValue).
    ignore_assignee: bool,
    priority: Option<i32>,
    minimum_priority: Option<i32>,
    maximum_priority: Option<i32>,
    due_date_millis: Option<i64>,
    due_before_millis: Option<i64>,
    due_after_millis: Option<i64>,
    without_due_date: bool,
}

impl HistoricTaskInstanceQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            process_instance_id: None,
            process_definition_id: None,
            task_definition_key: None,
            task_definition_key_like: None,
            assignee: None,
            owner: None,
            candidate_user: None,
            candidate_group: None,
            ignore_assignee: false,
            priority: None,
            minimum_priority: None,
            maximum_priority: None,
            due_date_millis: None,
            due_before_millis: None,
            due_after_millis: None,
            without_due_date: false,
        }
    }

    pub fn process_instance_id(mut self, process_instance_id: String) -> Self {
        self.process_instance_id = Some(process_instance_id);
        self
    }

    pub fn process_definition_id(mut self, process_definition_id: String) -> Self {
        self.process_definition_id = Some(process_definition_id);
        self
    }

    pub fn task_definition_key(mut self, task_definition_key: String) -> Self {
        self.task_definition_key = Some(task_definition_key);
        self
    }

    pub fn task_definition_key_like(mut self, task_definition_key_like: String) -> Self {
        self.task_definition_key_like = Some(task_definition_key_like);
        self
    }

    pub fn task_assignee(mut self, assignee: String) -> Self {
        self.assignee = Some(assignee);
        self
    }

    pub fn task_owner(mut self, owner: String) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn task_candidate_user(mut self, candidate_user: String) -> Self {
        self.candidate_user = Some(candidate_user);
        self
    }

    pub fn task_candidate_group(mut self, candidate_group: String) -> Self {
        self.candidate_group = Some(candidate_group);
        self
    }

    /// Java `HistoricTaskInstanceQuery.ignoreAssigneeValue` — keep assigned tasks
    /// in candidate queries (HistoricTaskInstanceQueryImpl.java:1972-1978).
    pub fn ignore_assignee_value(mut self) -> Self {
        self.ignore_assignee = true;
        self
    }

    pub fn task_priority(mut self, priority: i32) -> Self {
        self.priority = Some(priority);
        self
    }

    pub fn task_minimum_priority(mut self, priority: i32) -> Self {
        self.minimum_priority = Some(priority);
        self
    }

    pub fn task_maximum_priority(mut self, priority: i32) -> Self {
        self.maximum_priority = Some(priority);
        self
    }

    pub fn task_due_date_millis(mut self, due_date_millis: i64) -> Self {
        self.due_date_millis = Some(due_date_millis);
        self
    }

    pub fn task_due_before_millis(mut self, due_before_millis: i64) -> Self {
        self.due_before_millis = Some(due_before_millis);
        self
    }

    pub fn task_due_after_millis(mut self, due_after_millis: i64) -> Self {
        self.due_after_millis = Some(due_after_millis);
        self
    }

    pub fn task_without_due_date(mut self) -> Self {
        self.without_due_date = true;
        self
    }
}

pub struct HistoricTaskInstanceQueryCmd {
    query: HistoricTaskInstanceQuery,
}

impl Command<Vec<HistoricTaskInstance>> for HistoricTaskInstanceQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<HistoricTaskInstance>, crate::error::FlowableError> {
        let mut filters: Vec<(String, FilterOp)> = Vec::new();
        if let Some(pi_id) = &self.query.process_instance_id {
            filters.push((
                "process_instance_id".into(),
                FilterOp::Eq(Arc::from(pi_id.as_str())),
            ));
        }
        if let Some(process_definition_id) = &self.query.process_definition_id {
            filters.push((
                "process_definition_id".into(),
                FilterOp::Eq(Arc::from(process_definition_id.as_str())),
            ));
        }
        if let Some(task_definition_key) = &self.query.task_definition_key {
            filters.push((
                "task_definition_key".into(),
                FilterOp::Eq(Arc::from(task_definition_key.as_str())),
            ));
        }
        if let Some(task_definition_key_like) = &self.query.task_definition_key_like {
            filters.push((
                "task_definition_key".into(),
                FilterOp::Like(Arc::from(task_definition_key_like.as_str())),
            ));
        }
        if let Some(assignee) = &self.query.assignee {
            filters.push((
                "assignee".into(),
                FilterOp::Eq(Arc::from(assignee.as_str())),
            ));
        }
        if let Some(owner) = &self.query.owner {
            filters.push(("owner".into(), FilterOp::Eq(Arc::from(owner.as_str()))));
        }
        if let Some(priority) = self.query.priority {
            filters.push((
                "priority".into(),
                FilterOp::Eq(Arc::from(priority.to_string())),
            ));
        }
        if let Some(minimum_priority) = self.query.minimum_priority {
            filters.push((
                "priority".into(),
                FilterOp::GreaterThanOrEqual(minimum_priority as i64),
            ));
        }
        if let Some(maximum_priority) = self.query.maximum_priority {
            filters.push((
                "priority".into(),
                FilterOp::LessThanOrEqual(maximum_priority as i64),
            ));
        }
        if let Some(due_date_millis) = self.query.due_date_millis {
            filters.push((
                "due_date".into(),
                FilterOp::Eq(Arc::from(due_date_millis.to_string())),
            ));
        }
        if let Some(due_before_millis) = self.query.due_before_millis {
            filters.push((
                "due_date".into(),
                FilterOp::LessThanOrEqual(due_before_millis),
            ));
        }
        if let Some(due_after_millis) = self.query.due_after_millis {
            filters.push((
                "due_date".into(),
                FilterOp::GreaterThanOrEqual(due_after_millis),
            ));
        }
        if self.query.without_due_date {
            filters.push(("due_date".into(), FilterOp::IsNull));
        }

        let (store, session) = command_context.store_and_session();
        let mut tasks: Vec<HistoricTaskInstance> = session
            .find_with_filters("historic_task_instances", &filters, None, None)
            .unwrap();

        use std::collections::HashSet;
        // Java HistoricTaskInstanceQueryImpl.getCandidateGroups (HistoricTaskInstanceQueryImpl.java:2221-2246)
        // expands candidateUser to the user's group memberships and ORs them with a
        // direct user candidate link — same semantics as TaskQueryImpl (P49).
        // HistoricTaskInstance.xml:1484-1510 EXISTS (LINK.USER_ID_ OR LINK.GROUP_ID_ IN groups).
        // Default also requires ASSIGNEE_ is null unless ignoreAssigneeValue
        // (HistoricTaskInstance.xml:1485-1487) — same gate as runtime T4 (P49).
        if self.query.candidate_user.is_some() || self.query.candidate_group.is_some() {
            let task_ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
            let links: Vec<crate::identity::entities::IdentityLink> =
                store.find_identity_links_by_tasks(&task_ids, session);
            if let Some(candidate_user) = &self.query.candidate_user {
                let candidate_user = candidate_user.clone();
                let user_group_ids: HashSet<String> = store
                    .get_groups_by_user(&candidate_user, session)
                    .into_iter()
                    .map(|g| g.id)
                    .collect();
                let matching: HashSet<String> = links
                    .iter()
                    .filter(|l| {
                        if l.link_type != "candidate" {
                            return false;
                        }
                        if l.user_id.as_deref() == Some(candidate_user.as_str()) {
                            return true;
                        }
                        l.group_id
                            .as_ref()
                            .is_some_and(|gid| user_group_ids.contains(gid))
                    })
                    .filter_map(|l| l.task_id.clone())
                    .collect();
                tasks.retain(|task| matching.contains(&task.id));
            }
            if let Some(candidate_group) = &self.query.candidate_group {
                let candidate_group = candidate_group.clone();
                let matching: HashSet<String> = links
                    .iter()
                    .filter(|l| {
                        l.link_type == "candidate"
                            && l.group_id.as_deref() == Some(&candidate_group)
                    })
                    .filter_map(|l| l.task_id.clone())
                    .collect();
                tasks.retain(|task| matching.contains(&task.id));
            }
            // Java HistoricTaskInstance.xml:1485-1487 / Task.xml:868-870 parity.
            if !self.query.ignore_assignee {
                tasks.retain(|task| task.assignee.is_none());
            }
        }

        Ok(tasks)
    }
}

impl Query<HistoricTaskInstance, HistoricTaskInstanceQuery> for HistoricTaskInstanceQuery {
    fn list(&self) -> Result<Vec<HistoricTaskInstance>, crate::error::FlowableError> {
        let query_clone = HistoricTaskInstanceQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            process_instance_id: self.process_instance_id.clone(),
            process_definition_id: self.process_definition_id.clone(),
            task_definition_key: self.task_definition_key.clone(),
            task_definition_key_like: self.task_definition_key_like.clone(),
            assignee: self.assignee.clone(),
            owner: self.owner.clone(),
            candidate_user: self.candidate_user.clone(),
            candidate_group: self.candidate_group.clone(),
            ignore_assignee: self.ignore_assignee,
            priority: self.priority,
            minimum_priority: self.minimum_priority,
            maximum_priority: self.maximum_priority,
            due_date_millis: self.due_date_millis,
            due_before_millis: self.due_before_millis,
            due_after_millis: self.due_after_millis,
            without_due_date: self.without_due_date,
        };
        let cmd = HistoricTaskInstanceQueryCmd { query: query_clone };
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<HistoricTaskInstance>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

// ── Historic Variable Instance Query ──

pub struct HistoricVariableInstanceQuery {
    state: QueryState<HistoricVariableInstance>,
    process_instance_id: Option<String>,
    execution_id: Option<String>,
    task_id: Option<String>,
    variable_name: Option<String>,
    variable_name_like: Option<String>,
    variable_type: Option<String>,
    exclude_task_variables: bool,
}

impl HistoricVariableInstanceQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            process_instance_id: None,
            execution_id: None,
            task_id: None,
            variable_name: None,
            variable_name_like: None,
            variable_type: None,
            exclude_task_variables: false,
        }
    }

    pub fn process_instance_id(mut self, process_instance_id: String) -> Self {
        self.process_instance_id = Some(process_instance_id);
        self
    }

    pub fn execution_id(mut self, execution_id: String) -> Self {
        self.execution_id = Some(execution_id);
        self
    }

    pub fn task_id(mut self, task_id: String) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn variable_name(mut self, variable_name: String) -> Self {
        self.variable_name = Some(variable_name);
        self
    }

    pub fn variable_name_like(mut self, variable_name_like: String) -> Self {
        self.variable_name_like = Some(variable_name_like);
        self
    }

    pub fn variable_type(mut self, variable_type: String) -> Self {
        self.variable_type = Some(variable_type);
        self
    }

    pub fn exclude_task_variables(mut self) -> Self {
        self.exclude_task_variables = true;
        self
    }
}

pub struct HistoricVariableInstanceQueryCmd {
    query: HistoricVariableInstanceQuery,
}

impl Command<Vec<HistoricVariableInstance>> for HistoricVariableInstanceQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<HistoricVariableInstance>, crate::error::FlowableError> {
        let mut filters: Vec<(String, FilterOp)> = Vec::new();
        if let Some(pi_id) = &self.query.process_instance_id {
            filters.push((
                "process_instance_id".into(),
                FilterOp::Eq(Arc::from(pi_id.as_str())),
            ));
        }
        if let Some(name) = &self.query.variable_name {
            filters.push((
                "variable_name".into(),
                FilterOp::Eq(Arc::from(name.as_str())),
            ));
        }

        let mut variables: Vec<HistoricVariableInstance> = command_context
            .session()
            .find_with_filters("historic_variable_instances", &filters, None, None)
            .unwrap();

        if let Some(execution_id) = &self.query.execution_id {
            variables.retain(|variable| variable.execution_id.as_deref() == Some(execution_id));
        }
        if let Some(task_id) = &self.query.task_id {
            variables.retain(|variable| variable.task_id.as_deref() == Some(task_id));
        }
        if let Some(name_like) = &self.query.variable_name_like {
            variables.retain(|variable| sql_like_matches(name_like, &variable.name));
        }
        if let Some(variable_type) = &self.query.variable_type {
            variables.retain(|variable| variable.variable_type == *variable_type);
        }
        if self.query.exclude_task_variables {
            variables.retain(|variable| variable.task_id.is_none());
        }

        Ok(variables)
    }
}

fn sql_like_matches(pattern: &str, value: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, value)
}

impl Query<HistoricVariableInstance, HistoricVariableInstanceQuery>
    for HistoricVariableInstanceQuery
{
    fn list(&self) -> Result<Vec<HistoricVariableInstance>, crate::error::FlowableError> {
        let query_clone = HistoricVariableInstanceQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            process_instance_id: self.process_instance_id.clone(),
            execution_id: self.execution_id.clone(),
            task_id: self.task_id.clone(),
            variable_name: self.variable_name.clone(),
            variable_name_like: self.variable_name_like.clone(),
            variable_type: self.variable_type.clone(),
            exclude_task_variables: self.exclude_task_variables,
        };
        let cmd = HistoricVariableInstanceQueryCmd { query: query_clone };
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(
        &self,
    ) -> Result<Option<HistoricVariableInstance>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

// ── Historic Audit Log Query ──

pub struct HistoricAuditLogQuery {
    state: QueryState<HistoricAuditLog>,
    process_instance_id: Option<String>,
    event_type: Option<String>,
}

impl HistoricAuditLogQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            process_instance_id: None,
            event_type: None,
        }
    }

    pub fn process_instance_id(mut self, process_instance_id: String) -> Self {
        self.process_instance_id = Some(process_instance_id);
        self
    }

    pub fn event_type(mut self, event_type: String) -> Self {
        self.event_type = Some(event_type);
        self
    }
}

pub struct HistoricAuditLogQueryCmd {
    query: HistoricAuditLogQuery,
}

impl Command<Vec<HistoricAuditLog>> for HistoricAuditLogQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<HistoricAuditLog>, crate::error::FlowableError> {
        let rows = command_context
            .session()
            .find_all::<HistoricAuditLog>("historic_audit_logs")
            .unwrap();

        let mut audit_logs: Vec<HistoricAuditLog> = rows
            .into_iter()
            .filter(|entry| {
                self.query
                    .process_instance_id
                    .as_ref()
                    .is_none_or(|process_instance_id| {
                        entry.process_instance_id.as_ref() == Some(process_instance_id)
                    })
            })
            .filter(|entry| {
                self.query
                    .event_type
                    .as_ref()
                    .is_none_or(|event_type| &entry.event_type == event_type)
            })
            .collect();

        audit_logs.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.id.cmp(&right.id))
        });

        Ok(audit_logs)
    }
}

impl Query<HistoricAuditLog, HistoricAuditLogQuery> for HistoricAuditLogQuery {
    fn list(&self) -> Result<Vec<HistoricAuditLog>, crate::error::FlowableError> {
        let query_clone = HistoricAuditLogQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            process_instance_id: self.process_instance_id.clone(),
            event_type: self.event_type.clone(),
        };
        let cmd = HistoricAuditLogQueryCmd { query: query_clone };
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<HistoricAuditLog>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

// ── Process Instance Log Query ──

pub struct ProcessInstanceLog {
    process_instance_id: String,
    tasks: Vec<HistoricTaskInstance>,
    activities: Vec<HistoricActivityInstance>,
    variables: Vec<HistoricVariableInstance>,
}

impl ProcessInstanceLog {
    pub fn process_instance_id(&self) -> &String {
        &self.process_instance_id
    }

    pub fn tasks(&self) -> &[HistoricTaskInstance] {
        &self.tasks
    }

    pub fn activities(&self) -> &[HistoricActivityInstance] {
        &self.activities
    }

    pub fn variables(&self) -> &[HistoricVariableInstance] {
        &self.variables
    }
}

pub struct ProcessInstanceLogQuery {
    command_executor: Arc<DefaultCommandExecutor>,
    process_instance_id: String,
    include_tasks: bool,
    include_activities: bool,
    include_variables: bool,
}

impl ProcessInstanceLogQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>, process_instance_id: String) -> Self {
        Self {
            command_executor,
            process_instance_id,
            include_tasks: false,
            include_activities: false,
            include_variables: false,
        }
    }

    pub fn include_tasks(mut self) -> Self {
        self.include_tasks = true;
        self
    }

    pub fn include_activities(mut self) -> Self {
        self.include_activities = true;
        self
    }

    pub fn include_variables(mut self) -> Self {
        self.include_variables = true;
        self
    }

    pub fn single_result(&self) -> Result<Option<ProcessInstanceLog>, crate::error::FlowableError> {
        let mut tasks = Vec::new();
        let mut activities = Vec::new();
        let mut variables = Vec::new();

        if self.include_tasks {
            let cmd = HistoricTaskInstanceQueryCmd {
                query: HistoricTaskInstanceQuery::new(Arc::clone(&self.command_executor))
                    .process_instance_id(self.process_instance_id.clone()),
            };
            tasks = self.command_executor.execute(&cmd)?;
        }

        if self.include_activities {
            let cmd = HistoricActivityInstanceQueryCmd {
                query: HistoricActivityInstanceQuery::new(Arc::clone(&self.command_executor))
                    .process_instance_id(self.process_instance_id.clone()),
            };
            activities = self.command_executor.execute(&cmd)?;
        }

        if self.include_variables {
            let cmd = HistoricVariableInstanceQueryCmd {
                query: HistoricVariableInstanceQuery::new(Arc::clone(&self.command_executor))
                    .process_instance_id(self.process_instance_id.clone()),
            };
            variables = self.command_executor.execute(&cmd)?;
        }

        Ok(Some(ProcessInstanceLog {
            process_instance_id: self.process_instance_id.clone(),
            tasks,
            activities,
            variables,
        }))
    }
}

pub struct HistoryService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl HistoryService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    pub fn create_historic_process_instance_query(&self) -> HistoricProcessInstanceQuery {
        HistoricProcessInstanceQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn bulk_delete_historic_process_instances(
        &self,
        process_instance_ids: Vec<String>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = BulkDeleteHistoricProcessInstancesCmd::new(process_instance_ids);
        self.command_executor.execute(&cmd)
    }

    pub fn delete_historic_process_instance(
        &self,
        process_instance_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        self.bulk_delete_historic_process_instances(vec![process_instance_id])
    }

    pub fn delete_historic_task_instance(
        &self,
        task_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = DeleteHistoricTaskInstanceCmd::new(task_id);
        self.command_executor.execute(&cmd)
    }

    pub fn create_historic_activity_instance_query(&self) -> HistoricActivityInstanceQuery {
        HistoricActivityInstanceQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn create_historic_task_instance_query(&self) -> HistoricTaskInstanceQuery {
        HistoricTaskInstanceQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn create_historic_variable_instance_query(&self) -> HistoricVariableInstanceQuery {
        HistoricVariableInstanceQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn create_historic_audit_log_query(&self) -> HistoricAuditLogQuery {
        HistoricAuditLogQuery::new(Arc::clone(&self.command_executor))
    }

    /// Java `HistoryService.getHistoricIdentityLinksForTask` /
    /// `GetHistoricIdentityLinksForTaskCmd` — reads `ACT_HI_IDENTITYLINK`
    /// (Rust `historic_identity_links`), not runtime identity links.
    pub fn get_historic_identity_links_for_task(
        &self,
        task_id: &str,
    ) -> Result<Vec<HistoricIdentityLink>, crate::error::FlowableError> {
        self.create_historic_identity_link_query()
            .task_id(task_id.to_string())
            .list()
    }

    /// Java `HistoryService.getHistoricIdentityLinksForProcessInstance`.
    pub fn get_historic_identity_links_for_process_instance(
        &self,
        process_instance_id: &str,
    ) -> Result<Vec<HistoricIdentityLink>, crate::error::FlowableError> {
        self.create_historic_identity_link_query()
            .process_instance_id(process_instance_id.to_string())
            .list()
    }

    /// Java `HistoryService.createHistoricIdentityLinkQuery` dimensions
    /// (taskId / processInstanceId / scope).
    pub fn create_historic_identity_link_query(&self) -> HistoricIdentityLinkQuery {
        HistoricIdentityLinkQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn create_process_instance_log_query(
        &self,
        process_instance_id: String,
    ) -> ProcessInstanceLogQuery {
        ProcessInstanceLogQuery::new(Arc::clone(&self.command_executor), process_instance_id)
    }

    /// Java `TaskService.addComment(taskId, processInstanceId, message)` —
    /// type defaults to `"comment"`.
    pub fn create_task_comment(
        &self,
        task_id: &str,
        process_instance_id: Option<&str>,
        message: &str,
        author: Option<&str>,
    ) -> Result<HistoricComment, crate::error::FlowableError> {
        let cmd = CreateTaskCommentCmd::new(
            task_id.to_string(),
            process_instance_id.map(str::to_string),
            message.to_string(),
            author.map(str::to_string),
        );
        self.command_executor.execute(&cmd)
    }

    /// Java `TaskService.addComment(taskId, processInstanceId, type, message)`.
    pub fn create_task_comment_with_type(
        &self,
        task_id: &str,
        process_instance_id: Option<&str>,
        comment_type: &str,
        message: &str,
        author: Option<&str>,
    ) -> Result<HistoricComment, crate::error::FlowableError> {
        let cmd = CreateTaskCommentCmd::with_type(
            task_id.to_string(),
            process_instance_id.map(str::to_string),
            Some(comment_type.to_string()),
            message.to_string(),
            author.map(str::to_string),
        );
        self.command_executor.execute(&cmd)
    }

    pub fn delete_task_comment(
        &self,
        task_id: &str,
        comment_id: &str,
        _author: Option<&str>,
    ) -> Result<Option<HistoricComment>, crate::error::FlowableError> {
        let cmd = DeleteTaskCommentCmd::new(task_id.to_string(), comment_id.to_string());
        self.command_executor.execute(&cmd)
    }

    /// Java `TaskServiceImpl.addComment(null, processInstanceId, message)` as
    /// used by `HistoricProcessInstanceCommentCollectionResource`.
    pub fn create_process_instance_comment(
        &self,
        process_instance_id: &str,
        message: &str,
        author: Option<&str>,
    ) -> Result<HistoricComment, crate::error::FlowableError> {
        let cmd = CreateProcessInstanceCommentCmd::new(
            process_instance_id.to_string(),
            message.to_string(),
            author.map(str::to_string),
        );
        self.command_executor.execute(&cmd)
    }

    /// Java `TaskService.addComment(null, processInstanceId, type, message)`.
    pub fn create_process_instance_comment_with_type(
        &self,
        process_instance_id: &str,
        comment_type: &str,
        message: &str,
        author: Option<&str>,
    ) -> Result<HistoricComment, crate::error::FlowableError> {
        let cmd = CreateProcessInstanceCommentCmd::with_type(
            process_instance_id.to_string(),
            Some(comment_type.to_string()),
            message.to_string(),
            author.map(str::to_string),
        );
        self.command_executor.execute(&cmd)
    }

    /// Java `TaskService.saveComment(comment)` — updates type/message while
    /// preserving id and association fields supplied on the entity.
    pub fn save_comment(
        &self,
        comment: HistoricComment,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SaveCommentCmd::new(comment);
        self.command_executor.execute(&cmd)
    }

    /// Java `TaskServiceImpl.deleteComment(commentId)`. Returns `None` when
    /// the comment does not exist.
    pub fn delete_comment(
        &self,
        comment_id: &str,
    ) -> Result<Option<HistoricComment>, crate::error::FlowableError> {
        let cmd = DeleteCommentCmd::new(comment_id.to_string());
        self.command_executor.execute(&cmd)
    }

    pub fn get_comment(
        &self,
        comment_id: &str,
        session: &mut DbSession,
    ) -> Option<HistoricComment> {
        self.command_executor
            .runtime_store()
            .find_historic_comment(comment_id, session)
    }

    /// Java `getTaskComments(taskId)` — TYPE_COMMENT only, newest first.
    pub fn get_task_comments(
        &self,
        task_id: &str,
        session: &mut DbSession,
    ) -> Vec<HistoricComment> {
        self.command_executor
            .runtime_store()
            .find_historic_comments_by_task_id(task_id, session)
    }

    /// Java `getTaskComments(taskId, type)`.
    pub fn get_task_comments_by_type(
        &self,
        task_id: &str,
        comment_type: &str,
        session: &mut DbSession,
    ) -> Vec<HistoricComment> {
        self.command_executor
            .runtime_store()
            .find_historic_comments_by_task_id_and_type(task_id, comment_type, session)
    }

    /// Java `getProcessInstanceComments(processInstanceId)` — all types.
    pub fn get_process_instance_comments(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<HistoricComment> {
        self.command_executor
            .runtime_store()
            .find_historic_comments_by_process_instance_id(process_instance_id, session)
    }

    /// Java `getProcessInstanceComments(processInstanceId, type)`.
    pub fn get_process_instance_comments_by_type(
        &self,
        process_instance_id: &str,
        comment_type: &str,
        session: &mut DbSession,
    ) -> Vec<HistoricComment> {
        self.command_executor
            .runtime_store()
            .find_historic_comments_by_process_instance_id_and_type(
                process_instance_id,
                comment_type,
                session,
            )
    }

    /// Java `getCommentsByType(type)` — global type list, newest first.
    pub fn get_comments_by_type(
        &self,
        comment_type: &str,
        session: &mut DbSession,
    ) -> Vec<HistoricComment> {
        self.command_executor
            .runtime_store()
            .find_historic_comments_by_type(comment_type, session)
    }

    pub fn record_task_event(
        &self,
        task_id: &str,
        action: &str,
        message: Vec<String>,
        user_id: Option<&str>,
    ) -> Result<HistoricTaskEvent, crate::error::FlowableError> {
        let cmd = RecordTaskEventCmd::new(
            task_id.to_string(),
            action.to_string(),
            message,
            user_id.map(str::to_string),
        );
        self.command_executor.execute(&cmd)
    }

    pub fn get_task_events(
        &self,
        task_id: &str,
        session: &mut DbSession,
    ) -> Vec<HistoricTaskEvent> {
        self.command_executor
            .runtime_store()
            .find_historic_task_events_by_task_id(task_id, session)
    }

    pub fn get_task_event(
        &self,
        task_id: &str,
        event_id: &str,
        session: &mut DbSession,
    ) -> Option<HistoricTaskEvent> {
        let event = self
            .command_executor
            .runtime_store()
            .find_historic_task_event(event_id, session)?;
        (event.task_id == task_id).then_some(event)
    }

    /// Deletes a task event, mirroring Java `TaskEventResource.deleteEvent`
    /// (`taskService.deleteComment(eventId)` on the events table). Returns
    /// `None` when the event does not exist or belongs to another task.
    pub fn delete_task_event(
        &self,
        task_id: &str,
        event_id: &str,
    ) -> Result<Option<HistoricTaskEvent>, crate::error::FlowableError> {
        let cmd = DeleteTaskEventCmd::new(task_id.to_string(), event_id.to_string());
        self.command_executor.execute(&cmd)
    }

    pub fn record_form_property_detail(
        &self,
        process_instance_id: &str,
        task_id: Option<&str>,
        property_id: &str,
        property_value: serde_json::Value,
    ) -> Result<HistoricDetail, crate::error::FlowableError> {
        let cmd = RecordFormPropertyDetailCmd::new(
            process_instance_id.to_string(),
            task_id.map(str::to_string),
            property_id.to_string(),
            property_value,
        );
        self.command_executor.execute(&cmd)
    }

    pub fn get_historic_details(&self, session: &mut DbSession) -> Vec<HistoricDetail> {
        self.command_executor
            .runtime_store()
            .list_historic_details(session)
    }

    pub fn get_historic_detail(
        &self,
        detail_id: &str,
        session: &mut DbSession,
    ) -> Option<HistoricDetail> {
        self.command_executor
            .runtime_store()
            .get_historic_detail(detail_id, session)
    }

    pub fn get_historic_task_log_entries(
        &self,
        session: &mut DbSession,
    ) -> Vec<HistoricTaskLogEntry> {
        self.command_executor
            .runtime_store()
            .list_historic_task_log_entries(session)
    }
}

// ── Historic Identity Link Query (P77) ──
// Java HistoryService.createHistoricIdentityLinkQuery / GetHistoricIdentityLinksForTaskCmd.

pub struct HistoricIdentityLinkQuery {
    state: QueryState<HistoricIdentityLink>,
    task_id: Option<String>,
    process_instance_id: Option<String>,
    scope_id: Option<String>,
    scope_type: Option<String>,
    user_id: Option<String>,
}

impl HistoricIdentityLinkQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            task_id: None,
            process_instance_id: None,
            scope_id: None,
            scope_type: None,
            user_id: None,
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

    pub fn scope_id(mut self, scope_id: String) -> Self {
        self.scope_id = Some(scope_id);
        self
    }

    pub fn scope_type(mut self, scope_type: String) -> Self {
        self.scope_type = Some(scope_type);
        self
    }

    pub fn user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }
}

struct HistoricIdentityLinkQueryCmd {
    query: HistoricIdentityLinkQuery,
}

impl Command<Vec<HistoricIdentityLink>> for HistoricIdentityLinkQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<HistoricIdentityLink>, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        let mut links = if let Some(task_id) = &self.query.task_id {
            store.find_historic_identity_links_by_task(task_id, session)
        } else if let Some(process_instance_id) = &self.query.process_instance_id {
            store.find_historic_identity_links_by_process_instance(process_instance_id, session)
        } else if let Some(scope_id) = &self.query.scope_id {
            store.find_historic_identity_links_by_scope(
                scope_id,
                self.query.scope_type.as_deref(),
                session,
            )
        } else if let Some(user_id) = &self.query.user_id {
            store.find_historic_identity_links_by_user(user_id, session)
        } else {
            session
                .find_all("historic_identity_links")
                .unwrap_or_default()
        };
        // Additional filters when a primary dimension was used.
        if let Some(process_instance_id) = &self.query.process_instance_id
            && self.query.task_id.is_some()
        {
            links.retain(|l| l.process_instance_id.as_deref() == Some(process_instance_id.as_str()));
        }
        if let Some(user_id) = &self.query.user_id
            && (self.query.task_id.is_some()
                || self.query.process_instance_id.is_some()
                || self.query.scope_id.is_some())
        {
            links.retain(|l| l.user_id.as_deref() == Some(user_id.as_str()));
        }
        if let Some(scope_type) = &self.query.scope_type
            && self.query.scope_id.is_none()
        {
            links.retain(|l| l.scope_type.as_deref() == Some(scope_type.as_str()));
        }
        Ok(links)
    }
}

impl Query<HistoricIdentityLink, HistoricIdentityLinkQuery> for HistoricIdentityLinkQuery {
    fn list(&self) -> Result<Vec<HistoricIdentityLink>, crate::error::FlowableError> {
        let query_clone = HistoricIdentityLinkQuery {
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
            scope_id: self.scope_id.clone(),
            scope_type: self.scope_type.clone(),
            user_id: self.user_id.clone(),
        };
        let cmd = HistoricIdentityLinkQueryCmd { query: query_clone };
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<HistoricIdentityLink>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        Ok(self.list()?.len() as i64)
    }
}
