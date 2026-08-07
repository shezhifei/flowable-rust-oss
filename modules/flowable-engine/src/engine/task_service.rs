use crate::agenda::FlowableEngineAgenda;
use crate::cmd::task_variable_cmd::{
    MutateTaskVariablesCmd, RemoveTaskVariablesCmd, TaskVariableMutation, TaskVariableScope,
    VariableMutationMode, remove_task_variables,
};
use crate::el::expression::{Expression, SimpleExpression};
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::FilterOp;
use crate::persistence::runtime_store::{
    EventSubscriptionKind, RuntimeEventWaitKind, RuntimeEventWaitState,
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;

use crate::task::Task;
use flowable_bpmn_model::model::{
    FlowElementEnum, MultiInstanceLoopCharacteristics, VariableAggregationDefinition,
    VariableAggregationDefinitionVariable,
};
use serde_json::{Map, Value};

// ── Suspension guard helper ──

/// Returns an error if the task is suspended. Mirrors Java `NeedsActiveTaskCmd`.
pub(crate) fn require_active_task(task: &Task) -> Result<(), crate::error::FlowableError> {
    require_active_task_with_prefix(task, "Cannot execute operation for")
}

/// Java parity: `NeedsActiveTaskCmd` subclasses override
/// `getSuspendedTaskExceptionPrefix()` to provide operation-specific messages.
pub(crate) fn require_active_task_with_prefix(
    task: &Task,
    prefix: &str,
) -> Result<(), crate::error::FlowableError> {
    if task.is_suspended() {
        return Err(crate::error::FlowableError::ExecutionError(format!(
            "{} a suspended task '{}'",
            prefix, task.id
        )));
    }
    Ok(())
}

// ── Public API wait-state types ──

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventWaitKind {
    ReceiveTask,
    MessageIntermediateCatchEvent,
    SignalIntermediateCatchEvent,
    ConditionalIntermediateCatchEvent,
    ErrorIntermediateCatchEvent,
    CancelIntermediateCatchEvent,
    CompensateIntermediateCatchEvent,
    EscalationIntermediateCatchEvent,
    EventRegistryIntermediateCatchEvent,
    /// Send-event service task wait (P130 / `RuntimeEventWaitKind::SendEventTask`).
    SendEventTask,
}

/// Type alias retained for callers that import `MessageStyleWaitKind`.
pub type MessageStyleWaitKind = EventWaitKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventWaitState {
    pub wait_kind: EventWaitKind,
    pub process_instance_id: String,
    pub execution_id: String,
    pub task_id: Option<String>,
    pub activity_id: Option<String>,
    /// Human-readable BPMN activity name for visibility only.
    pub message_name: Option<String>,
    /// BPMN event definition reference used for correlation and trigger matching.
    /// Replaces the old parallel `message_ref` / `signal_ref` fields.
    pub event_ref: Option<String>,
    /// Convenience accessors that project back to the old message_ref / signal_ref shape
    /// so existing test assertions can stay unchanged.
    pub message_ref: Option<String>,
    pub signal_ref: Option<String>,
    /// Event-registry correlation key (Java `EventSubscriptionEntity.configuration`).
    /// See P93 / `RuntimeEventWaitState.configuration`.
    pub configuration: Option<String>,
}

/// Type alias retained for callers that import `MessageStyleWaitState`.
pub type MessageStyleWaitState = EventWaitState;

impl From<RuntimeEventWaitKind> for EventWaitKind {
    fn from(value: RuntimeEventWaitKind) -> Self {
        match value {
            RuntimeEventWaitKind::ReceiveTask => EventWaitKind::ReceiveTask,
            RuntimeEventWaitKind::MessageIntermediateCatchEvent => {
                EventWaitKind::MessageIntermediateCatchEvent
            }
            RuntimeEventWaitKind::SignalIntermediateCatchEvent => {
                EventWaitKind::SignalIntermediateCatchEvent
            }
            RuntimeEventWaitKind::ConditionalIntermediateCatchEvent => {
                EventWaitKind::ConditionalIntermediateCatchEvent
            }
            RuntimeEventWaitKind::ErrorIntermediateCatchEvent => {
                EventWaitKind::ErrorIntermediateCatchEvent
            }
            RuntimeEventWaitKind::CancelIntermediateCatchEvent => {
                EventWaitKind::CancelIntermediateCatchEvent
            }
            RuntimeEventWaitKind::CompensateIntermediateCatchEvent => {
                EventWaitKind::CompensateIntermediateCatchEvent
            }
            RuntimeEventWaitKind::EscalationIntermediateCatchEvent => {
                EventWaitKind::EscalationIntermediateCatchEvent
            }
            RuntimeEventWaitKind::EventRegistryIntermediateCatchEvent => {
                EventWaitKind::EventRegistryIntermediateCatchEvent
            }
            // P130: send-event triggerable wait (SendEventTaskActivityBehavior).
            RuntimeEventWaitKind::SendEventTask => EventWaitKind::SendEventTask,
        }
    }
}

impl From<RuntimeEventWaitState> for EventWaitState {
    fn from(value: RuntimeEventWaitState) -> Self {
        let (event_ref, message_ref, signal_ref) = match &value.event_subscription {
            Some(sub) => {
                let r = Some(sub.event_ref.clone());
                match sub.kind {
                    EventSubscriptionKind::Message => (r.clone(), r.clone(), None),
                    EventSubscriptionKind::Signal => (r.clone(), None, r.clone()),
                    EventSubscriptionKind::Conditional => (r.clone(), None, None),
                    EventSubscriptionKind::EventRegistry => (r.clone(), None, None),
                    EventSubscriptionKind::Error
                    | EventSubscriptionKind::Cancel
                    | EventSubscriptionKind::Compensate
                    | EventSubscriptionKind::Escalation => (None, None, None),
                }
            }
            None => (None, None, None),
        };
        Self {
            wait_kind: value.wait_kind.into(),
            process_instance_id: value.process_instance_id,
            execution_id: value.execution_id,
            task_id: value.task_id,
            activity_id: value.activity_id,
            message_name: value.display_name,
            event_ref,
            message_ref,
            signal_ref,
            configuration: value.configuration,
        }
    }
}

use crate::engine::query::{Direction, Query, QueryState};

/// Filter criteria for one AND-context of a task query: either the main
/// query or one `or()` block. Mirrors the filter fields of Java
/// `TaskQueryImpl` — an or-block is itself a `TaskQueryImpl` in Java
/// (TaskQueryImpl.java:172-174 orActive/orQueryObjects/currentOrQueryObject).
#[derive(Clone, Default)]
pub struct TaskQueryCriteria {
    process_instance_id: Option<String>,
    task_name: Option<String>,
    task_definition_key: Option<String>,
    task_definition_key_like: Option<String>,
    assignee: Option<String>,
    owner: Option<String>,
    category: Option<String>,
    tenant_id: Option<String>,
    candidate_user: Option<String>,
    candidate_group: Option<String>,
    /// Java `taskInvolvedGroups`: any identity-link group id in this set matches.
    involved_groups: Option<Vec<String>>,
    /// Java `ignoreAssigneeValue`: when true, candidate filters keep assigned tasks.
    /// Default false matches Java: candidate queries exclude tasks that already have an assignee.
    ignore_assignee: bool,
    priority: Option<i32>,
    minimum_priority: Option<i32>,
    maximum_priority: Option<i32>,
    due_date_millis: Option<i64>,
    due_before_millis: Option<i64>,
    due_after_millis: Option<i64>,
    without_due_date: bool,
    // Suspension state filter: None = no filter, Some(0) = active, Some(1) = suspended
    suspension_state: Option<i32>,
}

pub struct TaskQuery {
    state: QueryState<Task>,
    criteria: TaskQueryCriteria,
    /// Java `TaskQueryImpl.orActive` (TaskQueryImpl.java:172): while true,
    /// every filter setter routes into the newest `or()` block.
    or_active: bool,
    /// Java `TaskQueryImpl.orQueryObjects` (TaskQueryImpl.java:173): each
    /// block ANDs with the main criteria; conditions inside a block are OR'd.
    or_query_objects: Vec<TaskQueryCriteria>,
    /// Java throws FlowableException from or()/endOr() misuse immediately
    /// (TaskQueryImpl.java:2049-2050, 2066-2067); the consuming Rust builder
    /// defers the same message to list()/count()/single_result().
    pending_error: Option<String>,
}

impl TaskQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            criteria: TaskQueryCriteria::default(),
            or_active: false,
            or_query_objects: Vec::new(),
            pending_error: None,
        }
    }

    /// Java `TaskQueryImpl` setter routing: while `orActive`, setters write to
    /// `currentOrQueryObject` instead of the main query (e.g.
    /// TaskQueryImpl.java:215-219).
    fn target(&mut self) -> &mut TaskQueryCriteria {
        if self.or_active {
            self.or_query_objects
                .last_mut()
                .expect("or_active implies an open or() block")
        } else {
            &mut self.criteria
        }
    }

    /// Java `TaskQueryImpl.or()` (TaskQueryImpl.java:2048-2062): open an OR
    /// block. Nested `or()` is rejected with the Java message.
    pub fn or(mut self) -> Self {
        if self.or_active {
            self.pending_error = Some("the query is already in an or statement".to_string());
            return self;
        }
        self.or_active = true;
        self.or_query_objects.push(TaskQueryCriteria::default());
        self
    }

    /// Java `TaskQueryImpl.endOr()` (TaskQueryImpl.java:2065-2073).
    pub fn end_or(mut self) -> Self {
        if !self.or_active {
            self.pending_error =
                Some("endOr() can only be called after calling or()".to_string());
            return self;
        }
        self.or_active = false;
        self
    }

    pub fn process_instance_id(mut self, process_instance_id: String) -> Self {
        self.target().process_instance_id = Some(process_instance_id);
        self
    }

    pub fn task_name(mut self, task_name: String) -> Self {
        self.target().task_name = Some(task_name);
        self
    }

    pub fn task_definition_key(mut self, task_definition_key: String) -> Self {
        self.target().task_definition_key = Some(task_definition_key);
        self
    }

    pub fn task_definition_key_like(mut self, task_definition_key_like: String) -> Self {
        self.target().task_definition_key_like = Some(task_definition_key_like);
        self
    }

    pub fn task_assignee(mut self, assignee: String) -> Self {
        self.target().assignee = Some(assignee);
        self
    }

    pub fn task_owner(mut self, owner: String) -> Self {
        self.target().owner = Some(owner);
        self
    }

    pub fn task_category(mut self, category: String) -> Self {
        self.target().category = Some(category);
        self
    }

    pub fn task_tenant_id(mut self, tenant_id: String) -> Self {
        self.target().tenant_id = Some(tenant_id);
        self
    }

    pub fn task_candidate_user(mut self, candidate_user: String) -> Self {
        self.target().candidate_user = Some(candidate_user);
        self
    }

    pub fn task_candidate_group(mut self, candidate_group: String) -> Self {
        self.target().candidate_group = Some(candidate_group);
        self
    }

    /// Java `TaskQuery.taskInvolvedGroups` — match tasks with any identity-link
    /// group id in `involved_groups` (not limited to type `candidate`).
    pub fn task_involved_groups(mut self, involved_groups: Vec<String>) -> Self {
        self.target().involved_groups = Some(involved_groups);
        self
    }

    /// Java `TaskQuery.ignoreAssigneeValue` — keep assigned tasks in candidate queries.
    pub fn ignore_assignee_value(mut self) -> Self {
        self.target().ignore_assignee = true;
        self
    }

    pub fn task_priority(mut self, priority: i32) -> Self {
        self.target().priority = Some(priority);
        self
    }

    pub fn task_minimum_priority(mut self, priority: i32) -> Self {
        self.target().minimum_priority = Some(priority);
        self
    }

    pub fn task_maximum_priority(mut self, priority: i32) -> Self {
        self.target().maximum_priority = Some(priority);
        self
    }

    pub fn task_due_date_millis(mut self, due_date_millis: i64) -> Self {
        self.target().due_date_millis = Some(due_date_millis);
        self
    }

    pub fn task_due_before_millis(mut self, due_before_millis: i64) -> Self {
        self.target().due_before_millis = Some(due_before_millis);
        self
    }

    pub fn task_due_after_millis(mut self, due_after_millis: i64) -> Self {
        self.target().due_after_millis = Some(due_after_millis);
        self
    }

    pub fn task_without_due_date(mut self) -> Self {
        self.target().without_due_date = true;
        self
    }

    pub fn suspended(mut self) -> Self {
        self.target().suspension_state = Some(1);
        self
    }

    pub fn active(mut self) -> Self {
        self.target().suspension_state = Some(0);
        self
    }

    pub fn order_by_task_name(mut self) -> Self {
        self.state.order_by = Some("name".to_string());
        self
    }

    pub fn asc(mut self) -> Self {
        self.state.direction = Direction::Asc;
        self
    }

    pub fn desc(mut self) -> Self {
        self.state.direction = Direction::Desc;
        self
    }
}

pub struct TaskQueryCmd {
    query: TaskQuery,
}

impl TaskQueryCmd {
    pub fn new(query: TaskQuery) -> Self {
        Self { query }
    }
}

impl Command<Vec<Task>> for TaskQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<Task>, crate::error::FlowableError> {
        let mut filters: Vec<(String, FilterOp)> = Vec::new();
        if let Some(pi_id) = &self.query.criteria.process_instance_id {
            filters.push((
                "process_instance_id".into(),
                FilterOp::Eq(Arc::from(pi_id.as_str())),
            ));
        }
        if let Some(name) = &self.query.criteria.task_name {
            filters.push(("name".into(), FilterOp::Eq(Arc::from(name.as_str()))));
        }
        if let Some(task_definition_key) = &self.query.criteria.task_definition_key {
            filters.push((
                "task_definition_key".into(),
                FilterOp::Eq(Arc::from(task_definition_key.as_str())),
            ));
        }
        if let Some(task_definition_key_like) = &self.query.criteria.task_definition_key_like {
            filters.push((
                "task_definition_key".into(),
                FilterOp::Like(Arc::from(task_definition_key_like.as_str())),
            ));
        }
        if let Some(assignee) = &self.query.criteria.assignee {
            filters.push((
                "assignee".into(),
                FilterOp::Eq(Arc::from(assignee.as_str())),
            ));
        }
        if let Some(owner) = &self.query.criteria.owner {
            filters.push(("owner".into(), FilterOp::Eq(Arc::from(owner.as_str()))));
        }
        if let Some(priority) = self.query.criteria.priority {
            filters.push((
                "priority".into(),
                FilterOp::Eq(Arc::from(priority.to_string())),
            ));
        }
        if let Some(minimum_priority) = self.query.criteria.minimum_priority {
            filters.push((
                "priority".into(),
                FilterOp::GreaterThanOrEqual(minimum_priority as i64),
            ));
        }
        if let Some(maximum_priority) = self.query.criteria.maximum_priority {
            filters.push((
                "priority".into(),
                FilterOp::LessThanOrEqual(maximum_priority as i64),
            ));
        }
        if let Some(due_date_millis) = self.query.criteria.due_date_millis {
            filters.push((
                "due_date".into(),
                FilterOp::Eq(Arc::from(due_date_millis.to_string())),
            ));
        }
        if let Some(due_before_millis) = self.query.criteria.due_before_millis {
            filters.push((
                "due_date".into(),
                FilterOp::LessThanOrEqual(due_before_millis),
            ));
        }
        if let Some(due_after_millis) = self.query.criteria.due_after_millis {
            filters.push((
                "due_date".into(),
                FilterOp::GreaterThanOrEqual(due_after_millis),
            ));
        }
        if self.query.criteria.without_due_date {
            filters.push(("due_date".into(), FilterOp::IsNull));
        }

        let order_by: Option<(&str, bool)> = match &self.query.state.order_by {
            Some(col) => Some((
                col.as_str(),
                matches!(self.query.state.direction, Direction::Asc),
            )),
            None => None,
        };

        let mut tasks: Vec<Task> = command_context
            .session()
            .find_with_filters("tasks", &filters, order_by, None)
            .unwrap();

        // Filter by suspension state if set
        if let Some(state) = self.query.criteria.suspension_state {
            tasks.retain(|task| task.suspension_state == state);
        }

        // Identity-link filters: candidate user/group (Java Task.xml candidate
        // EXISTS), involved groups (any link type with matching GROUP_ID_).
        // Batch-load links once to avoid N+1.
        let needs_identity_links = self.query.criteria.candidate_user.is_some()
            || self.query.criteria.candidate_group.is_some()
            || self.query.criteria.involved_groups.is_some();
        if needs_identity_links {
            let task_ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
            let all_links = command_context
                .runtime_store
                .find_identity_links_by_tasks(&task_ids, &mut command_context.session);

            // Java TaskQueryImpl.getCandidateGroups: taskCandidateUser expands the
            // user's group memberships and ORs them with a direct user candidate link.
            // Java Task.xml: candidate filters also require ASSIGNEE_ IS NULL unless
            // ignoreAssigneeValue is set (TaskQueryImpl.ignoreAssigneeValue / Task.xml:868-870).
            if let Some(candidate_user) = &self.query.criteria.candidate_user {
                let candidate_user = candidate_user.clone();
                let user_group_ids: std::collections::HashSet<String> = command_context
                    .runtime_store
                    .get_groups_by_user(&candidate_user, &mut command_context.session)
                    .into_iter()
                    .map(|g| g.id)
                    .collect();
                let matching: std::collections::HashSet<String> = all_links
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
            if let Some(candidate_group) = &self.query.criteria.candidate_group {
                let candidate_group = candidate_group.clone();
                let matching: std::collections::HashSet<String> = all_links
                    .iter()
                    .filter(|l| {
                        l.link_type == "candidate"
                            && l.group_id.as_deref() == Some(candidate_group.as_str())
                    })
                    .filter_map(|l| l.task_id.clone())
                    .collect();
                tasks.retain(|task| matching.contains(&task.id));
            }
            // Java TaskQueryImpl.taskInvolvedGroups / Task.xml:904-919 — any
            // identity-link row whose GROUP_ID_ is in the set (no TYPE_ filter).
            if let Some(involved_groups) = &self.query.criteria.involved_groups {
                let involved: std::collections::HashSet<&str> =
                    involved_groups.iter().map(String::as_str).collect();
                let matching: std::collections::HashSet<String> = all_links
                    .iter()
                    .filter(|l| l.group_id.as_deref().is_some_and(|gid| involved.contains(gid)))
                    .filter_map(|l| l.task_id.clone())
                    .collect();
                tasks.retain(|task| matching.contains(&task.id));
            }

            // Java default for candidateUser/candidateGroup: exclude assigned tasks.
            if !self.query.criteria.ignore_assignee
                && (self.query.criteria.candidate_user.is_some() || self.query.criteria.candidate_group.is_some())
            {
                tasks.retain(|task| task.assignee.is_none());
            }
        }
        if let Some(category) = &self.query.criteria.category {
            tasks.retain(|task| task.category.as_deref() == Some(category.as_str()));
        }
        if let Some(tenant_id) = &self.query.criteria.tenant_id {
            tasks.retain(|task| task.tenant_id.as_deref() == Some(tenant_id.as_str()));
        }

        // Java Task.xml renders each orQueryObject as one parenthesised OR
        // group AND'ed with the main query (TaskQueryImpl.java:172-174
        // orQueryObjects). Evaluate each block in memory over the tasks that
        // already passed the main criteria.
        if !self.query.or_query_objects.is_empty() {
            let needs_links = self.query.or_query_objects.iter().any(|b| {
                b.candidate_user.is_some()
                    || b.candidate_group.is_some()
                    || b.involved_groups.is_some()
            });
            let all_links = if needs_links {
                let task_ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
                command_context
                    .runtime_store
                    .find_identity_links_by_tasks(&task_ids, &mut command_context.session)
            } else {
                Vec::new()
            };
            for block in &self.query.or_query_objects {
                // Java TaskQueryImpl.getCandidateGroups also expands the
                // candidateUser's group memberships inside an or-block.
                let candidate_user_groups: std::collections::HashSet<String> =
                    match &block.candidate_user {
                        Some(user) => command_context
                            .runtime_store
                            .get_groups_by_user(user, &mut command_context.session)
                            .into_iter()
                            .map(|g| g.id)
                            .collect(),
                        None => std::collections::HashSet::new(),
                    };
                tasks.retain(|task| {
                    or_block_matches(block, task, &all_links, &candidate_user_groups)
                });
            }
        }

        if order_by.is_none() {
            tasks.sort_by(|a, b| a.id.cmp(&b.id));
        }

        Ok(tasks)
    }
}

/// In-memory SQL LIKE matcher for or-block evaluation: `%` matches any run
/// of characters, `_` matches exactly one (same semantics the DB backend
/// applies to `FilterOp::Like` for the main criteria).
fn like_matches(pattern: &str, value: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, value)
}

/// Evaluate one `or()` block against a task: every condition set inside the
/// block is one OR term (Java Task.xml or-block rendering); a block with no
/// conditions matches everything. Candidate terms keep the Java assignee-null
/// gate (Task.xml:868-870) unless the block sets `ignoreAssigneeValue`.
fn or_block_matches(
    block: &TaskQueryCriteria,
    task: &Task,
    all_links: &[crate::identity::entities::IdentityLink],
    candidate_user_groups: &std::collections::HashSet<String>,
) -> bool {
    let mut any_condition = false;

    if let Some(v) = &block.process_instance_id {
        any_condition = true;
        if task.process_instance_id == *v {
            return true;
        }
    }
    if let Some(v) = &block.task_name {
        any_condition = true;
        if task.name == *v {
            return true;
        }
    }
    if let Some(v) = &block.task_definition_key {
        any_condition = true;
        if task.task_definition_key == *v {
            return true;
        }
    }
    if let Some(v) = &block.task_definition_key_like {
        any_condition = true;
        if like_matches(v, &task.task_definition_key) {
            return true;
        }
    }
    if let Some(v) = &block.assignee {
        any_condition = true;
        if task.assignee.as_deref() == Some(v.as_str()) {
            return true;
        }
    }
    if let Some(v) = &block.owner {
        any_condition = true;
        if task.owner.as_deref() == Some(v.as_str()) {
            return true;
        }
    }
    if let Some(v) = &block.category {
        any_condition = true;
        if task.category.as_deref() == Some(v.as_str()) {
            return true;
        }
    }
    if let Some(v) = &block.tenant_id {
        any_condition = true;
        if task.tenant_id.as_deref() == Some(v.as_str()) {
            return true;
        }
    }
    if let Some(v) = block.priority {
        any_condition = true;
        if task.priority == Some(v) {
            return true;
        }
    }
    if let Some(v) = block.minimum_priority {
        any_condition = true;
        if task.priority.is_some_and(|p| p >= v) {
            return true;
        }
    }
    if let Some(v) = block.maximum_priority {
        any_condition = true;
        if task.priority.is_some_and(|p| p <= v) {
            return true;
        }
    }
    if let Some(v) = block.due_date_millis {
        any_condition = true;
        if task.due_date.is_some_and(|d| d.timestamp_millis() == v) {
            return true;
        }
    }
    if let Some(v) = block.due_before_millis {
        any_condition = true;
        if task.due_date.is_some_and(|d| d.timestamp_millis() <= v) {
            return true;
        }
    }
    if let Some(v) = block.due_after_millis {
        any_condition = true;
        if task.due_date.is_some_and(|d| d.timestamp_millis() >= v) {
            return true;
        }
    }
    if block.without_due_date {
        any_condition = true;
        if task.due_date.is_none() {
            return true;
        }
    }
    if let Some(v) = block.suspension_state {
        any_condition = true;
        if task.suspension_state == v {
            return true;
        }
    }

    // Candidate/involved terms need identity links for this task.
    let assignee_gate = task.assignee.is_none() || block.ignore_assignee;
    if let Some(candidate_user) = &block.candidate_user {
        any_condition = true;
        // Java TaskQueryImpl.getCandidateGroups: direct user candidate link OR
        // a candidate link for any of the user's groups (P49 semantics).
        let matched = all_links.iter().any(|l| {
            l.task_id.as_deref() == Some(task.id.as_str())
                && l.link_type == "candidate"
                && (l.user_id.as_deref() == Some(candidate_user.as_str())
                    || l.group_id
                        .as_ref()
                        .is_some_and(|gid| candidate_user_groups.contains(gid)))
        });
        if matched && assignee_gate {
            return true;
        }
    }
    if let Some(candidate_group) = &block.candidate_group {
        any_condition = true;
        let matched = all_links.iter().any(|l| {
            l.task_id.as_deref() == Some(task.id.as_str())
                && l.link_type == "candidate"
                && l.group_id.as_deref() == Some(candidate_group.as_str())
        });
        if matched && assignee_gate {
            return true;
        }
    }
    if let Some(involved_groups) = &block.involved_groups {
        any_condition = true;
        // Java Task.xml:904-919 — any identity-link row with GROUP_ID_ in the
        // set, regardless of TYPE_.
        let involved: std::collections::HashSet<&str> =
            involved_groups.iter().map(String::as_str).collect();
        let matched = all_links.iter().any(|l| {
            l.task_id.as_deref() == Some(task.id.as_str())
                && l.group_id.as_deref().is_some_and(|gid| involved.contains(gid))
        });
        if matched {
            return true;
        }
    }

    // Empty or() block constrains nothing.
    !any_condition
}

impl Query<Task, TaskQuery> for TaskQuery {
    fn list(&self) -> Result<Vec<Task>, crate::error::FlowableError> {
        // Java throws FlowableException from or()/endOr() misuse immediately
        // (TaskQueryImpl.java:2049-2050, 2066-2067); the consuming Rust
        // builder surfaces the same message at execution time.
        if let Some(msg) = &self.pending_error {
            return Err(crate::error::FlowableError::Generic(msg.clone()));
        }
        // We need to clone the query because TaskQueryCmd takes ownership.
        // This is a bit inefficient, but following the current Command pattern.
        // Alternatively, we could change TaskQueryCmd to take &TaskQuery.
        let query_clone = TaskQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            criteria: self.criteria.clone(),
            or_active: self.or_active,
            or_query_objects: self.or_query_objects.clone(),
            pending_error: None,
        };
        let cmd = TaskQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<Task>, crate::error::FlowableError> {
        let mut list = self.list()?;
        if list.len() > 1 {
            return Err(crate::error::FlowableError::Generic(
                "Query returned more than one result".to_string(),
            ));
        }
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

pub struct QueryTasksByProcessInstanceCmd {
    process_instance_id: String,
}

impl QueryTasksByProcessInstanceCmd {
    pub fn new(process_instance_id: String) -> Self {
        Self {
            process_instance_id,
        }
    }
}

impl Command<Vec<Task>> for QueryTasksByProcessInstanceCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<Task>, crate::error::FlowableError> {
        let mut tasks = command_context
            .task_entity_manager
            .find_by_process_instance_id(&self.process_instance_id, &mut command_context.session);

        tasks.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(tasks)
    }
}

pub struct QuerySubTasksCmd {
    parent_task_id: String,
}

impl QuerySubTasksCmd {
    pub fn new(parent_task_id: String) -> Self {
        Self { parent_task_id }
    }
}

impl Command<Vec<Task>> for QuerySubTasksCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<Task>, crate::error::FlowableError> {
        let mut tasks = command_context
            .task_entity_manager
            .find_by_parent_task_id(&self.parent_task_id, &mut command_context.session);

        tasks.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(tasks)
    }
}

fn set_execution_variable_for_task_complete(
    command_context: &mut CommandContext,
    execution_id: &str,
    name: String,
    value: serde_json::Value,
) -> Result<(), crate::error::FlowableError> {
    let execution = command_context
        .runtime_store
        .find_execution(execution_id, &mut command_context.session)
        .ok_or_else(|| {
            crate::error::FlowableError::NotFound(format!(
                "Execution '{}' was not found",
                execution_id
            ))
        })?;

    // Java TaskHelper.completeTask → execution.setVariables (non-local): walk
    // the VariableScope chain so the name lands on the process-instance scope
    // (or an ancestor that already owns it). Writing only on the task child
    // row would hide values from siblings — e.g. ad-hoc completionCondition
    // `${completed}` must still evaluate true after a later sibling leaves
    // (AdhocSubProcessTest.testKeepRemainingInstancesAdhocSubProcess).
    let process_instance_id = execution
        .process_instance_id
        .clone()
        .unwrap_or_else(|| execution.id.clone());
    let target_id = if process_instance_id != execution.id {
        process_instance_id.clone()
    } else {
        execution.id.clone()
    };
    let mut target = command_context
        .runtime_store
        .find_execution(&target_id, &mut command_context.session)
        .ok_or_else(|| {
            crate::error::FlowableError::NotFound(format!(
                "Execution '{}' was not found",
                target_id
            ))
        })?;

    target.set_process_variable(name.clone(), value.clone());
    command_context
        .execution_entity_manager
        .update(&target, &mut command_context.session);

    let id = format!("{}:{}", target.id, name);
    if command_context
        .runtime_store
        .get_historic_variable_instance(&id, &mut command_context.session)
        .is_some()
    {
        command_context.history_manager.record_variable_updated(
            &id,
            value,
            &mut command_context.session,
        );
    } else {
        command_context.history_manager.record_variable_created(
            &id,
            &name,
            crate::engine::variable_service::variable_type_name(&value),
            value,
            &process_instance_id,
            Some(&target.id),
            None,
            &mut command_context.session,
        );
    }

    Ok(())
}

fn set_transient_execution_variable_for_task_complete(
    command_context: &mut CommandContext,
    execution_id: &str,
    name: String,
    value: serde_json::Value,
) -> Result<(), crate::error::FlowableError> {
    let mut execution = command_context
        .runtime_store
        .find_execution(execution_id, &mut command_context.session)
        .ok_or_else(|| {
            crate::error::FlowableError::NotFound(format!(
                "Execution '{}' was not found",
                execution_id
            ))
        })?;

    execution.set_process_variable(name, value);
    command_context
        .execution_entity_manager
        .update(&execution, &mut command_context.session);
    Ok(())
}

pub(crate) fn record_task_local_variable(
    command_context: &mut CommandContext,
    task: &Task,
    name: &str,
    value: serde_json::Value,
) {
    let id = format!("{}:{}", task.id, name);
    if command_context
        .runtime_store
        .get_historic_variable_instance(&id, &mut command_context.session)
        .is_some()
    {
        command_context.history_manager.record_variable_updated(
            &id,
            value,
            &mut command_context.session,
        );
    } else {
        command_context.history_manager.record_variable_created(
            &id,
            name,
            crate::engine::variable_service::variable_type_name(&value),
            value,
            &task.process_instance_id,
            Some(&task.execution_id),
            Some(&task.id),
            &mut command_context.session,
        );
    }
}

fn set_task_local_variable_for_task_complete(
    command_context: &mut CommandContext,
    task: &mut Task,
    name: String,
    value: serde_json::Value,
) {
    task.set_local_variable(name.clone(), value.clone());
    record_task_local_variable(command_context, task, &name, value);
}

pub(crate) fn complete_task_internal(
    command_context: &mut CommandContext,
    task: Task,
) -> Result<(), crate::error::FlowableError> {
    // Fire task complete listeners before the task is removed.
    fire_task_listeners_for_event(command_context, &task, "complete")?;

    let mut completed_task = task.clone();
    completed_task.mark_completed();
    command_context
        .task_entity_manager
        .update(&completed_task, &mut command_context.session);
    command_context
        .task_entity_manager
        .delete(&task.id, &mut command_context.session);

    // P53 layer 1: dispatch `TASK_COMPLETED` after the complete listener has
    // run and the task row is gone (Java `TaskHelper.deleteTask` flow). The
    // task id is still valid in the event payload even though the row was
    // just removed.
    crate::engine::event_dispatcher::dispatch_task_completed(
        command_context,
        &task.id,
        Some(&task.process_instance_id),
        Some(&task.execution_id),
    );

    command_context
        .history_manager
        .record_task_end(&task.id, None, &mut command_context.session);

    command_context.history_manager.record_audit_event(
        "complete",
        Some(&task.process_instance_id),
        None,
        Some(&format!("Task {} completed", task.id)),
        &mut command_context.session,
    );

    command_context
        .runtime_store
        .delete_event_wait_state_by_execution_id(&task.execution_id, &mut command_context.session);
    // Clean up any boundary event states associated with this execution
    command_context
        .runtime_store
        .delete_boundary_event_states_by_host_execution_id(
            &task.execution_id,
            &mut command_context.session,
        );
    command_context
        .runtime_store
        .delete_timer_job_states_by_execution_id(&task.execution_id, &mut command_context.session);

    let execution = match command_context
        .execution_entity_manager
        .find_by_id(&task.execution_id, &mut command_context.session)
    {
        Some(execution) => execution.clone(),
        None => {
            tracing::warn!(
                "No execution found for task {} with execution id {}",
                task.id,
                task.execution_id
            );
            return Ok(());
        }
    };

    // Apply data output associations
    if let Some(act_id) = &execution.activity_id
        && let Some(pd_id) = &execution.process_definition_id
    {
        let data_output_associations = {
            let mut assoc = Vec::new();
            if let Some(model) = command_context.deployment_manager.get_bpmn_model(pd_id)
                && let Some(process) = &model.main_process
                && let Some(flowable_bpmn_model::model::FlowElementEnum::UserTask(ut)) =
                    crate::agenda::continue_process_operation::find_flow_element(process, act_id)
            {
                assoc = ut.task.activity.data_output_associations.clone();
            }
            assoc
        };

        if !data_output_associations.is_empty() {
            let task_vars = execution.process_variables();

            let process_instance_id = execution.process_instance_id.clone().unwrap_or_default();
            let mut process_vars = HashMap::new();
            if let Some(root_exec) = command_context
                .execution_entity_manager
                .find_by_id(&process_instance_id, &mut command_context.session)
            {
                process_vars = root_exec.process_variables();
            }

            match crate::engine::data_routing::DataRoutingService::apply_data_output_associations(
                &data_output_associations,
                &task_vars,
                &mut process_vars,
            ) {
                Ok(_) => {
                    if let Some(mut root_exec) = command_context
                        .execution_entity_manager
                        .find_by_id(&process_instance_id, &mut command_context.session)
                    {
                        root_exec.set_process_variables(process_vars);
                        command_context
                            .execution_entity_manager
                            .update(&root_exec, &mut command_context.session);
                    }
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
    }

    if should_complete_adhoc_parent_on_task_completion(command_context, &execution)
        && let Some(parent_id) = execution.parent_id.as_deref()
        && let Some(mut parent_execution) = command_context
            .runtime_store
            .find_execution(parent_id, &mut command_context.session)
    {
        let behavior = crate::bpmn::behavior::adhoc_subprocess_activity_behavior::AdhocSubProcessActivityBehavior::new();
        return behavior.complete_task(
            &mut parent_execution,
            command_context,
            &task.task_definition_key,
        );
    }

    let mut current_execution = command_context
        .execution_entity_manager
        .find_by_id(&task.execution_id, &mut command_context.session)
        .unwrap_or(execution);

    // Default: mark this execution ended (non-MI / parallel-instance leave /
    // final sequential leave). Sequential MI continue reuses the same child
    // (Java `continueSequentialMultiInstance`) and overrides this below.
    current_execution.is_active = true;
    current_execution.is_ended = true;

    if let Some(pi_id) = &current_execution.process_instance_id {
        let executions = command_context
            .runtime_store
            .snapshot_executions(&mut command_context.session);
        let active_siblings = executions
            .into_values()
            .filter(|e| {
                e.process_instance_id.as_deref() == Some(pi_id)
                    && e.id != current_execution.id
                    && e.parent_id == current_execution.parent_id
                    && !e.is_ended
            })
            .count();

        if let Some(parent_id) = &current_execution.parent_id {
            let parent = command_context
                .runtime_store
                .find_execution(parent_id, &mut command_context.session);
            if let Some(mut p) = parent {
                // Java multi-instance leave only applies when the parent is the
                // dedicated MI root (`isMultiInstanceRoot`). Looking up loop
                // characteristics by parent activity alone is wrong after
                // cleanupMiRoot: a leave/after-MI child may hang under the PI
                // which still carries the MI activity id.
                let mut mi_characteristics = None;
                if p.is_multi_instance_root
                    && let Some(act_id) = &p.activity_id
                    && let Some(pd_id) = &p.process_definition_id
                    && let Some(model) = command_context.deployment_manager.get_bpmn_model(pd_id)
                    && let Some(process) = &model.main_process
                {
                    for el in &process.flow_elements {
                        match el {
                            flowable_bpmn_model::model::FlowElementEnum::UserTask(ut)
                                if ut
                                    .task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .as_deref()
                                    == Some(act_id) =>
                            {
                                mi_characteristics = ut.task.activity.loop_characteristics.clone();
                                break;
                            }
                            flowable_bpmn_model::model::FlowElementEnum::ServiceTask(st)
                                if st
                                    .task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .as_deref()
                                    == Some(act_id) =>
                            {
                                mi_characteristics = st.task.activity.loop_characteristics.clone();
                                break;
                            }
                            _ => {}
                        }
                    }
                }

                if let Some(mi) = mi_characteristics {
                    apply_multi_instance_variable_aggregations(
                        &mi,
                        &mut p,
                        &current_execution,
                        &task,
                    )?;

                    let nr_of_instances = p
                        .process_variable("nrOfInstances")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let nr_of_completed = p
                        .process_variable("nrOfCompletedInstances")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        + 1;

                    // Java parity: nrOf* bookkeeping is execution-local on the
                    // MI root (`MultiInstanceActivityBehavior#setLoopVariable`
                    // → `setVariableLocal`).
                    p.set_local_variable(
                        "nrOfCompletedInstances".to_string(),
                        nr_of_completed.into(),
                    );

                    if mi.sequential {
                        let complete_condition = multi_instance_completion_condition_satisfied(
                            command_context,
                            &mi,
                            &p,
                        )?;
                        let more_rounds = nr_of_completed < nr_of_instances;

                        if complete_condition || !more_rounds {
                            // Final sequential leave: end the reused child, then
                            // leave the MI root (Java super.leave → cleanupMiRoot).
                            crate::bpmn::behavior::multi_instance_support::record_mi_child_activity_end(
                                command_context,
                                &current_execution,
                            );
                            current_execution.is_ended = true;
                            current_execution.is_active = false;
                            command_context
                                .execution_entity_manager
                                .update(&current_execution, &mut command_context.session);
                            command_context
                                .execution_entity_manager
                                .update(&p, &mut command_context.session);
                            // SequentialMultiInstanceBehavior.java:90-97.
                            crate::bpmn::behavior::multi_instance_support::cleanup_mi_root_and_leave(
                                &p,
                                command_context,
                                complete_condition,
                            );
                            return Ok(());
                        }

                        // Java `continueSequentialMultiInstance`: keep the same
                        // child execution alive. The resume index is derived
                        // from nrOfCompletedInstances on re-entry — Java keeps
                        // no loopCounter on the MI root. Locals are cleared and
                        // re-applied in `execute_sequential`.
                        crate::bpmn::behavior::multi_instance_support::record_mi_child_activity_end(
                            command_context,
                            &current_execution,
                        );
                        current_execution.is_ended = false;
                        current_execution.is_active = true;
                        command_context
                            .execution_entity_manager
                            .update(&current_execution, &mut command_context.session);
                        command_context
                            .execution_entity_manager
                            .update(&p, &mut command_context.session);
                        command_context.agenda.plan_continue_process_operation(p);
                        return Ok(());
                    } else {
                        // Parallel instance leave: this child is done.
                        crate::bpmn::behavior::multi_instance_support::record_mi_child_activity_end(
                            command_context,
                            &current_execution,
                        );
                        current_execution.is_ended = true;
                        current_execution.is_active = false;
                        command_context
                            .execution_entity_manager
                            .update(&current_execution, &mut command_context.session);

                        let nr_of_active = p
                            .process_variable("nrOfActiveInstances")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                            - 1;
                        p.set_local_variable(
                            "nrOfActiveInstances".to_string(),
                            nr_of_active.into(),
                        );
                        if multi_instance_completion_condition_satisfied(command_context, &mi, &p)?
                        {
                            cancel_remaining_multi_instance_children(
                                command_context,
                                pi_id,
                                &current_execution,
                            );
                        } else if active_siblings > 0 {
                            command_context
                                .execution_entity_manager
                                .update(&p, &mut command_context.session);
                            return Ok(());
                        }
                    }

                    // All parallel instances completed or last sibling: Java
                    // `super.leave` → `cleanupMiRoot`.
                    // ParallelMultiInstanceBehavior.java:302-319 — pass whether
                    // the completion condition was already satisfied above.
                    let with_condition = multi_instance_completion_condition_satisfied(
                        command_context,
                        &mi,
                        &p,
                    )?;
                    command_context
                        .execution_entity_manager
                        .update(&p, &mut command_context.session);
                    crate::bpmn::behavior::multi_instance_support::cleanup_mi_root_and_leave(
                        &p,
                        command_context,
                        with_condition,
                    );
                    return Ok(());
                }
            }
        }
    }

    // Non-MI leave: persist the ended mark, then take outgoing flows.
    command_context
        .execution_entity_manager
        .update(&current_execution, &mut command_context.session);
    let latest_execution = command_context
        .execution_entity_manager
        .find_by_id(&task.execution_id, &mut command_context.session)
        .unwrap_or(current_execution);
    command_context
        .agenda
        .plan_take_outgoing_sequence_flows_operation(latest_execution);
    Ok(())
}

fn should_complete_adhoc_parent_on_task_completion(
    command_context: &mut CommandContext,
    execution: &crate::runtime::execution::Execution,
) -> bool {
    let Some(parent_id) = execution.parent_id.as_deref() else {
        return false;
    };
    let Some(parent_execution) = command_context
        .runtime_store
        .find_execution(parent_id, &mut command_context.session)
    else {
        return false;
    };
    let Some(parent_activity_id) = parent_execution.activity_id.as_deref() else {
        return false;
    };
    let Some(activity_id) = execution.activity_id.as_deref() else {
        return false;
    };
    let Some(process_definition_id) = execution.process_definition_id.as_deref() else {
        return false;
    };

    let Some(model) = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)
    else {
        return false;
    };
    let Some(process) = model.main_process.as_ref() else {
        return false;
    };
    let Some(parent_element) =
        crate::agenda::continue_process_operation::find_flow_element(process, parent_activity_id)
    else {
        return false;
    };
    if !matches!(parent_element, FlowElementEnum::AdhocSubProcess(_)) {
        return false;
    }

    let Some(child_element) =
        crate::agenda::continue_process_operation::find_flow_element(process, activity_id)
    else {
        return false;
    };

    flow_element_outgoing_count(child_element) == Some(0)
}

fn flow_element_outgoing_count(flow_element: &FlowElementEnum) -> Option<usize> {
    match flow_element {
        FlowElementEnum::Task(task) => Some(task.activity.flow_node.outgoing_flows.len()),
        FlowElementEnum::UserTask(task) => Some(task.task.activity.flow_node.outgoing_flows.len()),
        FlowElementEnum::ServiceTask(task) => {
            Some(task.task.activity.flow_node.outgoing_flows.len())
        }
        FlowElementEnum::CaseServiceTask(task) => {
            Some(task.service_task.task.activity.flow_node.outgoing_flows.len())
        }
        FlowElementEnum::ScriptTask(task) => {
            Some(task.task.activity.flow_node.outgoing_flows.len())
        }
        FlowElementEnum::ManualTask(task) => {
            Some(task.task.activity.flow_node.outgoing_flows.len())
        }
        FlowElementEnum::ReceiveTask(task) => {
            Some(task.task.activity.flow_node.outgoing_flows.len())
        }
        FlowElementEnum::BusinessRuleTask(task) => {
            Some(task.task.activity.flow_node.outgoing_flows.len())
        }
        FlowElementEnum::SubProcess(sub_process) => {
            Some(sub_process.activity.flow_node.outgoing_flows.len())
        }
        _ => None,
    }
}

fn apply_multi_instance_variable_aggregations(
    mi: &MultiInstanceLoopCharacteristics,
    parent_execution: &mut crate::runtime::execution::Execution,
    child_execution: &crate::runtime::execution::Execution,
    task: &Task,
) -> Result<(), crate::error::FlowableError> {
    let Some(aggregation_definitions) = mi.aggregations.as_ref() else {
        return Ok(());
    };

    for aggregation in &aggregation_definitions.aggregations {
        apply_multi_instance_variable_aggregation(
            aggregation,
            parent_execution,
            child_execution,
            task,
        )?;
    }
    for aggregation in &aggregation_definitions.overview_aggregations {
        apply_multi_instance_variable_aggregation(
            aggregation,
            parent_execution,
            child_execution,
            task,
        )?;
    }

    Ok(())
}

fn apply_multi_instance_variable_aggregation(
    aggregation: &VariableAggregationDefinition,
    parent_execution: &mut crate::runtime::execution::Execution,
    child_execution: &crate::runtime::execution::Execution,
    task: &Task,
) -> Result<(), crate::error::FlowableError> {
    if aggregation.implementation_type.is_some() {
        return Ok(());
    }

    let Some(target_variable) = resolve_aggregation_target(aggregation, child_execution) else {
        return Ok(());
    };

    let mut item = Map::new();
    for variable in &aggregation.definitions {
        if let Some(target) = resolve_aggregation_variable_target(variable, child_execution) {
            item.insert(
                target,
                resolve_aggregation_variable_value(variable, child_execution, task),
            );
        }
    }

    let mut aggregate = match parent_execution.process_variable(&target_variable) {
        Some(Value::Array(values)) => values,
        Some(value) => {
            return Err(crate::error::FlowableError::Generic(format!(
                "Multi-instance variable aggregation target '{}' must be an array, got {}",
                target_variable,
                crate::engine::variable_service::variable_type_name(&value)
            )));
        }
        None => Vec::new(),
    };
    aggregate.push(Value::Object(item));

    if aggregation.store_as_transient_variable {
        parent_execution.set_transient_variable(target_variable, Value::Array(aggregate));
    } else {
        parent_execution.set_process_variable(target_variable, Value::Array(aggregate));
    }

    Ok(())
}

fn resolve_aggregation_target(
    aggregation: &VariableAggregationDefinition,
    execution: &crate::runtime::execution::Execution,
) -> Option<String> {
    aggregation
        .target
        .as_deref()
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| expression_string_value(aggregation.target_expression.as_deref(), execution))
}

fn resolve_aggregation_variable_target(
    variable: &VariableAggregationDefinitionVariable,
    execution: &crate::runtime::execution::Execution,
) -> Option<String> {
    variable
        .target
        .as_deref()
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| expression_string_value(variable.target_expression.as_deref(), execution))
        .or_else(|| {
            variable
                .source
                .as_deref()
                .map(str::trim)
                .filter(|source| !source.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn resolve_aggregation_variable_value(
    variable: &VariableAggregationDefinitionVariable,
    execution: &crate::runtime::execution::Execution,
    task: &Task,
) -> Value {
    if let Some(source_expression) = variable.source_expression.as_ref()
        && let Some(value) = SimpleExpression::new(source_expression.clone()).get_value(execution)
    {
        return value;
    }

    let Some(source) = variable
        .source
        .as_deref()
        .map(str::trim)
        .filter(|source| !source.is_empty())
    else {
        return Value::Null;
    };

    task.local_variable(source)
        .or_else(|| execution.process_variable(source))
        .unwrap_or(Value::Null)
}

fn expression_string_value(
    expression: Option<&str>,
    execution: &crate::runtime::execution::Execution,
) -> Option<String> {
    let expression = expression?.trim();
    if expression.is_empty() {
        return None;
    }

    match SimpleExpression::new(expression.to_string()).get_value(execution)? {
        Value::String(value) => Some(value),
        value if !value.is_null() => Some(value.to_string()),
        _ => None,
    }
}

fn multi_instance_completion_condition_satisfied(
    command_context: &mut CommandContext,
    mi: &MultiInstanceLoopCharacteristics,
    execution: &crate::runtime::execution::Execution,
) -> Result<bool, crate::error::FlowableError> {
    let Some(condition) = &mi.completion_condition else {
        return Ok(false);
    };

    // Java parity: the completion condition is evaluated with
    // `expressionManager.createExpression(…).getValue(execution)`, and EL
    // variable resolution walks the VariableScope parent chain (see the P4-7a
    // evaluation execution).
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);
    match SimpleExpression::new(condition.clone()).get_value(&evaluation_execution) {
        Some(serde_json::Value::Bool(value)) => Ok(value),
        Some(value) => Err(crate::error::FlowableError::Generic(format!(
            "Multi-instance completionCondition must evaluate to a boolean, got {value}"
        ))),
        None => Ok(false),
    }
}

fn cancel_remaining_multi_instance_children(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    completed_execution: &crate::runtime::execution::Execution,
) {
    let sibling_execution_ids = command_context
        .runtime_store
        .snapshot_executions(&mut command_context.session)
        .into_values()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && execution.parent_id == completed_execution.parent_id
                && execution.id != completed_execution.id
                && !execution.is_ended
        })
        .map(|execution| execution.id)
        .collect::<Vec<_>>();

    for execution_id in sibling_execution_ids {
        if let Some(task) = command_context
            .task_entity_manager
            .find_by_execution_id(&execution_id, &mut command_context.session)
        {
            let mut cancelled_task = task.clone();
            cancelled_task.mark_completed();
            command_context
                .task_entity_manager
                .update(&cancelled_task, &mut command_context.session);
            command_context
                .task_entity_manager
                .delete(&task.id, &mut command_context.session);
            command_context.history_manager.record_task_end(
                &task.id,
                Some("multi-instance completion condition"),
                &mut command_context.session,
            );
        }

        command_context
            .runtime_store
            .delete_event_wait_state_by_execution_id(&execution_id, &mut command_context.session);
        command_context
            .runtime_store
            .delete_boundary_event_states_by_host_execution_id(
                &execution_id,
                &mut command_context.session,
            );
        command_context
            .runtime_store
            .delete_timer_job_states_by_execution_id(&execution_id, &mut command_context.session);

        if let Some(mut execution) = command_context
            .execution_entity_manager
            .find_by_id(&execution_id, &mut command_context.session)
        {
            execution.is_active = false;
            execution.is_ended = true;
            command_context
                .execution_entity_manager
                .update(&execution, &mut command_context.session);
        }
    }
}

struct WakeUpMessageByProcessInstanceIdCmd {
    process_instance_id: String,
}

impl WakeUpMessageByProcessInstanceIdCmd {
    pub fn new(process_instance_id: String) -> Self {
        Self {
            process_instance_id,
        }
    }
}

impl Command<()> for WakeUpMessageByProcessInstanceIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut wait_states = command_context
            .runtime_store
            .find_event_wait_states_by_process_instance_id(
                &self.process_instance_id,
                &mut command_context.session,
            );
        wait_states.sort_by(|left, right| left.execution_id.cmp(&right.execution_id));

        let wait_state = match wait_states
            .into_iter()
            .find(|wait_state| matches!(&wait_state.wait_kind, RuntimeEventWaitKind::ReceiveTask))
        {
            Some(wait_state) => wait_state,
            None => {
                return Ok(());
            }
        };

        let task = match wait_state.task_id.as_deref() {
            Some(task_id) => command_context
                .task_entity_manager
                .find_task_by_id(task_id, &mut command_context.session),
            None => None,
        };

        if let Some(task) = task {
            complete_task_internal(command_context, task)?
        }
        Ok(())
    }
}

pub struct WakeUpMessageByMessageRefCmd {
    process_instance_id: String,
    message_ref: String,
}

impl WakeUpMessageByMessageRefCmd {
    pub fn new(process_instance_id: String, message_ref: String) -> Self {
        Self {
            process_instance_id,
            message_ref,
        }
    }
}

impl Command<()> for WakeUpMessageByMessageRefCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut wait_states = command_context
            .runtime_store
            .find_event_wait_states_by_process_instance_id(
                &self.process_instance_id,
                &mut command_context.session,
            );
        wait_states.sort_by(|left, right| left.execution_id.cmp(&right.execution_id));

        let wait_state = match wait_states.into_iter().find(|ws| {
            matches!(ws.wait_kind, RuntimeEventWaitKind::ReceiveTask)
                && ws
                    .event_subscription
                    .as_ref()
                    .map(|sub| {
                        sub.kind == EventSubscriptionKind::Message
                            && sub.event_ref == self.message_ref
                    })
                    .unwrap_or(false)
        }) {
            Some(wait_state) => wait_state,
            None => {
                return Ok(());
            }
        };

        let task = match wait_state.task_id.as_deref() {
            Some(task_id) => command_context
                .task_entity_manager
                .find_task_by_id(task_id, &mut command_context.session),
            None => None,
        };

        if let Some(task) = task {
            complete_task_internal(command_context, task)?
        }
        Ok(())
    }
}

pub struct QueryEventWaitStatesByProcessInstanceIdCmd {
    process_instance_id: String,
}

impl QueryEventWaitStatesByProcessInstanceIdCmd {
    pub fn new(process_instance_id: String) -> Self {
        Self {
            process_instance_id,
        }
    }
}

/// Type alias for callers that depend on the older name
pub type QueryMessageStyleWaitStatesByProcessInstanceIdCmd =
    QueryEventWaitStatesByProcessInstanceIdCmd;

impl Command<Vec<EventWaitState>> for QueryEventWaitStatesByProcessInstanceIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<EventWaitState>, crate::error::FlowableError> {
        let mut states: Vec<EventWaitState> = command_context
            .runtime_store
            .find_event_wait_states_by_process_instance_id(
                &self.process_instance_id,
                &mut command_context.session,
            )
            .into_iter()
            .map(EventWaitState::from)
            .collect();

        states.sort_by_key(|state| {
            (
                state.wait_kind.clone(),
                state.process_instance_id.clone(),
                state.execution_id.clone(),
                state.task_id.clone(),
            )
        });

        Ok(states)
    }
}

pub struct CompleteTaskByIdCmd {
    task_id: String,
    variables: HashMap<String, serde_json::Value>,
    transient_variables: HashMap<String, serde_json::Value>,
    local_scope: bool,
}

impl CompleteTaskByIdCmd {
    pub fn new(task_id: String) -> Self {
        Self {
            task_id,
            variables: HashMap::new(),
            transient_variables: HashMap::new(),
            local_scope: false,
        }
    }

    pub fn with_variables(
        task_id: String,
        variables: HashMap<String, serde_json::Value>,
        local_scope: bool,
    ) -> Self {
        Self {
            task_id,
            variables,
            transient_variables: HashMap::new(),
            local_scope,
        }
    }

    pub fn with_variable_maps(
        task_id: String,
        variables: HashMap<String, serde_json::Value>,
        transient_variables: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            task_id,
            variables,
            transient_variables,
            local_scope: false,
        }
    }
}

impl Command<()> for CompleteTaskByIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        complete_task_by_id_in_context(
            command_context,
            &self.task_id,
            &self.variables,
            &self.transient_variables,
            self.local_scope,
        )
    }
}

/// Complete a task with optional variables inside an existing command session.
///
/// Java: `CompleteTaskCmd` / tail of `CompleteTaskWithFormCmd` after form
/// processing (`TaskHelper.completeTask`). Exposed so form-service can run
/// form-instance persistence + variable write + complete in one session.
pub fn complete_task_by_id_in_context(
    command_context: &mut CommandContext,
    task_id: &str,
    variables: &HashMap<String, serde_json::Value>,
    transient_variables: &HashMap<String, serde_json::Value>,
    local_scope: bool,
) -> Result<(), crate::error::FlowableError> {
    let mut task = match command_context
        .task_entity_manager
        .find_task_by_id(task_id, &mut command_context.session)
    {
        Some(task) => task,
        None => {
            return Err(crate::error::FlowableError::NotFound(format!(
                "No task found for task id {}",
                task_id
            )));
        }
    };

    // Java NeedsActiveTaskCmd / CompleteTaskWithFormCmd suspended prefix
    // "Cannot complete" — Rust uses the shared require_active_task message.
    require_active_task_with_prefix(&task, "Cannot complete")?;

    if local_scope {
        for (name, value) in variables {
            set_task_local_variable_for_task_complete(
                command_context,
                &mut task,
                name.clone(),
                value.clone(),
            );
        }
    } else {
        for (name, value) in variables {
            set_execution_variable_for_task_complete(
                command_context,
                &task.execution_id,
                name.clone(),
                value.clone(),
            )?;
        }
    }
    for (name, value) in transient_variables {
        set_transient_execution_variable_for_task_complete(
            command_context,
            &task.execution_id,
            name.clone(),
            value.clone(),
        )?;
    }

    complete_task_internal(command_context, task)?;
    Ok(())
}

pub struct SetTaskLocalVariableCmd {
    task_id: String,
    name: String,
    value: serde_json::Value,
}

impl SetTaskLocalVariableCmd {
    pub fn new(task_id: String, name: String, value: serde_json::Value) -> Self {
        Self {
            task_id,
            name,
            value,
        }
    }
}

impl Command<()> for SetTaskLocalVariableCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // Java parity: TaskService.setVariableLocal delegates to the shared
        // task-variable mutation as an upsert on the local scope.
        crate::cmd::task_variable_cmd::mutate_task_variables(
            command_context,
            &self.task_id,
            TaskVariableScope::Local,
            VariableMutationMode::Upsert,
            vec![TaskVariableMutation {
                name: self.name.clone(),
                value: self.value.clone(),
            }],
        )?;
        Ok(())
    }
}

pub struct DeleteTaskLocalVariableCmd {
    task_id: String,
    name: String,
}

impl DeleteTaskLocalVariableCmd {
    pub fn new(task_id: String, name: String) -> Self {
        Self { task_id, name }
    }
}

impl Command<()> for DeleteTaskLocalVariableCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // Java parity: TaskService.removeVariableLocal delegates to the shared
        // task-variable removal; a missing variable 404s (REST DELETE single).
        remove_task_variables(
            command_context,
            &self.task_id,
            TaskVariableScope::Local,
            Some(vec![self.name.clone()]),
            true,
        )
    }
}

pub struct GetTaskLocalVariablesCmd {
    task_id: String,
}

impl GetTaskLocalVariablesCmd {
    pub fn new(task_id: String) -> Self {
        Self { task_id }
    }
}

impl Command<HashMap<String, serde_json::Value>> for GetTaskLocalVariablesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HashMap<String, serde_json::Value>, crate::error::FlowableError> {
        let task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Task '{}' was not found",
                    self.task_id
                ))
            })?;
        Ok(task.local_variables())
    }
}

pub struct GetTaskLocalVariableCmd {
    task_id: String,
    name: String,
}

impl GetTaskLocalVariableCmd {
    pub fn new(task_id: String, name: String) -> Self {
        Self { task_id, name }
    }
}

impl Command<Option<serde_json::Value>> for GetTaskLocalVariableCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<serde_json::Value>, crate::error::FlowableError> {
        let task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Task '{}' was not found",
                    self.task_id
                ))
            })?;
        Ok(task.local_variable(&self.name))
    }
}

fn update_historic_task_assignment(command_context: &mut CommandContext, task: &Task) {
    command_context
        .history_manager
        .record_task_updated(task, &mut command_context.session);
}

fn record_assignee_identity_link_event(
    command_context: &mut CommandContext,
    task_id: &str,
    assignee: &str,
    action: &str,
) {
    // P97: delegate to the HistoryManager so history_disabled/async_history
    // gating applies (previously a direct, ungated store write).
    command_context.history_manager.record_task_assignment_event(
        task_id,
        action,
        assignee,
        &mut command_context.session,
    );
}

/// Load the user-task's task_listeners and invoke them for `event`.
fn fire_task_listeners_for_event(
    command_context: &mut CommandContext,
    task: &Task,
    event: &str,
) -> Result<(), crate::error::FlowableError> {
    let mut execution = match command_context
        .execution_entity_manager
        .find_by_id(&task.execution_id, &mut command_context.session)
    {
        Some(execution) => execution,
        None => return Ok(()),
    };

    let listeners =
        load_user_task_listeners(command_context, &execution, &task.task_definition_key);
    if listeners.is_empty() {
        return Ok(());
    }

    let mut task_mut = task.clone();
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, &execution);
    crate::bpmn::listener::notify_task_listeners(
        &mut task_mut,
        &mut execution,
        command_context,
        &listeners,
        event,
        &evaluation_execution,
    )?;
    command_context
        .execution_entity_manager
        .update(&execution, &mut command_context.session);
    // Task may have been mutated by the listener (e.g. name/assignee); keep store in sync
    // unless we are about to delete it (complete).
    if event != "complete" {
        // P97: mirror the listener side-effects into history as well — this
        // path previously relied on insert_task's silent historic sync, which
        // bypassed history gating and consumed the identity-link diff.
        command_context
            .history_manager
            .record_task_updated(&task_mut, &mut command_context.session);
        command_context
            .task_entity_manager
            .update(&task_mut, &mut command_context.session);
    }
    Ok(())
}

fn load_user_task_listeners(
    command_context: &CommandContext,
    execution: &crate::runtime::execution::Execution,
    task_definition_key: &str,
) -> Vec<flowable_bpmn_model::model::FlowableListener> {
    let process_def_id = match execution.process_definition_id.as_ref() {
        Some(id) => id,
        None => return Vec::new(),
    };
    let model = match command_context
        .deployment_manager
        .get_bpmn_model(process_def_id)
    {
        Some(m) => m,
        None => return Vec::new(),
    };
    let process = match model.main_process.as_ref() {
        Some(p) => p,
        None => return Vec::new(),
    };
    match crate::agenda::continue_process_operation::find_flow_element(process, task_definition_key)
    {
        Some(flowable_bpmn_model::model::FlowElementEnum::UserTask(ut)) => {
            ut.task_listeners.clone()
        }
        _ => Vec::new(),
    }
}

pub struct DeleteTaskCmd {
    task_id: String,
    delete_reason: Option<String>,
    cascade: bool,
}

impl DeleteTaskCmd {
    pub fn new(task_id: String, delete_reason: Option<String>, cascade: bool) -> Self {
        Self {
            task_id,
            delete_reason,
            cascade,
        }
    }
}

impl Command<()> for DeleteTaskCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No task found for task id {}",
                    self.task_id
                ))
            })?;

        require_active_task_with_prefix(&task, "Cannot delete")?;

        // Java parity (TaskHelper.java:433-468): fire the `delete` task
        // listener before the actual deletion so the listener can observe
        // and possibly short-circuit the operation by throwing a BpmnError
        // (which `notify_task_listeners` propagates to the execution). The
        // prior guard that refused deletion for any task tied to a live
        // execution diverged from the Java `TaskServiceImpl.deleteTask`
        // public API path; we now mirror Java and only the listener itself
        // can stop the deletion via BpmnError propagation.
        if !task.execution_id.is_empty() {
            fire_task_listeners_for_event(command_context, &task, "delete")?;
        }

        command_context
            .task_entity_manager
            .delete(&self.task_id, &mut command_context.session);

        if self.cascade {
            command_context
                .runtime_store
                .delete_historic_task_instance_cascade(&self.task_id, &mut command_context.session);
        } else if let Some(reason) = &self.delete_reason {
            if let Some(mut historic) = command_context
                .runtime_store
                .get_historic_task_instance(&self.task_id, &mut command_context.session)
            {
                historic.delete_reason = Some(reason.clone());
                command_context
                    .runtime_store
                    .update_historic_task_instance(historic, &mut command_context.session);
            }
        }

        Ok(())
    }
}

pub struct ClaimTaskByIdCmd {
    task_id: String,
    assignee: String,
}

impl ClaimTaskByIdCmd {
    pub fn new(task_id: String, assignee: String) -> Self {
        Self { task_id, assignee }
    }
}

impl Command<()> for ClaimTaskByIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No task found for task id {}",
                    self.task_id
                ))
            })?;

        require_active_task_with_prefix(&task, "Cannot claim")?;

        if let Some(current_assignee) = task.assignee.as_deref() {
            if current_assignee != self.assignee {
                return Err(crate::error::FlowableError::Conflict(format!(
                    "Task '{}' is already claimed by user '{}'",
                    self.task_id, current_assignee
                )));
            }
            // Java ClaimTaskCmd.java:50-54,62: claim time / claimedBy / state are
            // set unconditionally before the assignee check; a re-claim by the
            // same user is idempotent (post-conditions already met) but still
            // refreshes the claim state (recordTaskInfoChange).
            task.claim_time = Some(Utc::now());
            task.state = "claimed".to_string();
            update_historic_task_assignment(command_context, &task);
            command_context
                .task_entity_manager
                .update(&task, &mut command_context.session);
        } else {
            task.assignee = Some(self.assignee.clone());
            task.claim_time = Some(Utc::now());
            task.state = "claimed".to_string();
            update_historic_task_assignment(command_context, &task);
            command_context
                .task_entity_manager
                .update(&task, &mut command_context.session);
            record_assignee_identity_link_event(
                command_context,
                &task.id,
                &self.assignee,
                "AddUserLink",
            );
            fire_task_listeners_for_event(command_context, &task, "assignment")?;
            // P53 layer 1: dispatch `TASK_ASSIGNED` after the assignment
            // listener has run. Java `TaskHelper.changeTaskAssignee` emits
            // `TASK_ASSIGNED` for every successful assignee change.
            crate::engine::event_dispatcher::dispatch_task_assigned(
                command_context,
                &task.id,
                Some(&task.process_instance_id),
                Some(&task.execution_id),
            );
        }
        Ok(())
    }
}

pub struct SetTaskAssigneeByIdCmd {
    task_id: String,
    assignee: Option<String>,
}

impl SetTaskAssigneeByIdCmd {
    pub fn new(task_id: String, assignee: Option<String>) -> Self {
        Self { task_id, assignee }
    }
}

impl Command<()> for SetTaskAssigneeByIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut task = match command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
        {
            Some(task) => task,
            None => {
                return Err(crate::error::FlowableError::NotFound(format!(
                    "No task found for task id {}",
                    self.task_id
                )));
            }
        };

        require_active_task_with_prefix(&task, "Cannot claim")?;

        let previous_assignee = task.assignee.clone();
        if self.assignee.is_none() {
            task.claim_time = None;
            task.state = "created".to_string();
        }
        task.assignee = self.assignee.clone();
        if let Some(previous_assignee) = previous_assignee {
            record_assignee_identity_link_event(
                command_context,
                &task.id,
                &previous_assignee,
                "DeleteUserLink",
            );
        }
        update_historic_task_assignment(command_context, &task);
        command_context
            .task_entity_manager
            .update(&task, &mut command_context.session);
        fire_task_listeners_for_event(command_context, &task, "assignment")?;
        // P53 layer 1: `TASK_ASSIGNED` for the set-assignee path as well.
        crate::engine::event_dispatcher::dispatch_task_assigned(
            command_context,
            &task.id,
            Some(&task.process_instance_id),
            Some(&task.execution_id),
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub assignee: Option<Option<String>>,
    pub owner: Option<Option<String>>,
    pub delegation_state: Option<Option<String>>,
    pub parent_task_id: Option<Option<String>>,
    pub priority: Option<Option<i32>>,
    pub due_date: Option<Option<DateTime<Utc>>>,
    pub category: Option<Option<String>>,
    pub form_key: Option<Option<String>>,
    pub tenant_id: Option<Option<String>>,
}

impl TaskUpdate {
    fn apply_to(self, task: &mut Task) {
        if let Some(name) = self.name {
            task.name = name;
        }
        if let Some(description) = self.description {
            task.description = description;
        }
        if let Some(assignee) = self.assignee {
            task.assignee = assignee;
        }
        if let Some(owner) = self.owner {
            task.owner = owner;
        }
        if let Some(delegation_state) = self.delegation_state {
            task.delegation_state = delegation_state;
        }
        if let Some(parent_task_id) = self.parent_task_id {
            task.parent_task_id = parent_task_id;
        }
        if let Some(priority) = self.priority {
            task.priority = priority;
        }
        if let Some(due_date) = self.due_date {
            task.due_date = due_date;
        }
        if let Some(category) = self.category {
            task.category = category;
        }
        if let Some(form_key) = self.form_key {
            task.form_key = form_key;
        }
        if let Some(tenant_id) = self.tenant_id {
            task.tenant_id = tenant_id;
        }
    }
}

pub struct CreateTaskCmd {
    task: Task,
}

impl CreateTaskCmd {
    pub fn new(task: Task) -> Self {
        Self { task }
    }
}

impl Command<Task> for CreateTaskCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Task, crate::error::FlowableError> {
        let task = self.task.clone();
        command_context
            .task_entity_manager
            .insert(&task, &mut command_context.session);

        // P97: route through the HistoryManager like every other task-creation
        // path (Java TaskServiceImpl.saveTask → TaskEntityManager.insert →
        // HistoryManager.recordTaskCreated). The previous hand-built insert
        // bypassed history_disabled/async_history and skipped the P86a
        // assignee/owner identity links and full-extras log entries.
        command_context
            .history_manager
            .record_task_created(&task, &mut command_context.session);

        Ok(task)
    }
}

pub struct UpdateTaskByIdCmd {
    task_id: String,
    update: TaskUpdate,
}

impl UpdateTaskByIdCmd {
    pub fn new(task_id: String, update: TaskUpdate) -> Self {
        Self { task_id, update }
    }
}

impl Command<Task> for UpdateTaskByIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Task, crate::error::FlowableError> {
        let mut task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No task found for task id {}",
                    self.task_id
                ))
            })?;

        if task.is_completed {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                format!("Task '{}' is already completed", self.task_id),
            ));
        }

        // Java parity: REST PUT /runtime/tasks/{id} uses `taskService.saveTask()`
        // which does NOT check suspension (unlike SetTaskDueDateCmd / SetTaskPriorityCmd
        // which extend NeedsActiveTaskCmd but are only used via the Java API, not REST).
        // Therefore no `require_active_task` here.

        // Snapshot pre-update fields for P119 TASK_*_CHANGED events
        // (Java TaskEntityManagerImpl.logTaskUpdateEvents:271-305).
        let previous_owner = task.owner.clone();
        let previous_priority = task.priority;
        let previous_due_date = task.due_date;
        let previous_name = task.name.clone();

        self.update.clone().apply_to(&mut task);
        // P86a: must record history *before* the runtime update. `insert_task`
        // (via task_entity_manager.update) silently syncs the historic task
        // projection (`runtime_store.rs:2410-2413`); if that runs first,
        // `record_task_updated` sees no assignee/owner diff and skips the
        // accumulating historic identity-link insert
        // (`HistoricTaskServiceImpl.recordTaskInfoChange:142-152`).
        command_context
            .history_manager
            .record_task_updated(&task, &mut command_context.session);
        command_context
            .task_entity_manager
            .update(&task, &mut command_context.session);

        // P119: field-change events mirror Java logTaskUpdateEvents for
        // owner / priority / dueDate / name (assignee uses TASK_ASSIGNED).
        let pi = Some(task.process_instance_id.as_str());
        let exec = Some(task.execution_id.as_str());
        if previous_owner != task.owner {
            crate::engine::event_dispatcher::dispatch_task_owner_changed(
                command_context,
                &task.id,
                pi,
                exec,
            );
        }
        if previous_priority != task.priority {
            crate::engine::event_dispatcher::dispatch_task_priority_changed(
                command_context,
                &task.id,
                pi,
                exec,
            );
        }
        if previous_due_date != task.due_date {
            crate::engine::event_dispatcher::dispatch_task_duedate_changed(
                command_context,
                &task.id,
                pi,
                exec,
            );
        }
        if previous_name != task.name {
            crate::engine::event_dispatcher::dispatch_task_name_changed(
                command_context,
                &task.id,
                pi,
                exec,
            );
        }

        Ok(task)
    }
}

pub struct DelegateTaskByIdCmd {
    task_id: String,
    user_id: String,
}

impl DelegateTaskByIdCmd {
    pub fn new(task_id: String, user_id: String) -> Self {
        Self { task_id, user_id }
    }
}

impl Command<()> for DelegateTaskByIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No task found for task id {}",
                    self.task_id
                ))
            })?;

        require_active_task_with_prefix(&task, "Cannot delegate")?;

        // Java DelegateTaskCmd.java:37-40: PENDING + owner=assignee only when
        // owner is unset; there is no fallback to the delegate target, so a
        // never-assigned task keeps a null owner.
        if task.owner.is_none() {
            task.owner = task.assignee.clone();
        }
        task.assignee = Some(self.user_id.clone());
        task.delegation_state = Some("pending".to_string());
        update_historic_task_assignment(command_context, &task);
        command_context
            .task_entity_manager
            .update(&task, &mut command_context.session);
        Ok(())
    }
}

pub struct ResolveTaskByIdCmd {
    task_id: String,
    variables: HashMap<String, serde_json::Value>,
}

impl ResolveTaskByIdCmd {
    pub fn new(task_id: String) -> Self {
        Self {
            task_id,
            variables: HashMap::new(),
        }
    }

    pub fn with_variables(task_id: String, variables: HashMap<String, serde_json::Value>) -> Self {
        Self { task_id, variables }
    }
}

impl Command<()> for ResolveTaskByIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No task found for task id {}",
                    self.task_id
                ))
            })?;

        require_active_task_with_prefix(&task, "Cannot resolve")?;

        // Java ResolveTaskCmd.java:46-48: variables are applied first
        // (task.setVariables → execution scope).
        if !self.variables.is_empty() {
            let mutations = self
                .variables
                .iter()
                .map(|(name, value)| TaskVariableMutation {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect();
            crate::cmd::task_variable_cmd::mutate_task_variables(
                command_context,
                &self.task_id,
                TaskVariableScope::Global,
                VariableMutationMode::Upsert,
                mutations,
            )?;
        }

        // Java ResolveTaskCmd.java:53-54: no delegation-state precondition —
        // the task is unconditionally marked RESOLVED and the assignee is set
        // back to the owner (even when the task was never delegated).
        task.delegation_state = Some("resolved".to_string());
        task.assignee = task.owner.clone();
        update_historic_task_assignment(command_context, &task);
        command_context
            .task_entity_manager
            .update(&task, &mut command_context.session);
        Ok(())
    }
}

pub struct SetTaskDueDateByIdCmd {
    task_id: String,
    due_date: Option<DateTime<Utc>>,
}

impl SetTaskDueDateByIdCmd {
    pub fn new(task_id: String, due_date: Option<DateTime<Utc>>) -> Self {
        Self { task_id, due_date }
    }
}

impl Command<()> for SetTaskDueDateByIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No task found for task id {}",
                    self.task_id
                ))
            })?;

        // Java SetTaskDueDateCmd extends NeedsActiveTaskCmd (default suspended
        // message) — unlike REST saveTask, the engine API rejects suspended tasks.
        require_active_task(&task)?;

        let previous_due = task.due_date;
        task.due_date = self.due_date;
        command_context
            .history_manager
            .record_task_updated(&task, &mut command_context.session);
        command_context
            .task_entity_manager
            .update(&task, &mut command_context.session);
        // P119: TASK_DUEDATE_CHANGED — Java TaskEntityManagerImpl.java:291-295.
        if previous_due != task.due_date {
            crate::engine::event_dispatcher::dispatch_task_duedate_changed(
                command_context,
                &task.id,
                Some(&task.process_instance_id),
                Some(&task.execution_id),
            );
        }
        Ok(())
    }
}

pub struct SetTaskPriorityByIdCmd {
    task_id: String,
    priority: i32,
}

impl SetTaskPriorityByIdCmd {
    pub fn new(task_id: String, priority: i32) -> Self {
        Self { task_id, priority }
    }
}

impl Command<()> for SetTaskPriorityByIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No task found for task id {}",
                    self.task_id
                ))
            })?;

        // Java SetTaskPriorityCmd extends NeedsActiveTaskCmd (default suspended
        // message).
        require_active_task(&task)?;

        let previous_priority = task.priority;
        task.priority = Some(self.priority);
        command_context
            .history_manager
            .record_task_updated(&task, &mut command_context.session);
        command_context
            .task_entity_manager
            .update(&task, &mut command_context.session);
        // P119: TASK_PRIORITY_CHANGED — Java TaskEntityManagerImpl.java:284-288.
        if previous_priority != task.priority {
            crate::engine::event_dispatcher::dispatch_task_priority_changed(
                command_context,
                &task.id,
                Some(&task.process_instance_id),
                Some(&task.execution_id),
            );
        }
        Ok(())
    }
}

pub struct CompleteTaskByExecutionIdCmd {
    execution_id: String,
}

impl CompleteTaskByExecutionIdCmd {
    pub fn new(execution_id: String) -> Self {
        Self { execution_id }
    }
}

impl Command<()> for CompleteTaskByExecutionIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let task = match command_context
            .task_entity_manager
            .find_by_execution_id(&self.execution_id, &mut command_context.session)
        {
            Some(task) => task,
            None => {
                return Err(crate::error::FlowableError::NotFound(format!(
                    "No task found for execution id {}",
                    self.execution_id
                )));
            }
        };

        require_active_task_with_prefix(&task, "Cannot complete")?;

        complete_task_internal(command_context, task)?;
        Ok(())
    }
}

pub struct TaskService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl TaskService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    pub fn create_task_query(&self) -> TaskQuery {
        TaskQuery::new(Arc::clone(&self.command_executor))
    }

    /// Creates and persists a standalone task, mirroring Java
    /// `taskService.newTask()` + `taskService.saveTask(task)` as used by
    /// REST `POST /runtime/tasks`. Standalone tasks have no process
    /// instance/execution (empty strings) and default priority 50.
    pub fn create_task(&self, mut task: Task) -> Result<Task, crate::error::FlowableError> {
        if task.id.is_empty() {
            task.id = uuid::Uuid::new_v4().to_string();
        }
        if task.created_time.is_none() {
            task.created_time = Some(Utc::now());
        }
        if task.priority.is_none() {
            // Java `TaskEntity` default priority.
            task.priority = Some(50);
        }
        let cmd = CreateTaskCmd::new(task);
        self.command_executor.execute(&cmd)
    }

    pub fn get_tasks_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Result<Vec<Task>, crate::error::FlowableError> {
        let cmd = QueryTasksByProcessInstanceCmd::new(process_instance_id);
        self.command_executor.execute(&cmd)
    }

    pub fn get_sub_tasks(
        &self,
        parent_task_id: String,
    ) -> Result<Vec<Task>, crate::error::FlowableError> {
        let cmd = QuerySubTasksCmd::new(parent_task_id);
        self.command_executor.execute(&cmd)
    }

    pub fn get_event_wait_states_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Vec<EventWaitState> {
        let cmd = QueryEventWaitStatesByProcessInstanceIdCmd::new(process_instance_id);
        self.command_executor.execute(&cmd).unwrap()
    }

    /// Type alias for callers that depend on the older name
    pub fn get_message_style_wait_states_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Vec<EventWaitState> {
        self.get_event_wait_states_by_process_instance_id(process_instance_id)
    }

    /// Deletes the given task by task ID, optionally recording a delete reason
    /// and cascading the deletion to history. Mirrors Java `TaskService.deleteTask`.
    pub fn delete_task(
        &self,
        task_id: String,
        delete_reason: Option<String>,
        cascade: bool,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = DeleteTaskCmd::new(task_id, delete_reason, cascade);
        self.command_executor.execute(&cmd)
    }

    /// Completes the given task by task ID.
    pub fn complete_task_by_id(&self, task_id: String) -> Result<(), crate::error::FlowableError> {
        let cmd = CompleteTaskByIdCmd::new(task_id);
        self.command_executor.execute(&cmd)
    }

    pub fn complete_task_by_id_with_variables(
        &self,
        task_id: String,
        variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = CompleteTaskByIdCmd::with_variables(task_id, variables, false);
        self.command_executor.execute(&cmd)
    }

    pub fn complete_task_by_id_with_local_variables(
        &self,
        task_id: String,
        variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = CompleteTaskByIdCmd::with_variables(task_id, variables, true);
        self.command_executor.execute(&cmd)
    }

    pub fn complete_task_by_id_with_variable_maps(
        &self,
        task_id: String,
        variables: HashMap<String, serde_json::Value>,
        transient_variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = CompleteTaskByIdCmd::with_variable_maps(task_id, variables, transient_variables);
        self.command_executor.execute(&cmd)
    }

    pub fn set_task_local_variable(
        &self,
        task_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SetTaskLocalVariableCmd::new(task_id, name, value);
        self.command_executor.execute(&cmd)
    }

    pub fn get_task_local_variable(
        &self,
        task_id: String,
        name: String,
    ) -> Result<Option<serde_json::Value>, crate::error::FlowableError> {
        let cmd = GetTaskLocalVariableCmd::new(task_id, name);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `TaskService#getDataObjects(taskId)`.
    pub fn get_data_objects(
        &self,
        task_id: String,
    ) -> Result<
        HashMap<String, crate::engine::data_object_service::DataObject>,
        crate::error::FlowableError,
    > {
        let cmd = crate::engine::data_object_service::GetTaskDataObjectsCmd::new(task_id);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `TaskService#getDataObjects(taskId, names)`.
    pub fn get_data_objects_by_names(
        &self,
        task_id: String,
        names: Vec<String>,
    ) -> Result<
        HashMap<String, crate::engine::data_object_service::DataObject>,
        crate::error::FlowableError,
    > {
        let cmd =
            crate::engine::data_object_service::GetTaskDataObjectsCmd::with_names(task_id, names);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `TaskService#getDataObject(taskId, name)`.
    pub fn get_data_object(
        &self,
        task_id: String,
        name: String,
    ) -> Result<Option<crate::engine::data_object_service::DataObject>, crate::error::FlowableError>
    {
        let cmd = crate::engine::data_object_service::GetTaskDataObjectCmd::new(task_id, name);
        self.command_executor.execute(&cmd)
    }

    pub fn get_task_local_variables(
        &self,
        task_id: String,
    ) -> Result<HashMap<String, serde_json::Value>, crate::error::FlowableError> {
        let cmd = GetTaskLocalVariablesCmd::new(task_id);
        self.command_executor.execute(&cmd)
    }

    pub fn delete_task_local_variable(
        &self,
        task_id: String,
        name: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = DeleteTaskLocalVariableCmd::new(task_id, name);
        self.command_executor.execute(&cmd)
    }

    /// Sets a global (execution-scoped) variable, mirroring Java
    /// `TaskService.setVariable` on a task: upsert on the task's execution.
    pub fn set_task_variable(
        &self,
        task_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = MutateTaskVariablesCmd::new(
            task_id,
            TaskVariableScope::Global,
            VariableMutationMode::Upsert,
            vec![TaskVariableMutation { name, value }],
        );
        self.command_executor.execute(&cmd).map(|_| ())
    }

    /// Sets global (execution-scoped) variables, mirroring Java
    /// `TaskService.setVariables` on a task: upsert on the task's execution.
    pub fn set_task_variables(
        &self,
        task_id: String,
        variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let mutations = variables
            .into_iter()
            .map(|(name, value)| TaskVariableMutation { name, value })
            .collect();
        let cmd = MutateTaskVariablesCmd::new(
            task_id,
            TaskVariableScope::Global,
            VariableMutationMode::Upsert,
            mutations,
        );
        self.command_executor.execute(&cmd).map(|_| ())
    }

    /// Sets task-local variables, mirroring Java `TaskService.setVariablesLocal`.
    pub fn set_task_variables_local(
        &self,
        task_id: String,
        variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let mutations = variables
            .into_iter()
            .map(|(name, value)| TaskVariableMutation { name, value })
            .collect();
        let cmd = MutateTaskVariablesCmd::new(
            task_id,
            TaskVariableScope::Local,
            VariableMutationMode::Upsert,
            mutations,
        );
        self.command_executor.execute(&cmd).map(|_| ())
    }

    /// Create-only batch on the given scope (Java REST POST semantics): any
    /// variable already present on the scope fails the whole batch with
    /// `Conflict` and nothing is written.
    pub fn create_task_variables(
        &self,
        task_id: String,
        scope: TaskVariableScope,
        variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let mutations = variables
            .into_iter()
            .map(|(name, value)| TaskVariableMutation { name, value })
            .collect();
        let cmd = MutateTaskVariablesCmd::new(
            task_id,
            scope,
            VariableMutationMode::CreateOnly,
            mutations,
        );
        self.command_executor.execute(&cmd).map(|_| ())
    }

    /// Update-only single mutation on the given scope (Java REST PUT
    /// semantics): a variable absent from the scope fails with `NotFound`.
    pub fn update_task_variable(
        &self,
        task_id: String,
        scope: TaskVariableScope,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = MutateTaskVariablesCmd::new(
            task_id,
            scope,
            VariableMutationMode::UpdateOnly,
            vec![TaskVariableMutation { name, value }],
        );
        self.command_executor.execute(&cmd).map(|_| ())
    }

    /// Removes a global variable, mirroring Java `TaskService.removeVariable`
    /// on a task: a missing name is ignored.
    pub fn remove_task_variable(
        &self,
        task_id: String,
        name: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = RemoveTaskVariablesCmd::new(
            task_id,
            TaskVariableScope::Global,
            Some(vec![name]),
            false,
        );
        self.command_executor.execute(&cmd)
    }

    /// Removes global variables, mirroring Java `TaskService.removeVariables`:
    /// missing names are ignored.
    pub fn remove_task_variables(
        &self,
        task_id: String,
        names: Vec<String>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd =
            RemoveTaskVariablesCmd::new(task_id, TaskVariableScope::Global, Some(names), false);
        self.command_executor.execute(&cmd)
    }

    /// Removes task-local variables, mirroring Java
    /// `TaskService.removeVariablesLocal`: missing names are ignored.
    pub fn remove_task_variables_local(
        &self,
        task_id: String,
        names: Vec<String>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd =
            RemoveTaskVariablesCmd::new(task_id, TaskVariableScope::Local, Some(names), false);
        self.command_executor.execute(&cmd)
    }

    /// Removes ALL task-local variables (Java REST DELETE on the task variable
    /// collection). Global variables are left untouched.
    pub fn remove_all_task_local_variables(
        &self,
        task_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = RemoveTaskVariablesCmd::new(task_id, TaskVariableScope::Local, None, false);
        self.command_executor.execute(&cmd)
    }

    /// Removes a single variable from the given scope with Java REST DELETE
    /// single-variable semantics: a variable absent from the scope fails with
    /// `NotFound`.
    pub fn remove_task_variable_on_scope(
        &self,
        task_id: String,
        scope: TaskVariableScope,
        name: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = RemoveTaskVariablesCmd::new(task_id, scope, Some(vec![name]), true);
        self.command_executor.execute(&cmd)
    }

    /// Resolves a single variable with Java `TaskService.getVariable`
    /// semantics: task-local value first, then the execution scope as
    /// fallback. No suspension guard on reads.
    pub fn get_task_variable(
        &self,
        task_id: String,
        name: String,
    ) -> Result<Option<serde_json::Value>, crate::error::FlowableError> {
        let cmd = crate::cmd::task_variable_cmd::GetTaskVariableCmd::new(task_id, name);
        self.command_executor.execute(&cmd)
    }

    /// Merged variable map with Java `TaskService.getVariables` semantics:
    /// task-local values shadow execution (global) values on name clashes.
    pub fn get_task_variables(
        &self,
        task_id: String,
    ) -> Result<HashMap<String, serde_json::Value>, crate::error::FlowableError> {
        let cmd = crate::cmd::task_variable_cmd::GetTaskVariablesCmd::new(task_id);
        self.command_executor.execute(&cmd)
    }

    /// Claims the given task by setting its assignee.
    pub fn claim_task_by_id(
        &self,
        task_id: String,
        assignee: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = ClaimTaskByIdCmd::new(task_id, assignee);
        self.command_executor.execute(&cmd)
    }

    /// Unclaims the given task by clearing its assignee.
    pub fn unclaim_task_by_id(&self, task_id: String) -> Result<(), crate::error::FlowableError> {
        let cmd = SetTaskAssigneeByIdCmd::new(task_id, None);
        self.command_executor.execute(&cmd)
    }

    /// Updates mutable task metadata and returns the persisted task.
    pub fn update_task_by_id(
        &self,
        task_id: String,
        update: TaskUpdate,
    ) -> Result<Task, crate::error::FlowableError> {
        let cmd = UpdateTaskByIdCmd::new(task_id, update);
        self.command_executor.execute(&cmd)
    }

    /// Delegates the task to another user and marks it pending resolution.
    pub fn delegate_task_by_id(
        &self,
        task_id: String,
        user_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = DelegateTaskByIdCmd::new(task_id, user_id);
        self.command_executor.execute(&cmd)
    }

    /// Resolves a pending delegated task.
    pub fn resolve_task_by_id(&self, task_id: String) -> Result<(), crate::error::FlowableError> {
        let cmd = ResolveTaskByIdCmd::new(task_id);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `TaskService.resolveTask(taskId, variables)` — applies the
    /// variables (execution scope) before marking the task RESOLVED.
    pub fn resolve_task_by_id_with_variables(
        &self,
        task_id: String,
        variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = ResolveTaskByIdCmd::with_variables(task_id, variables);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `TaskService.setDueDate(taskId, dueDate)` (SetTaskDueDateCmd,
    /// NeedsActiveTaskCmd suspension guard).
    pub fn set_task_due_date(
        &self,
        task_id: String,
        due_date: Option<DateTime<Utc>>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SetTaskDueDateByIdCmd::new(task_id, due_date);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `TaskService.setPriority(taskId, priority)` (SetTaskPriorityCmd,
    /// NeedsActiveTaskCmd suspension guard).
    pub fn set_task_priority(
        &self,
        task_id: String,
        priority: i32,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SetTaskPriorityByIdCmd::new(task_id, priority);
        self.command_executor.execute(&cmd)
    }

    /// Older entry point retained for callers that still depend on it.
    pub fn complete_task(
        &self,
        execution_id: String,
        process_definition_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let _ = process_definition_id;
        let cmd = CompleteTaskByExecutionIdCmd::new(execution_id);
        self.command_executor.execute(&cmd)
    }

    /// Wakes a single receive-task wait state correlated to the given process instance ID.
    pub fn wake_up_message_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = WakeUpMessageByProcessInstanceIdCmd::new(process_instance_id);
        self.command_executor.execute(&cmd)
    }

    /// Wakes a single receive-task wait state correlated to the given process instance ID and message_ref.
    pub fn wake_up_message_by_message_ref(
        &self,
        process_instance_id: String,
        message_ref: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = WakeUpMessageByMessageRefCmd::new(process_instance_id, message_ref);
        self.command_executor.execute(&cmd)
    }

    pub fn add_candidate_user(
        &self,
        task_id: String,
        user_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = AddIdentityLinkCmd::new(task_id, Some(user_id), None, "candidate".to_string());
        self.command_executor.execute(&cmd)
    }

    pub fn add_candidate_group(
        &self,
        task_id: String,
        group_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = AddIdentityLinkCmd::new(task_id, None, Some(group_id), "candidate".to_string());
        self.command_executor.execute(&cmd)
    }

    pub fn delete_candidate_user(
        &self,
        task_id: String,
        user_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = DeleteIdentityLinkCmd::new(task_id, Some(user_id), None, "candidate".to_string());
        self.command_executor.execute(&cmd)
    }

    pub fn delete_candidate_group(
        &self,
        task_id: String,
        group_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd =
            DeleteIdentityLinkCmd::new(task_id, None, Some(group_id), "candidate".to_string());
        self.command_executor.execute(&cmd)
    }

    pub fn add_identity_link(
        &self,
        task_id: String,
        user_id: Option<String>,
        group_id: Option<String>,
        link_type: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = AddIdentityLinkCmd::new(task_id, user_id, group_id, link_type);
        self.command_executor.execute(&cmd)
    }

    pub fn delete_identity_link(
        &self,
        task_id: String,
        user_id: Option<String>,
        group_id: Option<String>,
        link_type: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = DeleteIdentityLinkCmd::new(task_id, user_id, group_id, link_type);
        self.command_executor.execute(&cmd)
    }

    pub fn get_identity_links_for_task(
        &self,
        task_id: String,
    ) -> Result<Vec<crate::identity::entities::IdentityLink>, crate::error::FlowableError> {
        let cmd = GetIdentityLinksForTaskCmd::new(task_id);
        self.command_executor.execute(&cmd)
    }
}

pub struct AddIdentityLinkCmd {
    task_id: String,
    user_id: Option<String>,
    group_id: Option<String>,
    link_type: String,
}

impl AddIdentityLinkCmd {
    pub fn new(
        task_id: String,
        user_id: Option<String>,
        group_id: Option<String>,
        link_type: String,
    ) -> Self {
        Self {
            task_id,
            user_id,
            group_id,
            link_type,
        }
    }
}

impl Command<()> for AddIdentityLinkCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No task found for task id {}",
                    self.task_id
                ))
            })?;

        require_active_task(&task)?;

        if self.link_type == "assignee" || self.link_type == "owner" {
            let value = self.user_id.clone();
            if self.link_type == "assignee" {
                task.assignee = value;
            } else {
                task.owner = value;
            }
            // P86a: Java `AddIdentityLinkCmd.java:95,108` routes assignee/owner
            // through `TaskHelper.changeTaskAssignee/changeTaskOwner`, which call
            // `TaskEntityManagerImpl:125,139` → `recordTaskInfoChange`. That both
            // syncs the historic task row and appends the accumulating historic
            // identity link; without it the historic assignee/owner went stale.
            update_historic_task_assignment(command_context, &task);
            command_context
                .task_entity_manager
                .update(&task, &mut command_context.session);
            return Ok(());
        }

        let execution = command_context
            .runtime_store
            .find_execution(&task.execution_id, &mut command_context.session);
        let process_definition_id = execution.and_then(|e| e.process_definition_id);

        let link = crate::identity::entities::IdentityLink {
            id: uuid::Uuid::new_v4().to_string(),
            link_type: self.link_type.clone(),
            user_id: self.user_id.clone(),
            group_id: self.group_id.clone(),
            task_id: Some(task.id.clone()),
            process_instance_id: Some(task.process_instance_id.clone()),
            process_definition_id,
        };

        // P77: Java IdentityLinkUtil.handleTaskIdentityLinkAddition:71
        // → HistoryManager.recordIdentityLinkCreated.
        command_context
            .history_manager
            .record_identity_link_created(&link, &mut command_context.session);
        command_context
            .runtime_store
            .insert_identity_link(link, &mut command_context.session);
        Ok(())
    }
}

pub struct DeleteIdentityLinkCmd {
    task_id: String,
    user_id: Option<String>,
    group_id: Option<String>,
    link_type: String,
}

impl DeleteIdentityLinkCmd {
    pub fn new(
        task_id: String,
        user_id: Option<String>,
        group_id: Option<String>,
        link_type: String,
    ) -> Self {
        Self {
            task_id,
            user_id,
            group_id,
            link_type,
        }
    }
}

impl Command<()> for DeleteIdentityLinkCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // Java parity: DeleteIdentityLinkCmd extends NeedsActiveTaskCmd.
        let store = command_context.runtime_store_handle();
        let task = store
            .find_task(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Cannot find task with id {}",
                    self.task_id
                ))
            })?;
        require_active_task(&task)?;

        if self.link_type == "assignee" || self.link_type == "owner" {
            let mut task = task;
            if self.link_type == "assignee" {
                task.assignee = None;
            } else {
                task.owner = None;
            }
            // P86a: Java `DeleteIdentityLinkCmd.java:82-86` clears assignee/owner
            // via `TaskHelper.changeTaskAssignee(task, null)` /
            // `changeTaskOwner(task, null)` → `recordTaskInfoChange`, which
            // appends a historic identity link carrying a null userId.
            update_historic_task_assignment(command_context, &task);
            command_context
                .task_entity_manager
                .update(&task, &mut command_context.session);
            return Ok(());
        }

        let links = command_context
            .runtime_store
            .find_identity_links_by_task(&self.task_id, &mut command_context.session);

        for link in links {
            if link.link_type == self.link_type
                && link.user_id == self.user_id
                && link.group_id == self.group_id
            {
                // P77: Java IdentityLinkUtil.handleTaskIdentityLinkDeletions:101
                // → HistoryManager.recordIdentityLinkDeleted (deletes historic row).
                command_context
                    .history_manager
                    .record_identity_link_deleted(&link.id, &mut command_context.session);
                command_context
                    .runtime_store
                    .delete_identity_link(&link.id, &mut command_context.session);
            }
        }
        Ok(())
    }
}

pub struct GetIdentityLinksForTaskCmd {
    task_id: String,
}

impl GetIdentityLinksForTaskCmd {
    pub fn new(task_id: String) -> Self {
        Self { task_id }
    }
}

impl Command<Vec<crate::identity::entities::IdentityLink>> for GetIdentityLinksForTaskCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<crate::identity::entities::IdentityLink>, crate::error::FlowableError> {
        Ok(command_context
            .runtime_store
            .find_identity_links_by_task(&self.task_id, &mut command_context.session))
    }
}
