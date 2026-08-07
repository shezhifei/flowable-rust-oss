use crate::agenda::FlowableEngineAgenda;
use crate::bpmn::behavior::inclusive_gateway_activity_behavior::execute_inactive_inclusive_joins;
use crate::engine::deployment_manager::DeploymentManager;
use crate::engine::event_dispatcher::TransactionState;
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_interceptor::{CommandInterceptorHandle, run_with_interceptors};
use crate::persistence::runtime_store::RuntimeStore;
use crate::service::config::ProcessEngineConfiguration;
use flowable_http_service::HttpRuntime;
use std::sync::Arc;

/// Sender for post-commit history job dispatch (8I).
/// When set, pending history job IDs are sent non-blocking after flush.
type HistoryDispatcherTx = Option<crossbeam_channel::Sender<Vec<String>>>;

fn dispatch_history_jobs(
    pending: Vec<String>,
    history_job_dispatcher_tx: Option<&crossbeam_channel::Sender<Vec<String>>>,
) {
    if !pending.is_empty()
        && let Some(tx) = history_job_dispatcher_tx
    {
        let _ = tx.try_send(pending);
    }
}

fn rollback_with_transaction_events(
    command_context: &mut CommandContext,
    primary_error: FlowableError,
) -> FlowableError {
    // Listener failures must never prevent the actual database rollback.
    let _ = command_context.dispatch_transaction_events(TransactionState::RollingBack);
    let _ = command_context.session.rollback();
    let _ = command_context.dispatch_transaction_events(TransactionState::RolledBack);
    primary_error
}

/// Interface for executing commands
pub trait CommandExecutor {
    fn execute<T>(&self, command: &dyn Command<T>) -> Result<T, FlowableError>;
}

/// A default simple implementation of CommandExecutor
pub struct DefaultCommandExecutor {
    deployment_manager: DeploymentManager,
    runtime_store: RuntimeStore,
    config: Arc<ProcessEngineConfiguration>,
    http_runtime: Arc<dyn HttpRuntime>,
    history_job_dispatcher_tx: HistoryDispatcherTx,
    /// Optional interceptor chain; empty by default. Terminal remains this executor.
    interceptors: Vec<CommandInterceptorHandle>,
}

impl DefaultCommandExecutor {
    pub fn new(
        deployment_manager: DeploymentManager,
        runtime_store: RuntimeStore,
        config: Arc<ProcessEngineConfiguration>,
        http_runtime: Arc<dyn HttpRuntime>,
    ) -> Self {
        let interceptors = config.command_interceptors.clone();
        Self {
            deployment_manager,
            runtime_store,
            config,
            http_runtime,
            history_job_dispatcher_tx: None,
            interceptors,
        }
    }

    pub fn with_history_job_dispatcher_tx(
        mut self,
        tx: crossbeam_channel::Sender<Vec<String>>,
    ) -> Self {
        self.history_job_dispatcher_tx = Some(tx);
        self
    }

    pub fn with_interceptors(mut self, interceptors: Vec<CommandInterceptorHandle>) -> Self {
        self.interceptors = interceptors;
        self
    }

    pub fn interceptors(&self) -> &[CommandInterceptorHandle] {
        &self.interceptors
    }

    pub fn deployment_manager(&self) -> &DeploymentManager {
        &self.deployment_manager
    }

    pub fn runtime_store(&self) -> &RuntimeStore {
        &self.runtime_store
    }

    pub fn config(&self) -> &ProcessEngineConfiguration {
        &self.config
    }

    pub fn http_runtime(&self) -> &dyn HttpRuntime {
        self.http_runtime.as_ref()
    }
}

impl DefaultCommandExecutor {
    /// Terminal command execution (session + agenda + commit). No interceptors.
    fn execute_terminal<T>(&self, command: &dyn Command<T>) -> Result<T, FlowableError> {
        // P45: reset the per-command dirty set used to strip transient on commit.
        crate::persistence::runtime_store::clear_transient_dirty_execution_ids();
        // P58: reset the per-command involved-process-instance set used by the
        // end-of-command inactive-behavior re-evaluation.
        crate::persistence::runtime_store::clear_involved_process_instances();

        let session = self
            .runtime_store
            .create_session()
            .map_err(|e| FlowableError::Internal(e.to_string()))?;

        let mut command_context = CommandContext::new(
            self.deployment_manager.clone(),
            self.runtime_store.clone(),
            session,
            Arc::clone(&self.config),
            Arc::clone(&self.http_runtime),
        );

        let result = command.execute(&mut command_context);

        let result = match result {
            Ok(r) => {
                let mut agenda_result = Ok(());
                // P58: Java CommandInvoker.java:79-88 — after the agenda loop
                // drains, an ExecuteInactiveBehaviorsOperation re-evaluates
                // inactive inclusive-join tokens (ExecuteInactiveBehaviors-
                // Operation.java:49-101) and, when it activates a join, the
                // agenda is drained again. Loop until a fixpoint: activating
                // one join can plan operations that unblock another.
                'agenda: loop {
                    while let Some(operation) = command_context.agenda.pop_operation() {
                        match operation.run(&mut command_context) {
                            Ok(()) => {}
                            Err(e) => {
                                agenda_result = Err(e);
                                break 'agenda;
                            }
                        }
                    }
                    if !execute_inactive_inclusive_joins(&mut command_context) {
                        break;
                    }
                }
                match agenda_result {
                    Ok(()) => match command_context.dispatch_post_agenda_events() {
                        Ok(()) => Ok(r),
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        };

        match result {
            Ok(r) => {
                if self.config.async_history.enabled {
                    command_context
                        .history_manager
                        .flush_history(&mut command_context.session);
                }
                let pending = if self.config.async_history.enabled {
                    command_context.history_manager.take_pending_jobs()
                } else {
                    Vec::new()
                };
                if let Err(error) =
                    command_context.dispatch_transaction_events(TransactionState::Committing)
                {
                    return Err(rollback_with_transaction_events(
                        &mut command_context,
                        error,
                    ));
                }
                // P45: Java VariableScopeImpl.transientVariables are pure memory —
                // drop them from persisted execution JSON before the transaction
                // commits. Same-command reloads already happened above (agenda).
                command_context
                    .runtime_store
                    .strip_transient_variables_before_commit(&mut command_context.session);
                if let Err(error) = command_context.session.flush_and_commit() {
                    return Err(rollback_with_transaction_events(
                        &mut command_context,
                        FlowableError::Internal(error.to_string()),
                    ));
                }
                // Flowable Java reports ROLLINGBACK / ROLLED_BACK lifecycle
                // notifications when a COMMITTED listener fails, even though
                // the original database transaction is already committed. Do
                // not invoke session.rollback here: these are post-commit
                // lifecycle notifications and cannot undo persisted state.
                if let Err(error) =
                    command_context.dispatch_transaction_events(TransactionState::Committed)
                {
                    let _ =
                        command_context.dispatch_transaction_events(TransactionState::RollingBack);
                    let _ =
                        command_context.dispatch_transaction_events(TransactionState::RolledBack);
                    return Err(error);
                }
                // Java parity: drain committed async-job hints only after the DB
                // transaction has committed, mirroring `JobAddedTransactionListener`
                // (`COMMITTED`). On rollback the hints were discarded with the
                // context above, so nothing is enqueued. The coordinator's submit
                // handle re-reads the row (skipping stale/deleted jobs) and, on a
                // queue-full rejection, dispatches `JOB_REJECTED` + CAS-releases the
                // pre-lock so a later acquisition can pick the job up again.
                let hints = command_context.take_pending_async_hints();
                if !hints.is_empty() {
                    let coordinator = command_context.activation_coordinator().clone();
                    for hint in hints {
                        if let crate::engine::activation_coordinator::HintSubmitOutcome::Fatal(
                            error,
                        ) = coordinator.submit(hint.job)
                        {
                            // The database transaction is already committed,
                            // matching Java's behavior when a COMMITTED
                            // JobAddedTransactionListener fails. Notify the
                            // remaining lifecycle listeners without attempting
                            // an impossible database rollback.
                            let _ = command_context
                                .dispatch_transaction_events(TransactionState::RollingBack);
                            let _ = command_context
                                .dispatch_transaction_events(TransactionState::RolledBack);
                            return Err(error);
                        }
                    }
                }
                dispatch_history_jobs(pending, self.history_job_dispatcher_tx.as_ref());
                Ok(r)
            }
            Err(e) => Err(rollback_with_transaction_events(&mut command_context, e)),
        }
    }
}

impl CommandExecutor for DefaultCommandExecutor {
    fn execute<T>(&self, command: &dyn Command<T>) -> Result<T, FlowableError> {
        crate::el::method_registry::with_expression_method_registry(
            &self.config.expression_method_registry,
            || run_with_interceptors(&self.interceptors, || self.execute_terminal(command)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch_history_jobs;
    use std::time::Duration;

    #[test]
    fn dispatches_pending_history_jobs() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        dispatch_history_jobs(vec!["history-job".to_string()], Some(&tx));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            vec!["history-job".to_string()]
        );
    }

    #[test]
    fn does_not_dispatch_empty_history_batch() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        dispatch_history_jobs(Vec::new(), Some(&tx));
        assert!(rx.try_recv().is_err());
    }
}
