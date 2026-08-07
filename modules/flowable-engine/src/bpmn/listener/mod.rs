//! BPMN execution and task listener runtime.
//!
//! Parsed `executionListener` / `taskListener` entries are executed via a local
//! registry (Rust equivalent of Spring bean / class-name mapping). JVM classloading
//! is intentionally out of scope.

pub mod execution_listener_util;
pub mod listener_registry;
pub mod task_listener_util;

pub use execution_listener_util::{
    execute_execution_listeners, flow_element_execution_listeners, notify_execution_listeners,
};
pub use listener_registry::{
    EXECUTION_LISTENER_REGISTRY_CACHE_KEY, ExecutionListenerContext, LocalExecutionListener,
    LocalExecutionListenerRegistry, LocalTaskListener, LocalTaskListenerRegistry,
    TASK_LISTENER_REGISTRY_CACHE_KEY, TaskListenerContext,
};
pub use task_listener_util::notify_task_listeners;
