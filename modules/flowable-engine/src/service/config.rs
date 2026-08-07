use crate::service::auth::{AuthConfig, RejectAllAuthProvider};
use crate::service::claim_mapping::ClaimMapping;
use crate::service::external_auth::ExternalAuthProvider;
use crate::service::issuer_profile::{
    ClaimMappingConfig as IpClaimMappingConfig, ClaimValidation, IssuerProfile, JwksRefreshPolicy,
    RoleMapping, RolloutState,
};
use crate::service::jwks::JwksCache;
use crate::service::principal::AuthProvider;
use crate::service::revocation::TokenRevocationRegistry;
use flowable_cmmn_engine::CmmnEngine;
use flowable_dmn_engine::DmnEngine;
use flowable_http_service::{
    AsyncHttpRuntime, AsyncHttpRuntimeConfig, DeterministicHttpRuntime, HttpRuntime,
    RealHttpClient, RealHttpClientConfig,
};
use flowable_mail_service::{
    DeterministicMailRuntime, MailRuntime, SmtpMailConfig, SmtpMailRuntime,
};
use flowable_persistence::{DatabaseConfig, DatabaseKind, SchemaMode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum JournalMode {
    #[serde(rename = "WAL")]
    #[default]
    Wal,
    #[serde(rename = "Delete")]
    Delete,
    #[serde(rename = "Truncate")]
    Truncate,
    #[serde(rename = "Persist")]
    Persist,
    #[serde(rename = "Memory")]
    Memory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AuthProviderKind {
    #[serde(rename = "local-static")]
    #[default]
    LocalStatic,
    #[serde(rename = "external")]
    External,
    #[serde(rename = "custom")]
    Custom,
}

/// AsString is not called; we just keep this enum for serde.
impl AuthProviderKind {
    pub fn is_external(&self) -> bool {
        matches!(self, Self::External)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessEngineConfiguration {
    #[serde(default)]
    pub enable_secure_scripting: bool,
    /// When false (default), shell service tasks refuse to execute OS commands.
    ///
    /// Security deviation from Java: Java `ShellActivityBehavior` is enabled by
    /// default — a known dangerous default that turns process deployment into RCE.
    /// Set `shell_tasks_enabled = true` explicitly to opt in.
    #[serde(default)]
    pub shell_tasks_enabled: bool,
    /// Java `ProcessEngineConfigurationImpl.enableEntityLinks` (default false).
    /// When true, call activities create parent→child entity links.
    #[serde(default)]
    pub enable_entity_links: bool,
    /// Java engine-wide `fallbackToDefaultTenant` used when a call activity's
    /// own `fallbackToDefaultTenant` is unset/false but the engine opts in.
    #[serde(default)]
    pub fallback_to_default_tenant: bool,
    /// Engine-local host methods exposed to UEL bean calls and `T(...)`
    /// static-type calls. The registry is skipped by serde because closures
    /// are runtime configuration, like listener and delegate registries.
    #[serde(skip)]
    pub expression_method_registry: crate::el::method_registry::ExpressionMethodRegistry,
    /// Engine-local named business calendars (Java `BusinessCalendarManager`).
    /// Seeded with `dueDate`, `duration`, and `cycle`; hosts add their own
    /// before engine construction. Skipped by serde like every other runtime
    /// registry — a raw `calendarName` is what gets persisted, never the
    /// resolved implementation (ADR-1 / ADR-2).
    #[serde(skip)]
    pub business_calendar_registry: crate::engine::business_calendar::BusinessCalendarRegistry,
    #[serde(default)]
    pub supported_script_languages: Vec<String>,
    #[serde(skip)]
    pub dmn_engine: Option<Arc<DmnEngine>>,
    /// Java `ProcessEngineConfiguration.alwaysUseArraysForDmnMultiHitPolicies`
    /// (default `true` — ProcessEngineConfiguration.java:133).
    /// When true, multi-hit DMN policies (RULE_ORDER / OUTPUT_ORDER / COLLECT
    /// without aggregator / Complete / Batch) write a JSON array even for a
    /// single matched rule (DmnActivityBehavior.java:153).
    #[serde(default = "default_true")]
    pub always_use_arrays_for_dmn_multi_hit_policies: bool,
    #[serde(skip)]
    pub cmmn_engine: Option<Arc<CmmnEngine>>,
    #[serde(default)]
    pub http_service: HttpServiceTaskConfiguration,
    #[serde(default)]
    pub mail_service: MailServiceTaskConfiguration,
    #[serde(default)]
    pub async_executor: AsyncExecutorConfiguration,
    #[serde(default)]
    pub async_history: AsyncHistoryConfiguration,
    /// Java-compatible immediate engine event dispatcher. Transaction-lifecycle
    /// listeners are added separately so immediate command semantics stay explicit.
    #[serde(skip)]
    pub engine_event_dispatcher: crate::engine::event_dispatcher::EngineEventDispatcher,
    /// Task 11: database connection pool configuration.
    #[serde(default)]
    pub database: DatabaseConfiguration,
    /// History level (NONE/INSTANCE/TASK/ACTIVITY/AUDIT/FULL).
    /// Java `ProcessEngineConfiguration.history` default is `"audit"`
    /// (`ProcessEngineConfiguration.java:88`); parsed via `HistoryLevel.getHistoryLevelForKey`
    /// (`ProcessEngineConfigurationImpl.java:2121-2122`).
    #[serde(default = "default_history_level")]
    pub history_level: HistoryLevel,
    /// When true, `flowable:historyLevel` on a process definition overrides the
    /// engine-level history setting for that definition's writes.
    /// Java `ProcessEngineConfiguration.enableProcessDefinitionHistoryLevel`
    /// (`ProcessEngineConfiguration.java:103`, default false).
    #[serde(default)]
    pub enable_process_definition_history_level: bool,
    /// Java `ProcessEngineConfiguration.enableHistoryCleaning`
    /// (`ProcessEngineConfiguration.java:149`, default false).
    /// When true, engine start ensures a single `bpmn-history-cleanup` timer job.
    #[serde(default)]
    pub enable_history_cleaning: bool,
    /// Java `ProcessEngineConfiguration.historyCleaningTimeCycleConfig`
    /// (`ProcessEngineConfiguration.java:150`, default `"0 0 1 * * ?"`).
    /// Quartz cron stored as the timer job's `time_cycle` / Java `repeat`.
    #[serde(default = "default_history_cleaning_time_cycle_config")]
    pub history_cleaning_time_cycle_config: String,
    /// Java `ProcessEngineConfiguration.cleanInstancesEndedAfter`
    /// (`ProcessEngineConfiguration.java:151`, default `Duration.ofDays(365)`).
    /// Historic process instances finished before `now - this` are cleanup candidates.
    #[serde(default = "default_clean_instances_ended_after")]
    pub clean_instances_ended_after: Duration,
    /// Java `ProcessEngineConfiguration.cleanInstancesBatchSize`
    /// (`ProcessEngineConfiguration.java:152`, default 100).
    /// Max historic process instances deleted per cleanup job fire.
    #[serde(default = "default_clean_instances_batch_size")]
    pub clean_instances_batch_size: u32,
    /// Optional local execution listener registry (seeded into each CommandContext).
    #[serde(skip)]
    pub execution_listener_registry:
        Option<crate::bpmn::listener::LocalExecutionListenerRegistry>,
    /// Optional local task listener registry (seeded into each CommandContext).
    #[serde(skip)]
    pub task_listener_registry: Option<crate::bpmn::listener::LocalTaskListenerRegistry>,
    /// Optional local service-task delegate registry (class / delegateExpression keys).
    #[serde(skip)]
    pub service_task_delegate_registry: Option<
        crate::bpmn::behavior::service_task_activity_behavior::LocalServiceTaskDelegateRegistry,
    >,
    /// Optional async-capable service-task delegate registry.
    #[serde(skip)]
    pub async_service_task_delegate_registry: Option<
        crate::bpmn::behavior::async_delegate_activity_behavior::AsyncLocalServiceTaskDelegateRegistry,
    >,
    /// Optional Java-compatible HTTP request/response handler registry.
    #[serde(skip)]
    pub http_handler_registry: Option<crate::bpmn::http_handler::HttpHandlerRegistry>,
    /// Shared pending-future registry used by WaitForFutureOperation and async delegates.
    #[serde(skip)]
    pub pending_future_registry:
        std::sync::Arc<crate::agenda::future_operations::PendingFutureRegistry>,
    /// Optional shared async task executor for async delegate submission.
    /// When absent, async delegate work runs synchronously on the calling thread.
    #[serde(skip)]
    pub future_task_executor: Option<
        std::sync::Arc<
            std::sync::Mutex<Option<crate::engine::async_task_executor::AsyncTaskExecutor>>,
        >,
    >,
    /// Optional command interceptors wrapping DefaultCommandExecutor.
    /// Empty by default; terminal executor is always the default command executor.
    #[serde(skip)]
    pub command_interceptors: Vec<crate::interceptor::CommandInterceptorHandle>,
    /// Java `ProcessEngineConfigurationImpl.createExternalWorkerJobInterceptor`.
    /// Invoked around external-worker service-task job creation (P54b S5).
    #[serde(skip)]
    pub create_external_worker_job_interceptor: Option<
        crate::engine::external_worker_service::CreateExternalWorkerJobInterceptorHandle,
    >,
    /// Shared activation coordinator (Java parity for active-async-executor job
    /// hints). `Arc`-backed and `Clone`-shared: cloning the configuration keeps
    /// the same live active flag / submit handle so the command executor and the
    /// async executor observe each other's runtime state. See
    /// [`crate::engine::activation_coordinator`].
    #[serde(skip)]
    pub activation_coordinator: crate::engine::activation_coordinator::ActivationCoordinator,
    /// BPMN `send-event` outbound dispatch hook (P94 / B-WP4). Service crate
    /// installs a configuration-backed transform+adapter pipeline after engine
    /// build; when empty the engine treats dispatch as in-memory no-op success.
    /// See [`crate::engine::outbound_event_dispatch`].
    #[serde(skip)]
    pub outbound_event_dispatch:
        crate::engine::outbound_event_dispatch::OutboundEventDispatchRegistry,
}

/// Task 11: configurable database connection pool.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabaseConfiguration {
    #[serde(default)]
    pub kind: EngineDatabaseKind,
    #[serde(default = "default_database_url")]
    pub url: String,
    #[serde(default = "default_db_pool_size")]
    pub pool_size: u32,
    #[serde(default = "default_db_busy_timeout_ms")]
    pub busy_timeout_ms: u32,
    #[serde(default)]
    pub journal_mode: JournalMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum EngineDatabaseKind {
    #[default]
    Memory,
    Sqlite,
    Postgres,
    Mysql,
}

impl From<EngineDatabaseKind> for DatabaseKind {
    fn from(kind: EngineDatabaseKind) -> Self {
        match kind {
            EngineDatabaseKind::Memory => DatabaseKind::Memory,
            EngineDatabaseKind::Sqlite => DatabaseKind::Sqlite,
            EngineDatabaseKind::Postgres => DatabaseKind::Postgres,
            EngineDatabaseKind::Mysql => DatabaseKind::Mysql,
        }
    }
}

fn default_database_url() -> String {
    ":memory:".to_string()
}

fn default_db_pool_size() -> u32 {
    8
}

fn default_db_busy_timeout_ms() -> u32 {
    5000
}

impl Default for DatabaseConfiguration {
    fn default() -> Self {
        if let Ok(url) = std::env::var("FLOWABLE_TEST_ENGINE_DATABASE_URL") {
            let kind = if url.starts_with("postgres://") {
                EngineDatabaseKind::Postgres
            } else if url.starts_with("mysql://") {
                EngineDatabaseKind::Mysql
            } else if url == ":memory:" || url.is_empty() {
                EngineDatabaseKind::Memory
            } else {
                EngineDatabaseKind::Sqlite
            };
            return Self {
                kind,
                url,
                pool_size: default_db_pool_size(),
                busy_timeout_ms: default_db_busy_timeout_ms(),
                journal_mode: JournalMode::default(),
            };
        }
        Self {
            kind: EngineDatabaseKind::default(),
            url: default_database_url(),
            pool_size: default_db_pool_size(),
            busy_timeout_ms: default_db_busy_timeout_ms(),
            journal_mode: JournalMode::default(),
        }
    }
}

impl DatabaseConfiguration {
    pub fn to_persistence_config(&self) -> DatabaseConfig {
        DatabaseConfig {
            kind: self.kind.into(),
            url: self.url.clone(),
            pool_size: self.pool_size,
            schema_mode: SchemaMode::True,
            table_prefix: None,
            schema: None,
            catalog: None,
        }
    }
}

/// Task 12: history recording level.
/// - `Full`: record everything (variable history, task events, audit logs) — current behavior
/// - `Audit`: record process/task instances + audit logs, skip task events
/// - `None`: skip all history recording
/// Java `org.flowable.common.engine.impl.history.HistoryLevel`
/// (`HistoryLevel.java:26`): declaration order is the `isAtLeast` rank
/// (`HistoryLevel.java:60-63` uses enum `compareTo`).
///
/// Order: NONE < INSTANCE < TASK < ACTIVITY < AUDIT < FULL.
/// Note: there is no VARIABLE level in OSS Flowable (common misremembering).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "UPPERCASE")]
#[derive(Default)]
pub enum HistoryLevel {
    None,
    Instance,
    Task,
    Activity,
    #[default]
    Audit,
    Full,
}

impl HistoryLevel {
    /// Java key strings (`HistoryLevel.java:26`).
    pub fn key(self) -> &'static str {
        match self {
            HistoryLevel::None => "none",
            HistoryLevel::Instance => "instance",
            HistoryLevel::Task => "task",
            HistoryLevel::Activity => "activity",
            HistoryLevel::Audit => "audit",
            HistoryLevel::Full => "full",
        }
    }

    /// Java `HistoryLevel.getHistoryLevelForKey` (`HistoryLevel.java:41-48`):
    /// case-insensitive match on key; unknown values are illegal.
    pub fn parse(key: &str) -> Result<HistoryLevel, String> {
        let trimmed = key.trim();
        for level in Self::ALL {
            if level.key().eq_ignore_ascii_case(trimmed) {
                return Ok(level);
            }
        }
        Err(format!("Illegal value for history-level: {key}"))
    }

    /// True when this level is the same as, or higher in declaration order than
    /// `other`. Java `HistoryLevel.isAtLeast` (`HistoryLevel.java:60-63`).
    pub fn is_at_least(self, other: HistoryLevel) -> bool {
        self as u8 >= other as u8
    }

    /// All levels in Java declaration order.
    pub const ALL: [HistoryLevel; 6] = [
        HistoryLevel::None,
        HistoryLevel::Instance,
        HistoryLevel::Task,
        HistoryLevel::Activity,
        HistoryLevel::Audit,
        HistoryLevel::Full,
    ];
}

fn default_history_level() -> HistoryLevel {
    // Java ProcessEngineConfiguration.history = HistoryLevel.AUDIT.getKey()
    // (ProcessEngineConfiguration.java:88).
    HistoryLevel::Audit
}

fn default_pool_size() -> usize {
    8
}

fn default_queue_size() -> usize {
    2048
}

fn default_async_job_acquire_wait_ms() -> u64 {
    10_000
}

fn default_timer_job_acquire_wait_ms() -> u64 {
    10_000
}

fn default_queue_full_wait_ms() -> u64 {
    5_000
}

fn default_max_jobs_per_acquisition() -> usize {
    512
}

fn default_async_job_lock_time_ms() -> u64 {
    3_600_000
}

fn default_async_executor_number_of_retries() -> i32 {
    3
}

fn default_async_history_executor_number_of_retries() -> i32 {
    // Java ProcessEngineConfigurationImpl.asyncHistoryExecutorNumberOfRetries
    10
}

fn default_async_failed_job_wait_time_ms() -> u64 {
    10_000
}

fn default_timer_lock_time_ms() -> u64 {
    3_600_000
}

fn default_reset_expired_interval_ms() -> u64 {
    60_000
}

fn default_reset_expired_page_size() -> usize {
    3
}

fn default_global_acquire_lock_wait_ms() -> u64 {
    60_000
}

fn default_global_acquire_lock_poll_rate_ms() -> u64 {
    500
}

fn default_global_acquire_lock_lease_ms() -> u64 {
    600_000
}

const GLOBAL_LOCK_NAME_MAX_LENGTH: usize = 64;

fn build_global_lock_name(prefix: &str, base_lock_name: &str) -> String {
    let full = format!("{}{}", prefix, base_lock_name);
    if full.len() > GLOBAL_LOCK_NAME_MAX_LENGTH {
        full.chars().take(GLOBAL_LOCK_NAME_MAX_LENGTH).collect()
    } else {
        full
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsyncExecutorTopology {
    #[default]
    Standard,
    SharedMultiTenant,
    ExecutorPerTenant,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsyncExecutorTenantScope {
    #[default]
    All,
    Tenants(Vec<String>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AsyncExecutorConfiguration {
    #[serde(default)]
    pub enabled: bool,
    /// Internal executor topology. Standard preserves the existing single
    /// executor behavior and remains the default for serialized configurations.
    #[serde(default)]
    pub topology: AsyncExecutorTopology,
    /// Starts the executor as part of the process-engine build lifecycle.
    /// This is independent from `enabled` so Java-style automatic activation
    /// can be opted into without changing the legacy manual-start flag.
    #[serde(default)]
    pub auto_activate: bool,
    /// Stable owner used for runtime job locks and global acquisition locks.
    /// When absent, the process engine resolves `<engine-name>:<uuid>`.
    #[serde(default)]
    pub lock_owner: Option<String>,
    /// Whether lifecycle start/shutdown releases executable jobs locked by this
    /// executor owner. Flowable Java enables this independently from automatic
    /// executor activation.
    #[serde(default = "default_true")]
    pub unlock_owned_jobs: bool,
    /// Java-compatible initial retry count for newly created async jobs.
    /// A BPMN `failedJobRetryTimeCycle` is applied on the first failure.
    #[serde(default = "default_async_executor_number_of_retries")]
    pub number_of_retries: i32,
    /// Delay before retrying a failed async job when the BPMN activity has no
    /// `failedJobRetryTimeCycle`. Flowable Java defaults this to ten seconds.
    #[serde(default = "default_async_failed_job_wait_time_ms")]
    pub async_failed_job_wait_time_ms: u64,
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,
    #[serde(default = "default_queue_size")]
    pub queue_size: usize,
    #[serde(default = "default_async_job_acquire_wait_ms")]
    pub async_job_acquire_wait_ms: u64,
    #[serde(default = "default_timer_job_acquire_wait_ms")]
    pub timer_job_acquire_wait_ms: u64,
    #[serde(default = "default_queue_full_wait_ms")]
    pub queue_full_wait_ms: u64,
    #[serde(default = "default_max_jobs_per_acquisition")]
    pub max_jobs_per_acquisition: usize,
    /// Java-compatible override for executable async/history acquisition.
    /// When absent, `max_jobs_per_acquisition` remains the additive Rust fallback.
    #[serde(default)]
    pub max_async_jobs_due_per_acquisition: Option<usize>,
    /// Java-compatible override for scheduled timer acquisition.
    /// When absent, `max_jobs_per_acquisition` remains the additive Rust fallback.
    #[serde(default)]
    pub max_timer_jobs_per_acquisition: Option<usize>,
    #[serde(default = "default_async_job_lock_time_ms")]
    pub async_job_lock_time_ms: u64,
    #[serde(default = "default_timer_lock_time_ms")]
    pub timer_lock_time_ms: u64,
    #[serde(default = "default_reset_expired_interval_ms")]
    pub reset_expired_jobs_interval_ms: u64,
    #[serde(default = "default_reset_expired_page_size")]
    pub reset_expired_jobs_page_size: usize,
    #[serde(default = "default_true")]
    pub async_job_acquisition_enabled: bool,
    #[serde(default = "default_true")]
    pub timer_job_acquisition_enabled: bool,
    #[serde(default = "default_true")]
    pub reset_expired_job_enabled: bool,
    #[serde(default)]
    pub global_acquire_lock_enabled: bool,
    #[serde(default)]
    pub global_acquire_lock_prefix: String,
    #[serde(default = "default_global_acquire_lock_wait_ms")]
    pub global_acquire_lock_wait_ms: u64,
    #[serde(default = "default_global_acquire_lock_poll_rate_ms")]
    pub global_acquire_lock_poll_rate_ms: u64,
    #[serde(default = "default_global_acquire_lock_lease_ms")]
    pub global_acquire_lock_lease_ms: u64,
    #[serde(default)]
    pub async_jobs_global_lock_wait_ms: Option<u64>,
    #[serde(default)]
    pub async_jobs_global_lock_poll_rate_ms: Option<u64>,
    #[serde(default)]
    pub async_jobs_global_lock_force_acquire_after_ms: Option<u64>,
    #[serde(default)]
    pub timer_global_lock_wait_ms: Option<u64>,
    #[serde(default)]
    pub timer_global_lock_poll_rate_ms: Option<u64>,
    #[serde(default)]
    pub timer_global_lock_force_acquire_after_ms: Option<u64>,
    /// When non-empty, only acquire jobs whose process instance `tenant_id` is in
    /// this list. Empty means all tenants (shared acquisition — the default).
    /// Use `""` to also match process instances with no tenant.
    #[serde(default)]
    pub tenant_ids: Vec<String>,
    /// When non-empty, only acquire jobs whose category is in this list.
    /// Jobs with NULL category are excluded when filtering is active, matching
    /// Java's AsyncExecutor enabledJobCategories semantics.
    /// This list applies to both async and timer job acquisition.
    /// Empty = no category filtering (default — acquires all job categories).
    #[serde(default)]
    pub enabled_job_categories: Vec<String>,
}

impl AsyncExecutorConfiguration {
    pub fn tenant_scope(&self) -> AsyncExecutorTenantScope {
        if self.tenant_ids.is_empty() {
            AsyncExecutorTenantScope::All
        } else {
            AsyncExecutorTenantScope::Tenants(self.tenant_ids.clone())
        }
    }

    pub fn with_tenant_scope(mut self, tenant_scope: AsyncExecutorTenantScope) -> Self {
        self.tenant_ids = match tenant_scope {
            AsyncExecutorTenantScope::All => Vec::new(),
            AsyncExecutorTenantScope::Tenants(tenant_ids) => tenant_ids,
        };
        self
    }

    pub fn shared_multi_tenant(mut self) -> Self {
        self.topology = AsyncExecutorTopology::SharedMultiTenant;
        self.unlock_owned_jobs = false;
        self
    }

    pub fn executor_per_tenant(mut self) -> Self {
        self.topology = AsyncExecutorTopology::ExecutorPerTenant;
        self
    }

    pub fn unlocks_owned_jobs_on_start(&self) -> bool {
        self.unlock_owned_jobs && !matches!(self.topology, AsyncExecutorTopology::SharedMultiTenant)
    }

    pub fn unlocks_owned_jobs_on_shutdown(&self) -> bool {
        self.unlock_owned_jobs
    }

    pub fn effective_max_async_jobs_due_per_acquisition(&self) -> usize {
        self.max_async_jobs_due_per_acquisition
            .unwrap_or(self.max_jobs_per_acquisition)
    }

    pub fn effective_max_timer_jobs_per_acquisition(&self) -> usize {
        self.max_timer_jobs_per_acquisition
            .unwrap_or(self.max_jobs_per_acquisition)
    }

    pub fn effective_async_jobs_global_lock_wait_ms(&self) -> u64 {
        self.async_jobs_global_lock_wait_ms
            .unwrap_or(self.global_acquire_lock_wait_ms)
    }

    pub fn effective_async_jobs_global_lock_poll_rate_ms(&self) -> u64 {
        self.async_jobs_global_lock_poll_rate_ms
            .unwrap_or(self.global_acquire_lock_poll_rate_ms)
    }

    pub fn effective_async_jobs_global_lock_force_acquire_after_ms(&self) -> u64 {
        self.async_jobs_global_lock_force_acquire_after_ms
            .unwrap_or(self.global_acquire_lock_lease_ms)
    }

    pub fn effective_timer_global_lock_wait_ms(&self) -> u64 {
        self.timer_global_lock_wait_ms
            .unwrap_or(self.global_acquire_lock_wait_ms)
    }

    pub fn effective_timer_global_lock_poll_rate_ms(&self) -> u64 {
        self.timer_global_lock_poll_rate_ms
            .unwrap_or(self.global_acquire_lock_poll_rate_ms)
    }

    pub fn effective_timer_global_lock_force_acquire_after_ms(&self) -> u64 {
        self.timer_global_lock_force_acquire_after_ms
            .unwrap_or(self.global_acquire_lock_lease_ms)
    }

    pub fn global_lock_name_for(&self, base_lock_name: &str) -> String {
        build_global_lock_name(&self.global_acquire_lock_prefix, base_lock_name)
    }
}

impl Default for AsyncExecutorConfiguration {
    fn default() -> Self {
        Self {
            enabled: false,
            topology: AsyncExecutorTopology::Standard,
            auto_activate: false,
            lock_owner: None,
            unlock_owned_jobs: true,
            number_of_retries: default_async_executor_number_of_retries(),
            async_failed_job_wait_time_ms: default_async_failed_job_wait_time_ms(),
            pool_size: default_pool_size(),
            queue_size: default_queue_size(),
            async_job_acquire_wait_ms: default_async_job_acquire_wait_ms(),
            timer_job_acquire_wait_ms: default_timer_job_acquire_wait_ms(),
            queue_full_wait_ms: default_queue_full_wait_ms(),
            max_jobs_per_acquisition: default_max_jobs_per_acquisition(),
            max_async_jobs_due_per_acquisition: None,
            max_timer_jobs_per_acquisition: None,
            async_job_lock_time_ms: default_async_job_lock_time_ms(),
            timer_lock_time_ms: default_timer_lock_time_ms(),
            reset_expired_jobs_interval_ms: default_reset_expired_interval_ms(),
            reset_expired_jobs_page_size: default_reset_expired_page_size(),
            async_job_acquisition_enabled: true,
            timer_job_acquisition_enabled: true,
            reset_expired_job_enabled: true,
            global_acquire_lock_enabled: false,
            global_acquire_lock_prefix: String::new(),
            global_acquire_lock_wait_ms: default_global_acquire_lock_wait_ms(),
            global_acquire_lock_poll_rate_ms: default_global_acquire_lock_poll_rate_ms(),
            global_acquire_lock_lease_ms: default_global_acquire_lock_lease_ms(),
            async_jobs_global_lock_wait_ms: None,
            async_jobs_global_lock_poll_rate_ms: None,
            async_jobs_global_lock_force_acquire_after_ms: None,
            timer_global_lock_wait_ms: None,
            timer_global_lock_poll_rate_ms: None,
            timer_global_lock_force_acquire_after_ms: None,
            tenant_ids: Vec::new(),
            enabled_job_categories: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AsyncExecutorConfiguration, AsyncExecutorTenantScope, AsyncExecutorTopology};
    use crate::engine::lock_manager::{
        ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK, ACQUIRE_TIMER_JOBS_GLOBAL_LOCK,
    };

    #[test]
    fn async_executor_lifecycle_extensions_preserve_legacy_defaults() {
        let config = AsyncExecutorConfiguration::default();

        assert!(!config.enabled);
        assert_eq!(config.topology, AsyncExecutorTopology::Standard);
        assert_eq!(config.tenant_scope(), AsyncExecutorTenantScope::All);
        assert!(!config.auto_activate);
        assert!(config.lock_owner.is_none());
        assert!(config.unlock_owned_jobs);
        assert!(config.unlocks_owned_jobs_on_start());
        assert!(config.unlocks_owned_jobs_on_shutdown());
        assert_eq!(config.async_job_lock_time_ms, 3_600_000);
        assert_eq!(config.timer_lock_time_ms, 3_600_000);
        assert_eq!(
            config.effective_max_async_jobs_due_per_acquisition(),
            config.max_jobs_per_acquisition
        );
        assert_eq!(
            config.effective_max_timer_jobs_per_acquisition(),
            config.max_jobs_per_acquisition
        );

        let deserialized: AsyncExecutorConfiguration =
            serde_json::from_str("{}").expect("deserialize default async executor config");
        assert_eq!(deserialized.topology, AsyncExecutorTopology::Standard);
        assert_eq!(deserialized.tenant_scope(), AsyncExecutorTenantScope::All);
        assert!(!deserialized.auto_activate);
        assert!(deserialized.lock_owner.is_none());
        assert!(deserialized.unlock_owned_jobs);
        assert_eq!(deserialized.async_job_lock_time_ms, 3_600_000);
        assert_eq!(deserialized.timer_lock_time_ms, 3_600_000);
        assert!(deserialized.max_async_jobs_due_per_acquisition.is_none());
        assert!(deserialized.max_timer_jobs_per_acquisition.is_none());
        assert_eq!(deserialized.global_acquire_lock_prefix, "");
        assert!(deserialized.async_jobs_global_lock_wait_ms.is_none());
        assert!(deserialized.async_jobs_global_lock_poll_rate_ms.is_none());
        assert!(
            deserialized
                .async_jobs_global_lock_force_acquire_after_ms
                .is_none()
        );
        assert!(deserialized.timer_global_lock_wait_ms.is_none());
        assert!(deserialized.timer_global_lock_poll_rate_ms.is_none());
        assert!(
            deserialized
                .timer_global_lock_force_acquire_after_ms
                .is_none()
        );
    }

    #[test]
    fn async_and_timer_acquisition_limits_override_the_shared_fallback_independently() {
        let config: AsyncExecutorConfiguration = serde_json::from_str(
            r#"{
                "max_jobs_per_acquisition": 9,
                "max_async_jobs_due_per_acquisition": 2,
                "max_timer_jobs_per_acquisition": 3
            }"#,
        )
        .expect("deserialize independent acquisition limits");

        assert_eq!(config.effective_max_async_jobs_due_per_acquisition(), 2);
        assert_eq!(config.effective_max_timer_jobs_per_acquisition(), 3);
    }

    #[test]
    fn async_executor_topology_uses_stable_snake_case_serde_names() {
        assert_eq!(
            serde_json::to_string(&AsyncExecutorTopology::Standard).unwrap(),
            r#""standard""#
        );
        assert_eq!(
            serde_json::to_string(&AsyncExecutorTopology::SharedMultiTenant).unwrap(),
            r#""shared_multi_tenant""#
        );
        assert_eq!(
            serde_json::to_string(&AsyncExecutorTopology::ExecutorPerTenant).unwrap(),
            r#""executor_per_tenant""#
        );
        assert_eq!(
            serde_json::to_string(&AsyncExecutorTenantScope::All).unwrap(),
            r#""all""#
        );
        assert_eq!(
            serde_json::to_value(AsyncExecutorTenantScope::Tenants(vec![
                "tenant-a".to_string(),
            ]))
            .unwrap(),
            serde_json::json!({ "tenants": ["tenant-a"] })
        );

        let config: AsyncExecutorConfiguration = serde_json::from_str(
            r#"{
                "topology": "shared_multi_tenant",
                "tenant_ids": ["tenant-a", ""]
            }"#,
        )
        .expect("deserialize shared multi-tenant topology");
        assert_eq!(config.topology, AsyncExecutorTopology::SharedMultiTenant);
        assert_eq!(
            config.tenant_scope(),
            AsyncExecutorTenantScope::Tenants(vec!["tenant-a".to_string(), "".to_string()])
        );
    }

    #[test]
    fn tenant_scope_builders_preserve_existing_tenant_ids_storage() {
        let tenant_ids = vec!["tenant-a".to_string(), "".to_string()];
        let config = AsyncExecutorConfiguration::default()
            .with_tenant_scope(AsyncExecutorTenantScope::Tenants(tenant_ids.clone()))
            .shared_multi_tenant();

        assert_eq!(config.tenant_ids, tenant_ids);
        assert_eq!(config.topology, AsyncExecutorTopology::SharedMultiTenant);
        assert!(!config.unlock_owned_jobs);
        assert_eq!(
            config.tenant_scope(),
            AsyncExecutorTenantScope::Tenants(config.tenant_ids.clone())
        );

        let all_tenants = config.with_tenant_scope(AsyncExecutorTenantScope::All);
        assert!(all_tenants.tenant_ids.is_empty());
        assert_eq!(all_tenants.tenant_scope(), AsyncExecutorTenantScope::All);

        let per_tenant = AsyncExecutorConfiguration::default().executor_per_tenant();
        assert_eq!(
            per_tenant.topology,
            AsyncExecutorTopology::ExecutorPerTenant
        );
    }

    #[test]
    fn shared_multi_tenant_explicit_unlock_is_shutdown_only() {
        let mut shared = AsyncExecutorConfiguration::default().shared_multi_tenant();
        shared.unlock_owned_jobs = true;

        assert!(!shared.unlocks_owned_jobs_on_start());
        assert!(shared.unlocks_owned_jobs_on_shutdown());

        shared.unlock_owned_jobs = false;
        assert!(!shared.unlocks_owned_jobs_on_start());
        assert!(!shared.unlocks_owned_jobs_on_shutdown());

        let per_tenant = AsyncExecutorConfiguration::default().executor_per_tenant();
        assert!(per_tenant.unlocks_owned_jobs_on_start());
        assert!(per_tenant.unlocks_owned_jobs_on_shutdown());
    }

    #[test]
    fn global_lock_name_with_empty_prefix_matches_java_default() {
        let config = AsyncExecutorConfiguration::default();
        assert_eq!(
            config.global_lock_name_for(ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK),
            ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK
        );
        assert_eq!(
            config.global_lock_name_for(ACQUIRE_TIMER_JOBS_GLOBAL_LOCK),
            ACQUIRE_TIMER_JOBS_GLOBAL_LOCK
        );
    }

    #[test]
    fn global_lock_name_with_short_prefix_prepends_directly() {
        let mut config = AsyncExecutorConfiguration::default();
        config.global_acquire_lock_prefix = "engine1-".to_string();
        assert_eq!(
            config.global_lock_name_for(ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK),
            "engine1-acquireAsyncJobsLock"
        );
        assert_eq!(
            config.global_lock_name_for(ACQUIRE_TIMER_JOBS_GLOBAL_LOCK),
            "engine1-acquireTimerJobsLock"
        );
    }

    #[test]
    fn global_lock_name_is_truncated_to_sixty_four_characters() {
        let mut config = AsyncExecutorConfiguration::default();
        let long_prefix = "abcdefghijklmnopqrstuvwxyz-abcdefghijklmnopqrstuvwxyz-abcdefghijk";
        assert!(long_prefix.len() > 64);
        config.global_acquire_lock_prefix = long_prefix.to_string();
        let async_name = config.global_lock_name_for(ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK);
        assert_eq!(async_name.len(), 64);
        assert!(async_name.starts_with("abcdefghijklmnopqrstuvwxyz-abcdefghijklmnopqrstuvwxyz-"));
        let timer_name = config.global_lock_name_for(ACQUIRE_TIMER_JOBS_GLOBAL_LOCK);
        assert_eq!(timer_name.len(), 64);
    }

    #[test]
    fn prefix_plus_base_exactly_sixty_four_chars_is_not_truncated() {
        let mut config = AsyncExecutorConfiguration::default();
        let prefix_44 = "a".repeat(44);
        config.global_acquire_lock_prefix = prefix_44;
        let async_name = config.global_lock_name_for(ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK);
        assert_eq!(async_name.len(), 64);
    }

    #[test]
    fn effective_async_global_lock_timing_falls_back_to_shared_defaults() {
        let config = AsyncExecutorConfiguration::default();
        assert_eq!(config.effective_async_jobs_global_lock_wait_ms(), 60_000);
        assert_eq!(config.effective_async_jobs_global_lock_poll_rate_ms(), 500);
        assert_eq!(
            config.effective_async_jobs_global_lock_force_acquire_after_ms(),
            600_000
        );
    }

    #[test]
    fn effective_timer_global_lock_timing_falls_back_to_shared_defaults() {
        let config = AsyncExecutorConfiguration::default();
        assert_eq!(config.effective_timer_global_lock_wait_ms(), 60_000);
        assert_eq!(config.effective_timer_global_lock_poll_rate_ms(), 500);
        assert_eq!(
            config.effective_timer_global_lock_force_acquire_after_ms(),
            600_000
        );
    }

    #[test]
    fn async_global_lock_overrides_apply_independently_from_timer() {
        let mut config = AsyncExecutorConfiguration::default();
        config.global_acquire_lock_wait_ms = 99_000;
        config.global_acquire_lock_poll_rate_ms = 777;
        config.global_acquire_lock_lease_ms = 999_000;
        config.async_jobs_global_lock_wait_ms = Some(30_000);
        config.async_jobs_global_lock_poll_rate_ms = Some(250);
        config.async_jobs_global_lock_force_acquire_after_ms = Some(120_000);

        assert_eq!(config.effective_async_jobs_global_lock_wait_ms(), 30_000);
        assert_eq!(config.effective_async_jobs_global_lock_poll_rate_ms(), 250);
        assert_eq!(
            config.effective_async_jobs_global_lock_force_acquire_after_ms(),
            120_000
        );
        assert_eq!(config.effective_timer_global_lock_wait_ms(), 99_000);
        assert_eq!(config.effective_timer_global_lock_poll_rate_ms(), 777);
        assert_eq!(
            config.effective_timer_global_lock_force_acquire_after_ms(),
            999_000
        );
    }

    #[test]
    fn timer_global_lock_overrides_apply_independently_from_async() {
        let mut config = AsyncExecutorConfiguration::default();
        config.timer_global_lock_wait_ms = Some(15_000);
        config.timer_global_lock_poll_rate_ms = Some(100);
        config.timer_global_lock_force_acquire_after_ms = Some(45_000);

        assert_eq!(config.effective_timer_global_lock_wait_ms(), 15_000);
        assert_eq!(config.effective_timer_global_lock_poll_rate_ms(), 100);
        assert_eq!(
            config.effective_timer_global_lock_force_acquire_after_ms(),
            45_000
        );
        assert_eq!(config.effective_async_jobs_global_lock_wait_ms(), 60_000);
        assert_eq!(config.effective_async_jobs_global_lock_poll_rate_ms(), 500);
        assert_eq!(
            config.effective_async_jobs_global_lock_force_acquire_after_ms(),
            600_000
        );
    }

    #[test]
    fn serde_empty_object_preserves_legacy_global_lock_defaults() {
        let deserialized: AsyncExecutorConfiguration =
            serde_json::from_str("{}").expect("deserialize defaults");
        assert!(!deserialized.global_acquire_lock_enabled);
        assert_eq!(deserialized.global_acquire_lock_prefix, "");
        assert_eq!(deserialized.global_acquire_lock_wait_ms, 60_000);
        assert_eq!(deserialized.global_acquire_lock_poll_rate_ms, 500);
        assert_eq!(deserialized.global_acquire_lock_lease_ms, 600_000);
        assert_eq!(
            deserialized.effective_async_jobs_global_lock_wait_ms(),
            deserialized.global_acquire_lock_wait_ms
        );
        assert_eq!(
            deserialized.effective_timer_global_lock_wait_ms(),
            deserialized.global_acquire_lock_wait_ms
        );
    }

    #[test]
    fn serde_prefix_and_independent_timing_parse_and_apply() {
        let config: AsyncExecutorConfiguration = serde_json::from_str(
            r#"{
                "global_acquire_lock_enabled": true,
                "global_acquire_lock_prefix": "prod-",
                "global_acquire_lock_wait_ms": 70000,
                "async_jobs_global_lock_wait_ms": 20000,
                "async_jobs_global_lock_poll_rate_ms": 300,
                "timer_global_lock_force_acquire_after_ms": 300000
            }"#,
        )
        .expect("deserialize with overrides");
        assert!(config.global_acquire_lock_enabled);
        assert_eq!(config.global_acquire_lock_prefix, "prod-");
        assert_eq!(
            config.global_lock_name_for(ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK),
            "prod-acquireAsyncJobsLock"
        );
        assert_eq!(config.effective_async_jobs_global_lock_wait_ms(), 20_000);
        assert_eq!(config.effective_async_jobs_global_lock_poll_rate_ms(), 300);
        assert_eq!(
            config.effective_async_jobs_global_lock_force_acquire_after_ms(),
            600_000
        );
        assert_eq!(config.effective_timer_global_lock_wait_ms(), 70_000);
        assert_eq!(config.effective_timer_global_lock_poll_rate_ms(), 500);
        assert_eq!(
            config.effective_timer_global_lock_force_acquire_after_ms(),
            300_000
        );
    }
}

fn default_history_cleaning_time_cycle_config() -> String {
    // ProcessEngineConfiguration.java:150
    "0 0 1 * * ?".to_string()
}

fn default_clean_instances_ended_after() -> Duration {
    // ProcessEngineConfiguration.java:151 — Duration.ofDays(365)
    Duration::from_secs(365 * 24 * 60 * 60)
}

fn default_clean_instances_batch_size() -> u32 {
    // ProcessEngineConfiguration.java:152
    100
}

impl Default for ProcessEngineConfiguration {
    fn default() -> Self {
        Self {
            enable_secure_scripting: false,
            // Security deviation from Java: shell tasks off by default.
            shell_tasks_enabled: false,
            enable_entity_links: false,
            fallback_to_default_tenant: false,
            expression_method_registry:
                crate::el::method_registry::ExpressionMethodRegistry::default(),
            business_calendar_registry:
                crate::engine::business_calendar::BusinessCalendarRegistry::default(),
            supported_script_languages: vec!["javascript".to_string()],
            dmn_engine: DmnEngine::new_in_memory().ok().map(Arc::new),
            always_use_arrays_for_dmn_multi_hit_policies: true,
            cmmn_engine: CmmnEngine::new_in_memory().ok().map(Arc::new),
            http_service: HttpServiceTaskConfiguration::default(),
            mail_service: MailServiceTaskConfiguration::default(),
            async_executor: AsyncExecutorConfiguration::default(),
            async_history: AsyncHistoryConfiguration::default(),
            engine_event_dispatcher:
                crate::engine::event_dispatcher::EngineEventDispatcher::default(),
            database: DatabaseConfiguration::default(),
            history_level: HistoryLevel::default(),
            enable_process_definition_history_level: false,
            enable_history_cleaning: false,
            history_cleaning_time_cycle_config: default_history_cleaning_time_cycle_config(),
            clean_instances_ended_after: default_clean_instances_ended_after(),
            clean_instances_batch_size: default_clean_instances_batch_size(),
            execution_listener_registry: None,
            task_listener_registry: None,
            service_task_delegate_registry: None,
            async_service_task_delegate_registry: None,
            http_handler_registry: None,
            pending_future_registry: std::sync::Arc::new(
                crate::agenda::future_operations::PendingFutureRegistry::new(),
            ),
            future_task_executor: None,
            command_interceptors: Vec::new(),
            create_external_worker_job_interceptor: None,
            activation_coordinator:
                crate::engine::activation_coordinator::ActivationCoordinator::new(),
            outbound_event_dispatch:
                crate::engine::outbound_event_dispatch::OutboundEventDispatchRegistry::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AsyncHistoryConfiguration {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_history_job_handler_type")]
    pub handler_type: String,
    /// Java `asyncHistoryExecutorNumberOfRetries` (default 10).
    /// Used for newly created history jobs and `moveToHistoryJob`.
    #[serde(default = "default_async_history_executor_number_of_retries")]
    pub number_of_retries: i32,
    /// When false, uses the shared `AsyncTaskExecutor` pool.
    /// When true, creates an independent executor with its own pool + acquisition.
    #[serde(default = "default_true")]
    pub use_shared_executor: bool,
    /// Pool size for the independent history executor (only when use_shared_executor = false).
    #[serde(default = "default_history_pool_size")]
    pub pool_size: usize,
    /// Queue size for the independent history executor.
    #[serde(default = "default_history_queue_size")]
    pub queue_size: usize,
    /// Acquisition interval in milliseconds for the independent executor.
    #[serde(default = "default_history_acquire_interval_ms")]
    pub acquire_interval_ms: u64,
    /// Optional override for independent history expired-lock reset.
    /// `None` inherits the history-executor default (`true` when independent).
    #[serde(default)]
    pub reset_expired_job_enabled: Option<bool>,
    /// Optional override for independent history reset interval milliseconds.
    /// `None` inherits the engine default of 60_000.
    #[serde(default)]
    pub reset_expired_jobs_interval_ms: Option<u64>,
    /// Optional override for independent history reset page size.
    /// `None` inherits the engine default of 3.
    #[serde(default)]
    pub reset_expired_jobs_page_size: Option<usize>,
}

impl AsyncHistoryConfiguration {
    pub fn resolved_reset_expired_job_enabled(&self) -> bool {
        self.reset_expired_job_enabled.unwrap_or(true)
    }

    pub fn resolved_reset_expired_jobs_interval_ms(&self) -> u64 {
        self.reset_expired_jobs_interval_ms
            .unwrap_or_else(default_reset_expired_interval_ms)
    }

    pub fn resolved_reset_expired_jobs_page_size(&self) -> usize {
        self.reset_expired_jobs_page_size
            .unwrap_or_else(default_reset_expired_page_size)
    }
}

fn default_history_job_handler_type() -> String {
    "default-history".to_string()
}

fn default_history_pool_size() -> usize {
    2
}

fn default_history_queue_size() -> usize {
    256
}

fn default_history_acquire_interval_ms() -> u64 {
    5_000
}

impl Default for AsyncHistoryConfiguration {
    fn default() -> Self {
        Self {
            enabled: false,
            handler_type: default_history_job_handler_type(),
            number_of_retries: default_async_history_executor_number_of_retries(),
            use_shared_executor: true,
            pool_size: default_history_pool_size(),
            queue_size: default_history_queue_size(),
            acquire_interval_ms: default_history_acquire_interval_ms(),
            reset_expired_job_enabled: None,
            reset_expired_jobs_interval_ms: None,
            reset_expired_jobs_page_size: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HttpServiceTaskConfiguration {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_http_supported_methods")]
    pub supported_methods: Vec<String>,
    #[serde(default)]
    pub runtime_mode: HttpServiceRuntimeMode,
    #[serde(default)]
    pub real_client: RealHttpClientConfiguration,
    /// Worker pool size when [`HttpServiceRuntimeMode::Async`] is selected.
    #[serde(default = "default_http_async_pool_size")]
    pub async_pool_size: usize,
}

impl Default for HttpServiceTaskConfiguration {
    fn default() -> Self {
        Self {
            enabled: true,
            supported_methods: default_http_supported_methods(),
            runtime_mode: HttpServiceRuntimeMode::default(),
            real_client: RealHttpClientConfiguration::default(),
            async_pool_size: default_http_async_pool_size(),
        }
    }
}

impl HttpServiceTaskConfiguration {
    fn real_client_config(&self) -> RealHttpClientConfig {
        RealHttpClientConfig {
            default_timeout_ms: self.real_client.default_timeout_ms,
            default_connect_timeout_ms: self.real_client.default_connect_timeout_ms,
            user_agent: self.real_client.user_agent.clone(),
            retry_count: self.real_client.retry_count,
            retry_backoff_ms: self.real_client.retry_backoff_ms,
            cache_enabled: self.real_client.cache_enabled,
            cache_ttl_ms: self.real_client.cache_ttl_ms,
            circuit_breaker_threshold: self.real_client.circuit_breaker_threshold,
            circuit_breaker_cooldown_ms: self.real_client.circuit_breaker_cooldown_ms,
            oauth2_client_id: self.real_client.oauth2_client_id.clone(),
            oauth2_client_secret: self.real_client.oauth2_client_secret.clone(),
            oauth2_token_url: self.real_client.oauth2_token_url.clone(),
            client_cert_pem: self.real_client.client_cert_pem.clone(),
            client_key_pem: self.real_client.client_key_pem.clone(),
            allow_private_networks: self.real_client.allow_private_networks,
            allowed_private_hosts: self.real_client.allowed_private_hosts.clone(),
        }
    }

    pub fn build_runtime(&self) -> Result<Arc<dyn HttpRuntime>, String> {
        match self.runtime_mode {
            HttpServiceRuntimeMode::Deterministic => Ok(Arc::new(DeterministicHttpRuntime::new(
                self.supported_methods.clone(),
            ))),
            HttpServiceRuntimeMode::Real => RealHttpClient::new(self.real_client_config())
                .map(|client| Arc::new(client) as Arc<dyn HttpRuntime>)
                .map_err(|error| error.to_string()),
            HttpServiceRuntimeMode::Async => {
                let real = RealHttpClient::new_async(self.real_client_config())
                    .map_err(|error| error.to_string())?;
                let async_config = AsyncHttpRuntimeConfig {
                    pool_size: self.async_pool_size.max(1),
                    execute_timeout_ms: self.real_client.default_timeout_ms.max(1),
                };
                Ok(
                    Arc::new(AsyncHttpRuntime::new(Arc::new(real), async_config))
                        as Arc<dyn HttpRuntime>,
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HttpServiceRuntimeMode {
    #[default]
    Deterministic,
    Real,
    Async,
}

fn default_http_async_pool_size() -> usize {
    4
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RealHttpClientConfiguration {
    #[serde(default = "default_http_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default = "default_http_connect_timeout_ms")]
    pub default_connect_timeout_ms: u64,
    #[serde(default = "default_http_user_agent")]
    pub user_agent: Option<String>,
    #[serde(default = "default_retry_count")]
    pub retry_count: u32,
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: u64,
    #[serde(default = "default_false")]
    pub cache_enabled: bool,
    #[serde(default = "default_cache_ttl_ms")]
    pub cache_ttl_ms: u64,
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: u32,
    #[serde(default = "default_circuit_breaker_cooldown_ms")]
    pub circuit_breaker_cooldown_ms: u64,
    #[serde(default)]
    pub oauth2_client_id: Option<String>,
    #[serde(default)]
    pub oauth2_client_secret: Option<String>,
    #[serde(default)]
    pub oauth2_token_url: Option<String>,
    #[serde(default)]
    pub client_cert_pem: Option<String>,
    #[serde(default)]
    pub client_key_pem: Option<String>,
    /// SSRF guard escape hatch (default false). Security deviation from Java.
    #[serde(default = "default_false")]
    pub allow_private_networks: bool,
    /// Explicit private hosts/IPs allowed even when `allow_private_networks` is false.
    #[serde(default)]
    pub allowed_private_hosts: Vec<String>,
}

impl Default for RealHttpClientConfiguration {
    fn default() -> Self {
        Self {
            default_timeout_ms: default_http_timeout_ms(),
            default_connect_timeout_ms: default_http_connect_timeout_ms(),
            user_agent: default_http_user_agent(),
            retry_count: default_retry_count(),
            retry_backoff_ms: default_retry_backoff_ms(),
            cache_enabled: false,
            cache_ttl_ms: default_cache_ttl_ms(),
            circuit_breaker_threshold: default_circuit_breaker_threshold(),
            circuit_breaker_cooldown_ms: default_circuit_breaker_cooldown_ms(),
            oauth2_client_id: None,
            oauth2_client_secret: None,
            oauth2_token_url: None,
            client_cert_pem: None,
            client_key_pem: None,
            allow_private_networks: false,
            allowed_private_hosts: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MailServiceRuntimeMode {
    #[default]
    Deterministic,
    Smtp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailServiceTaskConfiguration {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Runtime transport selection. Defaults to deterministic outbox.
    #[serde(default)]
    pub mode: MailServiceRuntimeMode,
    #[serde(default = "default_mail_from_address")]
    pub default_from: String,
    /// SMTP host (required when `mode = smtp`).
    #[serde(default)]
    pub host: Option<String>,
    /// SMTP port. Defaults to 25 for plain local relays.
    #[serde(default = "default_mail_smtp_port")]
    pub port: u16,
    /// Optional SMTP AUTH LOGIN username.
    #[serde(default)]
    pub username: Option<String>,
    /// Optional SMTP AUTH LOGIN password. Never logged by the engine.
    #[serde(default)]
    pub password: Option<String>,
    /// Request STARTTLS. The built-in thin SMTP client fails closed when true.
    #[serde(default)]
    pub starttls: bool,
}

impl Default for MailServiceTaskConfiguration {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: MailServiceRuntimeMode::default(),
            default_from: default_mail_from_address(),
            host: None,
            port: default_mail_smtp_port(),
            username: None,
            password: None,
            starttls: false,
        }
    }
}

impl MailServiceTaskConfiguration {
    /// Build the configured [`MailRuntime`]. Default is deterministic outbox.
    pub fn build_runtime(&self) -> Result<Arc<dyn MailRuntime>, String> {
        match self.mode {
            MailServiceRuntimeMode::Deterministic => Ok(Arc::new(DeterministicMailRuntime::new(
                self.default_from.clone(),
            ))),
            MailServiceRuntimeMode::Smtp => {
                let host = self
                    .host
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        "Mail SMTP mode requires a non-empty host configuration".to_string()
                    })?
                    .to_string();
                Ok(Arc::new(SmtpMailRuntime::new(SmtpMailConfig {
                    host,
                    port: self.port,
                    username: self.username.clone(),
                    password: self.password.clone(),
                    starttls: self.starttls,
                    default_from: self.default_from.clone(),
                    timeout: Duration::from_secs(10),
                })))
            }
        }
    }
}

fn default_http_supported_methods() -> Vec<String> {
    vec!["GET".to_string(), "POST".to_string()]
}

fn default_http_timeout_ms() -> u64 {
    30_000
}

fn default_http_connect_timeout_ms() -> u64 {
    10_000
}

fn default_http_user_agent() -> Option<String> {
    Some("Flowable-Rust-HTTP/0.1".to_string())
}

fn default_retry_count() -> u32 {
    3
}

fn default_retry_backoff_ms() -> u64 {
    100
}

fn default_cache_ttl_ms() -> u64 {
    60_000
}

fn default_circuit_breaker_threshold() -> u32 {
    5
}

fn default_circuit_breaker_cooldown_ms() -> u64 {
    10_000
}

fn default_mail_from_address() -> String {
    "noreply@flowable.local".to_string()
}

fn default_mail_smtp_port() -> u16 {
    25
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalClaimMappingConfig {
    #[serde(default = "default_actor_id_claim")]
    pub actor_id_claim: String,
    #[serde(default = "default_sub_claim")]
    pub subject_claim: String,
    #[serde(default = "default_iss_claim")]
    pub issuer_claim: String,
    #[serde(default)]
    pub tenant_id_claim: Option<String>,
    #[serde(default = "default_role_claim")]
    pub role_claim: String,
}

fn default_sub_claim() -> String {
    "sub".to_string()
}
fn default_iss_claim() -> String {
    "iss".to_string()
}
fn default_role_claim() -> String {
    "role".to_string()
}
fn default_actor_id_claim() -> String {
    "sub".to_string()
}

impl Default for ExternalClaimMappingConfig {
    fn default() -> Self {
        Self {
            actor_id_claim: default_actor_id_claim(),
            subject_claim: default_sub_claim(),
            issuer_claim: default_iss_claim(),
            tenant_id_claim: None,
            role_claim: default_role_claim(),
        }
    }
}

impl From<&ExternalClaimMappingConfig> for IpClaimMappingConfig {
    fn from(cfg: &ExternalClaimMappingConfig) -> Self {
        IpClaimMappingConfig {
            actor_id_claim: cfg.actor_id_claim.clone(),
            subject_claim: cfg.subject_claim.clone(),
            issuer_claim: cfg.issuer_claim.clone(),
            tenant_id_claim: cfg.tenant_id_claim.clone(),
            role_claim: cfg.role_claim.clone(),
        }
    }
}

impl From<&ExternalClaimMappingConfig> for ClaimMapping {
    fn from(cfg: &ExternalClaimMappingConfig) -> Self {
        ClaimMapping {
            actor_id_claim: cfg.actor_id_claim.clone(),
            subject_claim: cfg.subject_claim.clone(),
            issuer_claim: cfg.issuer_claim.clone(),
            tenant_id_claim: cfg.tenant_id_claim.clone(),
            role_claim: cfg.role_claim.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalRoleMappingConfig {
    pub external_role: String,
    pub internal_role: String,
}

impl From<ExternalRoleMappingConfig> for RoleMapping {
    fn from(cfg: ExternalRoleMappingConfig) -> Self {
        RoleMapping {
            external_role: cfg.external_role,
            internal_role: cfg.internal_role,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalValidationConfig {
    #[serde(default = "default_true")]
    pub validate_exp: bool,
    #[serde(default = "default_false")]
    pub validate_nbf: bool,
    #[serde(default = "default_false")]
    pub validate_iat: bool,
    #[serde(default = "default_true")]
    pub reject_empty_claims: bool,
}

fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}

impl Default for ExternalValidationConfig {
    fn default() -> Self {
        Self {
            validate_exp: true,
            validate_nbf: false,
            validate_iat: false,
            reject_empty_claims: true,
        }
    }
}

impl From<ExternalValidationConfig> for ClaimValidation {
    fn from(cfg: ExternalValidationConfig) -> Self {
        ClaimValidation {
            validate_exp: cfg.validate_exp,
            validate_nbf: cfg.validate_nbf,
            validate_iat: cfg.validate_iat,
            reject_empty_claims: cfg.reject_empty_claims,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalIssuerProfileConfig {
    pub id: String,
    pub issuer: String,
    pub audience: String,
    #[serde(default)]
    pub mapping: ExternalClaimMappingConfig,
    #[serde(default)]
    pub validation: ExternalValidationConfig,
    #[serde(default)]
    pub role_mappings: Vec<ExternalRoleMappingConfig>,
    #[serde(default)]
    pub required_tenant: bool,
    #[serde(default)]
    pub rollout_state: RolloutState,
    #[serde(default)]
    pub jwks_uri: Option<String>,
    #[serde(default)]
    pub allowed_algorithms: Option<Vec<String>>,
    #[serde(default = "default_cache_ttl_seconds")]
    pub jwks_cache_ttl_seconds: u64,
    #[serde(default)]
    pub jwks_refresh_policy: JwksRefreshPolicy,
    #[serde(default)]
    pub version: i64,
}

fn default_cache_ttl_seconds() -> u64 {
    3600
}

impl ExternalIssuerProfileConfig {
    pub fn to_issuer_profile(&self) -> IssuerProfile {
        IssuerProfile {
            id: self.id.clone(),
            issuer: self.issuer.clone(),
            audience: self.audience.clone(),
            mapping: IpClaimMappingConfig::from(&self.mapping),
            validation: ClaimValidation::from(self.validation.clone()),
            role_mappings: self
                .role_mappings
                .iter()
                .cloned()
                .map(RoleMapping::from)
                .collect(),
            required_tenant: self.required_tenant,
            rollout_state: self.rollout_state.clone(),
            allowed_algorithms: self
                .allowed_algorithms
                .clone()
                .unwrap_or_else(|| vec!["RS256".to_string()]),
            jwks_uri: self.jwks_uri.clone(),
            jwks_cache_ttl_seconds: self.jwks_cache_ttl_seconds,
            jwks_refresh_policy: self.jwks_refresh_policy.clone(),
            version: self.version,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ExternalAuthProviderConfig {
    #[serde(default)]
    pub issuer: String,
    #[serde(default)]
    pub audience: String,
    #[serde(default)]
    pub mapping: ExternalClaimMappingConfig,
    #[serde(default)]
    pub trusted_profiles: Vec<ExternalIssuerProfileConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthPolicy {
    pub actor_id: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub issuer: Option<String>,
    pub role: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServicePolicyConfig {
    pub bind_addr: String,
    pub max_request_size: usize,
    #[serde(default)]
    pub auth_provider: AuthProviderKind,
    #[serde(default = "default_policy_version")]
    pub policy_version: String,
    pub auth_keys: HashMap<String, AuthPolicy>,
    #[serde(default)]
    pub external_provider: Option<ExternalAuthProviderConfig>,
}

fn default_policy_version() -> String {
    "v1".to_string()
}

impl Default for ServicePolicyConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8080".to_string(),
            max_request_size: 1024 * 1024,
            auth_provider: AuthProviderKind::default(),
            policy_version: default_policy_version(),
            auth_keys: HashMap::new(),
            external_provider: None,
        }
    }
}

#[derive(Clone)]
pub struct IdentityRuntimeComponents {
    pub auth_provider: Arc<dyn AuthProvider>,
    pub profiles: Vec<IssuerProfile>,
    pub jwks_cache: Arc<JwksCache>,
    pub revocation_registry: Arc<TokenRevocationRegistry>,
    pub runtime_store: crate::persistence::runtime_store::RuntimeStore,
    pub rate_limiter: Arc<crate::service::rate_limit::RateLimiter>,
}

impl ServicePolicyConfig {
    pub fn load_from_file(path: &str) -> Result<Self, String> {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    }

    fn build_local_auth(&self) -> AuthConfig {
        let mut local_auth = AuthConfig::new("local-static");
        for (token, policy) in &self.auth_keys {
            local_auth = local_auth.with_key(
                token,
                &policy.actor_id,
                policy.subject.as_deref(),
                policy.issuer.as_deref(),
                &policy.role,
                policy.tenant_id.clone(),
            );
        }
        local_auth
    }

    fn build_external_profiles(&self) -> Vec<IssuerProfile> {
        let Some(ref ext_cfg) = self.external_provider else {
            return vec![];
        };

        if !ext_cfg.trusted_profiles.is_empty() {
            return ext_cfg
                .trusted_profiles
                .iter()
                .map(|p| p.to_issuer_profile())
                .collect();
        }

        if ext_cfg.issuer.is_empty() || ext_cfg.audience.is_empty() {
            return vec![];
        }

        vec![IssuerProfile {
            id: format!("{}-default", ext_cfg.issuer),
            issuer: ext_cfg.issuer.clone(),
            audience: ext_cfg.audience.clone(),
            mapping: IpClaimMappingConfig::from(&ext_cfg.mapping),
            validation: ClaimValidation::default(),
            role_mappings: vec![],
            required_tenant: false,
            rollout_state: RolloutState::Active,
            jwks_uri: None,
            allowed_algorithms: vec!["RS256".to_string()],
            jwks_cache_ttl_seconds: 3600,
            jwks_refresh_policy: JwksRefreshPolicy::default(),
            version: 0,
        }]
    }

    pub fn build_identity_runtime(
        &self,
        runtime_store: crate::persistence::runtime_store::RuntimeStore,
    ) -> IdentityRuntimeComponents {
        let profiles = if self.auth_provider.is_external() {
            self.build_external_profiles()
        } else {
            vec![]
        };

        self.build_identity_runtime_with_components(
            profiles,
            Arc::new(JwksCache::new()),
            Arc::new(TokenRevocationRegistry::new(runtime_store.clone())),
            runtime_store,
        )
    }

    pub fn build_identity_runtime_with_components(
        &self,
        profiles: Vec<IssuerProfile>,
        jwks_cache: Arc<JwksCache>,
        revocation_registry: Arc<TokenRevocationRegistry>,
        runtime_store: crate::persistence::runtime_store::RuntimeStore,
    ) -> IdentityRuntimeComponents {
        let local_auth = self.build_local_auth();
        let rate_limiter = Arc::new(crate::service::rate_limit::RateLimiter::new(
            Default::default(),
        ));

        let mut session = runtime_store.create_session().unwrap();
        let existing_profiles = runtime_store.list_issuer_profiles(&mut session);
        if existing_profiles.is_empty() {
            for profile in &profiles {
                runtime_store.insert_issuer_profile(profile.clone(), &mut session);
            }
            session.flush_and_commit().unwrap();
        } else {
            session.rollback().unwrap();
        }

        if !self.auth_provider.is_external() {
            return IdentityRuntimeComponents {
                auth_provider: Arc::new(local_auth),
                profiles,
                jwks_cache,
                revocation_registry,
                runtime_store,
                rate_limiter,
            };
        }

        let Some(_) = self.external_provider else {
            tracing::warn!(
                "External auth provider selected, but external_provider config is missing; rejecting all authenticated requests"
            );
            return IdentityRuntimeComponents {
                auth_provider: Arc::new(RejectAllAuthProvider),
                profiles,
                jwks_cache,
                revocation_registry,
                runtime_store,
                rate_limiter,
            };
        };

        let effective_profiles = if profiles.is_empty() {
            self.build_external_profiles()
        } else {
            profiles
        };

        let ext_auth = ExternalAuthProvider::new(runtime_store.clone())
            .with_jwks_cache(Arc::clone(&jwks_cache))
            .with_revocation_registry(Arc::clone(&revocation_registry))
            .with_rate_limiter(Arc::clone(&rate_limiter));

        let auth_provider: Arc<dyn AuthProvider> = Arc::new(ext_auth);

        IdentityRuntimeComponents {
            auth_provider,
            profiles: effective_profiles,
            jwks_cache,
            revocation_registry,
            runtime_store,
            rate_limiter,
        }
    }

    pub fn to_auth_provider(
        &self,
        runtime_store: crate::persistence::runtime_store::RuntimeStore,
    ) -> Arc<dyn AuthProvider> {
        self.build_identity_runtime(runtime_store).auth_provider
    }
}
