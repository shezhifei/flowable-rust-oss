//! Async local service-task delegates.
//!
//! Minimal equivalent of Java async delegate completion:
//! - [`AsyncLocalServiceTaskDelegate`] runs on a background worker
//! - result is delivered through [`PendingFutureRegistry`]
//! - the agenda continues via [`WaitForFutureOperation`]

use crate::agenda::future_operations::{
    PENDING_FUTURE_ID_VARIABLE, PendingFutureRegistry, WaitForFutureContinuation,
    plan_wait_for_future, resolve_pending_future_registry,
};
use crate::engine::async_task_executor::AsyncTaskExecutor;
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::ServiceTask;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

pub const ASYNC_SERVICE_TASK_DELEGATE_REGISTRY_CACHE_KEY: &str =
    "flowable.asyncServiceTaskDelegateRegistry";

/// Owned snapshot of the execution context needed by background work.
#[derive(Debug, Clone)]
pub struct AsyncLocalServiceTaskDelegateContext {
    pub service_task_id: String,
    pub execution_id: String,
    pub process_instance_id: Option<String>,
    pub fields: Map<String, Value>,
    pub variables: HashMap<String, Value>,
}

/// Async-capable local service-task delegate.
///
/// The engine invokes [`run`](Self::run) on a background thread (or synchronously
/// when no executor is available) and delivers the JSON result through a pending future.
pub trait AsyncLocalServiceTaskDelegate: Send + Sync {
    fn run(&self, context: &AsyncLocalServiceTaskDelegateContext) -> Result<Value, FlowableError>;
}

/// In-process registry for async service-task delegates.
#[derive(Clone, Default)]
pub struct AsyncLocalServiceTaskDelegateRegistry {
    delegates: BTreeMap<String, Arc<dyn AsyncLocalServiceTaskDelegate>>,
}

impl std::fmt::Debug for AsyncLocalServiceTaskDelegateRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncLocalServiceTaskDelegateRegistry")
            .field("delegates", &self.delegates.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl AsyncLocalServiceTaskDelegateRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        delegate: Arc<dyn AsyncLocalServiceTaskDelegate>,
    ) {
        self.delegates.insert(name.into(), delegate);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn AsyncLocalServiceTaskDelegate>> {
        self.delegates.get(name).cloned()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.delegates.contains_key(name)
    }
}

/// Resolve the async delegate registry from session cache or config.
pub fn resolve_async_delegate_registry(
    command_context: &CommandContext,
) -> Option<AsyncLocalServiceTaskDelegateRegistry> {
    if let Some(registry) = command_context
        .session_caches
        .get(ASYNC_SERVICE_TASK_DELEGATE_REGISTRY_CACHE_KEY)
        .and_then(|entry| entry.downcast_ref::<AsyncLocalServiceTaskDelegateRegistry>())
    {
        return Some(registry.clone());
    }
    command_context
        .config
        .async_service_task_delegate_registry
        .clone()
}

/// Submit `work` to the optional [`AsyncTaskExecutor`], otherwise run it synchronously.
///
/// Returns the pending future id.
pub fn submit_async_work<F>(
    registry: &PendingFutureRegistry,
    executor: Option<&Mutex<Option<AsyncTaskExecutor>>>,
    work: F,
) -> Result<String, FlowableError>
where
    F: FnOnce() -> Result<Value, FlowableError> + Send + 'static,
{
    let future = registry.create();
    let future_id = future.id.clone();
    let future_for_worker = Arc::clone(&future);

    let task: Box<dyn FnOnce() + Send> = Box::new(move || {
        let result = work().map_err(|err| err.to_string());
        future_for_worker.complete(result);
    });

    if let Some(executor_mutex) = executor {
        let guard = executor_mutex.lock().unwrap();
        if let Some(pool) = guard.as_ref() {
            if let Some(sender) = pool.try_clone_sender() {
                match sender.try_send(task) {
                    Ok(()) => return Ok(future_id),
                    Err(err) => {
                        // Queue full: recover the task and run it on a detached thread
                        // so the future still completes.
                        let recovered = err.into_inner();
                        std::thread::spawn(recovered);
                        return Ok(future_id);
                    }
                }
            }
        }
    }

    // No executor configured (or shut down): run synchronously so the future is done.
    task();
    Ok(future_id)
}

/// Build the owned async context snapshot from a service-task invocation.
pub fn build_async_context(
    service_task_id: &str,
    execution: &Execution,
    fields: Map<String, Value>,
) -> AsyncLocalServiceTaskDelegateContext {
    AsyncLocalServiceTaskDelegateContext {
        service_task_id: service_task_id.to_string(),
        execution_id: execution.id.clone(),
        process_instance_id: execution.process_instance_id.clone(),
        fields,
        variables: execution.process_variables(),
    }
}

/// Look up an optional shared async task executor from config.
pub fn resolve_future_task_executor(
    command_context: &CommandContext,
) -> Option<Arc<Mutex<Option<AsyncTaskExecutor>>>> {
    command_context
        .config
        .future_task_executor
        .as_ref()
        .map(Arc::clone)
}

/// Execute an async-capable local service-task delegate:
/// submit background work, store the future id on the execution, and plan wait-for-future.
pub fn execute_async_local_delegate_service_task(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &mut CommandContext,
    delegate_name: &str,
    fields: Map<String, Value>,
) -> Result<(), FlowableError> {
    let activity_id = service_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .id
        .clone()
        .unwrap_or_else(|| delegate_name.to_string());

    let async_registry = resolve_async_delegate_registry(command_context).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "No async service task delegate registry is configured for service task '{}'",
            activity_id
        ))
    })?;
    let delegate = async_registry.get(delegate_name).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "No async service task delegate '{}' is registered for service task '{}'",
            delegate_name, activity_id
        ))
    })?;

    let pending_registry = resolve_pending_future_registry(command_context).ok_or_else(|| {
        FlowableError::ExecutionError(
            "PendingFutureRegistry is not available for async service task".to_string(),
        )
    })?;

    let context = build_async_context(&activity_id, execution, fields);
    let executor = resolve_future_task_executor(command_context);
    let executor_ref = executor.as_deref();

    let work_delegate = Arc::clone(&delegate);
    let future_id = submit_async_work(pending_registry.as_ref(), executor_ref, move || {
        work_delegate.run(&context)
    })?;

    execution.set_transient_variable(
        PENDING_FUTURE_ID_VARIABLE.to_string(),
        Value::String(future_id.clone()),
    );

    command_context
        .execution_entity_manager
        .update(execution, &mut command_context.session);

    let continuation = WaitForFutureContinuation {
        result_variable_name: service_task.result_variable_name.clone(),
        store_result_as_transient: service_task.store_result_variable_as_transient,
        use_local_scope: service_task.use_local_scope_for_result_variable,
    };

    plan_wait_for_future(command_context, future_id, execution.clone(), continuation)?;
    Ok(())
}

/// Returns true when the resolved delegate name is registered as async-capable.
pub fn is_async_delegate_registered(command_context: &CommandContext, delegate_name: &str) -> bool {
    resolve_async_delegate_registry(command_context)
        .map(|registry| registry.contains(delegate_name))
        .unwrap_or(false)
}

/// Direct helper used by [`crate::engine::runtime_service::RuntimeService::execute_async_delegate`].
///
/// Submits work for `delegate_name`, waits for completion, and writes `result_variable`
/// on the process instance execution.
pub fn execute_async_delegate_on_process_instance(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    delegate_name: &str,
    result_variable: &str,
    fields: Map<String, Value>,
) -> Result<Value, FlowableError> {
    let mut execution = command_context
        .runtime_store
        .find_execution(process_instance_id, &mut command_context.session)
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Process instance / execution '{}' was not found",
                process_instance_id
            ))
        })?;

    let async_registry = resolve_async_delegate_registry(command_context).ok_or_else(|| {
        FlowableError::ExecutionError(
            "No async service task delegate registry is configured".to_string(),
        )
    })?;
    let delegate = async_registry.get(delegate_name).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "No async service task delegate '{}' is registered",
            delegate_name
        ))
    })?;

    let pending_registry = resolve_pending_future_registry(command_context).ok_or_else(|| {
        FlowableError::ExecutionError("PendingFutureRegistry is not available".to_string())
    })?;

    let context = build_async_context(delegate_name, &execution, fields);
    let executor = resolve_future_task_executor(command_context);
    let work_delegate = Arc::clone(&delegate);
    let future_id = submit_async_work(pending_registry.as_ref(), executor.as_deref(), move || {
        work_delegate.run(&context)
    })?;

    let future = pending_registry.get(&future_id).ok_or_else(|| {
        FlowableError::ExecutionError(format!("Pending future '{}' was not found", future_id))
    })?;

    let value = future.wait_timeout(std::time::Duration::from_secs(30))?;
    pending_registry.remove(&future_id);

    execution.set_process_variable(result_variable.to_string(), value.clone());
    command_context
        .execution_entity_manager
        .update(&execution, &mut command_context.session);

    Ok(value)
}
