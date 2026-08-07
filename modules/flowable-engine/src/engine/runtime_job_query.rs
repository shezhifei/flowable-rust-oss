//! Direct engine query for runtime job families (P65-job-query).
//!
//! Filtering, sorting, total count, and paging are pushed down into the
//! persistence layer (`RuntimeStore::query_runtime_jobs`) so REST can map
//! parameters into this object without post-page filtering or full-table
//! snapshots.

use crate::engine::query::Direction;
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::runtime_store::RuntimeTimerJobState;
use std::sync::Arc;

/// Job family for management queries (Java timer / executable / deadletter /
/// suspended / history collections).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RuntimeJobFamily {
    #[default]
    All,
    Executable,
    Timer,
    Deadletter,
    Suspended,
    History,
}

/// Criteria for a single page of runtime jobs.
#[derive(Clone, Debug)]
pub struct RuntimeJobQueryCriteria {
    pub family: RuntimeJobFamily,
    pub id: Option<String>,
    pub process_instance_id: Option<String>,
    pub without_process_instance_id: bool,
    pub process_definition_id: Option<String>,
    pub execution_id: Option<String>,
    pub element_id: Option<String>,
    pub element_name: Option<String>,
    pub handler_type: Option<String>,
    pub handler_types: Vec<String>,
    pub category: Option<String>,
    pub category_like: Option<String>,
    pub scope_id: Option<String>,
    pub without_scope_id: bool,
    pub sub_scope_id: Option<String>,
    pub scope_type: Option<String>,
    pub without_scope_type: bool,
    pub scope_definition_id: Option<String>,
    /// Case definition key (CMMN). Resolved to definition IDs before filtering.
    pub case_definition_key: Option<String>,
    /// Injected/resolved definition IDs for `case_definition_key` (never post-page).
    pub case_definition_ids: Vec<String>,
    pub correlation_id: Option<String>,
    pub external_workers: bool,
    pub timers_only: bool,
    pub messages_only: bool,
    pub with_retries_left: bool,
    pub no_retries_left: bool,
    pub executable: bool,
    pub due_before: Option<i64>,
    pub due_after: Option<i64>,
    pub with_exception: bool,
    pub without_exception: bool,
    pub exception_message: Option<String>,
    pub locked: bool,
    pub unlocked: bool,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub without_tenant_id: bool,
    pub sort: String,
    pub direction: Direction,
    pub start: usize,
    pub size: Option<usize>,
    /// Wall-clock used for the Java `executable` due-date gate.
    pub now_millis: Option<i64>,
}

impl Default for RuntimeJobQueryCriteria {
    fn default() -> Self {
        Self {
            family: RuntimeJobFamily::All,
            id: None,
            process_instance_id: None,
            without_process_instance_id: false,
            process_definition_id: None,
            execution_id: None,
            element_id: None,
            element_name: None,
            handler_type: None,
            handler_types: Vec::new(),
            category: None,
            category_like: None,
            scope_id: None,
            without_scope_id: false,
            sub_scope_id: None,
            scope_type: None,
            without_scope_type: false,
            scope_definition_id: None,
            case_definition_key: None,
            case_definition_ids: Vec::new(),
            correlation_id: None,
            external_workers: false,
            timers_only: false,
            messages_only: false,
            with_retries_left: false,
            no_retries_left: false,
            executable: false,
            due_before: None,
            due_after: None,
            with_exception: false,
            without_exception: false,
            exception_message: None,
            locked: false,
            unlocked: false,
            tenant_id: None,
            tenant_id_like: None,
            without_tenant_id: false,
            sort: "id".to_string(),
            direction: Direction::Asc,
            start: 0,
            size: None,
            now_millis: None,
        }
    }
}

impl RuntimeJobQueryCriteria {
    /// Reject structurally incompatible type flags (Java JobQueryImpl).
    pub fn validate(&self) -> Result<(), FlowableError> {
        let type_flags = [
            self.timers_only,
            self.messages_only,
            self.external_workers,
        ]
        .iter()
        .filter(|&&flag| flag)
        .count();
        if type_flags > 1 {
            return Err(FlowableError::ExecutionError(
                "Only one of timersOnly, messagesOnly, or externalWorkers can be supplied"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

/// Paged result after filters and before/after the page slice.
#[derive(Clone, Debug)]
pub struct RuntimeJobQueryResult {
    pub data: Vec<RuntimeTimerJobState>,
    pub total: usize,
    pub start: usize,
    pub size: usize,
    pub sort: String,
    pub order: String,
}

/// Fluent direct engine job query.
pub struct RuntimeJobQuery {
    command_executor: Arc<DefaultCommandExecutor>,
    criteria: RuntimeJobQueryCriteria,
}

impl RuntimeJobQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            command_executor,
            criteria: RuntimeJobQueryCriteria::default(),
        }
    }

    pub fn family(mut self, family: RuntimeJobFamily) -> Self {
        self.criteria.family = family;
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.criteria.id = Some(id.into());
        self
    }

    pub fn process_instance_id(mut self, id: impl Into<String>) -> Self {
        self.criteria.process_instance_id = Some(id.into());
        self
    }

    pub fn without_process_instance_id(mut self) -> Self {
        self.criteria.without_process_instance_id = true;
        self
    }

    pub fn process_definition_id(mut self, id: impl Into<String>) -> Self {
        self.criteria.process_definition_id = Some(id.into());
        self
    }

    pub fn execution_id(mut self, id: impl Into<String>) -> Self {
        self.criteria.execution_id = Some(id.into());
        self
    }

    pub fn element_id(mut self, id: impl Into<String>) -> Self {
        self.criteria.element_id = Some(id.into());
        self
    }

    pub fn element_name(mut self, name: impl Into<String>) -> Self {
        self.criteria.element_name = Some(name.into());
        self
    }

    pub fn handler_type(mut self, handler_type: impl Into<String>) -> Self {
        self.criteria.handler_type = Some(handler_type.into());
        self
    }

    pub fn handler_types(mut self, types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.criteria.handler_types = types.into_iter().map(Into::into).collect();
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.criteria.category = Some(category.into());
        self
    }

    pub fn category_like(mut self, category_like: impl Into<String>) -> Self {
        self.criteria.category_like = Some(category_like.into());
        self
    }

    pub fn scope_id(mut self, scope_id: impl Into<String>) -> Self {
        self.criteria.scope_id = Some(scope_id.into());
        self
    }

    pub fn without_scope_id(mut self) -> Self {
        self.criteria.without_scope_id = true;
        self
    }

    pub fn sub_scope_id(mut self, sub_scope_id: impl Into<String>) -> Self {
        self.criteria.sub_scope_id = Some(sub_scope_id.into());
        self
    }

    pub fn scope_type(mut self, scope_type: impl Into<String>) -> Self {
        self.criteria.scope_type = Some(scope_type.into());
        self
    }

    pub fn without_scope_type(mut self) -> Self {
        self.criteria.without_scope_type = true;
        self
    }

    pub fn scope_definition_id(mut self, scope_definition_id: impl Into<String>) -> Self {
        self.criteria.scope_definition_id = Some(scope_definition_id.into());
        self
    }

    pub fn case_definition_key(mut self, key: impl Into<String>) -> Self {
        self.criteria.case_definition_key = Some(key.into());
        self
    }

    /// Pre-resolved case definition IDs for `case_definition_key` (tests / hosts
    /// that already looked up CMMN definitions).
    pub fn case_definition_ids(mut self, ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.criteria.case_definition_ids = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.criteria.correlation_id = Some(correlation_id.into());
        self
    }

    pub fn external_workers(mut self) -> Self {
        self.criteria.external_workers = true;
        self
    }

    pub fn timers_only(mut self) -> Self {
        self.criteria.timers_only = true;
        self
    }

    pub fn messages_only(mut self) -> Self {
        self.criteria.messages_only = true;
        self
    }

    pub fn with_retries_left(mut self) -> Self {
        self.criteria.with_retries_left = true;
        self
    }

    pub fn no_retries_left(mut self) -> Self {
        self.criteria.no_retries_left = true;
        self
    }

    pub fn executable(mut self) -> Self {
        self.criteria.executable = true;
        self
    }

    pub fn due_before(mut self, millis: i64) -> Self {
        self.criteria.due_before = Some(millis);
        self
    }

    pub fn due_after(mut self, millis: i64) -> Self {
        self.criteria.due_after = Some(millis);
        self
    }

    pub fn with_exception(mut self) -> Self {
        self.criteria.with_exception = true;
        self
    }

    pub fn without_exception(mut self) -> Self {
        self.criteria.without_exception = true;
        self
    }

    pub fn exception_message(mut self, message: impl Into<String>) -> Self {
        self.criteria.exception_message = Some(message.into());
        self
    }

    pub fn locked(mut self) -> Self {
        self.criteria.locked = true;
        self
    }

    pub fn unlocked(mut self) -> Self {
        self.criteria.unlocked = true;
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.criteria.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn tenant_id_like(mut self, tenant_id_like: impl Into<String>) -> Self {
        self.criteria.tenant_id_like = Some(tenant_id_like.into());
        self
    }

    pub fn without_tenant_id(mut self) -> Self {
        self.criteria.without_tenant_id = true;
        self
    }

    pub fn order_by(mut self, sort: impl Into<String>) -> Self {
        self.criteria.sort = sort.into();
        self
    }

    pub fn asc(mut self) -> Self {
        self.criteria.direction = Direction::Asc;
        self
    }

    pub fn desc(mut self) -> Self {
        self.criteria.direction = Direction::Desc;
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.criteria.start = start;
        self.criteria.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<RuntimeTimerJobState>, FlowableError> {
        Ok(self.list_page()?.data)
    }

    pub fn count(&self) -> Result<i64, FlowableError> {
        Ok(self.list_page()?.total as i64)
    }

    pub fn list_page(&self) -> Result<RuntimeJobQueryResult, FlowableError> {
        self.criteria.validate()?;
        let cmd = RuntimeJobQueryCmd {
            criteria: self.criteria.clone(),
        };
        self.command_executor.execute(&cmd)
    }
}

struct RuntimeJobQueryCmd {
    criteria: RuntimeJobQueryCriteria,
}

impl Command<RuntimeJobQueryResult> for RuntimeJobQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<RuntimeJobQueryResult, FlowableError> {
        let mut criteria = self.criteria.clone();
        if criteria.case_definition_key.is_some() && criteria.case_definition_ids.is_empty() {
            criteria.case_definition_ids = resolve_case_definition_ids(
                command_context,
                criteria.case_definition_key.as_deref(),
            )?;
        }
        if criteria.now_millis.is_none() {
            criteria.now_millis = Some(
                command_context
                    .runtime_store_handle()
                    .time_source()
                    .now()
                    .timestamp_millis(),
            );
        }

        let store = command_context.runtime_store_handle();
        let (data, total) = store.query_runtime_jobs(&criteria, &mut command_context.session)?;

        let start = criteria.start.min(total);
        let page_size = data.len();
        let order = match criteria.direction {
            Direction::Asc => "asc",
            Direction::Desc => "desc",
        }
        .to_string();

        Ok(RuntimeJobQueryResult {
            data,
            total,
            start,
            size: page_size,
            sort: criteria.sort.clone(),
            order,
        })
    }
}

/// Resolve a CMMN case definition key to its definition IDs. Repository
/// failures are propagated instead of being read as "no case definitions"
/// (which would silently return an empty job list).
fn resolve_case_definition_ids(
    command_context: &CommandContext,
    key: Option<&str>,
) -> Result<Vec<String>, FlowableError> {
    let Some(key) = key else {
        return Ok(Vec::new());
    };
    let Some(cmmn) = command_context.config.cmmn_engine.as_ref() else {
        return Ok(Vec::new());
    };
    cmmn.repository_service()
        .create_case_definition_query()
        .key(key)
        .list()
        .map(|definitions| {
            definitions
                .into_iter()
                .map(|definition| definition.id)
                .collect()
        })
        .map_err(|error| {
            FlowableError::Internal(format!(
                "Case definition lookup for job query failed: {error}"
            ))
        })
}
