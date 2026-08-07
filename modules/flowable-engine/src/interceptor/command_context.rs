use crate::agenda::DefaultFlowableEngineAgenda;
use crate::bpmn::parser::factory::activity_behavior_factory::{
    ActivityBehaviorFactory, DefaultActivityBehaviorFactory,
};
use crate::engine::deployment_manager::DeploymentManager;
use crate::engine::event_dispatcher::{EngineEvent, TransactionEventInvocation, TransactionState};
use crate::history::history_manager::HistoryManager;
use crate::persistence::execution_entity_manager::{
    DefaultExecutionEntityManager, ExecutionEntityManager,
};
use crate::persistence::runtime_store::RuntimeStore;
use crate::persistence::runtime_store::RuntimeTimerJobState;
use crate::persistence::task_entity_manager::{DefaultTaskEntityManager, TaskEntityManager};
use crate::service::config::ProcessEngineConfiguration;
use flowable_http_service::HttpRuntime;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// The context for executing a command
pub struct CommandContext {
    // In a real engine, this would contain connections, transactions, caches, etc.
    pub(crate) session_caches: HashMap<String, Box<dyn std::any::Any>>,
    pub(crate) agenda: DefaultFlowableEngineAgenda,
    pub(crate) behavior_factory: Box<dyn ActivityBehaviorFactory>,
    pub(crate) execution_entity_manager: Box<dyn ExecutionEntityManager>,
    pub(crate) task_entity_manager: Box<dyn TaskEntityManager>,
    pub(crate) history_manager: HistoryManager,
    pub(crate) deployment_manager: DeploymentManager,
    pub(crate) runtime_store: RuntimeStore,
    pub(crate) session: crate::persistence::db_session::DbSession,
    pub(crate) config: Arc<ProcessEngineConfiguration>,
    pub(crate) http_runtime: Arc<dyn HttpRuntime>,
    post_agenda_events: Vec<EngineEvent>,
    transaction_events: BTreeMap<TransactionState, Vec<TransactionEventInvocation>>,
    automatic_job_for_future_success: Option<RuntimeTimerJobState>,
    automatic_future_success_dispatched: bool,
    /// Pending async-executor hints registered during this command. They are
    /// drained by the command executor *after* the database transaction commits,
    /// mirroring Java's `JobAddedTransactionListener` (`COMMITTED` state). A
    /// command must never submit to the executor directly: if the command rolls
    /// back these hints are discarded and nothing is enqueued.
    pending_async_hints: Vec<PendingAsyncJobHint>,
}

/// A committed-job hint for the live async executor. Records the pre-locked job
/// so the post-commit drain can offer it to the executor and, on rejection,
/// dispatch `JOB_REJECTED` + CAS-release the pre-lock (Java parity).
#[derive(Clone)]
pub(crate) struct PendingAsyncJobHint {
    pub(crate) job: RuntimeTimerJobState,
}

impl CommandContext {
    pub fn session_caches(&mut self) -> &mut HashMap<String, Box<dyn std::any::Any>> {
        &mut self.session_caches
    }

    pub fn agenda(&mut self) -> &mut DefaultFlowableEngineAgenda {
        &mut self.agenda
    }

    pub fn execution_entity_manager(&mut self) -> &mut dyn ExecutionEntityManager {
        self.execution_entity_manager.as_mut()
    }

    pub fn task_entity_manager(&mut self) -> &mut dyn TaskEntityManager {
        self.task_entity_manager.as_mut()
    }

    pub fn history_manager(&mut self) -> &mut HistoryManager {
        &mut self.history_manager
    }

    pub fn runtime_store(&self) -> &RuntimeStore {
        &self.runtime_store
    }

    pub fn deployment_manager(&self) -> &DeploymentManager {
        &self.deployment_manager
    }

    pub fn session(&mut self) -> &mut crate::persistence::db_session::DbSession {
        &mut self.session
    }

    pub fn runtime_store_handle(&self) -> RuntimeStore {
        self.runtime_store.clone()
    }

    pub fn deployment_manager_handle(&self) -> DeploymentManager {
        self.deployment_manager.clone()
    }

    pub(crate) fn set_automatic_job_for_future_success(&mut self, job: RuntimeTimerJobState) {
        self.automatic_job_for_future_success = Some(job);
        self.automatic_future_success_dispatched = false;
    }

    pub(crate) fn is_automatic_job_execution(&self) -> bool {
        self.automatic_job_for_future_success.is_some()
    }

    pub(crate) fn dispatch_automatic_future_started_success(
        &mut self,
    ) -> Result<(), crate::error::FlowableError> {
        if self.automatic_future_success_dispatched {
            return Ok(());
        }
        let Some(job) = self.automatic_job_for_future_success.clone() else {
            return Ok(());
        };
        if job.error_message.is_some() {
            return Ok(());
        }
        self.automatic_future_success_dispatched = true;
        let event = EngineEvent::Job {
            event_type: crate::engine::event_dispatcher::EngineEventType::JobExecutionSuccess,
            job,
        };
        let dispatcher = self.config.engine_event_dispatcher.clone();
        dispatcher.dispatch_in_context(&event, self)
    }

    pub(crate) fn add_transaction_event(
        &mut self,
        state: TransactionState,
        invocation: TransactionEventInvocation,
    ) {
        self.transaction_events
            .entry(state)
            .or_default()
            .push(invocation);
    }

    pub(crate) fn add_post_agenda_event(&mut self, event: EngineEvent) {
        self.post_agenda_events.push(event);
    }

    pub(crate) fn dispatch_post_agenda_events(
        &mut self,
    ) -> Result<(), crate::error::FlowableError> {
        let mut events = std::mem::take(&mut self.post_agenda_events);
        // P119: merge HISTORIC_* events recorded during the command (sync path).
        // Java fires them inline in DefaultHistoryManager; we batch with the
        // post-agenda stream so typed listeners still receive them.
        events.extend(self.history_manager.take_pending_events());
        let event_dispatcher = self.config.engine_event_dispatcher.clone();
        for event in events {
            event_dispatcher.dispatch_in_context(&event, self)?;
        }
        Ok(())
    }

    pub(crate) fn dispatch_transaction_events(
        &mut self,
        state: TransactionState,
    ) -> Result<(), crate::error::FlowableError> {
        let invocations = self.transaction_events.remove(&state).unwrap_or_default();
        for invocation in invocations {
            invocation.invoke()?;
        }
        Ok(())
    }

    /// Convenience: get an owned RuntimeStore clone and a mutable session reference in one call.
    /// This avoids borrow conflicts when calling store methods that need a session.
    pub fn store_and_session(
        &mut self,
    ) -> (RuntimeStore, &mut crate::persistence::db_session::DbSession) {
        (self.runtime_store.clone(), &mut self.session)
    }

    /// Convenience: get an owned DeploymentManager clone and a mutable session reference in one call.
    pub fn dm_and_session(
        &mut self,
    ) -> (
        DeploymentManager,
        &mut crate::persistence::db_session::DbSession,
    ) {
        (self.deployment_manager.clone(), &mut self.session)
    }

    pub fn new(
        deployment_manager: DeploymentManager,
        runtime_store: RuntimeStore,
        session: crate::persistence::db_session::DbSession,
        config: Arc<ProcessEngineConfiguration>,
        http_runtime: Arc<dyn HttpRuntime>,
    ) -> Self {
        let history_manager =
            HistoryManager::new(runtime_store.clone(), config.async_history.enabled)
                .with_history_level(config.history_level)
                .with_enable_process_definition_history_level(
                    config.enable_process_definition_history_level,
                )
                .with_async_history_number_of_retries(config.async_history.number_of_retries);
        let execution_entity_manager =
            Box::new(DefaultExecutionEntityManager::new(runtime_store.clone()));
        let task_entity_manager = Box::new(DefaultTaskEntityManager::new(runtime_store.clone()));
        let mut session_caches: HashMap<String, Box<dyn std::any::Any>> = HashMap::new();
        // Seed listener registries from engine configuration (mirrors service-task
        // delegate session-cache pattern).
        if let Some(ref registry) = config.execution_listener_registry {
            session_caches.insert(
                crate::bpmn::listener::EXECUTION_LISTENER_REGISTRY_CACHE_KEY.to_string(),
                Box::new(registry.clone()),
            );
        }
        if let Some(ref registry) = config.task_listener_registry {
            session_caches.insert(
                crate::bpmn::listener::TASK_LISTENER_REGISTRY_CACHE_KEY.to_string(),
                Box::new(registry.clone()),
            );
        }
        if let Some(ref registry) = config.service_task_delegate_registry {
            session_caches.insert(
                crate::bpmn::behavior::service_task_activity_behavior::SERVICE_TASK_DELEGATE_REGISTRY_CACHE_KEY
                    .to_string(),
                Box::new(registry.clone()),
            );
        }
        if let Some(ref registry) = config.async_service_task_delegate_registry {
            session_caches.insert(
                crate::bpmn::behavior::async_delegate_activity_behavior::ASYNC_SERVICE_TASK_DELEGATE_REGISTRY_CACHE_KEY
                    .to_string(),
                Box::new(registry.clone()),
            );
        }
        if let Some(ref registry) = config.http_handler_registry {
            session_caches.insert(
                crate::bpmn::http_handler::HTTP_HANDLER_REGISTRY_CACHE_KEY.to_string(),
                Box::new(registry.clone()),
            );
        }
        session_caches.insert(
            crate::agenda::future_operations::PENDING_FUTURE_REGISTRY_CACHE_KEY.to_string(),
            Box::new(std::sync::Arc::clone(&config.pending_future_registry)),
        );
        Self {
            session_caches,
            agenda: DefaultFlowableEngineAgenda::new(),
            behavior_factory: Box::new(DefaultActivityBehaviorFactory::new()),
            execution_entity_manager,
            task_entity_manager,
            history_manager,
            deployment_manager,
            runtime_store,
            session,
            config,
            http_runtime,
            post_agenda_events: Vec::new(),
            transaction_events: BTreeMap::new(),
            automatic_job_for_future_success: None,
            automatic_future_success_dispatched: false,
            pending_async_hints: Vec::new(),
        }
    }

    /// Register a committed-job hint. Called by the Java-compatible activation
    /// path after it has pre-locked the row inside this command's transaction.
    /// Nothing is submitted here — the drain happens post-commit.
    pub(crate) fn register_pending_async_hint(&mut self, job: RuntimeTimerJobState) {
        self.pending_async_hints.push(PendingAsyncJobHint { job });
    }

    /// Take the registered hints, leaving the context empty. The command
    /// executor calls this *only* on a successful commit; on rollback the hints
    /// are dropped with the context and nothing is enqueued.
    pub(crate) fn take_pending_async_hints(&mut self) -> Vec<PendingAsyncJobHint> {
        std::mem::take(&mut self.pending_async_hints)
    }

    /// The shared activation coordinator for this engine.
    pub(crate) fn activation_coordinator(
        &self,
    ) -> &crate::engine::activation_coordinator::ActivationCoordinator {
        &self.config.activation_coordinator
    }
}
