//! Atomic, typed task-variable mutation commands with local/global scope and
//! create-only/update-only/upsert modes.
//!
//! Java parity: mirrors `SetTaskVariablesCmd` / `RemoveTaskVariablesCmd` /
//! `SetExecutionVariablesCmd` plus the scope/mode semantics of the Java REST
//! task-variable resources (POST = create-only, PUT = update-only, plain
//! `TaskService.setVariable` = upsert). All validation for a batch happens
//! before any write; combined with the command executor's session rollback on
//! error this makes every mutation command atomic.

use crate::engine::task_service::{record_task_local_variable, require_active_task_with_prefix};
use crate::engine::variable_service::{
    collect_execution_variables, find_execution_variable, variable_type_name,
};
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use crate::task::Task;
use std::collections::HashMap;

/// Variable scope for a task-variable mutation, mirroring Java's local/global
/// distinction on `TaskService`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskVariableScope {
    /// Stored on the `Task` entity itself (Java `setVariableLocal`).
    Local,
    /// Stored on the owning execution of the task's process instance
    /// (Java `setVariable` on a task).
    Global,
}

/// Mutation mode mirroring the Java REST semantics: POST = create-only,
/// PUT = update-only, plain `TaskService.setVariable*` = upsert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariableMutationMode {
    /// Fails with `Conflict` when any variable is already present on the scope.
    CreateOnly,
    /// Fails with `NotFound` when any variable is absent on the scope.
    UpdateOnly,
    /// Creates missing variables and updates present ones.
    Upsert,
}

/// A single name/value pair to apply to the resolved scope.
#[derive(Clone, Debug)]
pub struct TaskVariableMutation {
    pub name: String,
    pub value: serde_json::Value,
}

/// Loads the task or fails with the Java `FlowableObjectNotFoundException`
/// message used by the abstract task-variable commands.
fn load_task(command_context: &mut CommandContext, task_id: &str) -> Result<Task, FlowableError> {
    command_context
        .task_entity_manager
        .find_task_by_id(task_id, &mut command_context.session)
        .ok_or_else(|| FlowableError::NotFound(format!("Cannot find task with id {}", task_id)))
}

/// Java `SetTaskVariablesCmd` / `RemoveTaskVariablesCmd`
/// `#getSuspendedTaskExceptionPrefix`: "Cannot add variables to" /
/// "Cannot remove variables from".
fn suspended_task_prefix(verb: &str) -> &'static str {
    match verb {
        "remove" => "Cannot remove variables from",
        _ => "Cannot add variables to",
    }
}

/// Resolves the execution owning the task's global variables and requires it
/// to be active. Mirrors Java `NeedsActiveExecutionCmd` plus the standalone
/// task check from `SetTaskVariablesCmd` ("task is not part of process.").
fn require_active_task_execution(
    command_context: &mut CommandContext,
    task: &Task,
    verb: &str,
) -> Result<Execution, FlowableError> {
    if task.execution_id.is_empty() {
        return Err(FlowableError::BadRequest(format!(
            "Cannot {} global variables on task '{}', task is not part of process.",
            verb, task.id
        )));
    }
    let execution = command_context
        .runtime_store
        .find_execution(&task.execution_id, &mut command_context.session)
        .ok_or_else(|| {
            FlowableError::NotFound(format!("Execution '{}' was not found", task.execution_id))
        })?;
    if execution.is_suspended {
        // Only reachable when the execution is suspended but the task is not;
        // align with the execution-side prefixes (`SetExecutionVariablesCmd` /
        // `RemoveExecutionVariablesCmd#getSuspendedExceptionMessagePrefix`).
        let prefix = match verb {
            "remove" => "Cannot remove variables from",
            _ => "Cannot set variables to",
        };
        return Err(FlowableError::ExecutionError(format!(
            "{} a suspended execution '{}'",
            prefix, execution.id
        )));
    }
    Ok(execution)
}

/// Resolves which execution owns a variable, exactly like `SetVariableCmd`:
/// an existing variable is updated in place on its owning execution in the
/// ancestry chain; a new variable lands on the root process-instance
/// execution.
fn resolve_variable_owner(
    command_context: &mut CommandContext,
    execution_id: &str,
    name: &str,
) -> Result<String, FlowableError> {
    let store = command_context.runtime_store.clone();
    if let Some((owner_id, _)) =
        find_execution_variable(&store, &mut command_context.session, execution_id, name)
    {
        return Ok(owner_id);
    }
    let mut current_id = execution_id.to_string();
    let mut root_id = current_id.clone();
    while let Some(execution) = store.find_execution(&current_id, &mut command_context.session) {
        root_id = execution.id.clone();
        if let Some(parent) = execution.parent_id.clone() {
            current_id = parent;
        } else {
            break;
        }
    }
    Ok(root_id)
}

/// Whether the variable is currently present on the resolved scope. Local
/// means a key in `task.local_variables`; global means an ownership-resolved
/// lookup through the execution parent chain (same semantics as
/// `SetVariableCmd`'s `find_execution_variable`).
fn is_variable_present(
    command_context: &mut CommandContext,
    task: &Task,
    scope: TaskVariableScope,
    task_execution_id: Option<&str>,
    name: &str,
) -> bool {
    match scope {
        TaskVariableScope::Local => task.local_variables.contains_key(name),
        TaskVariableScope::Global => {
            let Some(execution_id) = task_execution_id else {
                return false;
            };
            let store = command_context.runtime_store.clone();
            find_execution_variable(&store, &mut command_context.session, execution_id, name)
                .is_some()
        }
    }
}

/// Validates the scope guard for a mutation batch. Java `NeedsActiveTaskCmd`
/// checks the TASK's suspension before the command body runs — for local and
/// global writes alike — with the operation-specific prefix from
/// `getSuspendedTaskExceptionPrefix`; the task's execution must additionally
/// exist and be active for global writes. Returns the resolved task execution
/// id for the global scope.
fn validate_scope_guard(
    command_context: &mut CommandContext,
    task: &Task,
    scope: TaskVariableScope,
    verb: &str,
) -> Result<Option<String>, FlowableError> {
    require_active_task_with_prefix(task, suspended_task_prefix(verb))?;
    match scope {
        TaskVariableScope::Local => Ok(None),
        TaskVariableScope::Global => {
            let execution = require_active_task_execution(command_context, task, verb)?;
            Ok(Some(execution.id))
        }
    }
}

/// Shared logic for `MutateTaskVariablesCmd`, also used by the legacy
/// single-variable commands in `engine::task_service` (Java parity: the old
/// methods delegate to the new command). Validates everything up front and
/// only then applies the whole batch, returning the resulting variable map of
/// the target scope.
pub(crate) fn mutate_task_variables(
    command_context: &mut CommandContext,
    task_id: &str,
    scope: TaskVariableScope,
    mode: VariableMutationMode,
    mutations: Vec<TaskVariableMutation>,
) -> Result<HashMap<String, serde_json::Value>, FlowableError> {
    let mut task = load_task(command_context, task_id)?;
    let task_execution_id = validate_scope_guard(command_context, &task, scope, "set")?;

    for mutation in &mutations {
        if mutation.name.is_empty() {
            return Err(FlowableError::BadRequest(
                "Variable name is required".to_string(),
            ));
        }
    }

    // Mode checks run against the CURRENT state of the resolved scope, still
    // before any write: create-only conflicts (Java REST POST), update-only
    // 404s missing variables (Java REST PUT).
    for mutation in &mutations {
        let present = is_variable_present(
            command_context,
            &task,
            scope,
            task_execution_id.as_deref(),
            &mutation.name,
        );
        match mode {
            VariableMutationMode::CreateOnly if present => {
                return Err(FlowableError::Conflict(format!(
                    "Variable '{}' is already present on task '{}'.",
                    mutation.name, task.id
                )));
            }
            VariableMutationMode::UpdateOnly if !present => {
                return Err(FlowableError::NotFound(format!(
                    "Task '{}' does not have a variable with name: '{}'.",
                    task.id, mutation.name
                )));
            }
            _ => {}
        }
    }

    match scope {
        TaskVariableScope::Local => {
            for mutation in &mutations {
                task.set_local_variable(mutation.name.clone(), mutation.value.clone());
                record_task_local_variable(
                    command_context,
                    &task,
                    &mutation.name,
                    mutation.value.clone(),
                );
            }
            command_context
                .task_entity_manager
                .update(&task, &mut command_context.session);
            Ok(task.local_variables())
        }
        TaskVariableScope::Global => {
            let task_execution_id =
                task_execution_id.expect("global scope resolves the task execution");
            // Apply every variable to its owning execution, updating each
            // touched execution entity once.
            let mut touched: HashMap<String, Execution> = HashMap::new();
            for mutation in &mutations {
                let owner_id =
                    resolve_variable_owner(command_context, &task_execution_id, &mutation.name)?;
                if !touched.contains_key(&owner_id) {
                    let execution = command_context
                        .runtime_store
                        .find_execution(&owner_id, &mut command_context.session)
                        .ok_or_else(|| {
                            FlowableError::NotFound(format!(
                                "Execution '{}' was not found",
                                owner_id
                            ))
                        })?;
                    touched.insert(owner_id.clone(), execution);
                }
                let owner = touched.get_mut(&owner_id).expect("owner execution cached");
                owner.set_process_variable(mutation.name.clone(), mutation.value.clone());
                let process_instance_id = owner
                    .process_instance_id
                    .clone()
                    .unwrap_or_else(|| owner.id.clone());
                let owner_execution_id = owner.id.clone();
                let id = format!("{}:{}", owner_execution_id, mutation.name);
                if command_context
                    .runtime_store
                    .get_historic_variable_instance(&id, &mut command_context.session)
                    .is_some()
                {
                    command_context.history_manager.record_variable_updated(
                        &id,
                        mutation.value.clone(),
                        &mut command_context.session,
                    );
                } else {
                    command_context.history_manager.record_variable_created(
                        &id,
                        &mutation.name,
                        variable_type_name(&mutation.value),
                        mutation.value.clone(),
                        &process_instance_id,
                        Some(&owner_execution_id),
                        None,
                        &mut command_context.session,
                    );
                }
            }
            for execution in touched.values() {
                command_context
                    .execution_entity_manager
                    .update(execution, &mut command_context.session);
            }
            let store = command_context.runtime_store.clone();
            Ok(collect_execution_variables(
                &store,
                &mut command_context.session,
                &task_execution_id,
            ))
        }
    }
}

/// Shared logic for `RemoveTaskVariablesCmd`, also used by the legacy
/// single-variable delete in `engine::task_service`. `names = None` removes
/// ALL task-local variables and is only meaningful for the local scope (Java
/// REST DELETE on the task variable collection). `require_exists` mirrors the
/// Java REST DELETE single-variable semantics: any absent name 404s before
/// anything is removed.
pub(crate) fn remove_task_variables(
    command_context: &mut CommandContext,
    task_id: &str,
    scope: TaskVariableScope,
    names: Option<Vec<String>>,
    require_exists: bool,
) -> Result<(), FlowableError> {
    let mut task = load_task(command_context, task_id)?;
    let task_execution_id = validate_scope_guard(command_context, &task, scope, "remove")?;

    if names.is_none() && scope == TaskVariableScope::Global {
        return Err(FlowableError::BadRequest(
            "Removing all variables is only supported for task-local variables".to_string(),
        ));
    }

    if require_exists && let Some(ref names) = names {
        for name in names {
            if !is_variable_present(
                command_context,
                &task,
                scope,
                task_execution_id.as_deref(),
                name,
            ) {
                return Err(FlowableError::NotFound(format!(
                    "Task '{}' does not have a variable with name: '{}'.",
                    task.id, name
                )));
            }
        }
    }

    match scope {
        TaskVariableScope::Local => {
            let removed: Vec<String> = match &names {
                Some(names) => names
                    .iter()
                    .filter(|name| task.local_variables.contains_key(*name))
                    .cloned()
                    .collect(),
                None => task.local_variables.keys().cloned().collect(),
            };
            for name in &removed {
                task.local_variables.remove(name);
            }
            command_context
                .task_entity_manager
                .update(&task, &mut command_context.session);
            for name in &removed {
                command_context.history_manager.record_variable_removed(
                    &format!("{}:{}", task.id, name),
                    &mut command_context.session,
                );
            }
            Ok(())
        }
        TaskVariableScope::Global => {
            let task_execution_id =
                task_execution_id.expect("global scope resolves the task execution");
            let names = names.unwrap_or_default();
            let store = command_context.runtime_store.clone();
            let mut touched: HashMap<String, Execution> = HashMap::new();
            for name in &names {
                let Some((owner_id, _)) = find_execution_variable(
                    &store,
                    &mut command_context.session,
                    &task_execution_id,
                    name,
                ) else {
                    // Java removeVariables ignores missing names.
                    continue;
                };
                if !touched.contains_key(&owner_id) {
                    let execution = store
                        .find_execution(&owner_id, &mut command_context.session)
                        .ok_or_else(|| {
                            FlowableError::NotFound(format!(
                                "Execution '{}' was not found",
                                owner_id
                            ))
                        })?;
                    touched.insert(owner_id.clone(), execution);
                }
                let owner = touched.get_mut(&owner_id).expect("owner execution cached");
                owner.variables.remove(name);
                owner.local_variables.remove(name);
                owner.transient_variables.remove(name);
                store.delete_variable_by_execution_id_and_name(
                    &owner_id,
                    name,
                    &mut command_context.session,
                );
                command_context.history_manager.record_variable_removed(
                    &format!("{}:{}", owner_id, name),
                    &mut command_context.session,
                );
            }
            for execution in touched.values() {
                command_context
                    .execution_entity_manager
                    .update(execution, &mut command_context.session);
            }
            Ok(())
        }
    }
}

/// Atomic batch mutation of a task's variables on one scope.
pub struct MutateTaskVariablesCmd {
    task_id: String,
    scope: TaskVariableScope,
    mode: VariableMutationMode,
    mutations: Vec<TaskVariableMutation>,
}

impl MutateTaskVariablesCmd {
    pub fn new(
        task_id: String,
        scope: TaskVariableScope,
        mode: VariableMutationMode,
        mutations: Vec<TaskVariableMutation>,
    ) -> Self {
        Self {
            task_id,
            scope,
            mode,
            mutations,
        }
    }
}

impl Command<HashMap<String, serde_json::Value>> for MutateTaskVariablesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HashMap<String, serde_json::Value>, FlowableError> {
        mutate_task_variables(
            command_context,
            &self.task_id,
            self.scope,
            self.mode,
            self.mutations.clone(),
        )
    }
}

/// Atomic removal of a task's variables from one scope.
pub struct RemoveTaskVariablesCmd {
    task_id: String,
    scope: TaskVariableScope,
    names: Option<Vec<String>>,
    require_exists: bool,
}

impl RemoveTaskVariablesCmd {
    pub fn new(
        task_id: String,
        scope: TaskVariableScope,
        names: Option<Vec<String>>,
        require_exists: bool,
    ) -> Self {
        Self {
            task_id,
            scope,
            names,
            require_exists,
        }
    }
}

impl Command<()> for RemoveTaskVariablesCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        remove_task_variables(
            command_context,
            &self.task_id,
            self.scope,
            self.names.clone(),
            self.require_exists,
        )
    }
}

/// Resolves a single task variable with Java `TaskService.getVariable`
/// semantics: the task-local value first, then the execution (global) scope
/// as fallback. Reads carry no suspension guard, like the other get commands.
pub struct GetTaskVariableCmd {
    task_id: String,
    name: String,
}

impl GetTaskVariableCmd {
    pub fn new(task_id: String, name: String) -> Self {
        Self { task_id, name }
    }
}

impl Command<Option<serde_json::Value>> for GetTaskVariableCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<serde_json::Value>, FlowableError> {
        let task = load_task(command_context, &self.task_id)?;
        if let Some(value) = task.local_variable(&self.name) {
            return Ok(Some(value));
        }
        if task.execution_id.is_empty() {
            return Ok(None);
        }
        let store = command_context.runtime_store.clone();
        Ok(find_execution_variable(
            &store,
            &mut command_context.session,
            &task.execution_id,
            &self.name,
        )
        .map(|(_, value)| value))
    }
}

/// Merged variable map for a task with Java `TaskService.getVariables`
/// semantics: task-local values shadow execution (global) values on name
/// clashes.
pub struct GetTaskVariablesCmd {
    task_id: String,
}

impl GetTaskVariablesCmd {
    pub fn new(task_id: String) -> Self {
        Self { task_id }
    }
}

impl Command<HashMap<String, serde_json::Value>> for GetTaskVariablesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HashMap<String, serde_json::Value>, FlowableError> {
        let task = load_task(command_context, &self.task_id)?;
        let mut variables = if task.execution_id.is_empty() {
            HashMap::new()
        } else {
            let store = command_context.runtime_store.clone();
            collect_execution_variables(&store, &mut command_context.session, &task.execution_id)
        };
        for (name, value) in task.local_variables() {
            variables.insert(name, value);
        }
        Ok(variables)
    }
}
