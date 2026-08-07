use crate::engine::async_executor::AsyncExecutor;
use crate::engine::async_history_executor::AsyncHistoryExecutor;
use crate::engine::batch_service::BatchService;
use crate::engine::deployment_manager::DeploymentManager;
use crate::engine::entity_link_service::EntityLinkService;
use crate::engine::event_subscription_service::EventSubscriptionService;
use crate::engine::external_worker_service::ExternalWorkerService;
use crate::engine::historical_migration::{
    HistoricalMigrationBundleExportResult, HistoricalMigrationImportResult,
    HistoricalMigrationRawDialect, HistoricalMigrationReport, export_historical_migration_bundle,
    import_historical_migration_bundle, import_historical_migration_source_manifest,
    import_historical_migration_sql_dump, import_historical_migration_sqlite,
    import_historical_migration_sqlite_dump, inspect_historical_migration_bundle,
    inspect_historical_migration_live_url, inspect_historical_migration_source_manifest,
    inspect_historical_migration_sql_dump, inspect_historical_migration_sqlite,
    inspect_historical_migration_sqlite_dump,
};
use crate::engine::history_job_dispatcher::HistoryJobDispatcher;
use crate::engine::history_service::HistoryService;
use crate::engine::identity_link_service::IdentityLinkService;
use crate::engine::identity_service::IdentityService;
use crate::engine::job_service::JobService;
use crate::engine::management_service::ManagementService;
use crate::engine::repository_service::RepositoryService;
use crate::engine::runtime_service::RuntimeService;
use crate::engine::task_service::EventWaitState;
use crate::engine::task_service::TaskService;
use crate::engine::timer_executor::TimerExecutor;
use crate::engine::variable_service::VariableService;
use crate::error::FlowableError;
use crate::interceptor::command_executor::DefaultCommandExecutor;
use crate::persistence::recovery_snapshot::{RecoverySnapshot, SnapshotDeployment};
use crate::persistence::runtime_store::{
    EventSubscriptionKind, ProcessEventStartSubscription, ProcessTimerStartSubscription,
    RuntimeStore,
};
use crate::service::config::ProcessEngineConfiguration;
use flowable_bpmn_converter::BpmnXMLConverter;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

/// Type alias retained for callers that still depend on the older name
pub use crate::engine::task_service::MessageStyleWaitState;

pub struct ProcessEngine {
    name: String,
    repository_service: Arc<RepositoryService>,
    runtime_service: Arc<RuntimeService>,
    task_service: Arc<TaskService>,
    variable_service: Arc<VariableService>,
    job_service: Arc<JobService>,
    management_service: Arc<ManagementService>,
    history_service: Arc<HistoryService>,
    event_subscription_service: Arc<EventSubscriptionService>,
    external_worker_service: Arc<ExternalWorkerService>,
    identity_service: Arc<IdentityService>,
    identity_link_service: Arc<IdentityLinkService>,
    entity_link_service: Arc<EntityLinkService>,
    batch_service: Arc<BatchService>,
    runtime_store: RuntimeStore,
    command_executor: Arc<DefaultCommandExecutor>,
    timer_executor: Arc<TimerExecutor>,
    async_executor: Option<Arc<AsyncExecutor>>,
    async_history_executor: Option<Arc<AsyncHistoryExecutor>>,
    history_job_dispatcher: Option<std::sync::Mutex<HistoryJobDispatcher>>,
    config: Arc<ProcessEngineConfiguration>,
}

impl ProcessEngine {
    fn rebuild_bpmn_model_cache(deployment_manager: &DeploymentManager) {
        let mut session = deployment_manager.create_session().unwrap();
        let deployments = deployment_manager.get_deployments(&mut session);
        let process_definitions = deployment_manager.get_process_definitions(&mut session);
        let converter = BpmnXMLConverter::new();
        deployment_manager.invalidate_bpmn_model_cache();

        for deployment in deployments.values() {
            for (resource_name, bytes) in &deployment.resources {
                if (resource_name.ends_with(".bpmn") || resource_name.ends_with(".bpmn20.xml"))
                    && let Ok(xml_str) = std::str::from_utf8(bytes)
                {
                    match converter.try_convert_to_bpmn_model(xml_str) {
                        Ok(model) => {
                            for process_definition in process_definitions.values() {
                                if process_definition.deployment_id.as_deref()
                                    == Some(&deployment.id)
                                    && process_definition.resource_name.as_ref()
                                        == Some(resource_name)
                                {
                                    deployment_manager
                                        .insert_bpmn_model(&process_definition.id, model.clone());
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                "failed to rebuild BPMN model cache for resource {resource_name}: {error}"
                            );
                        }
                    }
                }
            }
        }
        // Read-only session; explicitly roll back so the pooled SQLite connection
        // is returned to the pool without an active transaction.
        let _ = session.rollback();
    }

    fn write_recovery_snapshot_file(
        snapshot: &RecoverySnapshot,
        path: &Path,
    ) -> Result<(), FlowableError> {
        let file = File::create(path).map_err(|error| {
            FlowableError::Internal(format!(
                "failed to create recovery snapshot file {}: {}",
                path.display(),
                error
            ))
        })?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, snapshot).map_err(|error| {
            FlowableError::Internal(format!(
                "failed to serialize recovery snapshot to {}: {}",
                path.display(),
                error
            ))
        })?;
        writer.flush().map_err(|error| {
            FlowableError::Internal(format!(
                "failed to flush recovery snapshot file {}: {}",
                path.display(),
                error
            ))
        })
    }

    fn read_recovery_snapshot_file(path: &Path) -> Result<RecoverySnapshot, FlowableError> {
        let file = File::open(path).map_err(|error| {
            FlowableError::Internal(format!(
                "failed to open recovery snapshot file {}: {}",
                path.display(),
                error
            ))
        })?;
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).map_err(|error| {
            FlowableError::Internal(format!(
                "failed to deserialize recovery snapshot from {}: {}",
                path.display(),
                error
            ))
        })
    }

    pub fn new(name: String) -> Self {
        Self::with_time_source(name, Arc::new(crate::engine::time_source::SystemTimeSource))
    }

    pub fn new_with_config(name: String, config: ProcessEngineConfiguration) -> Self {
        match Self::try_new_with_config(name.clone(), config) {
            Ok(engine) => engine,
            Err(error) => {
                tracing::warn!(
                    "Failed to initialize process engine '{}': {}; falling back to default in-memory configuration for legacy constructor",
                    name,
                    error
                );
                Self::new(name)
            }
        }
    }

    pub fn try_new_with_config(
        name: String,
        config: ProcessEngineConfiguration,
    ) -> Result<Self, FlowableError> {
        let config = Arc::new(config);
        let db_store = Arc::new(Self::create_db_store(&config)?);
        let http_runtime = match config.http_service.build_runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::warn!(
                    "Failed to initialize HTTP runtime for process engine '{}': {}; falling back to deterministic runtime for legacy constructor",
                    name,
                    error
                );
                Arc::new(flowable_http_service::DeterministicHttpRuntime::default())
            }
        };
        Ok(Self::build_with_runtime(
            name,
            Arc::new(crate::engine::time_source::SystemTimeSource),
            db_store,
            config,
            http_runtime,
        ))
    }

    fn create_db_store(
        config: &ProcessEngineConfiguration,
    ) -> Result<crate::persistence::db_store::DbStore, FlowableError> {
        let database = config.database.to_persistence_config();
        crate::persistence::db_store::DbStore::from_config(database)
            .map_err(|error| FlowableError::Internal(error.to_string()))
    }

    pub fn with_time_source(
        name: String,
        time_source: Arc<dyn crate::engine::time_source::TimeSource>,
    ) -> Self {
        // Honor FLOWABLE_TEST_ENGINE_DATABASE_URL (and any non-memory DatabaseConfiguration
        // default) so full multi-backend matrices can drive ProcessEngine::new without
        // rewriting every test constructor. Explicit new_with_memory_backend /
        // new_with_db_path paths remain isolated backends.
        let config = ProcessEngineConfiguration::default();
        if !matches!(
            config.database.kind,
            crate::service::config::EngineDatabaseKind::Memory
        ) {
            let kind = config.database.kind;
            let url = config.database.url.clone();
            return Self::build_with_config(name.clone(), time_source, config).unwrap_or_else(
                |error| {
                    panic!(
                        "Failed to initialize process engine '{name}' against configured database {kind:?} ({url}): {error}"
                    )
                },
            );
        }
        let db_store = Arc::new(crate::persistence::db_store::DbStore::new_in_memory().unwrap());
        Self::build_with_runtime(
            name,
            time_source,
            db_store,
            Arc::new(config),
            Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
        )
    }

    pub fn new_with_db_path(name: String, path: &str) -> Self {
        let db_store = Arc::new(crate::persistence::db_store::DbStore::new_file(path).unwrap());
        Self::build_with_runtime(
            name,
            Arc::new(crate::engine::time_source::SystemTimeSource),
            db_store,
            Arc::new(ProcessEngineConfiguration::default()),
            Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
        )
    }

    pub fn new_with_memory_backend(name: String) -> Self {
        let db_store = Arc::new(crate::persistence::db_store::DbStore::new_in_memory().unwrap());
        Self::build_with_runtime(
            name,
            Arc::new(crate::engine::time_source::SystemTimeSource),
            db_store,
            Arc::new(ProcessEngineConfiguration::default()),
            Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
        )
    }

    pub fn build(
        name: String,
        time_source: Arc<dyn crate::engine::time_source::TimeSource>,
        db_store: Arc<crate::persistence::db_store::DbStore>,
    ) -> Self {
        Self::build_with_runtime(
            name,
            time_source,
            db_store,
            Arc::new(ProcessEngineConfiguration::default()),
            Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
        )
    }

    /// Build a ProcessEngine from a ProcessEngineConfiguration.
    ///
    /// The database backend is selected by `config.database` (a `DatabaseConfiguration`),
    /// which is converted to a `flowable_persistence::DatabaseConfig` and used to construct
    /// a `DbSessionFactory` internally.  Callers do **not** need to know about `DbStore`.
    pub fn build_with_config(
        name: String,
        time_source: Arc<dyn crate::engine::time_source::TimeSource>,
        config: ProcessEngineConfiguration,
    ) -> Result<Self, FlowableError> {
        let config = Arc::new(config);
        let http_runtime = config.http_service.build_runtime().map_err(|error| {
            FlowableError::ExecutionError(format!(
                "Failed to initialize HTTP runtime for process engine '{}': {}",
                name, error
            ))
        })?;
        let db_store = Arc::new(Self::create_db_store(&config)?);
        Ok(Self::build_with_runtime(
            name,
            time_source,
            db_store,
            config,
            http_runtime,
        ))
    }

    /// Build a ProcessEngine from a shared `DbStore` and a `ProcessEngineConfiguration`.
    ///
    /// This constructor is intended for scenarios where multiple engine instances must
    /// share the same physical database (e.g. cluster / failover tests).  In all other
    /// cases prefer [`build_with_config`](Self::build_with_config), which constructs the
    /// database layer from `DatabaseConfiguration`.
    pub fn build_with_db_store_and_config(
        name: String,
        time_source: Arc<dyn crate::engine::time_source::TimeSource>,
        db_store: Arc<crate::persistence::db_store::DbStore>,
        config: ProcessEngineConfiguration,
    ) -> Result<Self, FlowableError> {
        let config = Arc::new(config);
        let http_runtime = config.http_service.build_runtime().map_err(|error| {
            FlowableError::ExecutionError(format!(
                "Failed to initialize HTTP runtime for process engine '{}': {}",
                name, error
            ))
        })?;
        Ok(Self::build_with_runtime(
            name,
            time_source,
            db_store,
            config,
            http_runtime,
        ))
    }

    fn build_with_runtime(
        name: String,
        time_source: Arc<dyn crate::engine::time_source::TimeSource>,
        db_store: Arc<crate::persistence::db_store::DbStore>,
        config: Arc<ProcessEngineConfiguration>,
        http_runtime: Arc<dyn flowable_http_service::HttpRuntime>,
    ) -> Self {
        let resolved_lock_owner = config
            .async_executor
            .lock_owner
            .clone()
            .unwrap_or_else(|| format!("{}:{}", name, Uuid::new_v4()));
        let mut resolved_config = (*config).clone();
        resolved_config.async_executor.lock_owner = Some(resolved_lock_owner.clone());
        let config = Arc::new(resolved_config);
        let timer_owner_id: Arc<str> = Arc::from(resolved_lock_owner);
        let session_factory = {
            let db_store = Arc::clone(&db_store);
            Arc::new(move || db_store.create_session())
                as Arc<
                    dyn Fn() -> Result<
                            crate::persistence::db_session::DbSession,
                            crate::persistence::storage_error::StorageError,
                        > + Send
                        + Sync,
                >
        };
        let deployment_manager =
            DeploymentManager::new(Arc::clone(&db_store), Arc::clone(&session_factory));
        let runtime_store =
            RuntimeStore::with_backend(Arc::clone(&db_store), session_factory, time_source)
                .with_bpmn_model_cache(Arc::clone(&deployment_manager.bpmn_model_cache));
        // 8I: Create post-commit channel for history job dispatch
        let (dispatcher_tx, dispatcher_rx) = crossbeam_channel::unbounded();

        let command_executor = Arc::new(
            DefaultCommandExecutor::new(
                deployment_manager.clone(),
                runtime_store.clone(),
                Arc::clone(&config),
                http_runtime,
            )
            .with_history_job_dispatcher_tx(dispatcher_tx),
        );

        let repository_service = Arc::new(RepositoryService::new(Arc::clone(&command_executor)));
        let runtime_service = Arc::new(RuntimeService::new(
            Arc::clone(&command_executor),
            timer_owner_id,
        ));
        let task_service = Arc::new(TaskService::new(Arc::clone(&command_executor)));
        let variable_service = Arc::new(VariableService::new(Arc::clone(&command_executor)));
        let job_service = Arc::new(JobService::new(Arc::clone(&command_executor)));
        let management_service = Arc::new({
            let ms = ManagementService::new(Arc::clone(&command_executor));
            if config.async_history.enabled {
                ms.with_history_job_handler(Arc::new(
                    crate::history::async_history_job_handler::AsyncHistoryJobHandler,
                ))
            } else {
                ms
            }
        });
        let history_service = Arc::new(HistoryService::new(Arc::clone(&command_executor)));
        let event_subscription_service =
            Arc::new(EventSubscriptionService::new(Arc::clone(&command_executor)));
        let external_worker_service =
            Arc::new(ExternalWorkerService::new(Arc::clone(&command_executor)));
        let identity_service = Arc::new(IdentityService::new(Arc::clone(&command_executor)));
        let identity_link_service =
            Arc::new(IdentityLinkService::new(Arc::clone(&command_executor)));
        let entity_link_service = Arc::new(EntityLinkService::new(Arc::clone(&command_executor)));
        let batch_service = Arc::new(BatchService::new(Arc::clone(&command_executor)));

        Self::rebuild_bpmn_model_cache(&deployment_manager);

        // P76: wire CMMN → BPMN caseServiceTask completion callback
        // (Java ChildBpmnCaseInstanceStateChangeCallback).
        if let Some(cmmn_engine) = config.cmmn_engine.as_ref() {
            let callback = Arc::new(
                crate::engine::bpmn_case_task_callback::ProcessEngineBpmnCaseTaskCallback::new(
                    Arc::clone(&command_executor),
                ),
            );
            cmmn_engine
                .runtime_service()
                .set_bpmn_case_task_callback(callback);
        }

        let history_job_dispatcher = if config.async_history.enabled {
            Some(std::sync::Mutex::new(HistoryJobDispatcher::new(
                dispatcher_rx,
            )))
        } else {
            None
        };

        // 8K: Independent history executor when use_shared_executor = false
        let async_history_executor =
            if config.async_history.enabled && !config.async_history.use_shared_executor {
                Some(Arc::new(AsyncHistoryExecutor::new(
                    config.async_history.clone(),
                )))
            } else {
                None
            };

        // Java parity: configure the shared activation coordinator with the
        // resolved executor identity so a command can pre-lock + hint jobs while
        // the executor is live. The coordinator's active flag is shared with the
        // executor below (same Arc), and its submit handle is installed once the
        // executor and runtime service exist.
        config.activation_coordinator.configure(
            runtime_service.timer_owner_id().to_string(),
            config.async_executor.async_job_lock_time_ms as i64,
            config.async_executor.enabled_job_categories.clone(),
            config.async_executor.tenant_ids.clone(),
        );

        let async_executor = if config.async_executor.enabled || config.async_executor.auto_activate
        {
            let executor = Arc::new(
                AsyncExecutor::new_with_lock_owner(
                    config.async_executor.clone(),
                    runtime_service.timer_owner_id(),
                )
                .with_shared_active_flag(config.activation_coordinator.active_flag()),
            );
            // Post-commit submit handle: offer a committed, pre-locked job to the
            // live executor. Runs on the command-executor thread after the DB
            // transaction commits (see CommandContext pending-hint drain).
            //
            // The handle is stored on the activation coordinator, which lives
            // inside the engine `config` `Arc`. Capturing strong `Arc`s to the
            // runtime service / executor here would form a reference cycle
            // (config -> coordinator -> closure -> runtime_service ->
            // command_executor -> config) that keeps the whole engine — and its
            // SQLite store — alive after the engine is dropped. Capture `Weak`
            // references instead and upgrade them at call time; if the engine is
            // gone the hint is simply dropped.
            let submit_runtime_service = Arc::downgrade(&runtime_service);
            let submit_executor = Arc::downgrade(&executor);
            config.activation_coordinator.set_submit_handle(Arc::new(
                move |job: crate::persistence::runtime_store::RuntimeTimerJobState| {
                    use crate::engine::activation_coordinator::HintSubmitOutcome;
                    let (Some(runtime_service), Some(executor)) =
                        (submit_runtime_service.upgrade(), submit_executor.upgrade())
                    else {
                        // Engine is shutting down / already dropped.
                        return HintSubmitOutcome::NoExecutor;
                    };
                    // Delegate to RuntimeService so the committed hint re-reads the
                    // row (skipping stale/deleted jobs) and, on a pool rejection,
                    // dispatches JOB_REJECTED + CAS-releases the pre-lock. The
                    // closure here is only the raw "offer to the pool" step.
                    let offer_runtime_service = Arc::clone(&runtime_service);
                    let offer =
                        move |current: crate::persistence::runtime_store::RuntimeTimerJobState| {
                            // Pre-locked hint: execute through the direct-hint path so
                            // the coordinator lease is skipped and the executor row
                            // lock is re-verified (no fake fencing token).
                            executor
                                .submit_direct_hint_job(Arc::clone(&offer_runtime_service), current)
                        };
                    match runtime_service.submit_committed_async_hint(&job, &offer) {
                        Ok(()) => HintSubmitOutcome::Submitted,
                        Err(error) => HintSubmitOutcome::Fatal(error),
                    }
                },
            ));
            Some(executor)
        } else {
            None
        };
        let auto_activate_async_executor = config.async_executor.auto_activate;
        let enable_history_cleaning = config.enable_history_cleaning;

        let engine = Self {
            name,
            repository_service,
            runtime_service,
            task_service,
            variable_service,
            job_service,
            management_service,
            history_service,
            event_subscription_service,
            external_worker_service,
            identity_service,
            identity_link_service,
            entity_link_service,
            batch_service,
            runtime_store,
            command_executor,
            timer_executor: Arc::new(TimerExecutor::new()),
            async_executor,
            async_history_executor,
            history_job_dispatcher,
            config,
        };
        if auto_activate_async_executor {
            engine.start_timer_executor();
        }
        // Java ProcessEngineImpl.java:105-110 — when enableHistoryCleaning, ensure
        // the single bpmn-history-cleanup timer job exists at engine construction.
        // Java swallows optimistic-lock races from concurrent nodes; Rust is single-
        // writer for the ensure cmd so we surface real errors.
        if enable_history_cleaning {
            if let Err(error) = engine
                .management_service
                .handle_history_cleanup_timer_job()
            {
                tracing::warn!(
                    "failed to ensure BPMN history cleanup timer job on engine start: {error}"
                );
            }
        }
        engine
    }

    pub fn get_config(&self) -> Arc<ProcessEngineConfiguration> {
        Arc::clone(&self.config)
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn get_repository_service(&self) -> Arc<RepositoryService> {
        Arc::clone(&self.repository_service)
    }

    pub fn get_runtime_service(&self) -> Arc<RuntimeService> {
        Arc::clone(&self.runtime_service)
    }

    pub fn get_task_service(&self) -> Arc<TaskService> {
        Arc::clone(&self.task_service)
    }

    pub fn get_variable_service(&self) -> Arc<VariableService> {
        Arc::clone(&self.variable_service)
    }

    pub fn get_job_service(&self) -> Arc<JobService> {
        Arc::clone(&self.job_service)
    }

    pub fn get_management_service(&self) -> Arc<ManagementService> {
        Arc::clone(&self.management_service)
    }

    pub fn get_history_service(&self) -> Arc<HistoryService> {
        Arc::clone(&self.history_service)
    }

    pub fn get_event_subscription_service(&self) -> Arc<EventSubscriptionService> {
        Arc::clone(&self.event_subscription_service)
    }

    pub fn get_external_worker_service(&self) -> Arc<ExternalWorkerService> {
        Arc::clone(&self.external_worker_service)
    }

    pub fn get_identity_service(&self) -> Arc<IdentityService> {
        Arc::clone(&self.identity_service)
    }

    pub fn get_identity_link_service(&self) -> Arc<IdentityLinkService> {
        Arc::clone(&self.identity_link_service)
    }

    pub fn get_entity_link_service(&self) -> Arc<EntityLinkService> {
        Arc::clone(&self.entity_link_service)
    }

    pub fn get_batch_service(&self) -> Arc<BatchService> {
        Arc::clone(&self.batch_service)
    }

    pub fn wake_up_message_by_process_instance_id(&self, process_instance_id: String) {
        if let Err(e) = self
            .task_service
            .wake_up_message_by_process_instance_id(process_instance_id)
        {
            tracing::warn!("Error in wake_up_message_by_process_instance_id: {:?}", e);
        }
    }

    pub fn wake_up_message_by_message_ref(&self, process_instance_id: String, message_ref: String) {
        let _ = self
            .task_service
            .wake_up_message_by_message_ref(process_instance_id, message_ref);
    }

    pub fn trigger_intermediate_catch_event_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) {
        self.runtime_service
            .trigger_intermediate_catch_event_by_process_instance_id(process_instance_id);
    }

    /// Unified: triggers an intermediate catch event by subscription kind + event_ref + execution_id.
    pub fn trigger_event_intermediate_catch(
        &self,
        subscription_kind: EventSubscriptionKind,
        event_ref: String,
        execution_id: String,
    ) {
        self.runtime_service.trigger_event_intermediate_catch(
            subscription_kind,
            event_ref,
            execution_id,
        );
    }

    /// Stable entry point for message intermediate catch.
    pub fn trigger_intermediate_catch_event_by_message_ref_and_execution_id(
        &self,
        message_ref: String,
        execution_id: String,
    ) {
        self.runtime_service
            .trigger_intermediate_catch_event_by_message_ref_and_execution_id(
                message_ref,
                execution_id,
            );
    }

    pub fn trigger_timer_intermediate_catch_event(&self, execution_id: String) {
        let _ = self
            .runtime_service
            .trigger_timer_intermediate_catch_event(execution_id);
    }

    pub fn get_event_wait_states_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Vec<EventWaitState> {
        self.runtime_service
            .get_event_wait_states_by_process_instance_id(process_instance_id)
    }

    /// Type alias for callers that depend on the older name
    pub fn get_message_style_wait_states_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Vec<EventWaitState> {
        self.get_event_wait_states_by_process_instance_id(process_instance_id)
    }

    pub fn trigger_boundary_event(&self, boundary_event_id: String, process_instance_id: String) {
        let _ = self
            .runtime_service
            .trigger_boundary_event(boundary_event_id, process_instance_id);
    }

    /// Unified: triggers a boundary event by subscription kind + event_ref.
    pub fn trigger_boundary_event_by_event_ref(
        &self,
        subscription_kind: EventSubscriptionKind,
        event_ref: String,
        process_instance_id: String,
    ) {
        self.runtime_service.trigger_boundary_event_by_event_ref(
            subscription_kind,
            event_ref,
            process_instance_id,
        );
    }

    /// Stable entry point for message boundary trigger.
    pub fn trigger_boundary_event_by_message_ref(
        &self,
        message_ref: String,
        process_instance_id: String,
    ) {
        self.runtime_service
            .trigger_boundary_event_by_message_ref(message_ref, process_instance_id);
    }

    /// Stable entry point for signal boundary trigger.
    pub fn trigger_boundary_event_by_signal_ref(
        &self,
        signal_ref: String,
        process_instance_id: String,
    ) {
        self.runtime_service
            .trigger_boundary_event_by_signal_ref(signal_ref, process_instance_id);
    }

    pub fn trigger_timer_boundary_event(
        &self,
        boundary_event_id: String,
        process_instance_id: String,
    ) {
        self.runtime_service
            .trigger_timer_boundary_event(boundary_event_id, process_instance_id);
    }

    pub fn get_runtime_store(&self) -> RuntimeStore {
        self.runtime_store.clone()
    }

    pub fn get_command_executor(&self) -> Arc<DefaultCommandExecutor> {
        Arc::clone(&self.command_executor)
    }

    pub fn run_due_timers(&self) -> Vec<String> {
        self.runtime_service.run_due_timers().unwrap()
    }

    pub fn start_timer_executor(&self) {
        if let Err(error) = self.try_start_timer_executor() {
            tracing::error!("failed to start process-engine async executor: {error}");
        }
    }

    pub fn try_start_timer_executor(&self) -> Result<(), FlowableError> {
        if let Some(async_exec) = &self.async_executor {
            async_exec.try_start(Arc::clone(&self.runtime_service))?;
            // 8I: Also start the history job dispatcher (post-commit channel)
            // only when history shares the main async executor.
            if self.async_history_executor.is_none()
                && let Some(dispatcher_lock) = &self.history_job_dispatcher
                && let Ok(mut dispatcher) = dispatcher_lock.lock()
            {
                async_exec.start_history_dispatcher(
                    Arc::clone(&self.runtime_service),
                    &mut dispatcher,
                    self.runtime_store.clone(),
                );
            }
        } else {
            self.timer_executor.start(Arc::clone(&self.runtime_service));
        }
        // 8K: Independent history executor runs alongside the main executor
        // when use_shared_executor = false.
        if let Some(hist_exec) = &self.async_history_executor {
            hist_exec.start(Arc::clone(&self.runtime_service));
        }
        Ok(())
    }

    pub fn stop_timer_executor(&self) {
        if let Err(error) = self.try_stop_timer_executor() {
            tracing::error!("failed to stop process-engine async executor: {error}");
        }
    }

    pub fn try_stop_timer_executor(&self) -> Result<(), FlowableError> {
        if let Some(hist_exec) = &self.async_history_executor {
            hist_exec.shutdown();
        }
        if let Some(async_exec) = &self.async_executor {
            if let Some(dispatcher_lock) = &self.history_job_dispatcher
                && let Ok(dispatcher) = dispatcher_lock.lock()
            {
                dispatcher.stop();
            }
            async_exec.try_shutdown()?;
        } else {
            self.timer_executor.stop();
        }
        Ok(())
    }

    pub fn get_async_executor(&self) -> Option<Arc<AsyncExecutor>> {
        self.async_executor.as_ref().map(Arc::clone)
    }

    pub fn async_executor_is_active(&self) -> bool {
        self.async_executor
            .as_ref()
            .is_some_and(|executor| executor.is_active())
    }

    pub fn is_async_executor_active(&self) -> bool {
        self.async_executor_is_active()
    }

    /// Explicitly closes engine-owned background executors.
    ///
    /// `ProcessEngine` deliberately has no `Drop` implementation: its runtime
    /// services and executors are exposed through shared `Arc` handles, so an
    /// implicit drop-triggered shutdown could stop work still owned by callers.
    pub fn close(&self) {
        self.stop_timer_executor();
    }

    pub fn get_async_history_executor(&self) -> Option<&Arc<AsyncHistoryExecutor>> {
        self.async_history_executor.as_ref()
    }

    /// Stops acquiring new timer work without waiting for in-flight items.
    pub fn stop_acquiring_timer_work(&self) {
        self.timer_executor.stop_acquiring();
    }

    /// Blocks until all in-flight timer work has completed.
    pub fn drain_timer_executor(&self) {
        self.timer_executor.drain();
    }

    /// Returns the number of timer work items currently in-flight.
    pub fn timer_executor_in_flight_count(&self) -> usize {
        self.timer_executor.in_flight_count()
    }

    /// Returns whether the timer executor is currently acquiring new work.
    pub fn timer_executor_is_acquiring(&self) -> bool {
        self.timer_executor.is_acquiring()
    }

    pub fn set_timer_executor_before_execute_hook(
        &self,
        hook: Option<Arc<dyn Fn() + Send + Sync>>,
    ) {
        self.timer_executor.set_before_execute_hook(hook);
    }

    pub fn set_timer_executor_config(
        &self,
        config: crate::engine::timer_worker::TimerWorkerConfig,
    ) {
        self.timer_executor.set_config(config);
    }

    // ── Timer Coordination Control Surface ──

    /// Returns the current coordinator status (leader, fencing token, expiry, state).
    pub fn get_timer_coordinator_status(
        &self,
    ) -> crate::persistence::runtime_store::TimerCoordinatorStatus {
        self.runtime_service.get_timer_coordinator_status()
    }

    /// Returns a snapshot of all registered timer worker nodes with liveness status.
    pub fn list_timer_nodes(&self) -> Vec<crate::persistence::runtime_store::TimerNodeStatus> {
        self.runtime_service.list_timer_nodes().unwrap()
    }

    /// Owner-safe release: release leadership for the caller's owner identity.
    pub fn release_timer_leadership(&self, fencing_token: i64) -> bool {
        self.runtime_service
            .release_leadership(fencing_token)
            .unwrap()
    }

    /// Admin step-down: force release the current leader, advancing the fencing token.
    pub fn admin_step_down(&self) -> (bool, i64) {
        self.runtime_service.admin_step_down().unwrap()
    }

    /// Deregister a specific timer node by ID.
    pub fn deregister_timer_node(&self, node_id: &str) -> bool {
        self.runtime_service.deregister_timer_node(node_id).unwrap()
    }

    /// Remove all expired timer nodes from the registry.
    pub fn cleanup_expired_timer_nodes(&self) -> usize {
        self.runtime_service.cleanup_expired_timer_nodes().unwrap()
    }

    pub fn get_timer_start_subscriptions(&self) -> Vec<ProcessTimerStartSubscription> {
        let mut session = self.runtime_store.create_session().unwrap();
        self.command_executor
            .deployment_manager()
            .get_timer_start_subscriptions(&mut session)
    }

    pub fn get_event_start_subscriptions(&self) -> Vec<ProcessEventStartSubscription> {
        let mut session = self.runtime_store.create_session().unwrap();
        self.command_executor
            .deployment_manager()
            .get_event_start_subscriptions(&mut session)
    }

    // ── Message/Signal Start Event API ──

    /// Starts a new process instance by triggering a message start event subscription.
    pub fn start_process_instance_by_message(
        &self,
        message_ref: String,
    ) -> crate::runtime::process_instance::ProcessInstance {
        self.runtime_service
            .start_process_instance_by_message(message_ref)
            .unwrap()
    }

    /// Starts a new process instance by triggering a signal start event subscription.
    pub fn start_process_instance_by_signal(
        &self,
        signal_ref: String,
    ) -> crate::runtime::process_instance::ProcessInstance {
        self.runtime_service
            .start_process_instance_by_signal(signal_ref)
            .unwrap()
    }

    // ── Event Subprocess Trigger API (message/signal) ──

    /// Triggers a message event subprocess within a running process instance.
    pub fn trigger_event_subprocess_by_message(
        &self,
        message_ref: String,
        process_instance_id: String,
    ) -> Vec<String> {
        self.runtime_service
            .trigger_event_subprocess_by_message(message_ref, process_instance_id)
    }

    /// Triggers a signal event subprocess within a running process instance.
    pub fn trigger_event_subprocess_by_signal(
        &self,
        signal_ref: String,
        process_instance_id: String,
    ) -> Vec<String> {
        self.runtime_service
            .trigger_event_subprocess_by_signal(signal_ref, process_instance_id)
    }

    // ── Snapshot / Recovery ──

    pub fn export_recovery_snapshot(&self) -> RecoverySnapshot {
        let mut session = self.runtime_store.create_session().unwrap();
        let deployment_manager = self.command_executor.deployment_manager();
        let deployments = deployment_manager
            .get_deployments(&mut session)
            .into_values()
            .map(|d| SnapshotDeployment {
                resources: d.resources.clone(),
                deployment: d.clone(),
            })
            .collect();
        let process_definitions = deployment_manager
            .get_process_definitions(&mut session)
            .into_values()
            .collect();
        let process_timer_start_subscriptions =
            deployment_manager.get_timer_start_subscriptions(&mut session);
        let process_event_start_subscriptions =
            deployment_manager.get_event_start_subscriptions(&mut session);

        let process_instances = self
            .runtime_store
            .snapshot_process_instances(&mut session)
            .into_values()
            .collect();
        let executions = self
            .runtime_store
            .snapshot_executions(&mut session)
            .into_values()
            .collect();
        let event_wait_states = self
            .runtime_store
            .snapshot_event_wait_states(&mut session)
            .into_values()
            .collect();
        let boundary_event_states = self
            .runtime_store
            .snapshot_boundary_event_states(&mut session)
            .into_values()
            .collect();
        let timer_job_states = self
            .runtime_store
            .snapshot_timer_job_states(&mut session)
            .into_values()
            .collect();
        let event_subprocess_timer_subscriptions = self
            .runtime_store
            .snapshot_event_subprocess_timer_subscriptions(&mut session)
            .into_values()
            .collect();
        let event_subprocess_event_subscriptions = self
            .runtime_store
            .snapshot_event_subprocess_event_subscriptions(&mut session)
            .into_values()
            .collect();
        let tasks = self
            .runtime_store
            .snapshot_tasks(&mut session)
            .into_values()
            .collect();

        RecoverySnapshot {
            deployments,
            process_definitions,
            process_timer_start_subscriptions,
            process_event_start_subscriptions,
            process_instances,
            executions,
            event_wait_states,
            boundary_event_states,
            timer_job_states,
            event_subprocess_timer_subscriptions,
            event_subprocess_event_subscriptions,
            tasks,
        }
    }

    pub fn export_recovery_snapshot_to_file<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<(), FlowableError> {
        let snapshot = self.export_recovery_snapshot();
        Self::write_recovery_snapshot_file(&snapshot, path.as_ref())
    }

    pub fn import_recovery_snapshot(&self, snapshot: RecoverySnapshot) {
        let mut session = self.runtime_store.create_session().unwrap();
        let deployment_manager = self.command_executor.deployment_manager();

        for snap_dep in snapshot.deployments {
            let mut deployment = snap_dep.deployment.clone();
            deployment.resources = snap_dep.resources.clone();
            deployment_manager.register_deployment(deployment, &mut session);
        }

        for pd in snapshot.process_definitions {
            deployment_manager.insert_process_definition(pd, &mut session);
        }

        deployment_manager.register_timer_start_subscriptions(
            snapshot.process_timer_start_subscriptions,
            &mut session,
        );
        deployment_manager.register_event_start_subscriptions(
            snapshot.process_event_start_subscriptions,
            &mut session,
        );

        for pi in snapshot.process_instances {
            self.runtime_store
                .insert_process_instance(&pi, &mut session);
        }
        for ex in snapshot.executions {
            self.runtime_store.insert_execution(&ex, &mut session);
        }
        for ws in snapshot.event_wait_states {
            self.runtime_store
                .insert_event_wait_state(&ws, &mut session);
        }
        for bs in snapshot.boundary_event_states {
            self.runtime_store
                .insert_boundary_event_state(bs, &mut session);
        }
        for tj in snapshot.timer_job_states {
            self.runtime_store.insert_timer_job_state(&tj, &mut session);
        }
        for sub in snapshot.event_subprocess_timer_subscriptions {
            self.runtime_store
                .insert_event_subprocess_timer_subscription(sub, &mut session);
        }
        for sub in snapshot.event_subprocess_event_subscriptions {
            self.runtime_store
                .insert_event_subprocess_event_subscription(sub, &mut session);
        }
        for task in snapshot.tasks {
            self.runtime_store.insert_task(&task, &mut session);
        }
        session.flush_and_commit().unwrap();

        Self::rebuild_bpmn_model_cache(deployment_manager);
    }

    pub fn import_recovery_snapshot_from_file<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<(), FlowableError> {
        let snapshot = Self::read_recovery_snapshot_file(path.as_ref())?;
        self.import_recovery_snapshot(snapshot);
        Ok(())
    }

    pub fn inspect_historical_migration_sqlite<P: AsRef<Path>>(
        path: P,
    ) -> Result<HistoricalMigrationReport, FlowableError> {
        inspect_historical_migration_sqlite(path.as_ref())
    }

    pub fn export_historical_migration_bundle<P: AsRef<Path>, Q: AsRef<Path>>(
        source_db: P,
        bundle_path: Q,
    ) -> Result<HistoricalMigrationBundleExportResult, FlowableError> {
        export_historical_migration_bundle(source_db.as_ref(), bundle_path.as_ref())
    }

    pub fn inspect_historical_migration_bundle<P: AsRef<Path>>(
        path: P,
    ) -> Result<HistoricalMigrationReport, FlowableError> {
        inspect_historical_migration_bundle(path.as_ref())
    }

    pub fn inspect_historical_migration_source_manifest<P: AsRef<Path>>(
        path: P,
    ) -> Result<HistoricalMigrationReport, FlowableError> {
        inspect_historical_migration_source_manifest(path.as_ref())
    }

    pub fn inspect_historical_migration_sql_dump<P: AsRef<Path>>(
        path: P,
        dialect: HistoricalMigrationRawDialect,
    ) -> Result<HistoricalMigrationReport, FlowableError> {
        inspect_historical_migration_sql_dump(path.as_ref(), dialect)
    }

    pub fn inspect_historical_migration_sqlite_dump<P: AsRef<Path>>(
        path: P,
    ) -> Result<HistoricalMigrationReport, FlowableError> {
        inspect_historical_migration_sqlite_dump(path.as_ref())
    }

    /// Bounded live-database historical inspect (`sqlite` | `postgres` | `mysql`).
    ///
    /// Postgres/MySQL require the corresponding cargo features and a reachable URL.
    pub fn inspect_historical_migration_live_url(
        url: impl AsRef<str>,
        kind: impl AsRef<str>,
    ) -> Result<HistoricalMigrationReport, FlowableError> {
        inspect_historical_migration_live_url(url.as_ref(), kind.as_ref())
    }

    pub fn import_historical_migration_from_sqlite<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<HistoricalMigrationImportResult, FlowableError> {
        import_historical_migration_sqlite(
            path.as_ref(),
            self.command_executor.deployment_manager(),
            &self.runtime_store,
            &self.config.business_calendar_registry,
        )
    }

    pub fn import_historical_migration_from_bundle<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<HistoricalMigrationImportResult, FlowableError> {
        import_historical_migration_bundle(
            path.as_ref(),
            self.command_executor.deployment_manager(),
            &self.runtime_store,
            &self.config.business_calendar_registry,
        )
    }

    pub fn import_historical_migration_from_source_manifest<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<HistoricalMigrationImportResult, FlowableError> {
        import_historical_migration_source_manifest(
            path.as_ref(),
            self.command_executor.deployment_manager(),
            &self.runtime_store,
            &self.config.business_calendar_registry,
        )
    }

    pub fn import_historical_migration_from_sql_dump<P: AsRef<Path>>(
        &self,
        path: P,
        dialect: HistoricalMigrationRawDialect,
    ) -> Result<HistoricalMigrationImportResult, FlowableError> {
        import_historical_migration_sql_dump(
            path.as_ref(),
            dialect,
            self.command_executor.deployment_manager(),
            &self.runtime_store,
            &self.config.business_calendar_registry,
        )
    }

    pub fn import_historical_migration_from_sqlite_dump<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<HistoricalMigrationImportResult, FlowableError> {
        import_historical_migration_sqlite_dump(
            path.as_ref(),
            self.command_executor.deployment_manager(),
            &self.runtime_store,
            &self.config.business_calendar_registry,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::config::AsyncExecutorConfiguration;

    fn lifecycle_test_config() -> AsyncExecutorConfiguration {
        AsyncExecutorConfiguration {
            pool_size: 1,
            queue_size: 8,
            async_job_acquisition_enabled: false,
            timer_job_acquisition_enabled: false,
            reset_expired_job_enabled: false,
            ..AsyncExecutorConfiguration::default()
        }
    }

    #[test]
    fn enabled_executor_preserves_manual_start_lifecycle() {
        let mut config = ProcessEngineConfiguration::default();
        config.async_executor = AsyncExecutorConfiguration {
            enabled: true,
            ..lifecycle_test_config()
        };

        let engine = ProcessEngine::build_with_config(
            "manual-async-lifecycle".to_string(),
            Arc::new(crate::engine::time_source::SystemTimeSource),
            config,
        )
        .expect("build engine with manually activated executor");

        assert!(engine.get_async_executor().is_some());
        assert!(!engine.async_executor_is_active());
        let executor = engine.get_async_executor().unwrap();
        let resolved_owner = executor.lock_owner().to_string();
        assert!(!resolved_owner.is_empty());
        assert_eq!(
            engine.get_config().async_executor.lock_owner.as_deref(),
            Some(resolved_owner.as_str())
        );
        assert_eq!(
            engine.get_runtime_service().timer_owner_id(),
            resolved_owner
        );
        engine.close();
        assert!(!engine.async_executor_is_active());
    }

    #[test]
    fn auto_activation_constructs_starts_and_closes_with_configured_owner() {
        let mut config = ProcessEngineConfiguration::default();
        config.async_executor = AsyncExecutorConfiguration {
            auto_activate: true,
            lock_owner: Some("configured-engine-owner".to_string()),
            ..lifecycle_test_config()
        };

        let engine = ProcessEngine::build_with_config(
            "automatic-async-lifecycle".to_string(),
            Arc::new(crate::engine::time_source::SystemTimeSource),
            config,
        )
        .expect("build engine with automatically activated executor");

        assert!(engine.get_async_executor().is_some());
        assert!(engine.is_async_executor_active());
        assert_eq!(
            engine.get_runtime_service().timer_owner_id(),
            "configured-engine-owner"
        );

        engine.close();
        assert!(!engine.async_executor_is_active());
    }

    #[test]
    fn process_engine_public_lifecycle_restarts_the_executor_pool() {
        let mut config = ProcessEngineConfiguration::default();
        config.async_executor = AsyncExecutorConfiguration {
            enabled: true,
            ..lifecycle_test_config()
        };

        let engine = ProcessEngine::build_with_config(
            "restartable-async-lifecycle".to_string(),
            Arc::new(crate::engine::time_source::SystemTimeSource),
            config,
        )
        .expect("build engine with restartable executor");
        let executor = engine
            .get_async_executor()
            .expect("enabled executor should be constructed");

        engine.start_timer_executor();
        assert!(engine.async_executor_is_active());
        assert_eq!(executor.remaining_capacity(), 8);

        engine.stop_timer_executor();
        assert!(!engine.async_executor_is_active());
        assert_eq!(executor.remaining_capacity(), 0);

        engine.start_timer_executor();
        assert!(engine.async_executor_is_active());
        assert_eq!(executor.remaining_capacity(), 8);

        engine.close();
        assert!(!engine.async_executor_is_active());
        assert_eq!(executor.remaining_capacity(), 0);
    }
}
