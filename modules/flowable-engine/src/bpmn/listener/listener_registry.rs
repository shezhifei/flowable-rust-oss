use crate::error::FlowableError;
use crate::runtime::execution::Execution;
use crate::task::Task;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// CommandContext session-cache key for the local execution listener registry.
pub const EXECUTION_LISTENER_REGISTRY_CACHE_KEY: &str = "flowable.executionListenerRegistry";

/// CommandContext session-cache key for the local task listener registry.
pub const TASK_LISTENER_REGISTRY_CACHE_KEY: &str = "flowable.taskListenerRegistry";

/// Context passed to a registered execution listener.
pub struct ExecutionListenerContext<'a> {
    pub event: &'a str,
    pub activity_id: Option<&'a str>,
    pub execution: &'a mut Execution,
    pub fields: &'a Map<String, Value>,
}

/// Local (in-process) execution listener — Rust equivalent of Java `ExecutionListener`.
pub trait LocalExecutionListener: Send + Sync {
    fn notify(&self, ctx: &mut ExecutionListenerContext<'_>) -> Result<(), FlowableError>;
}

/// Context passed to a registered task listener.
pub struct TaskListenerContext<'a> {
    pub event: &'a str,
    pub task: &'a mut Task,
    pub execution: &'a mut Execution,
    pub fields: &'a Map<String, Value>,
}

/// Local (in-process) task listener — Rust equivalent of Java `TaskListener`.
pub trait LocalTaskListener: Send + Sync {
    fn notify(&self, ctx: &mut TaskListenerContext<'_>) -> Result<(), FlowableError>;
}

/// Registry of named local execution listeners.
#[derive(Clone, Default)]
pub struct LocalExecutionListenerRegistry {
    listeners: BTreeMap<String, Arc<dyn LocalExecutionListener>>,
}

impl std::fmt::Debug for LocalExecutionListenerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalExecutionListenerRegistry")
            .field("listeners", &self.listeners.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl LocalExecutionListenerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: impl Into<String>, listener: Arc<dyn LocalExecutionListener>) {
        self.listeners.insert(name.into(), listener);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn LocalExecutionListener>> {
        self.listeners.get(name).cloned()
    }
}

/// Registry of named local task listeners.
#[derive(Clone, Default)]
pub struct LocalTaskListenerRegistry {
    listeners: BTreeMap<String, Arc<dyn LocalTaskListener>>,
}

impl std::fmt::Debug for LocalTaskListenerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalTaskListenerRegistry")
            .field("listeners", &self.listeners.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl LocalTaskListenerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: impl Into<String>, listener: Arc<dyn LocalTaskListener>) {
        self.listeners.insert(name.into(), listener);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn LocalTaskListener>> {
        self.listeners.get(name).cloned()
    }
}
