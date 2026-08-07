//! Atomic, typed execution-variable mutation commands with local/global scope
//! and create-only/update-only/upsert modes.
//!
//! Java parity: mirrors the scope and mode semantics of
//! `BaseExecutionVariableResource` / `BaseVariableCollectionResource`, which the
//! Java REST execution- and process-instance-variable resources share:
//!
//! * the local scope is the execution's own variable scope
//!   (`RuntimeService#setVariableLocal`);
//! * the global scope is the PARENT execution (`execution.getParentId()`), and a
//!   root execution has no global scope at all;
//! * `hasVariableOnScope` decides create conflicts and update misses per scope,
//!   so the same name can live on both scopes independently.
//!
//! All validation for a batch happens before any write; combined with the
//! command executor's session rollback on error this makes every mutation
//! command atomic.
//!
//! The `*AsyncCmd` variant runs the same synchronous validation but schedules a
//! `set-async-variables` job instead of writing, mirroring the Java REST
//! `variables-async` endpoints (`createExecutionVariable` with `async = true`).

use crate::cmd::task_variable_cmd::VariableMutationMode;
use crate::engine::variable_service::{find_execution_variable, variable_type_name};
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use std::collections::HashMap;

/// Variable scope for an execution-variable mutation, mirroring Java's
/// `RestVariableScope` as it is applied by `BaseExecutionVariableResource`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionVariableScope {
    /// The execution's own scope (Java `setVariableLocal` / `getVariableLocal`).
    Local,
    /// The parent execution's scope (Java uses `execution.getParentId()`).
    Global,
}

/// A single name/value pair to apply to the resolved scope.
#[derive(Clone, Debug)]
pub struct ExecutionVariableMutation {
    pub name: String,
    pub value: serde_json::Value,
}

/// Loads the execution or fails with the message the other execution-variable
/// commands use.
fn load_execution(
    command_context: &mut CommandContext,
    execution_id: &str,
) -> Result<Execution, FlowableError> {
    let (store, session) = command_context.store_and_session();
    store.find_execution(execution_id, session).ok_or_else(|| {
        FlowableError::NotFound(format!("Execution '{}' was not found", execution_id))
    })
}

/// Java `SetExecutionVariablesCmd#getSuspendedExceptionMessagePrefix` /
/// `SetAsyncExecutionVariablesCmd`: "Cannot set variables to".
fn require_active_for_set(execution: &Execution) -> Result<(), FlowableError> {
    if execution.is_suspended {
        return Err(FlowableError::ExecutionError(format!(
            "Cannot set variables to a suspended execution '{}'",
            execution.id
        )));
    }
    Ok(())
}

/// Java `RemoveExecutionVariablesCmd#getSuspendedExceptionMessagePrefix`:
/// "Cannot remove variables from".
fn require_active_for_remove(execution: &Execution) -> Result<(), FlowableError> {
    if execution.is_suspended {
        return Err(FlowableError::ExecutionError(format!(
            "Cannot remove variables from a suspended execution '{}'",
            execution.id
        )));
    }
    Ok(())
}

/// Resolves the execution the global scope writes to. Java uses
/// `execution.getParentId()` and rejects the write when the execution is a root
/// (`BaseVariableCollectionResource`: "task is not part of process.").
/// Suspension is checked separately on the resolved target (after the REST-layer
/// mode checks), matching `NeedsActiveExecutionCmd` on the write command.
fn resolve_global_target(
    command_context: &mut CommandContext,
    execution: &Execution,
    verb: &str,
) -> Result<Execution, FlowableError> {
    let Some(parent_id) = execution.parent_id.clone() else {
        return Err(FlowableError::BadRequest(format!(
            "Cannot {} global variables on execution '{}', task is not part of process.",
            verb, execution.id
        )));
    };
    load_execution(command_context, &parent_id)
}

/// The names this execution owns in its own (local) scope.
///
/// In Java an `ExecutionEntityImpl` *is* a `VariableScope`, so one execution row
/// has exactly one variable scope. Rust splits the same row into two persisted
/// maps, so the Java-equivalent row-level scope is their union, with
/// `local_variables` winning a name clash (the same precedence
/// `Execution::process_variable` applies).
fn local_scope_contains(execution: &Execution, name: &str) -> bool {
    execution.local_variables.contains_key(name) || execution.variables.contains_key(name)
}

/// The value this execution owns in its own (local) scope, if any.
fn local_scope_value(execution: &Execution, name: &str) -> Option<serde_json::Value> {
    execution
        .local_variables
        .get(name)
        .or_else(|| execution.variables.get(name))
        .cloned()
}

/// Every name/value pair of this execution's own (local) scope.
fn local_scope_variables(execution: &Execution) -> HashMap<String, serde_json::Value> {
    let mut variables = execution.variables.clone();
    for (name, value) in &execution.local_variables {
        variables.insert(name.clone(), value.clone());
    }
    variables
}

/// Writes into the execution's own scope without creating a shadowed duplicate:
/// a name the row already holds as a process variable is updated in place, a new
/// name lands in the local map.
fn set_local_scope_variable(execution: &mut Execution, name: &str, value: serde_json::Value) {
    if !execution.local_variables.contains_key(name) && execution.variables.contains_key(name) {
        execution.set_process_variable(name.to_string(), value);
    } else {
        execution.set_local_variable(name.to_string(), value);
    }
}

/// Java `hasVariableOnScope`: the local scope checks this execution's own
/// variables, the global scope resolves through the parent chain starting at the
/// parent execution.
fn is_variable_present(
    command_context: &mut CommandContext,
    execution: &Execution,
    scope: ExecutionVariableScope,
    name: &str,
) -> bool {
    match scope {
        ExecutionVariableScope::Local => local_scope_contains(execution, name),
        ExecutionVariableScope::Global => {
            let Some(parent_id) = execution.parent_id.clone() else {
                return false;
            };
            let store = command_context.runtime_store.clone();
            find_execution_variable(&store, &mut command_context.session, &parent_id, name)
                .is_some()
        }
    }
}

/// Records a variable write in history against its owning execution.
fn record_variable(
    command_context: &mut CommandContext,
    execution_id: &str,
    process_instance_id: &str,
    name: &str,
    value: serde_json::Value,
) {
    let id = format!("{}:{}", execution_id, name);
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
            process_instance_id,
            Some(execution_id),
            None,
            &mut command_context.session,
        );
    }
}

/// Shared logic for `MutateExecutionVariablesCmd`. Validates the whole batch
/// against the current state of the resolved scope and only then applies it,
/// returning the resulting variable map of that scope.
pub(crate) fn mutate_execution_variables(
    command_context: &mut CommandContext,
    execution_id: &str,
    scope: ExecutionVariableScope,
    mode: VariableMutationMode,
    mutations: Vec<ExecutionVariableMutation>,
) -> Result<HashMap<String, serde_json::Value>, FlowableError> {
    let execution = load_execution(command_context, execution_id)?;

    for mutation in &mutations {
        if mutation.name.is_empty() {
            return Err(FlowableError::BadRequest(
                "Variable name is required".to_string(),
            ));
        }
    }

    // Java `BaseExecutionVariableResource.setVariable:217-224` runs the
    // scope-strict `hasVariableOnScope` mode checks BEFORE the engine cmd
    // (and thus before `NeedsActiveExecutionCmd`). An update miss is a 404
    // even on a suspended execution — the suspended guard is unreachable.
    // Only a batch that passes the mode checks reaches target resolution and
    // the suspended guard (`SetExecutionVariablesCmd`, prefix
    // "Cannot set variables to"; see also the async path below).
    for mutation in &mutations {
        let present = is_variable_present(command_context, &execution, scope, &mutation.name);
        match mode {
            VariableMutationMode::CreateOnly if present => {
                return Err(FlowableError::Conflict(format!(
                    "Variable '{}' is already present on execution '{}'.",
                    mutation.name, execution.id
                )));
            }
            VariableMutationMode::UpdateOnly if !present => {
                return Err(FlowableError::NotFound(format!(
                    "Execution '{}' does not have a variable with name: '{}'.",
                    execution.id, mutation.name
                )));
            }
            _ => {}
        }
    }

    // Resolving the global target is itself a validation step (root executions
    // have no global scope → 400). The suspended guard applies to the TARGET
    // of the write (the execution itself for LOCAL, the parent for GLOBAL),
    // matching Java `NeedsActiveExecutionCmd` on `SetExecutionVariablesCmd`.
    let mut target = match scope {
        ExecutionVariableScope::Local => execution.clone(),
        ExecutionVariableScope::Global => {
            resolve_global_target(command_context, &execution, "set")?
        }
    };
    require_active_for_set(&target)?;

    let process_instance_id = target
        .process_instance_id
        .clone()
        .unwrap_or_else(|| target.id.clone());
    for mutation in &mutations {
        match scope {
            ExecutionVariableScope::Local => {
                set_local_scope_variable(&mut target, &mutation.name, mutation.value.clone());
            }
            ExecutionVariableScope::Global => {
                // Java writes the global scope through `setVariable` on the
                // parent, which updates the variable in place on whichever
                // ancestor already owns it; a new name lands on the parent.
                target.set_process_variable(mutation.name.clone(), mutation.value.clone());
            }
        }
    }
    command_context
        .execution_entity_manager
        .update(&target, &mut command_context.session);
    let target_id = target.id.clone();
    // The runtime `variables` projection is dual-written from both maps by the
    // entity-manager update above (`RuntimeStore::insert_execution` projects
    // `variables` ∪ `local_variables`), so no cmd-layer upsert is needed here.
    for mutation in &mutations {
        record_variable(
            command_context,
            &target_id,
            &process_instance_id,
            &mutation.name,
            mutation.value.clone(),
        );
    }

    Ok(match scope {
        ExecutionVariableScope::Local => local_scope_variables(&target),
        ExecutionVariableScope::Global => target.process_variables(),
    })
}

/// Shared logic for `MutateExecutionVariablesAsyncCmd`. Runs the same
/// synchronous validation Java REST applies before dispatching
/// `SetAsyncExecutionVariablesCmd` — the `hasVariableOnScope` mode checks and
/// the scope-target resolution happen in the request thread
/// (`BaseExecutionVariableResource.setVariable`,
/// `BaseVariableCollectionResource.createExecutionVariable`) — but instead of
/// writing it schedules a `set-async-variables` job on the resolved target:
/// the execution itself for the local scope, the parent for the global scope.
pub(crate) fn mutate_execution_variables_async(
    command_context: &mut CommandContext,
    execution_id: &str,
    scope: ExecutionVariableScope,
    mode: VariableMutationMode,
    mutations: Vec<ExecutionVariableMutation>,
) -> Result<(), FlowableError> {
    let execution = load_execution(command_context, execution_id)?;

    for mutation in &mutations {
        if mutation.name.is_empty() {
            return Err(FlowableError::BadRequest(
                "Variable name is required".to_string(),
            ));
        }
    }

    // Java runs the scope-strict `hasVariableOnScope` mode checks BEFORE the
    // async command is dispatched, so an update miss is a 404 even on a
    // suspended execution — the dispatch-time guard is never reached.
    for mutation in &mutations {
        let present = is_variable_present(command_context, &execution, scope, &mutation.name);
        match mode {
            VariableMutationMode::CreateOnly if present => {
                return Err(FlowableError::Conflict(format!(
                    "Variable '{}' is already present on execution '{}'.",
                    mutation.name, execution.id
                )));
            }
            VariableMutationMode::UpdateOnly if !present => {
                return Err(FlowableError::NotFound(format!(
                    "Execution '{}' does not have a variable with name: '{}'.",
                    execution.id, mutation.name
                )));
            }
            _ => {}
        }
    }

    // A root execution has no global target at all
    // (`BaseVariableCollectionResource`: "task is not part of process."). The
    // dispatch-time suspended guard applies to the TARGET execution, with
    // `SetAsyncExecutionVariablesCmd#getSuspendedExceptionMessagePrefix`.
    let target = match scope {
        ExecutionVariableScope::Local => execution,
        ExecutionVariableScope::Global => {
            resolve_global_target(command_context, &execution, "set")?
        }
    };
    if target.is_suspended {
        return Err(FlowableError::ExecutionError(format!(
            "Cannot set variables to a suspended execution '{}'",
            target.id
        )));
    }

    // Java `SetAsyncExecutionVariablesCmd`: an empty map validates the
    // execution but schedules no job.
    if !mutations.is_empty() {
        let variables: HashMap<String, serde_json::Value> = mutations
            .into_iter()
            .map(|mutation| (mutation.name, mutation.value))
            .collect();
        crate::engine::variable_service::create_set_async_variables_job(
            command_context,
            &target,
            &variables,
            scope == ExecutionVariableScope::Local,
        );
    }
    Ok(())
}

/// Shared logic for `RemoveExecutionVariablesCmd`. `names = None` removes ALL
/// variables of the resolved scope (Java REST DELETE on the variable
/// collection, which is local-only). `require_exists` mirrors the Java REST
/// DELETE single-variable semantics: an absent name 404s before anything is
/// removed.
pub(crate) fn remove_execution_variables(
    command_context: &mut CommandContext,
    execution_id: &str,
    scope: ExecutionVariableScope,
    names: Option<Vec<String>>,
    require_exists: bool,
) -> Result<(), FlowableError> {
    let execution = load_execution(command_context, execution_id)?;

    if names.is_none() && scope == ExecutionVariableScope::Global {
        return Err(FlowableError::BadRequest(
            "Removing all variables is only supported for execution-local variables".to_string(),
        ));
    }

    // Java `ExecutionVariableResource.deleteVariable:197-199` runs
    // `hasVariableOnScope` before the write cmd, so an absent name is a 404
    // even on a suspended execution (or when the scope has no write target).
    if require_exists && let Some(ref names) = names {
        for name in names {
            if !is_variable_present(command_context, &execution, scope, name) {
                return Err(FlowableError::NotFound(format!(
                    "Execution '{}' does not have a variable '{}' in scope {}",
                    execution.id,
                    name,
                    match scope {
                        ExecutionVariableScope::Local => "local",
                        ExecutionVariableScope::Global => "global",
                    }
                )));
            }
        }
    }

    // Suspended guard after the existence checks, on the write TARGET, with
    // Java `RemoveExecutionVariablesCmd` prefix "Cannot remove variables from".
    let mut target = match scope {
        ExecutionVariableScope::Local => execution.clone(),
        ExecutionVariableScope::Global => {
            resolve_global_target(command_context, &execution, "remove")?
        }
    };
    require_active_for_remove(&target)?;

    let removed: Vec<String> = match scope {
        ExecutionVariableScope::Local => match &names {
            Some(names) => names
                .iter()
                .filter(|name| local_scope_contains(&target, name))
                .cloned()
                .collect(),
            None => local_scope_variables(&target).keys().cloned().collect(),
        },
        ExecutionVariableScope::Global => {
            let names = names.clone().unwrap_or_default();
            names
                .iter()
                .filter(|name| target.process_variable(name).is_some())
                .cloned()
                .collect()
        }
    };
    if removed.is_empty() {
        return Ok(());
    }

    for name in &removed {
        match scope {
            ExecutionVariableScope::Local => {
                // The row-level scope spans both maps, so both copies go.
                target.local_variables.remove(name);
                target.variables.remove(name);
            }
            ExecutionVariableScope::Global => {
                target.variables.remove(name);
                target.local_variables.remove(name);
                target.transient_variables.remove(name);
            }
        }
    }
    command_context
        .execution_entity_manager
        .update(&target, &mut command_context.session);
    let store = command_context.runtime_store.clone();
    for name in &removed {
        store.delete_variable_by_execution_id_and_name(
            &target.id,
            name,
            &mut command_context.session,
        );
        command_context.history_manager.record_variable_removed(
            &format!("{}:{}", target.id, name),
            &mut command_context.session,
        );
    }
    Ok(())
}

/// Atomic batch mutation of an execution's variables on one scope.
pub struct MutateExecutionVariablesCmd {
    execution_id: String,
    scope: ExecutionVariableScope,
    mode: VariableMutationMode,
    mutations: Vec<ExecutionVariableMutation>,
}

impl MutateExecutionVariablesCmd {
    pub fn new(
        execution_id: String,
        scope: ExecutionVariableScope,
        mode: VariableMutationMode,
        mutations: Vec<ExecutionVariableMutation>,
    ) -> Self {
        Self {
            execution_id,
            scope,
            mode,
            mutations,
        }
    }
}

impl Command<HashMap<String, serde_json::Value>> for MutateExecutionVariablesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HashMap<String, serde_json::Value>, FlowableError> {
        mutate_execution_variables(
            command_context,
            &self.execution_id,
            self.scope,
            self.mode,
            self.mutations.clone(),
        )
    }
}

/// Async batch mutation of an execution's variables on one scope: the Java REST
/// synchronous validation, then a `set-async-variables` job performs the write.
pub struct MutateExecutionVariablesAsyncCmd {
    execution_id: String,
    scope: ExecutionVariableScope,
    mode: VariableMutationMode,
    mutations: Vec<ExecutionVariableMutation>,
}

impl MutateExecutionVariablesAsyncCmd {
    pub fn new(
        execution_id: String,
        scope: ExecutionVariableScope,
        mode: VariableMutationMode,
        mutations: Vec<ExecutionVariableMutation>,
    ) -> Self {
        Self {
            execution_id,
            scope,
            mode,
            mutations,
        }
    }
}

impl Command<()> for MutateExecutionVariablesAsyncCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        mutate_execution_variables_async(
            command_context,
            &self.execution_id,
            self.scope,
            self.mode,
            self.mutations.clone(),
        )
    }
}

/// Atomic removal of an execution's variables from one scope.
pub struct RemoveExecutionVariablesCmd {
    execution_id: String,
    scope: ExecutionVariableScope,
    names: Option<Vec<String>>,
    require_exists: bool,
}

impl RemoveExecutionVariablesCmd {
    pub fn new(
        execution_id: String,
        scope: ExecutionVariableScope,
        names: Option<Vec<String>>,
        require_exists: bool,
    ) -> Self {
        Self {
            execution_id,
            scope,
            names,
            require_exists,
        }
    }
}

impl Command<()> for RemoveExecutionVariablesCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        remove_execution_variables(
            command_context,
            &self.execution_id,
            self.scope,
            self.names.clone(),
            self.require_exists,
        )
    }
}

/// Resolves a single execution variable with the Java REST read semantics of
/// `getVariableFromRequestWithoutAccessCheck`: without a scope the local value
/// wins and the parent (global) scope is the fallback; an explicit scope reads
/// only that scope. Reads carry no suspension guard.
pub struct GetScopedExecutionVariableCmd {
    execution_id: String,
    name: String,
    scope: Option<ExecutionVariableScope>,
}

impl GetScopedExecutionVariableCmd {
    pub fn new(execution_id: String, name: String, scope: Option<ExecutionVariableScope>) -> Self {
        Self {
            execution_id,
            name,
            scope,
        }
    }
}

impl Command<Option<(serde_json::Value, ExecutionVariableScope)>>
    for GetScopedExecutionVariableCmd
{
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<(serde_json::Value, ExecutionVariableScope)>, FlowableError> {
        let execution = load_execution(command_context, &self.execution_id)?;
        if self.scope != Some(ExecutionVariableScope::Global)
            && let Some(value) = local_scope_value(&execution, &self.name)
        {
            return Ok(Some((value, ExecutionVariableScope::Local)));
        }
        if self.scope != Some(ExecutionVariableScope::Local)
            && let Some(parent_id) = execution.parent_id.clone()
        {
            let store = command_context.runtime_store.clone();
            if let Some((_, value)) = find_execution_variable(
                &store,
                &mut command_context.session,
                &parent_id,
                &self.name,
            ) {
                return Ok(Some((value, ExecutionVariableScope::Global)));
            }
        }
        Ok(None)
    }
}

/// Merged variable map for one execution with the Java REST collection-read
/// semantics of `BaseVariableCollectionResource.processVariables`: without a
/// scope the local variables are added first and the global (parent chain)
/// variables only fill names that are not already present.
pub struct GetScopedExecutionVariablesCmd {
    execution_id: String,
    scope: Option<ExecutionVariableScope>,
}

impl GetScopedExecutionVariablesCmd {
    pub fn new(execution_id: String, scope: Option<ExecutionVariableScope>) -> Self {
        Self {
            execution_id,
            scope,
        }
    }
}

impl Command<Vec<(String, serde_json::Value, ExecutionVariableScope)>>
    for GetScopedExecutionVariablesCmd
{
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<(String, serde_json::Value, ExecutionVariableScope)>, FlowableError> {
        let execution = load_execution(command_context, &self.execution_id)?;
        let mut result: Vec<(String, serde_json::Value, ExecutionVariableScope)> = Vec::new();

        if self.scope != Some(ExecutionVariableScope::Global) {
            let mut locals = local_scope_variables(&execution)
                .into_iter()
                .collect::<Vec<_>>();
            locals.sort_by(|left, right| left.0.cmp(&right.0));
            for (name, value) in locals {
                result.push((name, value, ExecutionVariableScope::Local));
            }
        }

        if self.scope != Some(ExecutionVariableScope::Local)
            && let Some(parent_id) = execution.parent_id.clone()
        {
            let store = command_context.runtime_store.clone();
            let globals = crate::engine::variable_service::collect_execution_variables(
                &store,
                &mut command_context.session,
                &parent_id,
            );
            let mut globals = globals.into_iter().collect::<Vec<_>>();
            globals.sort_by(|left, right| left.0.cmp(&right.0));
            for (name, value) in globals {
                if result.iter().any(|(existing, _, _)| existing == &name) {
                    continue;
                }
                result.push((name, value, ExecutionVariableScope::Global));
            }
        }

        Ok(result)
    }
}
