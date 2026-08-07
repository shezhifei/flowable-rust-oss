use crate::cmd::execution_variable_cmd::{
    ExecutionVariableMutation, ExecutionVariableScope, GetScopedExecutionVariableCmd,
    GetScopedExecutionVariablesCmd, MutateExecutionVariablesAsyncCmd, MutateExecutionVariablesCmd,
    RemoveExecutionVariablesCmd,
};
use crate::cmd::task_variable_cmd::VariableMutationMode;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::db_session::DbSession;
use crate::persistence::runtime_store::{
    RuntimeStore, RuntimeTimerJobState, job_handler_types, stamp_new_job_metadata,
};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) fn find_execution_variable(
    store: &RuntimeStore,
    session: &mut DbSession,
    execution_id: &str,
    name: &str,
) -> Option<(String, serde_json::Value)> {
    let mut current_id = Some(execution_id.to_string());
    while let Some(id) = current_id {
        if let Some(execution) = store.find_execution(&id, session) {
            if let Some(val) = execution.process_variable(name) {
                return Some((execution.id, val));
            }
            current_id = execution.parent_id.clone();
        } else {
            break;
        }
    }
    None
}

pub(crate) fn collect_execution_variables(
    store: &RuntimeStore,
    session: &mut DbSession,
    execution_id: &str,
) -> HashMap<String, serde_json::Value> {
    let mut all_variables = HashMap::new();
    let mut current_id = Some(execution_id.to_string());
    while let Some(id) = current_id {
        if let Some(execution) = store.find_execution(&id, session) {
            for (key, val) in &execution.transient_variables {
                all_variables.entry(key.clone()).or_insert(val.clone());
            }
            for (key, val) in &execution.local_variables {
                all_variables.entry(key.clone()).or_insert(val.clone());
            }
            for (key, val) in execution.process_variables() {
                all_variables.entry(key).or_insert(val);
            }
            current_id = execution.parent_id.clone();
        } else {
            break;
        }
    }
    all_variables
}

/// Builds a temporary execution used solely for in-flight EL evaluation.
///
/// Java parity: `ExecutionEntity` is a `VariableScope`; expression evaluation
/// resolves names via `VariableScopeImpl#getVariable`, which checks the
/// execution's own scope and then delegates to the parent chain. Rust's
/// `Execution::process_variable` only reads the row's own three maps, so this
/// helper clones the row and fills missing names from ancestors (nearest wins).
///
/// The process-instance scope row is merged as an outermost fallback for
/// topologies where `parent_id` does not reach it. The clone's own maps stay
/// intact so in-row precedence remains transient → local → process; inherited
/// values are inserted into `variables` only for names the row does not already
/// own in that map (same pattern as the P3-3 conditional-event path).
pub(crate) fn evaluation_execution(
    command_context: &mut CommandContext,
    execution: &crate::runtime::execution::Execution,
) -> crate::runtime::execution::Execution {
    let mut evaluation_execution = execution.clone();

    let mut inherited: HashMap<String, serde_json::Value> = HashMap::new();
    if let Some(parent_id) = execution.parent_id.clone() {
        let (store, session) = command_context.store_and_session();
        inherited = collect_execution_variables(&store, session, &parent_id);
    }

    let process_instance_id = execution
        .process_instance_id
        .clone()
        .unwrap_or_else(|| execution.id.clone());
    let root = {
        let (store, session) = command_context.store_and_session();
        store.find_execution(&process_instance_id, session)
    };
    if let Some(root_execution) = root {
        for (name, value) in root_execution.variables {
            inherited.entry(name).or_insert(value);
        }
        for (name, value) in root_execution.local_variables {
            inherited.entry(name).or_insert(value);
        }
        for (name, value) in root_execution.transient_variables {
            inherited.entry(name).or_insert(value);
        }
    }

    for (name, value) in inherited {
        evaluation_execution.variables.entry(name).or_insert(value);
    }
    evaluation_execution
}

pub struct SetVariableCmd {
    execution_id: String,
    name: String,
    value: serde_json::Value,
}

impl SetVariableCmd {
    pub fn new(execution_id: String, name: String, value: serde_json::Value) -> Self {
        Self {
            execution_id,
            name,
            value,
        }
    }
}

impl Command<()> for SetVariableCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // 1. Find which Execution in the ancestry tree already defines this variable
        let store = command_context.runtime_store.clone();
        let existing_owner = find_execution_variable(
            &store,
            &mut command_context.session,
            &self.execution_id,
            &self.name,
        );
        let event_type = if existing_owner.is_some() {
            crate::bpmn::behavior::variable_listener_event_behavior::VariableEventType::Update
        } else {
            crate::bpmn::behavior::variable_listener_event_behavior::VariableEventType::Create
        };
        let target_execution_id = if let Some((owner_id, _)) = existing_owner {
            owner_id
        } else {
            // 2. Default to the root process instance execution
            let mut current_id = self.execution_id.clone();
            let mut root_id = current_id.clone();
            while let Some(execution) =
                store.find_execution(&current_id, &mut command_context.session)
            {
                root_id = execution.id.clone();
                if let Some(parent) = execution.parent_id.clone() {
                    current_id = parent;
                } else {
                    break;
                }
            }
            root_id
        };

        if let Some(mut execution) =
            store.find_execution(&target_execution_id, &mut command_context.session)
        {

            // Java parity: `setVariable` updates the variable in the scope that already owns
            // it. When the owning scope holds the name as an execution-local variable, the
            // local copy must be updated in place rather than shadowed by a new global one.
            if execution.local_variables.contains_key(&self.name) {
                execution.set_local_variable(self.name.clone(), self.value.clone());
            } else {
                execution.set_process_variable(self.name.clone(), self.value.clone());
            }
            command_context
                .execution_entity_manager
                .update(&execution, &mut command_context.session);

            let process_instance_id = execution
                .process_instance_id
                .clone()
                .unwrap_or_else(|| execution.id.clone());
            let id = format!("{}:{}", execution.id, self.name);
            let store2 = command_context.runtime_store.clone();
            if store2
                .get_historic_variable_instance(&id, &mut command_context.session)
                .is_some()
            {
                command_context.history_manager.record_variable_updated(
                    &id,
                    self.value.clone(),
                    &mut command_context.session,
                );
            } else {
                command_context.history_manager.record_variable_created(
                    &id,
                    &self.name,
                    variable_type_name(&self.value),
                    self.value.clone(),
                    &process_instance_id,
                    Some(&execution.id),
                    None,
                    &mut command_context.session,
                );
            }

            crate::bpmn::behavior::variable_listener_event_behavior::evaluate_variable_listener_event_subprocesses(
                command_context,
                &process_instance_id,
                &self.name,
                &event_type,
            )?;
            Ok(())
        } else {
            Err(crate::error::FlowableError::NotFound(format!(
                "Execution '{}' was not found",
                target_execution_id
            )))
        }
    }
}

pub struct GetVariableCmd {
    execution_id: String,
    name: String,
}

impl GetVariableCmd {
    pub fn new(execution_id: String, name: String) -> Self {
        Self { execution_id, name }
    }
}

impl Command<Option<serde_json::Value>> for GetVariableCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<serde_json::Value>, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        Ok(
            find_execution_variable(&store, session, &self.execution_id, &self.name)
                .map(|(_, val)| val),
        )
    }
}

pub struct DeleteVariableCmd {
    execution_id: String,
    name: String,
}

impl DeleteVariableCmd {
    pub fn new(execution_id: String, name: String) -> Self {
        Self { execution_id, name }
    }
}

impl Command<()> for DeleteVariableCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        // Find which ancestor owns the variable. Java `VariableScopeImpl.removeVariable`
        // (VariableScopeImpl.java:801-811) only consults persistent instances
        // (`variableInstances.containsKey`) while walking the parent chain; transient
        // variables are never looked up nor removed here — they are managed by the
        // separate `removeTransientVariable` family (VariableScopeImpl.java:1027-1036).
        let store = command_context.runtime_store.clone();
        let target_execution_id = {
            let mut owner_id = None;
            let mut current_id = Some(self.execution_id.to_string());
            while let Some(id) = current_id {
                if let Some(execution) = store.find_execution(&id, &mut command_context.session) {
                    if execution.local_variables.contains_key(&self.name)
                        || execution.variables.contains_key(&self.name)
                    {
                        owner_id = Some(execution.id);
                        break;
                    }
                    current_id = execution.parent_id.clone();
                } else {
                    break;
                }
            }
            owner_id
        };
        let target_execution_id = target_execution_id.ok_or_else(|| {
            crate::error::FlowableError::NotFound(format!(
                "Variable '{}' was not found for execution '{}'",
                self.name, self.execution_id
            ))
        })?;

        let mut execution = store
            .find_execution(&target_execution_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Execution '{}' was not found",
                    target_execution_id
                ))
            })?;

        execution.variables.remove(&self.name);
        execution.local_variables.remove(&self.name);

        command_context
            .execution_entity_manager
            .update(&execution, &mut command_context.session);
        store.delete_variable_by_execution_id_and_name(
            &execution.id,
            &self.name,
            &mut command_context.session,
        );
        command_context.history_manager.record_variable_removed(
            &format!("{}:{}", execution.id, &self.name),
            &mut command_context.session,
        );

        let process_instance_id = execution
            .process_instance_id
            .clone()
            .unwrap_or_else(|| execution.id.clone());
        crate::bpmn::behavior::variable_listener_event_behavior::evaluate_variable_listener_event_subprocesses(
            command_context,
            &process_instance_id,
            &self.name,
            &crate::bpmn::behavior::variable_listener_event_behavior::VariableEventType::Delete,
        )?;
        Ok(())
    }
}

/// Shared lookup used by every execution-local command: Java's local-scope operations act
/// on one specific execution and raise `FlowableObjectNotFoundException` when it is absent,
/// instead of walking the parent chain.
pub(crate) fn require_execution(
    command_context: &mut CommandContext,
    execution_id: &str,
) -> Result<crate::runtime::execution::Execution, crate::error::FlowableError> {
    let (store, session) = command_context.store_and_session();
    store.find_execution(execution_id, session).ok_or_else(|| {
        crate::error::FlowableError::NotFound(format!("Execution '{}' was not found", execution_id))
    })
}

/// Records a local variable write in history, scoped to the owning execution.
///
/// Mirrors `record_task_local_variable`, which does the same for task-local variables.
fn record_execution_local_variable(
    command_context: &mut CommandContext,
    execution: &crate::runtime::execution::Execution,
    name: &str,
    value: serde_json::Value,
) {
    let id = format!("{}:{}", execution.id, name);
    let process_instance_id = execution
        .process_instance_id
        .clone()
        .unwrap_or_else(|| execution.id.clone());
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
            variable_type_name(&value),
            value,
            &process_instance_id,
            Some(&execution.id),
            None,
            &mut command_context.session,
        );
    }
}

/// Java parity: `RuntimeService#setVariableLocal` / `#setVariablesLocal`.
pub struct SetVariablesLocalCmd {
    execution_id: String,
    variables: HashMap<String, serde_json::Value>,
}

impl SetVariablesLocalCmd {
    pub fn new(execution_id: String, variables: HashMap<String, serde_json::Value>) -> Self {
        Self {
            execution_id,
            variables,
        }
    }
}

impl Command<()> for SetVariablesLocalCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut execution = require_execution(command_context, &self.execution_id)?;
        for (name, value) in &self.variables {
            execution.set_local_variable(name.clone(), value.clone());
        }
        command_context
            .execution_entity_manager
            .update(&execution, &mut command_context.session);
        for (name, value) in &self.variables {
            record_execution_local_variable(command_context, &execution, name, value.clone());
        }
        Ok(())
    }
}

/// Java parity: `RuntimeService#setVariableAsync` / `#setVariablesAsync` /
/// `#setVariableLocalAsync` / `#setVariablesLocalAsync`
/// (`SetAsyncExecutionVariablesCmd`). No variable is written synchronously: the
/// values ride as the payload of a `set-async-variables` job on the execution,
/// and become visible only when the async executor runs that job
/// (`SetAsyncVariablesJobHandler`: a local payload is applied with
/// `setVariableLocal`, a global one with `setVariable`).
pub struct SetAsyncExecutionVariablesCmd {
    execution_id: String,
    variables: HashMap<String, serde_json::Value>,
    is_local: bool,
}

impl SetAsyncExecutionVariablesCmd {
    pub fn new(
        execution_id: String,
        variables: HashMap<String, serde_json::Value>,
        is_local: bool,
    ) -> Self {
        Self {
            execution_id,
            variables,
            is_local,
        }
    }
}

impl Command<()> for SetAsyncExecutionVariablesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let execution = require_execution(command_context, &self.execution_id)?;
        // Java `NeedsActiveExecutionCmd` with the overridden prefix of
        // `SetAsyncExecutionVariablesCmd#getSuspendedExceptionMessagePrefix`.
        if execution.is_suspended {
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "Cannot set variables to a suspended execution '{}'",
                execution.id
            )));
        }
        // Java: `if (variables != null && !variables.isEmpty())` — an empty map
        // validates the execution but schedules no job.
        if !self.variables.is_empty() {
            create_set_async_variables_job(
                command_context,
                &execution,
                &self.variables,
                self.is_local,
            );
        }
        Ok(())
    }
}

/// Payload stored in `job_handler_configuration` of a `set-async-variables` job.
///
/// Java keeps the pending values as `VariableInstanceEntity` rows with scope type
/// `bpmn-async-variables` and `metaInfo = String.valueOf(isLocal)`; the Rust engine
/// has no runtime variable table, so — like the CMMN engine's
/// `cmmn-set-async-variables` handler — the values travel inside the job row.
/// Through the public API surface this is indistinguishable: pending values are not
/// visible until the job applies them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SetAsyncVariablesPayload {
    #[serde(default)]
    variables: HashMap<String, serde_json::Value>,
    /// Java `metaInfo`: `"true"` → `setVariableLocal`, otherwise `setVariable`.
    #[serde(default, rename = "isLocal")]
    is_local: bool,
}

/// Java `SetAsyncExecutionVariablesCmd#createSetAsyncVariablesJob`: an async job
/// attached to the execution, due immediately, with the async executor's retries.
pub(crate) fn create_set_async_variables_job(
    command_context: &mut CommandContext,
    execution: &crate::runtime::execution::Execution,
    variables: &HashMap<String, serde_json::Value>,
    is_local: bool,
) {
    let payload = serde_json::to_string(&SetAsyncVariablesPayload {
        variables: variables.clone(),
        is_local,
    })
    .expect("serializing a string-keyed map cannot fail");
    let process_instance_id = execution
        .process_instance_id
        .clone()
        .unwrap_or_else(|| execution.id.clone());
    let store = command_context.runtime_store.clone();
    let now = store.time_source().now().timestamp_millis();
    let mut job = RuntimeTimerJobState {
        timer_job_id: uuid::Uuid::new_v4().to_string(),
        process_instance_id,
        execution_id: execution.id.clone(),
        activity_id: execution.activity_id.clone().unwrap_or_default(),
        // Same family as async continuations: acquired by `acquire_async_jobs`,
        // listed as an executable job, executed via `ExecuteTimerWorkCmd`.
        job_state: Some("async".to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: None,
        time_date: None,
        time_cycle: None,
        end_date: None,
        due_time: Some(now),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(
            command_context
                .config
                .async_executor
                .number_of_retries
                .max(0),
        ),
        error_message: None,
        error_details: None,
        category: None,
        job_handler_configuration: Some(payload),
        // Java SetAsyncExecutionVariablesCmd.java:91: createAsyncJob(job, true).
        exclusive: true,
        ..Default::default()
    };
    stamp_new_job_metadata(
        &mut job,
        now,
        job_handler_types::SET_ASYNC_VARIABLES,
        execution.tenant_id.clone(),
        execution.process_definition_id.clone(),
        execution.activity_name.clone(),
    );
    store.insert_timer_job_state(&job, &mut command_context.session);
}

/// Java `SetAsyncVariablesJobHandler#execute`: applies the pending values to the
/// job's execution (`setVariableLocal` for `isLocal`, `setVariable` otherwise) and
/// consumes the job. Called from `ExecuteTimerWorkCmd`'s handler dispatch.
pub(crate) fn execute_set_async_variables_job(
    command_context: &mut CommandContext,
    job: &RuntimeTimerJobState,
) -> Result<(), crate::error::FlowableError> {
    let payload: SetAsyncVariablesPayload = match job.job_handler_configuration.as_deref() {
        Some(raw) if !raw.trim().is_empty() => serde_json::from_str(raw).map_err(|e| {
            crate::error::FlowableError::ExecutionError(format!(
                "Invalid set-async-variables job '{}' payload: {e}",
                job.timer_job_id
            ))
        })?,
        _ => SetAsyncVariablesPayload {
            variables: HashMap::new(),
            is_local: false,
        },
    };

    if payload.is_local {
        let mut execution = {
            let (store, session) = command_context.store_and_session();
            store.find_execution(&job.execution_id, session)
        }
        .ok_or_else(|| {
            // ExecutionError (not NotFound) so REST job execute maps to 500 like Java,
            // matching the async continuation job handler.
            crate::error::FlowableError::ExecutionError(format!(
                "Execution '{}' for set-async-variables job '{}' not found",
                job.execution_id, job.timer_job_id
            ))
        })?;
        for (name, value) in &payload.variables {
            execution.set_local_variable(name.clone(), value.clone());
        }
        command_context
            .execution_entity_manager
            .update(&execution, &mut command_context.session);
        for (name, value) in &payload.variables {
            record_execution_local_variable(command_context, &execution, name, value.clone());
        }
    } else {
        // Java: `executionEntity.setVariable(name, value)` — the owning-scope
        // resolution of `SetVariableCmd`.
        for (name, value) in payload.variables {
            SetVariableCmd::new(job.execution_id.clone(), name, value).execute(command_context)?;
        }
    }

    let store = command_context.runtime_store.clone();
    store.delete_timer_job_state(&job.timer_job_id, &mut command_context.session);
    Ok(())
}

/// Java parity: `RuntimeService#getVariablesLocal`.
pub struct GetVariablesLocalCmd {
    execution_id: String,
}

impl GetVariablesLocalCmd {
    pub fn new(execution_id: String) -> Self {
        Self { execution_id }
    }
}

impl Command<HashMap<String, serde_json::Value>> for GetVariablesLocalCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HashMap<String, serde_json::Value>, crate::error::FlowableError> {
        let execution = require_execution(command_context, &self.execution_id)?;
        // Java `VariableScopeImpl.getVariablesLocal` (VariableScopeImpl.java:455-469):
        // persistent locals first, then transient entries overwrite same names.
        let mut variables = execution.local_variables.clone();
        for (name, value) in &execution.transient_variables {
            variables.insert(name.clone(), value.clone());
        }
        Ok(variables)
    }
}

/// Java parity: `RuntimeService#getVariableLocal`.
pub struct GetVariableLocalCmd {
    execution_id: String,
    name: String,
}

impl GetVariableLocalCmd {
    pub fn new(execution_id: String, name: String) -> Self {
        Self { execution_id, name }
    }
}

impl Command<Option<serde_json::Value>> for GetVariableLocalCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<serde_json::Value>, crate::error::FlowableError> {
        let execution = require_execution(command_context, &self.execution_id)?;
        // Java `VariableScopeImpl.getVariableInstanceLocal` (VariableScopeImpl.java:348-352):
        // transient variables shadow same-named persistent locals.
        Ok(execution
            .transient_variables
            .get(&self.name)
            .or_else(|| execution.local_variables.get(&self.name))
            .cloned())
    }
}

/// Java parity: `RuntimeService#hasVariableLocal` — strictly this execution's own scope.
pub struct HasVariableLocalCmd {
    execution_id: String,
    name: String,
}

impl HasVariableLocalCmd {
    pub fn new(execution_id: String, name: String) -> Self {
        Self { execution_id, name }
    }
}

impl Command<bool> for HasVariableLocalCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<bool, crate::error::FlowableError> {
        let execution = require_execution(command_context, &self.execution_id)?;
        // Java `VariableScopeImpl.hasVariableLocal` (VariableScopeImpl.java:425-427):
        // a transient variable alone answers true.
        Ok(execution.transient_variables.contains_key(&self.name)
            || execution.local_variables.contains_key(&self.name))
    }
}

/// Java parity: `RuntimeService#hasVariable` — follows the parent chain.
pub struct HasVariableCmd {
    execution_id: String,
    name: String,
}

impl HasVariableCmd {
    pub fn new(execution_id: String, name: String) -> Self {
        Self { execution_id, name }
    }
}

impl Command<bool> for HasVariableCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<bool, crate::error::FlowableError> {
        require_execution(command_context, &self.execution_id)?;
        let (store, session) = command_context.store_and_session();
        Ok(find_execution_variable(&store, session, &self.execution_id, &self.name).is_some())
    }
}

/// Java parity: `RuntimeService#removeVariableLocal` / `#removeVariablesLocal`.
///
/// Removal is scoped to this execution only: an ancestor's same-named variable survives.
pub struct RemoveVariablesLocalCmd {
    execution_id: String,
    names: Vec<String>,
}

impl RemoveVariablesLocalCmd {
    pub fn new(execution_id: String, names: Vec<String>) -> Self {
        Self {
            execution_id,
            names,
        }
    }
}

impl Command<()> for RemoveVariablesLocalCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut execution = require_execution(command_context, &self.execution_id)?;
        let removed = self
            .names
            .iter()
            .filter(|name| execution.local_variables.remove(*name).is_some())
            .cloned()
            .collect::<Vec<_>>();
        if removed.is_empty() {
            return Ok(());
        }
        command_context
            .execution_entity_manager
            .update(&execution, &mut command_context.session);
        let store = command_context.runtime_store.clone();
        for name in &removed {
            store.delete_variable_by_execution_id_and_name(
                &execution.id,
                name,
                &mut command_context.session,
            );
            command_context.history_manager.record_variable_removed(
                &format!("{}:{}", execution.id, name),
                &mut command_context.session,
            );
        }
        Ok(())
    }
}

pub struct GetVariablesCmd {
    execution_id: String,
}

impl GetVariablesCmd {
    pub fn new(execution_id: String) -> Self {
        Self { execution_id }
    }
}

impl Command<HashMap<String, serde_json::Value>> for GetVariablesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HashMap<String, serde_json::Value>, crate::error::FlowableError> {
        let (store, session) = command_context.store_and_session();
        Ok(collect_execution_variables(
            &store,
            session,
            &self.execution_id,
        ))
    }
}

use crate::engine::query::{Direction, Query, QueryState};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VariableInstance {
    pub id: String,
    pub execution_id: String,
    pub process_instance_id: String,
    pub name: String,
    pub value: serde_json::Value,
    pub variable_type: String,
}

pub struct VariableInstanceQuery {
    state: QueryState<VariableInstance>,
}

impl VariableInstanceQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
        }
    }
}

pub struct VariableInstanceQueryCmd {
    _query: VariableInstanceQuery,
}

impl VariableInstanceQueryCmd {
    pub fn new(query: VariableInstanceQuery) -> Self {
        Self { _query: query }
    }
}

impl Command<Vec<VariableInstance>> for VariableInstanceQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<VariableInstance>, crate::error::FlowableError> {
        let rows = command_context.session().find_raw_all("variables").unwrap();

        Ok(rows
            .into_iter()
            .map(|r| {
                let value = serde_json::from_str::<serde_json::Value>(&r.data).unwrap();
                VariableInstance {
                    id: r.id,
                    execution_id: r
                        .extras
                        .get("execution_id")
                        .cloned()
                        .flatten()
                        .unwrap_or_default(),
                    process_instance_id: r
                        .extras
                        .get("process_instance_id")
                        .cloned()
                        .flatten()
                        .unwrap_or_default(),
                    name: r.extras.get("name").cloned().flatten().unwrap_or_default(),
                    variable_type: variable_type_name(&value).to_string(),
                    value,
                }
            })
            .collect())
    }
}

pub fn variable_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        serde_json::Value::Number(_) => "double",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => "json",
    }
}

impl Query<VariableInstance, VariableInstanceQuery> for VariableInstanceQuery {
    fn list(&self) -> Result<Vec<VariableInstance>, crate::error::FlowableError> {
        let query_clone = VariableInstanceQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
        };
        let cmd = VariableInstanceQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<VariableInstance>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

pub struct VariableService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl VariableService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    pub fn create_variable_instance_query(&self) -> VariableInstanceQuery {
        VariableInstanceQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn set_variable(
        &self,
        execution_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SetVariableCmd::new(execution_id, name, value);
        self.command_executor.execute(&cmd)
    }

    pub fn get_variable(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<Option<serde_json::Value>, crate::error::FlowableError> {
        let cmd = GetVariableCmd::new(execution_id, name);
        self.command_executor.execute(&cmd)
    }

    pub fn get_variables(
        &self,
        execution_id: String,
    ) -> Result<HashMap<String, serde_json::Value>, crate::error::FlowableError> {
        let cmd = GetVariablesCmd::new(execution_id);
        self.command_executor.execute(&cmd)
    }

    pub fn delete_variable(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = DeleteVariableCmd::new(execution_id, name);
        self.command_executor.execute(&cmd)
    }

    pub fn set_variable_local(
        &self,
        execution_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SetVariablesLocalCmd::new(execution_id, HashMap::from([(name, value)]));
        self.command_executor.execute(&cmd)
    }

    pub fn set_variables_local(
        &self,
        execution_id: String,
        variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SetVariablesLocalCmd::new(execution_id, variables);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariableAsync`. The value is not written
    /// synchronously; a `set-async-variables` job applies it with owning-scope
    /// (`setVariable`) resolution when the async executor runs it.
    pub fn set_variable_async(
        &self,
        execution_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd =
            SetAsyncExecutionVariablesCmd::new(execution_id, HashMap::from([(name, value)]), false);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariablesAsync`.
    pub fn set_variables_async(
        &self,
        execution_id: String,
        variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SetAsyncExecutionVariablesCmd::new(execution_id, variables, false);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariableLocalAsync`. The value is not written
    /// synchronously; a `set-async-variables` job applies it to this execution's own
    /// scope when the async executor runs it.
    pub fn set_variable_local_async(
        &self,
        execution_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd =
            SetAsyncExecutionVariablesCmd::new(execution_id, HashMap::from([(name, value)]), true);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariablesLocalAsync`.
    pub fn set_variables_local_async(
        &self,
        execution_id: String,
        variables: HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SetAsyncExecutionVariablesCmd::new(execution_id, variables, true);
        self.command_executor.execute(&cmd)
    }

    pub fn get_variable_local(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<Option<serde_json::Value>, crate::error::FlowableError> {
        let cmd = GetVariableLocalCmd::new(execution_id, name);
        self.command_executor.execute(&cmd)
    }

    pub fn get_variables_local(
        &self,
        execution_id: String,
    ) -> Result<HashMap<String, serde_json::Value>, crate::error::FlowableError> {
        let cmd = GetVariablesLocalCmd::new(execution_id);
        self.command_executor.execute(&cmd)
    }

    pub fn has_variable_local(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<bool, crate::error::FlowableError> {
        let cmd = HasVariableLocalCmd::new(execution_id, name);
        self.command_executor.execute(&cmd)
    }

    pub fn has_variable(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<bool, crate::error::FlowableError> {
        let cmd = HasVariableCmd::new(execution_id, name);
        self.command_executor.execute(&cmd)
    }

    pub fn remove_variable_local(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = RemoveVariablesLocalCmd::new(execution_id, vec![name]);
        self.command_executor.execute(&cmd)
    }

    pub fn remove_variables_local(
        &self,
        execution_id: String,
        names: Vec<String>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = RemoveVariablesLocalCmd::new(execution_id, names);
        self.command_executor.execute(&cmd)
    }

    /// Atomic batch mutation of one execution scope with Java REST create/update
    /// semantics. See [`crate::cmd::execution_variable_cmd`].
    pub fn mutate_variables_on_scope(
        &self,
        execution_id: String,
        scope: ExecutionVariableScope,
        mode: VariableMutationMode,
        mutations: Vec<ExecutionVariableMutation>,
    ) -> Result<HashMap<String, serde_json::Value>, crate::error::FlowableError> {
        let cmd = MutateExecutionVariablesCmd::new(execution_id, scope, mode, mutations);
        self.command_executor.execute(&cmd)
    }

    /// The async counterpart of [`Self::mutate_variables_on_scope`] (Java REST
    /// `createExecutionVariable` / `setSimpleVariable` with `async = true`):
    /// the same synchronous scope/mode validation, then a
    /// `set-async-variables` job on the resolved target instead of an
    /// immediate write.
    pub fn mutate_variables_on_scope_async(
        &self,
        execution_id: String,
        scope: ExecutionVariableScope,
        mode: VariableMutationMode,
        mutations: Vec<ExecutionVariableMutation>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = MutateExecutionVariablesAsyncCmd::new(execution_id, scope, mode, mutations);
        self.command_executor.execute(&cmd)
    }

    /// Removes named variables from one execution scope, or every variable of
    /// that scope when `names` is `None`.
    pub fn remove_variables_on_scope(
        &self,
        execution_id: String,
        scope: ExecutionVariableScope,
        names: Option<Vec<String>>,
        require_exists: bool,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = RemoveExecutionVariablesCmd::new(execution_id, scope, names, require_exists);
        self.command_executor.execute(&cmd)
    }

    /// Java REST single-variable read: local first, parent (global) scope as
    /// fallback, or one explicit scope.
    pub fn get_variable_on_scope(
        &self,
        execution_id: String,
        name: String,
        scope: Option<ExecutionVariableScope>,
    ) -> Result<Option<(serde_json::Value, ExecutionVariableScope)>, crate::error::FlowableError>
    {
        let cmd = GetScopedExecutionVariableCmd::new(execution_id, name, scope);
        self.command_executor.execute(&cmd)
    }

    /// Java REST collection read: local variables shadow the parent chain.
    pub fn get_variables_on_scope(
        &self,
        execution_id: String,
        scope: Option<ExecutionVariableScope>,
    ) -> Result<Vec<(String, serde_json::Value, ExecutionVariableScope)>, crate::error::FlowableError>
    {
        let cmd = GetScopedExecutionVariablesCmd::new(execution_id, scope);
        self.command_executor.execute(&cmd)
    }
}
