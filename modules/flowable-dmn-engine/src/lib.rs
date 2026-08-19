mod editor;
mod error;
pub mod feel;
mod history;
mod models;
mod repository;
mod runtime;
mod store;

pub use editor::{
    DmnEditorCapabilities, EDITOR_COLLECT_OPERATORS, EDITOR_CREATABLE_HIT_POLICIES,
    EDITOR_INPUT_UNARY_TEST_FORMS, EDITOR_OUTPUT_EXPRESSION_FUNCTIONS,
    EDITOR_OUTPUT_EXPRESSION_OPERATORS, EDITOR_ROUND_TRIP_HIT_POLICIES, EDITOR_VALUE_TYPE_REFS,
    editor_capabilities, evaluate_editor_expression, validate_editor_definition,
    validate_editor_expression,
};
pub use error::DmnError;
pub use history::{DmnExecutionHistoryQuery, DmnHistoryService};
pub use models::{
    Annotation, CollectOperator, DecisionService, DmnComparisonOperator, DmnDecision,
    DmnDecisionDefinition, DmnDeferredOperator, DmnDeployment, DmnDeploymentRequest,
    DmnDeploymentResource, DmnExecutionRequest, DmnExecutionResult, DmnExpressionExecution,
    DmnHitPolicy, DmnInputClause, DmnListContainsNeedle, DmnModel, DmnOutputClause, DmnRule,
    DmnRuleExecutionAudit, DmnRuleInputEntry, DmnRuleOutputEntry, DmnStringFunction,
    DmnStringTransform, DmnUnaryTest, FeelExpressionEngine, HistoricDecisionExecution, PagedResult,
    columnar_outputs_to_rows, stack_variables_from_rows,
};
pub use repository::{
    DmnDecisionQuery, DmnDeploymentQuery, DmnDeploymentResourceData, DmnRepositoryService,
    dmn_content_type_for_name,
};
pub use runtime::DmnDecisionService;
use store::DmnStore;

pub use flowable_persistence::{DatabaseConfig, DatabaseKind, SchemaMode};

use std::path::Path;

#[derive(Clone)]
pub struct DmnEngine {
    repository_service: DmnRepositoryService,
    decision_service: DmnDecisionService,
    history_service: DmnHistoryService,
}

impl std::fmt::Debug for DmnEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DmnEngine { .. }")
    }
}

/// Builder for [`DmnEngine`]. Defaults match Java `DmnEngineConfiguration`
/// (`strictMode = true` at `DmnEngineConfiguration.java:202`).
#[derive(Clone, Debug)]
pub struct DmnEngineBuilder {
    /// Java `DmnEngineConfiguration.strictMode` (:197-202, getter :1109-1111).
    /// When true, hit-policy violations raise; when false, validation messages
    /// are recorded and evaluation continues (`HitPolicyUnique.java:44-51` etc.).
    strict_mode: bool,
}

impl Default for DmnEngineBuilder {
    fn default() -> Self {
        Self { strict_mode: true }
    }
}

impl DmnEngineBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Java `DmnEngineConfiguration.setStrictMode` — default `true`.
    pub fn strict_mode(mut self, strict_mode: bool) -> Self {
        self.strict_mode = strict_mode;
        self
    }

    pub fn build_in_memory(self) -> Result<DmnEngine, DmnError> {
        DmnEngine::from_store(DmnStore::in_memory()?, self.strict_mode)
    }

    pub fn build_sqlite(self, path: impl AsRef<Path>) -> Result<DmnEngine, DmnError> {
        DmnEngine::from_store(DmnStore::sqlite(path)?, self.strict_mode)
    }

    /// Build against an explicitly configured backend (the production
    /// `FLOWABLE_DATABASE_URL` path — MySQL/Postgres as well as SQLite).
    pub fn build_from_database_config(
        self,
        config: DatabaseConfig,
    ) -> Result<DmnEngine, DmnError> {
        DmnEngine::from_store(DmnStore::from_config(config)?, self.strict_mode)
    }
}

impl DmnEngine {
    pub fn new_in_memory() -> Result<Self, DmnError> {
        Self::builder().build_in_memory()
    }

    pub fn new_sqlite(path: impl AsRef<Path>) -> Result<Self, DmnError> {
        Self::builder().build_sqlite(path)
    }

    /// Start a builder (default `strict_mode = true`, Java `:202`).
    pub fn builder() -> DmnEngineBuilder {
        DmnEngineBuilder::new()
    }

    pub fn repository_service(&self) -> DmnRepositoryService {
        self.repository_service.clone()
    }

    pub fn decision_service(&self) -> DmnDecisionService {
        self.decision_service.clone()
    }

    pub fn history_service(&self) -> DmnHistoryService {
        self.history_service.clone()
    }

    pub fn deploy(&self, request: DmnDeploymentRequest) -> Result<DmnDeployment, DmnError> {
        self.repository_service.deploy(request)
    }

    pub fn execute_by_key(
        &self,
        decision_key: &str,
        request: DmnExecutionRequest,
    ) -> Result<DmnExecutionResult, DmnError> {
        self.decision_service.execute_by_key(decision_key, request)
    }

    fn from_store(store: DmnStore, strict_mode: bool) -> Result<Self, DmnError> {
        let repository_service = DmnRepositoryService::new(store.clone());
        let decision_service =
            DmnDecisionService::new(store.clone(), repository_service.clone(), strict_mode);
        let history_service = DmnHistoryService::new(store);

        Ok(Self {
            repository_service,
            decision_service,
            history_service,
        })
    }
}
