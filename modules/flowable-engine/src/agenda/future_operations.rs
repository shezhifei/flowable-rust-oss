//! Bounded async future support for the engine agenda.
//!
//! Minimal equivalent of Java `WaitForAnyFuture` / async delegate completion:
//! - [`PendingFutureRegistry`] holds in-flight future handles (session cache or config)
//! - [`WaitForFutureOperation`] polls a future; re-queues until done, then continues the process

use crate::agenda::{AgendaOperation, FlowableEngineAgenda};
use crate::bpmn::fault::{
    EngineFault, clear_boundaries_for_execution, propagate_bpmn_error, uncaught_bpmn_error,
};
use crate::bpmn::http_task::{HttpTaskOutcome, PendingHttpCompletion};
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Session-cache / config key for the pending-future registry.
pub const PENDING_FUTURE_REGISTRY_CACHE_KEY: &str = "flowable.pendingFutureRegistry";

/// Variable stored on an execution while it is waiting for an async future.
pub const PENDING_FUTURE_ID_VARIABLE: &str = "__flowable_pending_future_id";

/// Poll interval used when re-queueing [`WaitForFutureOperation`].
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Safety cap so a stuck future cannot hang a command forever.
const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub enum FutureState {
    Pending,
    Completed(Result<Value, String>),
}

/// Internal typed result for engine-owned asynchronous operations. Public
/// delegate futures keep using `Result<Value, String>` unchanged.
#[derive(Clone, Debug)]
pub(crate) enum PendingOperationResult {
    Http(PendingHttpCompletion),
}

/// Shared handle for one async unit of work.
#[derive(Debug)]
pub struct PendingFuture {
    pub id: String,
    state: Mutex<FutureState>,
    operation_result: Mutex<Option<PendingOperationResult>>,
    condvar: Condvar,
}

impl PendingFuture {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            state: Mutex::new(FutureState::Pending),
            operation_result: Mutex::new(None),
            condvar: Condvar::new(),
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(*self.state.lock().unwrap(), FutureState::Completed(_))
    }

    pub fn complete(&self, result: Result<Value, String>) {
        let mut guard = self.state.lock().unwrap();
        if matches!(*guard, FutureState::Completed(_)) {
            return;
        }
        *guard = FutureState::Completed(result);
        self.condvar.notify_all();
    }

    pub(crate) fn complete_operation(&self, result: PendingOperationResult) {
        *self.operation_result.lock().unwrap() = Some(result);
        self.complete(Ok(Value::Null));
    }

    pub(crate) fn operation_result(&self) -> Option<PendingOperationResult> {
        self.operation_result.lock().unwrap().clone()
    }

    pub fn state(&self) -> FutureState {
        self.state.lock().unwrap().clone()
    }

    /// Blocking wait with a timeout (used by direct RuntimeService APIs).
    pub fn wait_timeout(&self, timeout: Duration) -> Result<Value, FlowableError> {
        let mut guard = self.state.lock().unwrap();
        let deadline = Instant::now() + timeout;
        loop {
            match &*guard {
                FutureState::Completed(Ok(value)) => return Ok(value.clone()),
                FutureState::Completed(Err(err)) => {
                    return Err(FlowableError::ExecutionError(err.clone()));
                }
                FutureState::Pending => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(FlowableError::ExecutionError(format!(
                            "Pending future '{}' timed out after {:?}",
                            self.id, timeout
                        )));
                    }
                    let (g, wait_result) = self.condvar.wait_timeout(guard, remaining).unwrap();
                    guard = g;
                    if wait_result.timed_out() && matches!(*guard, FutureState::Pending) {
                        return Err(FlowableError::ExecutionError(format!(
                            "Pending future '{}' timed out after {:?}",
                            self.id, timeout
                        )));
                    }
                }
            }
        }
    }
}

/// Process-wide (or command-scoped) registry of pending futures.
#[derive(Debug, Default)]
pub struct PendingFutureRegistry {
    futures: Mutex<HashMap<String, Arc<PendingFuture>>>,
}

impl PendingFutureRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self) -> Arc<PendingFuture> {
        let id = uuid::Uuid::new_v4().to_string();
        let future = Arc::new(PendingFuture::new(id.clone()));
        self.futures.lock().unwrap().insert(id, Arc::clone(&future));
        future
    }

    pub fn get(&self, id: &str) -> Option<Arc<PendingFuture>> {
        self.futures.lock().unwrap().get(id).cloned()
    }

    pub fn remove(&self, id: &str) -> Option<Arc<PendingFuture>> {
        self.futures.lock().unwrap().remove(id)
    }

    pub fn len(&self) -> usize {
        self.futures.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.futures.lock().unwrap().is_empty()
    }

    pub fn complete(&self, id: &str, result: Result<Value, String>) -> bool {
        if let Some(future) = self.get(id) {
            future.complete(result);
            true
        } else {
            false
        }
    }
}

/// Optional metadata applied when a future completes so the process can continue.
#[derive(Debug, Clone)]
pub struct WaitForFutureContinuation {
    /// Variable that receives the future result (process scope).
    pub result_variable_name: Option<String>,
    /// When true, store the result as a transient variable.
    pub store_result_as_transient: bool,
    /// When true, store the result as a local variable.
    pub use_local_scope: bool,
}

impl Default for WaitForFutureContinuation {
    fn default() -> Self {
        Self {
            result_variable_name: None,
            store_result_as_transient: false,
            use_local_scope: false,
        }
    }
}

/// Agenda operation: wait until a pending future completes, then continue the process.
#[derive(Debug, Clone)]
pub struct WaitForFutureOperation {
    pub future_id: String,
    pub execution: Execution,
    pub continuation: WaitForFutureContinuation,
    pub poll_interval: Duration,
    pub max_wait: Duration,
    pub started_at: Instant,
}

impl WaitForFutureOperation {
    pub fn new(future_id: String, execution: Execution) -> Self {
        Self {
            future_id,
            execution,
            continuation: WaitForFutureContinuation::default(),
            poll_interval: DEFAULT_POLL_INTERVAL,
            max_wait: DEFAULT_MAX_WAIT,
            started_at: Instant::now(),
        }
    }

    pub fn with_continuation(mut self, continuation: WaitForFutureContinuation) -> Self {
        self.continuation = continuation;
        self
    }

    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    pub fn with_max_wait(mut self, max_wait: Duration) -> Self {
        self.max_wait = max_wait;
        self
    }
}

impl AgendaOperation for WaitForFutureOperation {
    fn run(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        let registry = resolve_pending_future_registry(command_context).ok_or_else(|| {
            FlowableError::ExecutionError(
                "PendingFutureRegistry is not available on CommandContext".to_string(),
            )
        })?;

        let future = registry.get(&self.future_id).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Pending future '{}' was not found",
                self.future_id
            ))
        })?;

        match future.state() {
            FutureState::Pending => {
                if self.started_at.elapsed() >= self.max_wait {
                    return Err(FlowableError::ExecutionError(format!(
                        "WaitForFutureOperation timed out for future '{}' after {:?}",
                        self.future_id, self.max_wait
                    )));
                }
                std::thread::sleep(self.poll_interval);
                // Re-queue: keep waiting without blocking the rest of the agenda forever
                // in a single stack frame (mirrors a bounded WaitForAnyFuture loop).
                command_context
                    .agenda
                    .plan_operation(Box::new(self.clone()));
                Ok(())
            }
            FutureState::Completed(Ok(value)) => {
                // Completion processing may still fail (for example in an HTTP
                // response handler). Detach the finished future before invoking
                // user code so every terminal path releases the registry entry.
                // The local Arc keeps the completion alive for this operation.
                registry.remove(&self.future_id);
                let mut execution = self.execution.clone();
                let effective_value = match future.operation_result() {
                    Some(PendingOperationResult::Http(completion)) => {
                        let outcome = match completion.transport_result {
                            Ok(mut exchange) => {
                                if let Some(handler) = &completion.response_handler {
                                    handler.invoke(&mut execution, &mut exchange)?;
                                }
                                HttpTaskOutcome::success(&completion.spec, &exchange)
                            }
                            Err(error) => {
                                HttpTaskOutcome::ignored_transport_error(&completion.spec, &error)
                            }
                        };
                        match outcome.apply_to(&mut execution) {
                            Ok(value) => {
                                clear_boundaries_for_execution(&execution.id, command_context);
                                value
                            }
                            Err(EngineFault::BpmnError { code, .. }) => {
                                if propagate_bpmn_error(&mut execution, &code, command_context)? {
                                    return Ok(());
                                }
                                return Err(uncaught_bpmn_error(&code));
                            }
                            Err(fault) => return Err(fault.into_flowable_error()),
                        }
                    }
                    None => value,
                };
                apply_future_result(&mut execution, &self.continuation, &effective_value);
                execution.variables.remove(PENDING_FUTURE_ID_VARIABLE);
                execution.local_variables.remove(PENDING_FUTURE_ID_VARIABLE);
                execution
                    .transient_variables
                    .remove(PENDING_FUTURE_ID_VARIABLE);

                command_context
                    .execution_entity_manager
                    .update(&execution, &mut command_context.session);

                command_context
                    .agenda
                    .plan_take_outgoing_sequence_flows_operation(execution);
                Ok(())
            }
            FutureState::Completed(Err(err)) => {
                registry.remove(&self.future_id);
                Err(FlowableError::ExecutionError(format!(
                    "Async future '{}' failed: {}",
                    self.future_id, err
                )))
            }
        }
    }
}

fn apply_future_result(
    execution: &mut Execution,
    continuation: &WaitForFutureContinuation,
    value: &Value,
) {
    let Some(name) = continuation.result_variable_name.as_ref() else {
        return;
    };
    if continuation.store_result_as_transient {
        execution.set_transient_variable(name.clone(), value.clone());
    } else if continuation.use_local_scope {
        execution.set_local_variable(name.clone(), value.clone());
    } else {
        execution.set_process_variable(name.clone(), value.clone());
    }
}

/// Resolve the pending-future registry from session cache, falling back to config.
pub fn resolve_pending_future_registry(
    command_context: &CommandContext,
) -> Option<Arc<PendingFutureRegistry>> {
    if let Some(registry) = command_context
        .session_caches
        .get(PENDING_FUTURE_REGISTRY_CACHE_KEY)
        .and_then(|entry| entry.downcast_ref::<Arc<PendingFutureRegistry>>())
    {
        return Some(Arc::clone(registry));
    }
    Some(Arc::clone(&command_context.config.pending_future_registry))
}

/// Plan a wait-for-future operation on the agenda.
pub fn plan_wait_for_future(
    command_context: &mut CommandContext,
    future_id: String,
    execution: Execution,
    continuation: WaitForFutureContinuation,
) -> Result<(), FlowableError> {
    let op = WaitForFutureOperation::new(future_id, execution).with_continuation(continuation);
    command_context.agenda.plan_operation(Box::new(op));
    command_context.dispatch_automatic_future_started_success()
}
