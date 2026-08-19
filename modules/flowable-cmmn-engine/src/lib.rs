mod case_file;
mod deployment;
mod error;
mod event_registry_correlation;
mod history;
mod history_cleaning;
mod identity;
mod job;
mod lifecycle_listener;
mod management;
mod models;
mod parent_state_resolver;
mod process_cleanup;
mod query_variable;
mod repository;
mod runtime;
mod store;
mod timer_util;

pub use case_file::CaseFileGraph;
pub use deployment::CmmnDeploymentBuilder;
pub use error::CmmnError;
pub use history::{
    CmmnHistoricCaseInstanceQuery, CmmnHistoricHumanTaskQuery, CmmnHistoricMilestoneQuery,
    CmmnHistoryService,
};
pub use history_cleaning::CmmnHistoryCleaningConfiguration;
pub use identity::CmmnIdentityLinkService;
pub use lifecycle_listener::{
    CmmnLifecycleListenerContext, CmmnLifecycleListenerHandler, CmmnLifecycleListenerRegistry,
    CmmnLifecycleScope,
};
pub use job::{
    ALL_HANDLER_TYPES, CmmnJobExecutionContext, CmmnJobHandler, CmmnJobHandlerRegistry,
    MIGRATION_STATUS_COMPLETED, MIGRATION_STATUS_FAIL, MIGRATION_STATUS_IN_PROGRESS,
    TYPE_ASYNC_ACTIVATE_PLAN_ITEM, TYPE_ASYNC_COMPLETE_PLAN_ITEM, TYPE_ASYNC_DISABLE_PLAN_ITEM,
    TYPE_ASYNC_ENABLE_PLAN_ITEM, TYPE_ASYNC_INIT_PLAN_MODEL, TYPE_ASYNC_LEAVE_ACTIVE_PLAN_ITEM,
    TYPE_ASYNC_REACTIVATE_PLAN_ITEM, TYPE_ASYNC_START_CASE, TYPE_ASYNC_TERMINATE,
    TYPE_CASE_MIGRATION, TYPE_CASE_MIGRATION_STATUS, TYPE_EXTERNAL_WORKER_COMPLETE,
    TYPE_HISTORIC_CASE_MIGRATION, TYPE_HISTORY_CLEANUP, TYPE_SET_ASYNC_VARIABLES,
    TYPE_TRIGGER_TIMER,
};
pub use management::{CmmnManagementJobQuery, CmmnManagementService};
pub use event_registry_correlation::{
    correlation_params_from_payload, generate_correlation_key, generate_event_correlation_keys,
    matches_subscription_configuration,
};
pub use models::{
    CMMN_SCOPE_TYPE, REFERENCE_TYPE_EVENT_CASE, START_EVENT_CORRELATION_MANUAL,
    START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID, CmmnCase, CmmnCaseDefinition,
    CmmnCaseFileItem, CmmnCaseFileItemDefinition, CmmnCaseFileItemDefinitionNode,
    CmmnCaseFileItemOnPart, CmmnCaseFileItemState, CmmnCaseFileModel, CmmnCaseInstance,
    CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel, CmmnCaseTask,
    CmmnChangePlanItemStateRequest, CmmnDecisionTask, CmmnDelegationState, CmmnDeployment,
    CmmnDeploymentRequest, CmmnDeploymentResource, CmmnDiscretionaryItem,
    CmmnEventCorrelationParameter, CmmnEventListener, CmmnEventOutParameter, CmmnEventSubscription,
    CmmnGenericPlanItem, CmmnHistoricCaseInstance, CmmnHistoricHumanTaskInstance,
    CmmnHistoricMilestoneInstance, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskCompletionResult, CmmnHumanTaskInstance, CmmnHumanTaskState, CmmnHumanTaskUpdate,
    CmmnIOParameter, CmmnIdentityLink, CmmnJob, CmmnJobFamily, CmmnLifecycleListener,
    CmmnListenerImplementationType, CmmnMigrationDocument, CmmnMigrationValidationResult,
    CmmnMilestone, CmmnModel, CmmnPlanFragment, CmmnPlanItem, CmmnPlanItemDefinitionWithTargetIds,
    CmmnPlanItemInstance, CmmnPlanItemOnPart, CmmnPlanningTable, CmmnProcessTask,
    CmmnProcessTaskStartRequest, CmmnProcessTaskStartResult, CmmnSentry, CmmnSentryIfPartCondition,
    CmmnSentryIfPartExpression, CmmnSentryIfPartLiteral, CmmnSentryIfPartLogicalOperator,
    CmmnSentryIfPartOperator, CmmnStage, CmmnStageInstance, CmmnStageInstanceState,
    CmmnStageOverview, CmmnTaskAssociationKind, CmmnTaskAssociationState,
    CmmnTaskInstanceAssociation, PagedResult, SentryLifecycleEvent, SentryVariableContext,
    SentryVariableMap,
};
pub use parent_state_resolver::{
    ensure_cmmn_job_parent_allows_activation, is_cmmn_job_parent_suspended, parent_not_cmmn_error,
    parent_suspended_error,
};
pub use process_cleanup::ProcessInstanceCleanup;
pub use query_variable::{
    QueryVariableCondition, QueryVariableOperation, variables_match_conditions,
};
pub use repository::{
    CaseDefinitionSortField, CmmnCaseDefinitionQuery, CmmnDecisionResolver, CmmnDeploymentQuery,
    CmmnDeploymentResourceData, CmmnFormResolver, CmmnRepositoryService, DeploymentSortField,
    ReferencedDecision, ReferencedFormDefinition, SortDirection, cmmn_content_type_for_name,
};
pub use runtime::{
    BpmnCaseTaskCallback, CmmnCaseFileItemService, CmmnCaseInstanceQuery,
    CmmnEventSubscriptionQuery, CmmnHumanTaskQuery, CmmnPlanItemInstanceQuery,
    CmmnProcessTaskRunner, CmmnRuntimeService, CmmnTaskAssociationQuery, CmmnUserGroupResolver,
    TaskSuspensionState,
};
use store::CmmnStore;

use flowable_persistence::DatabaseConfig;
use std::path::Path;
use std::sync::Arc;

pub const CMMN_ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CMMN_PROCESS_TASK_CALLBACK_TYPE: &str = "cmmn-process-task";
/// Java `CallbackTypes.EXECUTION_CHILD_CASE` /
/// `ReferenceTypes.EXECUTION_CHILD_CASE` = `bpmn-2.0-to-cmmn-1.1-child-case`.
pub const CMMN_EXECUTION_CHILD_CASE_CALLBACK_TYPE: &str = "bpmn-2.0-to-cmmn-1.1-child-case";

#[derive(Clone)]
pub struct CmmnEngine {
    repository_service: CmmnRepositoryService,
    runtime_service: CmmnRuntimeService,
    history_service: CmmnHistoryService,
    identity_link_service: CmmnIdentityLinkService,
    management_service: CmmnManagementService,
    job_handler_registry: std::sync::Arc<CmmnJobHandlerRegistry>,
    /// Java `CmmnEngineConfiguration` history-cleaning knobs (P127).
    history_cleaning: CmmnHistoryCleaningConfiguration,
}

impl std::fmt::Debug for CmmnEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CmmnEngine { .. }")
    }
}

impl CmmnEngine {
    pub fn new_in_memory() -> Result<Self, CmmnError> {
        Self::from_store(CmmnStore::in_memory()?)
    }

    pub fn new_sqlite(path: impl AsRef<Path>) -> Result<Self, CmmnError> {
        Self::from_store(CmmnStore::sqlite(path)?)
    }

    pub fn from_database_config(config: DatabaseConfig) -> Result<Self, CmmnError> {
        Self::from_store(CmmnStore::from_config(config)?)
    }

    pub fn new_in_memory_with_process_task_runner(
        process_task_runner: Arc<dyn CmmnProcessTaskRunner>,
    ) -> Result<Self, CmmnError> {
        Self::from_store_with_integrations(CmmnStore::in_memory()?, Some(process_task_runner), None)
    }

    pub fn new_sqlite_with_process_task_runner(
        path: impl AsRef<Path>,
        process_task_runner: Arc<dyn CmmnProcessTaskRunner>,
    ) -> Result<Self, CmmnError> {
        Self::from_store_with_integrations(
            CmmnStore::sqlite(path)?,
            Some(process_task_runner),
            None,
        )
    }

    /// Build an in-memory CMMN engine with optional process-task start and BPMN
    /// cascade cleanup integrations.
    pub fn new_in_memory_with_process_integrations(
        process_task_runner: Option<Arc<dyn CmmnProcessTaskRunner>>,
        process_instance_cleanup: Option<Arc<dyn ProcessInstanceCleanup>>,
    ) -> Result<Self, CmmnError> {
        Self::from_store_with_integrations(
            CmmnStore::in_memory()?,
            process_task_runner,
            process_instance_cleanup,
        )
    }

    /// Build a SQLite-backed CMMN engine with optional process-task start and BPMN
    /// cascade cleanup integrations.
    pub fn new_sqlite_with_process_integrations(
        path: impl AsRef<Path>,
        process_task_runner: Option<Arc<dyn CmmnProcessTaskRunner>>,
        process_instance_cleanup: Option<Arc<dyn ProcessInstanceCleanup>>,
    ) -> Result<Self, CmmnError> {
        Self::from_store_with_integrations(
            CmmnStore::sqlite(path)?,
            process_task_runner,
            process_instance_cleanup,
        )
    }

    /// Build a CMMN engine on an explicitly configured backend (the production
    /// `FLOWABLE_DATABASE_URL` path) with the same process integrations.
    pub fn from_database_config_with_process_integrations(
        config: DatabaseConfig,
        process_task_runner: Option<Arc<dyn CmmnProcessTaskRunner>>,
        process_instance_cleanup: Option<Arc<dyn ProcessInstanceCleanup>>,
    ) -> Result<Self, CmmnError> {
        Self::from_store_with_integrations(
            CmmnStore::from_config(config)?,
            process_task_runner,
            process_instance_cleanup,
        )
    }

    pub fn repository_service(&self) -> CmmnRepositoryService {
        self.repository_service.clone()
    }

    pub fn runtime_service(&self) -> CmmnRuntimeService {
        self.runtime_service.clone()
    }

    /// P126: register a handler for a `class` / `delegateExpression` lifecycle listener.
    /// Rust has no bean container, so Java's Spring/class resolution
    /// (CmmnListenerNotificationHelper.java:162-169) becomes a name → handler registry.
    pub fn register_lifecycle_listener(
        &self,
        name: impl Into<String>,
        handler: Arc<dyn CmmnLifecycleListenerHandler>,
    ) {
        self.runtime_service.register_lifecycle_listener(name, handler);
    }

    /// P126: register a bean method callable from an `expression` lifecycle listener body
    /// (`${auditBean.record(...)}`). Rust's `SimpleExpression` is read-only, so this is how an
    /// expression listener produces a side effect.
    pub fn register_lifecycle_listener_expression_method<F>(
        &self,
        bean: &str,
        method: &str,
        function: F,
    ) where
        F: Fn(&[serde_json::Value]) -> Result<serde_json::Value, String> + Send + Sync + 'static,
    {
        self.runtime_service
            .register_lifecycle_listener_expression_method(bean, method, function);
    }

    pub fn history_service(&self) -> CmmnHistoryService {
        self.history_service.clone()
    }

    pub fn identity_link_service(&self) -> CmmnIdentityLinkService {
        self.identity_link_service.clone()
    }

    pub fn management_service(&self) -> CmmnManagementService {
        self.management_service.clone()
    }

    pub fn job_handler_registry(&self) -> &CmmnJobHandlerRegistry {
        &self.job_handler_registry
    }

    /// P127 history-cleaning configuration (Java `CmmnEngineConfiguration` knobs).
    pub fn history_cleaning_config(&self) -> &CmmnHistoryCleaningConfiguration {
        &self.history_cleaning
    }

    /// Replace history-cleaning configuration (e.g. tests / hosts enabling cleanup).
    pub fn set_history_cleaning_config(&mut self, config: CmmnHistoryCleaningConfiguration) {
        self.history_cleaning = config;
    }

    /// Java `CmmnManagementService.handleHistoryCleanupTimerJob`
    /// (`CmmnManagementServiceImpl.java:274-275`).
    pub fn handle_history_cleanup_timer_job(&self) -> Result<(), CmmnError> {
        history_cleaning::handle_history_cleanup_timer_job(
            &self.management_service,
            &self.history_cleaning,
            chrono::Utc::now(),
        )
    }

    /// Execute a persisted CMMN job via its registered handler, then delete it on success.
    ///
    /// Migration-status jobs are retained after success so callers can read the aggregated
    /// progress written into the job configuration (`completedCount` / `failedCount` / …).
    pub fn execute_job(&self, job_id: &str) -> Result<(), CmmnError> {
        let job = self.management_service.get_job(job_id)?;
        let retain_after_success = job.handler_type.as_deref() == Some(TYPE_CASE_MIGRATION_STATUS);
        let ctx = CmmnJobExecutionContext {
            runtime: &self.runtime_service,
            history: &self.history_service,
            management: &self.management_service,
            history_cleaning: &self.history_cleaning,
        };
        self.job_handler_registry.execute(&job, &ctx)?;
        if retain_after_success {
            return Ok(());
        }
        // Handler side-effects (e.g. case terminate) may cascade-delete the job.
        match self.management_service.delete_job(job_id) {
            Ok(()) => Ok(()),
            Err(CmmnError::NotFound { .. }) => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Execute an existing history-family job through the normal registered-handler path.
    ///
    /// Java `ExecuteHistoryJobCmd.java:45-66` first performs a history-family lookup,
    /// executes the handler and lets the job manager complete/remove the row. This method
    /// deliberately adds no asynchronous history-job producer; callers must supply an
    /// existing history row (for example a deadletter move or an explicit fixture).
    pub fn execute_history_job(&self, job_id: &str) -> Result<(), CmmnError> {
        let job = self.management_service.get_job(job_id).map_err(|_| {
            CmmnError::not_found(format!("CMMN history job '{job_id}' was not found"))
        })?;
        if job.family != CmmnJobFamily::History {
            return Err(CmmnError::not_found(format!(
                "CMMN history job '{job_id}' was not found"
            )));
        }
        self.execute_job(job_id)
    }

    /// Fire every CMMN timer job whose due date has passed (Java
    /// `DefaultJobManager.executeTimerJob` loop). Each due job occurs its timer event
    /// listener plan item, reschedules repeating cycles and deletes the fired row, all
    /// in one transaction. Also fires due `cmmn-history-cleanup` timers (P127).
    /// Returns the ids of the triggered jobs.
    pub fn run_due_timer_jobs(&self) -> Result<Vec<String>, CmmnError> {
        let mut triggered = self.runtime_service.run_due_timer_jobs()?;
        triggered.extend(self.run_due_history_cleanup_timer_jobs()?);
        Ok(triggered)
    }

    /// Fire due `cmmn-history-cleanup` timer jobs via the job-handler path.
    fn run_due_history_cleanup_timer_jobs(&self) -> Result<Vec<String>, CmmnError> {
        let now = chrono::Utc::now();
        let due_jobs = self
            .management_service
            .create_job_query()
            .family(CmmnJobFamily::Timer)
            .handler_type(TYPE_HISTORY_CLEANUP)
            .list()?
            .into_iter()
            .filter(|job| job.due_date.is_some_and(|due| due <= now))
            .map(|job| job.id)
            .collect::<Vec<_>>();
        let mut triggered = Vec::new();
        for job_id in due_jobs {
            self.execute_job(&job_id)?;
            triggered.push(job_id);
        }
        Ok(triggered)
    }

    pub fn deploy(&self, request: CmmnDeploymentRequest) -> Result<CmmnDeployment, CmmnError> {
        self.repository_service.deploy(request)
    }

    pub fn start_case_instance_by_key(
        &self,
        case_definition_key: &str,
        request: CmmnCaseInstanceStartRequest,
    ) -> Result<CmmnCaseInstance, CmmnError> {
        self.runtime_service
            .start_case_instance_by_key(case_definition_key, request)
    }

    pub fn complete_human_task(
        &self,
        task_id: &str,
        request: CmmnHumanTaskCompletionRequest,
    ) -> Result<CmmnHumanTaskCompletionResult, CmmnError> {
        self.runtime_service.complete_human_task(task_id, request)
    }

    pub fn terminate_external_worker_job(
        &self,
        job_id: &str,
        worker_id: &str,
    ) -> Result<(), CmmnError> {
        let job = self.management_service.get_job(job_id)?;
        match job.lock_owner.as_deref() {
            Some(owner) if owner == worker_id => {}
            Some(_) => {
                return Err(CmmnError::conflict(format!(
                    "external worker job {job_id} is locked by a different worker"
                )));
            }
            None => {
                return Err(CmmnError::execution(format!(
                    "external worker job {job_id} is not locked"
                )));
            }
        }

        let case_instance_id = job.scope_id.as_deref().ok_or_else(|| {
            CmmnError::execution(format!(
                "external worker CMMN terminate job {job_id} is missing a case instance scope"
            ))
        })?;

        if let Some(plan_item_definition_id) = job.element_id.as_deref() {
            self.runtime_service.change_plan_item_state(
                case_instance_id,
                CmmnChangePlanItemStateRequest {
                    terminate_plan_item_definition_ids: vec![plan_item_definition_id.to_string()],
                    ..Default::default()
                },
            )?;
        } else {
            self.runtime_service
                .terminate_case_instance(case_instance_id)?;
        }

        self.management_service.delete_job(job_id)
    }

    fn from_store(store: CmmnStore) -> Result<Self, CmmnError> {
        Self::from_store_with_integrations(store, None, None)
    }

    fn from_store_with_integrations(
        store: CmmnStore,
        process_task_runner: Option<Arc<dyn CmmnProcessTaskRunner>>,
        process_instance_cleanup: Option<Arc<dyn ProcessInstanceCleanup>>,
    ) -> Result<Self, CmmnError> {
        Self::from_store_with_integrations_and_cleaning(
            store,
            process_task_runner,
            process_instance_cleanup,
            CmmnHistoryCleaningConfiguration::default(),
        )
    }

    fn from_store_with_integrations_and_cleaning(
        store: CmmnStore,
        process_task_runner: Option<Arc<dyn CmmnProcessTaskRunner>>,
        process_instance_cleanup: Option<Arc<dyn ProcessInstanceCleanup>>,
        history_cleaning: CmmnHistoryCleaningConfiguration,
    ) -> Result<Self, CmmnError> {
        let repository_service =
            CmmnRepositoryService::new(store.clone(), process_instance_cleanup);
        let runtime_service = CmmnRuntimeService::new(
            store.clone(),
            repository_service.clone(),
            process_task_runner,
        );
        let history_service = CmmnHistoryService::new(store.clone(), repository_service.clone());
        let identity_link_service = CmmnIdentityLinkService::new(store.clone());
        let management_service = CmmnManagementService::new(store);

        let engine = Self {
            repository_service,
            runtime_service,
            history_service,
            identity_link_service,
            management_service,
            job_handler_registry: std::sync::Arc::new(
                CmmnJobHandlerRegistry::with_default_handlers(),
            ),
            history_cleaning,
        };
        // Java CmmnEngineImpl.java:90-97 — ensure cleanup timer when enabled.
        if engine.history_cleaning.enable_history_cleaning {
            if let Err(error) = engine.handle_history_cleanup_timer_job() {
                tracing::warn!(
                    "failed to ensure CMMN history cleanup timer job on engine start: {error}"
                );
            }
        }
        Ok(engine)
    }

    /// In-memory engine with history-cleaning config applied before start hook.
    pub fn new_in_memory_with_history_cleaning(
        history_cleaning: CmmnHistoryCleaningConfiguration,
    ) -> Result<Self, CmmnError> {
        Self::from_store_with_integrations_and_cleaning(
            CmmnStore::in_memory()?,
            None,
            None,
            history_cleaning,
        )
    }
}
