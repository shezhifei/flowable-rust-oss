use flowable_app_engine::{AppEngine, DefinitionCatalog, DefinitionType, ResolvedDefinition};
use flowable_cmmn_engine::{
    CMMN_PROCESS_TASK_CALLBACK_TYPE, CmmnEngine, CmmnError, CmmnProcessTaskRunner,
    CmmnProcessTaskStartRequest, CmmnProcessTaskStartResult, ProcessInstanceCleanup,
};
use flowable_dmn_engine::DmnEngine;
use flowable_dmn_engine::{DatabaseConfig, DatabaseKind, SchemaMode};
use flowable_engine::engine::process_engine::ProcessEngine;use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::identity::entities::{Group, Membership, User};
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use flowable_engine::service::config::{HttpServiceRuntimeMode, ProcessEngineConfiguration};
use flowable_event_registry_service::FlowableEventRegistryService;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct PlatformBootstrapError(String);

impl PlatformBootstrapError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for PlatformBootstrapError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PlatformBootstrapError {}

impl From<rusqlite::Error> for PlatformBootstrapError {
    fn from(value: rusqlite::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<flowable_dmn_engine::DmnError> for PlatformBootstrapError {
    fn from(value: flowable_dmn_engine::DmnError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<flowable_cmmn_engine::CmmnError> for PlatformBootstrapError {
    fn from(value: flowable_cmmn_engine::CmmnError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<flowable_app_engine::AppError> for PlatformBootstrapError {
    fn from(value: flowable_app_engine::AppError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<flowable_engine::error::FlowableError> for PlatformBootstrapError {
    fn from(value: flowable_engine::error::FlowableError) -> Self {
        Self::new(value.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlatformConfiguration {
    #[serde(default)]
    pub server: ServerConfiguration,
    #[serde(default)]
    pub process: ProcessConfiguration,
    #[serde(default)]
    pub security: SecurityConfiguration,
    #[serde(default)]
    pub dmn: ModuleConfiguration,
    #[serde(default)]
    pub cmmn: ModuleConfiguration,
    #[serde(default)]
    pub app: ModuleConfiguration,
    #[serde(default)]
    pub http_service: HttpServiceConfiguration,
    #[serde(default)]
    pub embedding: RuntimeEmbeddingConfiguration,
    #[serde(default)]
    pub enterprise: EnterpriseAdapterConfiguration,
    #[serde(default)]
    pub directory: DirectoryConfiguration,
    #[serde(default)]
    pub operations: OperationsConfiguration,
    #[serde(default)]
    pub topology: TopologyConfiguration,
    #[serde(default)]
    pub bootstrap: BootstrapConfiguration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfiguration {
    pub bind_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessConfiguration {
    pub engine_name: String,
    pub database_path: String,
    /// Full backend URL (`FLOWABLE_DATABASE_URL`). When present it wins over
    /// `database_path`, which stays the SQLite-only default.
    #[serde(default)]
    pub database_url: Option<String>,
    /// Backend family derived from `database_url` (`mysql`, `postgres`,
    /// `memory`, or `sqlite`).
    #[serde(default)]
    pub database_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfiguration {
    pub auth_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleConfiguration {
    #[serde(default)]
    pub database_path: Option<String>,
    /// Full backend URL shared with the process engine (see
    /// [`ProcessConfiguration::database_url`]).
    #[serde(default)]
    pub database_url: Option<String>,
    /// Backend family derived from `database_url`.
    #[serde(default)]
    pub database_kind: Option<String>,
    /// DMN hit-policy strict mode (`DmnEngineConfiguration.strictMode`,
    /// `DmnEngineConfiguration.java:202` — default true; false tolerates
    /// UNIQUE/ANY/PRIORITY/OUTPUT_ORDER violations with validationMessage).
    #[serde(default = "default_true")]
    pub strict_mode: bool,
}

impl Default for ModuleConfiguration {
    fn default() -> Self {
        // Hand-written so strict_mode matches the serde default (and Java
        // `DmnEngineConfiguration.java:202`) — derived Default would give false.
        Self {
            database_path: None,
            database_url: None,
            database_kind: None,
            strict_mode: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpServiceConfiguration {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_http_supported_methods")]
    pub supported_methods: Vec<String>,
    #[serde(default)]
    pub runtime_mode: HttpServiceRuntimeMode,
    #[serde(default = "default_http_timeout_ms")]
    pub default_timeout_ms: u64,
    #[serde(default = "default_http_connect_timeout_ms")]
    pub default_connect_timeout_ms: u64,
    #[serde(default = "default_http_user_agent")]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapConfiguration {
    pub create_default_admin: bool,
    pub admin_user_id: String,
    pub admin_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEmbeddingConfiguration {
    pub mode: String,
    pub profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnterpriseAdapterConfiguration {
    #[serde(default)]
    pub adapters: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectoryConfiguration {
    pub provider: String,
    pub sync_on_bootstrap: bool,
    pub bundle_path: Option<String>,
    pub transport: String,
    pub auth_mode: String,
    pub deployment_mode: String,
    pub conflict_policy: String,
    pub filter_breadth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationsConfiguration {
    pub exposure: String,
    #[serde(default)]
    pub management_api_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyConfiguration {
    pub profile: String,
    #[serde(default)]
    pub ingress: String,
    #[serde(default)]
    pub packaging: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeEmbeddingMode {
    Standalone,
    Embedded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeEmbeddingProfile {
    StandaloneService,
    CdiCompatible,
    OsgiManaged,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EnterpriseAdapterFamily {
    Camel,
    Cxf,
    Cdi,
    Osgi,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EnterpriseSupportKind {
    CompatibilityLayer,
    ReplacementArchitecture,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DirectoryProviderKind {
    Internal,
    LdapMirror,
    LdapLive,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OperationsExposureKind {
    MetricsOnly,
    JmxBridge,
    JmxNativeCompatible,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OperationsObjectFamilyBreadth {
    MetricsSurfacesOnly,
    LedgersOnly,
    CoreRuntimeAndPlatformLedgers,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CertifiedTopologyProfile {
    RepositoryDefined,
    ReverseProxyTerminated,
    CdiSidecar,
    OsgiOperationsNode,
    DeclaredExternal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeEmbeddingContract {
    pub mode: RuntimeEmbeddingMode,
    pub profile: RuntimeEmbeddingProfile,
    pub adapters: Vec<EnterpriseAdapterFamily>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnterpriseAdapterSupportContract {
    pub family: EnterpriseAdapterFamily,
    pub support_kind: EnterpriseSupportKind,
    pub supported_profiles: Vec<RuntimeEmbeddingProfile>,
    pub external_source_anchor: &'static str,
    pub support_statement: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectorySupportContract {
    pub provider: DirectoryProviderKind,
    pub sync_on_bootstrap: bool,
    pub bundle_path: Option<String>,
    pub transport: String,
    pub auth_mode: String,
    pub deployment_mode: String,
    pub conflict_policy: String,
    pub filter_breadth: String,
    pub external_source_anchor: &'static str,
    pub support_statement: &'static str,
    pub imported_user_count: usize,
    pub imported_group_count: usize,
    pub imported_membership_count: usize,
    pub runtime_user_read_enabled: bool,
    pub runtime_group_read_enabled: bool,
    pub runtime_membership_read_enabled: bool,
    pub runtime_user_write_enabled: bool,
    pub runtime_group_write_enabled: bool,
    pub runtime_membership_write_enabled: bool,
    pub runtime_reconcile_enabled: bool,
    pub runtime_bidirectional_sync_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct DirectoryReadSnapshot {
    pub users: Vec<User>,
    pub groups: Vec<Group>,
    pub memberships: Vec<Membership>,
}

#[derive(Debug)]
pub struct BoundedLdapLiveDirectoryProvider {
    bundle_path: PathBuf,
    bundle_lock: Mutex<()>,
}

#[derive(Debug)]
pub enum LiveDirectoryMutationError {
    NotFound(String),
    Conflict(String),
    InvalidReference(String),
    Storage(String),
}

impl std::fmt::Display for LiveDirectoryMutationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message)
            | Self::Conflict(message)
            | Self::InvalidReference(message)
            | Self::Storage(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for LiveDirectoryMutationError {}

impl From<PlatformBootstrapError> for LiveDirectoryMutationError {
    fn from(value: PlatformBootstrapError) -> Self {
        Self::Storage(value.to_string())
    }
}

impl BoundedLdapLiveDirectoryProvider {
    fn new(bundle_path: PathBuf) -> Self {
        Self {
            bundle_path,
            bundle_lock: Mutex::new(()),
        }
    }

    pub fn load_snapshot(&self) -> Result<DirectoryReadSnapshot, PlatformBootstrapError> {
        let _guard = self
            .bundle_lock
            .lock()
            .map_err(|_| PlatformBootstrapError::new("Directory bundle lock poisoned"))?;
        let bundle = load_directory_bundle(&self.bundle_path)?;
        Ok(DirectoryReadSnapshot {
            users: bundle.users,
            groups: bundle.groups,
            memberships: bundle.memberships,
        })
    }

    pub fn bundle_path(&self) -> &Path {
        &self.bundle_path
    }

    pub fn save_user(&self, user: User) -> Result<User, LiveDirectoryMutationError> {
        self.with_mutable_bundle(|bundle| {
            bundle.users.retain(|existing| existing.id != user.id);
            bundle.users.push(user.clone());
            Ok(user)
        })
    }

    pub fn delete_user(&self, user_id: &str) -> Result<bool, LiveDirectoryMutationError> {
        self.with_mutable_bundle(|bundle| {
            let original_len = bundle.users.len();
            bundle.users.retain(|user| user.id != user_id);
            if bundle.users.len() == original_len {
                return Ok(false);
            }
            bundle
                .memberships
                .retain(|membership| membership.user_id != user_id);
            Ok(true)
        })
    }

    pub fn save_group(&self, group: Group) -> Result<Group, LiveDirectoryMutationError> {
        self.with_mutable_bundle(|bundle| {
            bundle.groups.retain(|existing| existing.id != group.id);
            bundle.groups.push(group.clone());
            Ok(group)
        })
    }

    pub fn delete_group(&self, group_id: &str) -> Result<bool, LiveDirectoryMutationError> {
        self.with_mutable_bundle(|bundle| {
            let original_len = bundle.groups.len();
            bundle.groups.retain(|group| group.id != group_id);
            if bundle.groups.len() == original_len {
                return Ok(false);
            }
            bundle
                .memberships
                .retain(|membership| membership.group_id != group_id);
            Ok(true)
        })
    }

    pub fn create_membership(
        &self,
        user_id: &str,
        group_id: &str,
    ) -> Result<Membership, LiveDirectoryMutationError> {
        self.with_mutable_bundle(|bundle| {
            if !bundle.users.iter().any(|user| user.id == user_id) {
                return Err(LiveDirectoryMutationError::InvalidReference(format!(
                    "Cannot create live LDAP membership: user '{}' is not present in the bounded directory bundle",
                    user_id
                )));
            }
            if !bundle.groups.iter().any(|group| group.id == group_id) {
                return Err(LiveDirectoryMutationError::InvalidReference(format!(
                    "Cannot create live LDAP membership: group '{}' is not present in the bounded directory bundle",
                    group_id
                )));
            }

            let membership = Membership {
                user_id: user_id.to_string(),
                group_id: group_id.to_string(),
            };
            bundle.memberships.retain(|existing| {
                existing.user_id != membership.user_id || existing.group_id != membership.group_id
            });
            bundle.memberships.push(membership.clone());
            Ok(membership)
        })
    }

    pub fn delete_membership(
        &self,
        user_id: &str,
        group_id: &str,
    ) -> Result<bool, LiveDirectoryMutationError> {
        self.with_mutable_bundle(|bundle| {
            let original_len = bundle.memberships.len();
            bundle.memberships.retain(|membership| {
                membership.user_id != user_id || membership.group_id != group_id
            });
            Ok(bundle.memberships.len() != original_len)
        })
    }

    fn with_mutable_bundle<T>(
        &self,
        mutator: impl FnOnce(&mut DirectoryBundle) -> Result<T, LiveDirectoryMutationError>,
    ) -> Result<T, LiveDirectoryMutationError> {
        let _guard = self.bundle_lock.lock().map_err(|_| {
            LiveDirectoryMutationError::Storage("Directory bundle lock poisoned".to_string())
        })?;
        let mut bundle =
            load_directory_bundle(&self.bundle_path).map_err(LiveDirectoryMutationError::from)?;
        let result = mutator(&mut bundle)?;
        normalize_directory_bundle(&mut bundle);
        persist_directory_bundle(&self.bundle_path, &bundle)?;
        Ok(result)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationsSupportContract {
    pub exposure: OperationsExposureKind,
    pub management_api_enabled: bool,
    pub external_source_anchor: &'static str,
    pub support_statement: &'static str,
    pub runtime_ledger_enabled: bool,
    pub timer_ledger_enabled: bool,
    pub topology_ledger_enabled: bool,
    pub native_compatible_connector_enabled: bool,
    pub mbean_registry_enabled: bool,
    pub operations_bus_enabled: bool,
    pub object_family_breadth: OperationsObjectFamilyBreadth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologyCertificationContract {
    pub profile: CertifiedTopologyProfile,
    pub ingress: String,
    pub packaging: String,
    pub external_source_anchor: &'static str,
    pub support_statement: &'static str,
    pub startup_certified: bool,
    pub auth_certified: bool,
    pub cutover_certified: bool,
    pub rollback_certified: bool,
    pub recovery_certified: bool,
    pub supported_historical_ingress: Vec<String>,
}

impl Default for ServerConfiguration {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8080".to_string(),
        }
    }
}

impl Default for ProcessConfiguration {
    fn default() -> Self {
        Self {
            engine_name: "flowable-rest-engine".to_string(),
            database_path: "flowable-rest.db".to_string(),
            database_url: None,
            database_kind: None,
        }
    }
}

impl Default for SecurityConfiguration {
    fn default() -> Self {
        Self {
            auth_mode: "basic".to_string(),
        }
    }
}

impl Default for BootstrapConfiguration {
    fn default() -> Self {
        // Security deviation from Java: Java Flowable seeds admin/admin by default.
        // That is a known weak-default security bug; we default to no admin seed.
        Self {
            create_default_admin: false,
            admin_user_id: "admin".to_string(),
            admin_password: "admin".to_string(),
        }
    }
}

impl Default for RuntimeEmbeddingConfiguration {
    fn default() -> Self {
        Self {
            mode: "standalone".to_string(),
            profile: "standalone-service".to_string(),
        }
    }
}

impl Default for HttpServiceConfiguration {
    fn default() -> Self {
        Self {
            enabled: true,
            supported_methods: default_http_supported_methods(),
            runtime_mode: HttpServiceRuntimeMode::default(),
            default_timeout_ms: default_http_timeout_ms(),
            default_connect_timeout_ms: default_http_connect_timeout_ms(),
            user_agent: default_http_user_agent(),
        }
    }
}

impl Default for DirectoryConfiguration {
    fn default() -> Self {
        Self {
            provider: "internal".to_string(),
            sync_on_bootstrap: false,
            bundle_path: None,
            transport: "ldaps".to_string(),
            auth_mode: "service-account-bind".to_string(),
            deployment_mode: "sidecar-session".to_string(),
            conflict_policy: "live-wins".to_string(),
            filter_breadth: "identity-surface-full".to_string(),
        }
    }
}

impl Default for OperationsConfiguration {
    fn default() -> Self {
        Self {
            exposure: "metrics-only".to_string(),
            management_api_enabled: false,
        }
    }
}

impl Default for TopologyConfiguration {
    fn default() -> Self {
        Self {
            profile: "repository-defined".to_string(),
            ingress: String::new(),
            packaging: String::new(),
        }
    }
}

fn default_true() -> bool {
    true
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

impl PlatformConfiguration {
    pub fn load() -> Result<Self, PlatformBootstrapError> {
        Self::load_from_sources(None)
    }

    pub fn load_from_sources(path: Option<PathBuf>) -> Result<Self, PlatformBootstrapError> {
        let resolved_path = resolve_configuration_path(path)?;
        let mut configuration = if let Some(path) = resolved_path {
            load_configuration_file(&path)?
        } else {
            Self::default()
        };
        configuration.apply_environment_overrides()?;
        Ok(configuration)
    }

    fn apply_environment_overrides(&mut self) -> Result<(), PlatformBootstrapError> {
        apply_optional_string_override(
            &mut self.server.bind_address,
            &["FLOWABLE_REST_BIND_ADDRESS", "FLOWABLE_SERVER_BIND_ADDRESS"],
        );
        apply_optional_string_override(
            &mut self.process.engine_name,
            &["FLOWABLE_REST_ENGINE_NAME", "FLOWABLE_PROCESS_ENGINE_NAME"],
        );
        apply_optional_string_override(
            &mut self.process.database_path,
            &["FLOWABLE_REST_DB_PATH", "FLOWABLE_PROCESS_DATABASE_PATH"],
        );
        apply_optional_option_string_override(
            &mut self.dmn.database_path,
            &["FLOWABLE_DMN_DATABASE_PATH"],
        );
        apply_optional_option_string_override(
            &mut self.cmmn.database_path,
            &["FLOWABLE_CMMN_DATABASE_PATH"],
        );
        apply_optional_option_string_override(
            &mut self.app.database_path,
            &["FLOWABLE_APP_DATABASE_PATH"],
        );
        apply_optional_bool_override(
            &mut self.http_service.enabled,
            &["FLOWABLE_HTTP_SERVICE_ENABLED"],
        )?;
        apply_optional_csv_override(
            &mut self.http_service.supported_methods,
            &["FLOWABLE_HTTP_SERVICE_SUPPORTED_METHODS"],
        );
        apply_optional_http_runtime_mode_override(
            &mut self.http_service.runtime_mode,
            &["FLOWABLE_HTTP_SERVICE_RUNTIME_MODE"],
        )?;
        apply_optional_u64_override(
            &mut self.http_service.default_timeout_ms,
            &["FLOWABLE_HTTP_SERVICE_DEFAULT_TIMEOUT_MS"],
        )?;
        apply_optional_u64_override(
            &mut self.http_service.default_connect_timeout_ms,
            &["FLOWABLE_HTTP_SERVICE_DEFAULT_CONNECT_TIMEOUT_MS"],
        )?;
        apply_optional_option_string_override(
            &mut self.http_service.user_agent,
            &["FLOWABLE_HTTP_SERVICE_USER_AGENT"],
        );
        apply_optional_string_override(
            &mut self.security.auth_mode,
            &["FLOWABLE_SECURITY_AUTH_MODE", "FLOWABLE_REST_AUTH_MODE"],
        );
        apply_optional_string_override(&mut self.embedding.mode, &["FLOWABLE_EMBEDDING_MODE"]);
        apply_optional_string_override(
            &mut self.embedding.profile,
            &["FLOWABLE_EMBEDDING_PROFILE"],
        );
        apply_optional_csv_override(
            &mut self.enterprise.adapters,
            &["FLOWABLE_ENTERPRISE_ADAPTERS"],
        );
        apply_optional_string_override(
            &mut self.directory.provider,
            &["FLOWABLE_DIRECTORY_PROVIDER"],
        );
        apply_optional_bool_override(
            &mut self.directory.sync_on_bootstrap,
            &["FLOWABLE_DIRECTORY_SYNC_ON_BOOTSTRAP"],
        )?;
        apply_optional_option_string_override(
            &mut self.directory.bundle_path,
            &["FLOWABLE_DIRECTORY_BUNDLE_PATH"],
        );
        apply_optional_string_override(
            &mut self.directory.transport,
            &["FLOWABLE_DIRECTORY_TRANSPORT"],
        );
        apply_optional_string_override(
            &mut self.directory.auth_mode,
            &["FLOWABLE_DIRECTORY_AUTH_MODE"],
        );
        apply_optional_string_override(
            &mut self.directory.deployment_mode,
            &["FLOWABLE_DIRECTORY_DEPLOYMENT_MODE"],
        );
        apply_optional_string_override(
            &mut self.directory.conflict_policy,
            &["FLOWABLE_DIRECTORY_CONFLICT_POLICY"],
        );
        apply_optional_string_override(
            &mut self.directory.filter_breadth,
            &["FLOWABLE_DIRECTORY_FILTER_BREADTH"],
        );
        apply_optional_string_override(
            &mut self.operations.exposure,
            &["FLOWABLE_OPERATIONS_EXPOSURE"],
        );
        apply_optional_bool_override(
            &mut self.operations.management_api_enabled,
            &["FLOWABLE_MANAGEMENT_API_ENABLED"],
        )?;
        apply_optional_string_override(&mut self.topology.profile, &["FLOWABLE_TOPOLOGY_PROFILE"]);
        apply_optional_bool_override(
            &mut self.bootstrap.create_default_admin,
            &["FLOWABLE_BOOTSTRAP_CREATE_DEFAULT_ADMIN"],
        )?;
        apply_optional_string_override(
            &mut self.bootstrap.admin_user_id,
            &["FLOWABLE_BOOTSTRAP_ADMIN_USER_ID"],
        );
        apply_optional_string_override(
            &mut self.bootstrap.admin_password,
            &["FLOWABLE_BOOTSTRAP_ADMIN_PASSWORD"],
        );
        self.apply_database_url_override();
        Ok(())
    }

    /// `FLOWABLE_DATABASE_URL` selects one production backend for the whole
    /// platform: process, DMN, CMMN, and App engines all open the same URL.
    /// Absent, every engine keeps its SQLite path (the default binary stays
    /// SQLite-only).
    fn apply_database_url_override(&mut self) {
        let Some(url) = env::var("FLOWABLE_DATABASE_URL")
            .ok()
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty())
        else {
            return;
        };
        let kind = kind_from_database_url(&url).to_string();

        self.process.database_url = Some(url.clone());
        self.process.database_kind = Some(kind.clone());
        for module in [&mut self.dmn, &mut self.cmmn, &mut self.app] {
            module.database_url = Some(url.clone());
            module.database_kind = Some(kind.clone());
        }
    }
}

/// Maps a database URL scheme onto the backend family names understood by
/// `EngineDatabaseKind` / `flowable_persistence::DatabaseKind`.
fn kind_from_database_url(url: &str) -> &'static str {
    if url.starts_with("mysql://") {
        "mysql"
    } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        "postgres"
    } else if url == ":memory:" {
        "memory"
    } else {
        "sqlite"
    }
}

#[derive(Clone)]
pub struct FlowablePlatform {
    config: PlatformConfiguration,
    process_engine: Arc<ProcessEngine>,
    dmn_engine: Arc<DmnEngine>,
    cmmn_engine: Arc<CmmnEngine>,
    app_engine: Arc<AppEngine>,
    embedding_contract: RuntimeEmbeddingContract,
    enterprise_support_contracts: Vec<EnterpriseAdapterSupportContract>,
    directory_support_contract: DirectorySupportContract,
    live_directory_provider: Option<Arc<BoundedLdapLiveDirectoryProvider>>,
    operations_support_contract: OperationsSupportContract,
    topology_certification_contract: TopologyCertificationContract,
}

impl FlowablePlatform {
    pub fn bootstrap(config: PlatformConfiguration) -> Result<Self, PlatformBootstrapError> {
        let embedding_contract = resolve_runtime_embedding_contract(&config)?;
        let enterprise_support_contracts =
            resolve_enterprise_support_contracts(&embedding_contract)?;
        let mut directory_support_contract = resolve_directory_support_contract(&config)?;
        let live_directory_provider = build_live_directory_provider(&config.directory)?;
        let operations_support_contract = resolve_operations_support_contract(&config)?;
        let topology_certification_contract = resolve_topology_certification_contract(
            &config,
            &embedding_contract,
            &enterprise_support_contracts,
            &directory_support_contract,
            &operations_support_contract,
        )?;
        let dmn_engine = Arc::new(build_dmn_engine(&config.dmn)?);
        let process_task_runner = Arc::new(PlatformProcessTaskRunner::default());
        let process_instance_cleanup = Arc::new(PlatformProcessInstanceCleanup::default());
        let cmmn_process_task_runner: Arc<dyn CmmnProcessTaskRunner> = process_task_runner.clone();
        let cmmn_process_cleanup: Arc<dyn ProcessInstanceCleanup> =
            process_instance_cleanup.clone();
        let cmmn_engine = Arc::new(build_cmmn_engine(
            &config.cmmn,
            cmmn_process_task_runner,
            Some(cmmn_process_cleanup),
        )?);
        let process_engine = Arc::new(build_process_engine(
            &config.process,
            &config.http_service,
            Arc::clone(&dmn_engine),
            Arc::clone(&cmmn_engine),
        )?);
        process_task_runner.set_process_engine(Arc::clone(&process_engine));
        process_instance_cleanup.set_process_engine(Arc::clone(&process_engine));
        let app_engine = Arc::new(build_app_engine(
            &config.app,
            Arc::clone(&process_engine),
            Arc::clone(&dmn_engine),
            Arc::clone(&cmmn_engine),
        )?);

        if config.bootstrap.create_default_admin {
            // Security deviation from Java: refuse the well-known default password.
            // Set bootstrap.admin_password (or FLOWABLE_BOOTSTRAP_ADMIN_PASSWORD) to a
            // non-default value when create_default_admin is true.
            if config.bootstrap.admin_password == "admin" {
                return Err(PlatformBootstrapError::new(
                    "Refusing to create default admin with password \"admin\". \
                     Set bootstrap.admin_password (or FLOWABLE_BOOTSTRAP_ADMIN_PASSWORD) \
                     to a non-default value when create_default_admin is true \
                     (security deviation from Java weak default admin/admin).",
                ));
            }
            let identity_service = process_engine.get_identity_service();
            identity_service.save_user(User {
                id: config.bootstrap.admin_user_id.clone(),
                first_name: None,
                last_name: None,
                email: None,
                password: Some(config.bootstrap.admin_password.clone()),
                tenant_id: None,
            });
        }

        import_directory_bundle(
            process_engine.as_ref(),
            &config.directory,
            &mut directory_support_contract,
        )?;

        Ok(Self {
            config,
            process_engine,
            dmn_engine,
            cmmn_engine,
            app_engine,
            embedding_contract,
            enterprise_support_contracts,
            directory_support_contract,
            live_directory_provider,
            operations_support_contract,
            topology_certification_contract,
        })
    }

    pub fn bootstrap_from_sources(path: Option<PathBuf>) -> Result<Self, PlatformBootstrapError> {
        let configuration = PlatformConfiguration::load_from_sources(path)?;
        Self::bootstrap(configuration)
    }

    pub fn config(&self) -> &PlatformConfiguration {
        &self.config
    }

    pub fn process_engine(&self) -> Arc<ProcessEngine> {
        Arc::clone(&self.process_engine)
    }

    pub fn dmn_engine(&self) -> Arc<DmnEngine> {
        Arc::clone(&self.dmn_engine)
    }

    pub fn cmmn_engine(&self) -> Arc<CmmnEngine> {
        Arc::clone(&self.cmmn_engine)
    }

    pub fn app_engine(&self) -> Arc<AppEngine> {
        Arc::clone(&self.app_engine)
    }

    pub fn runtime_embedding_contract(&self) -> &RuntimeEmbeddingContract {
        &self.embedding_contract
    }

    pub fn enterprise_adapter_support_contracts(&self) -> &[EnterpriseAdapterSupportContract] {
        &self.enterprise_support_contracts
    }

    pub fn directory_support_contract(&self) -> &DirectorySupportContract {
        &self.directory_support_contract
    }

    pub fn live_directory_provider(&self) -> Option<Arc<BoundedLdapLiveDirectoryProvider>> {
        self.live_directory_provider.as_ref().map(Arc::clone)
    }

    pub fn operations_support_contract(&self) -> &OperationsSupportContract {
        &self.operations_support_contract
    }

    pub fn topology_certification_contract(&self) -> &TopologyCertificationContract {
        &self.topology_certification_contract
    }

    pub fn enterprise_support_statement(&self) -> String {
        let adapters = if self.enterprise_support_contracts.is_empty() {
            "no enterprise adapters enabled".to_string()
        } else {
            self.enterprise_support_contracts
                .iter()
                .map(|contract| format!("{:?}", contract.family).to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "bounded enterprise runtime embedding contract: mode={:?}, profile={:?}, adapters={}",
            self.embedding_contract.mode, self.embedding_contract.profile, adapters
        )
    }
}

fn build_process_engine(
    config: &ProcessConfiguration,
    http_service: &HttpServiceConfiguration,
    dmn_engine: Arc<DmnEngine>,
    cmmn_engine: Arc<CmmnEngine>,
) -> Result<ProcessEngine, PlatformBootstrapError> {
    let mut process_engine_config = ProcessEngineConfiguration {
        dmn_engine: Some(dmn_engine),
        cmmn_engine: Some(cmmn_engine),
        ..ProcessEngineConfiguration::default()
    };
    process_engine_config.http_service.enabled = http_service.enabled;
    process_engine_config.http_service.supported_methods = http_service.supported_methods.clone();
    process_engine_config.http_service.runtime_mode = http_service.runtime_mode;
    process_engine_config
        .http_service
        .real_client
        .default_timeout_ms = http_service.default_timeout_ms;
    process_engine_config
        .http_service
        .real_client
        .default_connect_timeout_ms = http_service.default_connect_timeout_ms;
    process_engine_config.http_service.real_client.user_agent = http_service.user_agent.clone();
    if let Some(url) = config.database_url.as_deref() {
        // FLOWABLE_DATABASE_URL wins over the SQLite `database_path` default.
        process_engine_config.database.kind = match config.database_kind.as_deref() {
            Some("mysql") => flowable_engine::service::config::EngineDatabaseKind::Mysql,
            Some("postgres") => flowable_engine::service::config::EngineDatabaseKind::Postgres,
            Some("memory") => flowable_engine::service::config::EngineDatabaseKind::Memory,
            _ => flowable_engine::service::config::EngineDatabaseKind::Sqlite,
        };
        process_engine_config.database.url = url.to_string();
    } else if config.database_path == ":memory:" {
        process_engine_config.database.kind =
            flowable_engine::service::config::EngineDatabaseKind::Memory;
        process_engine_config.database.url = ":memory:".to_string();
    } else {
        process_engine_config.database.kind =
            flowable_engine::service::config::EngineDatabaseKind::Sqlite;
        process_engine_config.database.url = config.database_path.clone();
    }
    Ok(ProcessEngine::build_with_config(
        config.engine_name.clone(),
        Arc::new(SystemTimeSource),
        process_engine_config,
    )?)
}

fn build_dmn_engine(config: &ModuleConfiguration) -> Result<DmnEngine, PlatformBootstrapError> {
    let builder = DmnEngine::builder().strict_mode(config.strict_mode);
    if let Some(database_config) = module_database_config(config) {
        return Ok(builder.build_from_database_config(database_config)?);
    }
    Ok(
        if let Some(path) = config
            .database_path
            .as_deref()
            .filter(|path| *path != ":memory:")
        {
            builder.build_sqlite(path)?
        } else {
            builder.build_in_memory()?
        },
    )
}

fn build_cmmn_engine(
    config: &ModuleConfiguration,
    process_task_runner: Arc<dyn CmmnProcessTaskRunner>,
    process_instance_cleanup: Option<Arc<dyn ProcessInstanceCleanup>>,
) -> Result<CmmnEngine, PlatformBootstrapError> {
    let process_task_runner = Some(process_task_runner);
    if let Some(database_config) = module_database_config(config) {
        return Ok(CmmnEngine::from_database_config_with_process_integrations(
            database_config,
            process_task_runner,
            process_instance_cleanup,
        )?);
    }
    Ok(
        if let Some(path) = config
            .database_path
            .as_deref()
            .filter(|path| *path != ":memory:")
        {
            CmmnEngine::new_sqlite_with_process_integrations(
                path,
                process_task_runner,
                process_instance_cleanup,
            )?
        } else {
            CmmnEngine::new_in_memory_with_process_integrations(
                process_task_runner,
                process_instance_cleanup,
            )?
        },
    )
}

/// Turns a module's `database_url` / `database_kind` (populated from
/// `FLOWABLE_DATABASE_URL`) into the portable persistence config the DMN, CMMN,
/// and App stores accept. `None` keeps the SQLite/in-memory constructors.
fn module_database_config(config: &ModuleConfiguration) -> Option<DatabaseConfig> {
    let url = config.database_url.as_deref()?;
    Some(DatabaseConfig {
        kind: match config.database_kind.as_deref() {
            Some("mysql") => DatabaseKind::Mysql,
            Some("postgres") => DatabaseKind::Postgres,
            Some("memory") => DatabaseKind::Memory,
            _ => DatabaseKind::Sqlite,
        },
        url: url.to_string(),
        pool_size: 8,
        schema_mode: SchemaMode::True,
        table_prefix: None,
        schema: None,
        catalog: None,
    })
}

#[derive(Default)]
struct PlatformProcessTaskRunner {
    process_engine: Mutex<Option<Arc<ProcessEngine>>>,
}

impl PlatformProcessTaskRunner {
    fn set_process_engine(&self, process_engine: Arc<ProcessEngine>) {
        *self
            .process_engine
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(process_engine);
    }
}

/// Adapter that deletes BPMN process instances during CMMN cascade deployment delete.
///
/// Clears CMMN process-task callback metadata before delete so cascade does not
/// re-enter the CMMN state machine while the parent case is being purged.
#[derive(Default)]
struct PlatformProcessInstanceCleanup {
    process_engine: Mutex<Option<Arc<ProcessEngine>>>,
}

impl PlatformProcessInstanceCleanup {
    fn set_process_engine(&self, process_engine: Arc<ProcessEngine>) {
        *self
            .process_engine
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(process_engine);
    }
}

impl ProcessInstanceCleanup for PlatformProcessInstanceCleanup {
    fn delete_process_instance_cascade(&self, process_instance_id: &str) -> Result<(), CmmnError> {
        let process_engine = self
            .process_engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                CmmnError::unsupported(
                    "process task child cleanup",
                    "platform process cleanup was not attached to a BPMN process engine",
                )
            })?;
        let runtime_service = process_engine.get_runtime_service();

        // Suppress CMMN process-task failure callbacks during cascade purge.
        // The parent case is already being deleted; re-entry would race with it.
        let _ = runtime_service.update_process_instance(
            process_instance_id.to_string(),
            ProcessInstanceUpdate {
                callback_id: Some(None),
                callback_type: Some(None),
                ..Default::default()
            },
        );

        match runtime_service.delete_process_instance(
            process_instance_id.to_string(),
            Some(format!(
                "CMMN cascade delete of parent case association for process instance '{process_instance_id}'"
            )),
        ) {
            Ok(()) => Ok(()),
            Err(flowable_engine::error::FlowableError::NotFound(_)) => {
                // Already gone — safe for cascade (no orphan remains).
                Ok(())
            }
            Err(error) => Err(CmmnError::execution(format!(
                "failed to cascade-delete BPMN child process instance '{process_instance_id}': {error}"
            ))),
        }
    }
}

impl CmmnProcessTaskRunner for PlatformProcessTaskRunner {
    fn start_process(
        &self,
        request: CmmnProcessTaskStartRequest,
    ) -> Result<CmmnProcessTaskStartResult, CmmnError> {
        let process_engine = self
            .process_engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                CmmnError::unsupported(
                    "processTask runtime",
                    "platform process task runner was not attached to a BPMN process engine",
                )
            })?;
        let runtime_service = process_engine.get_runtime_service();
        let mut builder = runtime_service
            .create_process_instance_builder()
            .process_definition_key(request.process_definition_key.clone());
        if let Some(business_key) = request.business_key.clone() {
            builder = builder.business_key(business_key);
        }
        if let Some(tenant_id) = request.tenant_id.clone() {
            builder = builder.tenant_id(tenant_id);
        }
        for (name, value) in request.variables.clone() {
            builder = builder.variable(name, value);
        }

        let process_instance = runtime_service
            .start_process_instance(builder)
            .map_err(|error| CmmnError::execution(error.to_string()))?;
        let completed = {
            let store = process_engine.get_runtime_store();
            let mut session = store.create_session().unwrap();
            let completed = store
                .find_process_instance(&process_instance.id, &mut session)
                .map(|stored| stored.is_ended)
                .unwrap_or(process_instance.is_ended);
            session.rollback().unwrap();
            completed
        };

        if !completed {
            runtime_service
                .update_process_instance(
                    process_instance.id.clone(),
                    ProcessInstanceUpdate {
                        callback_id: Some(Some(request.parent_plan_item_id.clone())),
                        callback_type: Some(Some(CMMN_PROCESS_TASK_CALLBACK_TYPE.to_string())),
                        reference_id: Some(Some(request.parent_case_instance_id.clone())),
                        reference_type: Some(Some("cmmn-case-instance".to_string())),
                        ..Default::default()
                    },
                )
                .map_err(|error| CmmnError::execution(error.to_string()))?;
        }

        Ok(CmmnProcessTaskStartResult {
            process_instance_id: process_instance.id,
            completed,
        })
    }
}

fn build_app_engine(
    config: &ModuleConfiguration,
    process_engine: Arc<ProcessEngine>,
    dmn_engine: Arc<DmnEngine>,
    cmmn_engine: Arc<CmmnEngine>,
) -> Result<AppEngine, PlatformBootstrapError> {
    let catalog = Arc::new(AppDefinitionCatalogAdapter {
        process_engine: Arc::clone(&process_engine),
        dmn_engine,
        cmmn_engine,
        // P92: wire BPMN Event Registry consumer (Java bpmnEventConsumer) so
        // inbound deliveries reach BPMN wait-states. Service-unit tests keep
        // FlowableEventRegistryService::new (NoOp default).
        // Java: ProcessEngineConfigurationImpl.java:1608-1616 → BpmnEventRegistryEventConsumer.
        event_registry_service: FlowableEventRegistryService::with_bpmn_consumer(process_engine),
    });

    if let Some(database_config) = module_database_config(config) {
        return Ok(AppEngine::from_database_config_with_catalog(
            database_config,
            catalog,
        )?);
    }

    Ok(
        if let Some(path) = config
            .database_path
            .as_deref()
            .filter(|path| *path != ":memory:")
        {
            AppEngine::new_sqlite_with_catalog(path, catalog)?
        } else {
            AppEngine::new_in_memory_with_catalog(catalog)?
        },
    )
}

#[derive(Clone)]
struct AppDefinitionCatalogAdapter {
    process_engine: Arc<ProcessEngine>,
    dmn_engine: Arc<DmnEngine>,
    cmmn_engine: Arc<CmmnEngine>,
    event_registry_service: FlowableEventRegistryService,
}

impl DefinitionCatalog for AppDefinitionCatalogAdapter {
    fn resolve_definition(
        &self,
        definition_type: DefinitionType,
        definition_key: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<ResolvedDefinition>, flowable_app_engine::AppError> {
        match definition_type {
            DefinitionType::BpmnProcess => {
                let definition = self
                    .process_engine
                    .get_repository_service()
                    .latest_process_definition_by_key(definition_key, tenant_id)
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name.unwrap_or_else(|| "Process".to_string()),
                    deployment_id: definition
                        .deployment_id
                        .unwrap_or_else(|| "process-deployment".to_string()),
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::DmnDecision => {
                let mut query = self
                    .dmn_engine
                    .repository_service()
                    .create_decision_query()
                    .key(definition_key);
                if let Some(tenant_id) = tenant_id {
                    query = query.tenant_id(tenant_id.to_string());
                }
                let definition = query
                    .single_result()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::CmmnCase => {
                let mut query = self
                    .cmmn_engine
                    .repository_service()
                    .create_case_definition_query()
                    .key(definition_key);
                if let Some(tenant_id) = tenant_id {
                    query = query.tenant_id(tenant_id.to_string());
                }
                let definition = query
                    .single_result()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::EventRegistry => {
                // Reconcile cache against durable change log so App composition sees
                // recent Event Registry deploy/delete activity on shared process storage.
                let _ = self.event_registry_service.detect_and_reconcile_changes();
                let mut query = self
                    .event_registry_service
                    .create_event_definition_query()
                    .key(definition_key)
                    .latest();
                if let Some(tenant_id) = tenant_id {
                    query = query.tenant_id(tenant_id.to_string());
                }
                let definition = query
                    .list()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?
                    .into_iter()
                    .next()
                    .or_else(|| {
                        // Explicit default-tenant fallback when tenant-specific missing.
                        if tenant_id.is_some() {
                            self.event_registry_service
                                .create_event_definition_query()
                                .key(definition_key)
                                .latest()
                                .list()
                                .ok()
                                .and_then(|definitions| {
                                    definitions
                                        .into_iter()
                                        .find(|definition| definition.tenant_id.is_none())
                                })
                        } else {
                            None
                        }
                    });
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
        }
    }

    fn resolve_definition_by_id(
        &self,
        definition_type: DefinitionType,
        definition_id: &str,
    ) -> Result<Option<ResolvedDefinition>, flowable_app_engine::AppError> {
        match definition_type {
            DefinitionType::BpmnProcess => {
                let definition = match self
                    .process_engine
                    .get_repository_service()
                    .get_process_definition(definition_id)
                {
                    Ok(definition) => Some(definition),
                    Err(flowable_engine::error::FlowableError::NotFound(_)) => None,
                    Err(error) => {
                        return Err(flowable_app_engine::AppError::execution(error.to_string()));
                    }
                };
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name.unwrap_or_else(|| "Process".to_string()),
                    deployment_id: definition
                        .deployment_id
                        .unwrap_or_else(|| "process-deployment".to_string()),
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::DmnDecision => {
                let definition = self
                    .dmn_engine
                    .repository_service()
                    .create_decision_query()
                    .id(definition_id)
                    .single_result()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::CmmnCase => {
                let definition = self
                    .cmmn_engine
                    .repository_service()
                    .create_case_definition_query()
                    .id(definition_id)
                    .single_result()
                    .map_err(|error| flowable_app_engine::AppError::execution(error.to_string()))?;
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
            DefinitionType::EventRegistry => {
                // Reconcile cache against the durable change log first, matching
                // the by-key resolution path above.
                let _ = self.event_registry_service.detect_and_reconcile_changes();
                let definition = match self
                    .event_registry_service
                    .get_event_definition(definition_id)
                {
                    Ok(definition) => Some(definition),
                    Err(flowable_engine::error::FlowableError::NotFound(_)) => None,
                    Err(error) => {
                        return Err(flowable_app_engine::AppError::execution(error.to_string()));
                    }
                };
                Ok(definition.map(|definition| ResolvedDefinition {
                    definition_type,
                    definition_id: definition.id,
                    definition_key: definition.key,
                    definition_name: definition.name,
                    deployment_id: definition.deployment_id,
                    version: definition.version,
                    tenant_id: definition.tenant_id,
                }))
            }
        }
    }
}

fn resolve_configuration_path(
    path: Option<PathBuf>,
) -> Result<Option<PathBuf>, PlatformBootstrapError> {
    if let Some(path) = path {
        return require_existing_path(path);
    }

    if let Some(path) = env::var_os("FLOWABLE_PLATFORM_CONFIG") {
        return require_existing_path(PathBuf::from(path));
    }

    let default_paths = [
        PathBuf::from("flowable-platform.toml"),
        PathBuf::from("config/flowable-platform.toml"),
        PathBuf::from("application.properties"),
        PathBuf::from("config/application.properties"),
        PathBuf::from("flowable.properties"),
        PathBuf::from("config/flowable.properties"),
    ];
    Ok(default_paths.into_iter().find(|path| path.exists()))
}

fn require_existing_path(path: PathBuf) -> Result<Option<PathBuf>, PlatformBootstrapError> {
    if path.exists() {
        Ok(Some(path))
    } else {
        Err(PlatformBootstrapError::new(format!(
            "Configuration file '{}' was not found",
            path.display()
        )))
    }
}

fn load_configuration_file(path: &Path) -> Result<PlatformConfiguration, PlatformBootstrapError> {
    let contents = std::fs::read_to_string(path).map_err(|error| {
        PlatformBootstrapError::new(format!(
            "Failed to read configuration file '{}': {}",
            path.display(),
            error
        ))
    })?;

    if is_properties_path(path) {
        load_properties_configuration(path, &contents)
    } else {
        toml::from_str(&contents).map_err(|error| {
            PlatformBootstrapError::new(format!(
                "Failed to parse configuration file '{}': {}",
                path.display(),
                error
            ))
        })
    }
}

fn apply_optional_string_override(target: &mut String, keys: &[&str]) {
    for key in keys {
        if let Ok(value) = env::var(key) {
            *target = value;
        }
    }
}

fn apply_optional_option_string_override(target: &mut Option<String>, keys: &[&str]) {
    for key in keys {
        if let Ok(value) = env::var(key) {
            *target = Some(value);
        }
    }
}

fn apply_optional_csv_override(target: &mut Vec<String>, keys: &[&str]) {
    for key in keys {
        if let Ok(value) = env::var(key) {
            *target = value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect();
        }
    }
}

fn apply_optional_bool_override(
    target: &mut bool,
    keys: &[&str],
) -> Result<(), PlatformBootstrapError> {
    for key in keys {
        if let Ok(value) = env::var(key) {
            *target = parse_bool(&value, key)?;
        }
    }
    Ok(())
}

fn apply_optional_u64_override(
    target: &mut u64,
    keys: &[&str],
) -> Result<(), PlatformBootstrapError> {
    for key in keys {
        if let Ok(value) = env::var(key) {
            *target = value.trim().parse::<u64>().map_err(|error| {
                PlatformBootstrapError::new(format!(
                    "Environment variable '{key}' must be an unsigned integer, got '{value}': {error}"
                ))
            })?;
        }
    }
    Ok(())
}

fn apply_optional_http_runtime_mode_override(
    target: &mut HttpServiceRuntimeMode,
    keys: &[&str],
) -> Result<(), PlatformBootstrapError> {
    for key in keys {
        if let Ok(value) = env::var(key) {
            *target = match value.trim().to_ascii_lowercase().as_str() {
                "deterministic" | "mock" | "stub" => HttpServiceRuntimeMode::Deterministic,
                "real" | "http" => HttpServiceRuntimeMode::Real,
                "async" | "pooled" => HttpServiceRuntimeMode::Async,
                _ => {
                    return Err(PlatformBootstrapError::new(format!(
                        "Environment variable '{key}' must be one of deterministic, real, or async, got '{value}'"
                    )));
                }
            };
        }
    }
    Ok(())
}

fn parse_bool(value: &str, key: &str) -> Result<bool, PlatformBootstrapError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(PlatformBootstrapError::new(format!(
            "Environment variable '{key}' must be a boolean, got '{value}'"
        ))),
    }
}

fn is_properties_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("properties"))
        .unwrap_or(false)
}

fn load_properties_configuration(
    path: &Path,
    contents: &str,
) -> Result<PlatformConfiguration, PlatformBootstrapError> {
    let mut configuration = PlatformConfiguration::default();

    for (index, raw_line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }

        let (key, value) = split_properties_entry(line).ok_or_else(|| {
            PlatformBootstrapError::new(format!(
                "Failed to parse configuration file '{}': invalid properties entry at line {}",
                path.display(),
                line_number
            ))
        })?;

        apply_properties_entry(&mut configuration, key.trim(), value.trim())?;
    }

    Ok(configuration)
}

fn split_properties_entry(line: &str) -> Option<(&str, &str)> {
    if let Some((key, value)) = line.split_once('=') {
        return Some((key, value));
    }

    line.split_once(':')
}

fn apply_properties_entry(
    configuration: &mut PlatformConfiguration,
    key: &str,
    value: &str,
) -> Result<(), PlatformBootstrapError> {
    let normalized_value = normalize_properties_value(value);
    match key {
        "server.bind-address" => {
            configuration.server.bind_address = normalized_value;
        }
        "server.address" => {
            configuration.server.bind_address = merge_bind_address(
                &configuration.server.bind_address,
                Some(normalized_value.as_str()),
                None,
            )?;
        }
        "server.port" => {
            configuration.server.bind_address = merge_bind_address(
                &configuration.server.bind_address,
                None,
                Some(normalized_value.as_str()),
            )?;
        }
        "flowable.process.engine-name" => {
            configuration.process.engine_name = normalized_value;
        }
        "flowable.process.database-path"
        | "flowable.process.datasource.url"
        | "spring.datasource.url" => {
            configuration.process.database_path = normalize_database_path(&normalized_value);
        }
        "flowable.dmn.database-path" | "flowable.dmn.datasource.url" => {
            configuration.dmn.database_path = Some(normalize_database_path(&normalized_value));
        }
        "flowable.cmmn.database-path" | "flowable.cmmn.datasource.url" => {
            configuration.cmmn.database_path = Some(normalize_database_path(&normalized_value));
        }
        "flowable.app.database-path" | "flowable.app.datasource.url" => {
            configuration.app.database_path = Some(normalize_database_path(&normalized_value));
        }
        "flowable.security.auth-mode" => {
            configuration.security.auth_mode = normalized_value;
        }
        "flowable.http-service.enabled" => {
            configuration.http_service.enabled = parse_properties_bool(key, &normalized_value)?;
        }
        "flowable.http-service.supported-methods" => {
            configuration.http_service.supported_methods = normalized_value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect();
        }
        "flowable.http-service.runtime-mode" => {
            configuration.http_service.runtime_mode =
                parse_http_runtime_mode(key, &normalized_value)?;
        }
        "flowable.http-service.default-timeout-ms" => {
            configuration.http_service.default_timeout_ms =
                parse_properties_u64(key, &normalized_value)?;
        }
        "flowable.http-service.default-connect-timeout-ms" => {
            configuration.http_service.default_connect_timeout_ms =
                parse_properties_u64(key, &normalized_value)?;
        }
        "flowable.http-service.user-agent" => {
            configuration.http_service.user_agent = Some(normalized_value);
        }
        "flowable.embedding.mode" => {
            configuration.embedding.mode = normalized_value;
        }
        "flowable.embedding.profile" => {
            configuration.embedding.profile = normalized_value;
        }
        "flowable.enterprise.adapters" => {
            configuration.enterprise.adapters = normalized_value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect();
        }
        "flowable.directory.provider" => {
            configuration.directory.provider = normalized_value;
        }
        "flowable.directory.sync-on-bootstrap" => {
            configuration.directory.sync_on_bootstrap =
                parse_properties_bool(key, &normalized_value)?;
        }
        "flowable.directory.bundle-path" => {
            configuration.directory.bundle_path = Some(normalized_value);
        }
        "flowable.operations.exposure" => {
            configuration.operations.exposure = normalized_value;
        }
        "flowable.operations.management-api-enabled" => {
            configuration.operations.management_api_enabled =
                parse_properties_bool(key, &normalized_value)?;
        }
        "flowable.bootstrap.admin.enabled" => {
            configuration.bootstrap.create_default_admin =
                parse_properties_bool(key, &normalized_value)?;
        }
        "flowable.bootstrap.admin.user-id" => {
            configuration.bootstrap.admin_user_id = normalized_value;
        }
        "flowable.bootstrap.admin.password" => {
            configuration.bootstrap.admin_password = normalized_value;
        }
        _ => {}
    }

    Ok(())
}

fn resolve_runtime_embedding_contract(
    config: &PlatformConfiguration,
) -> Result<RuntimeEmbeddingContract, PlatformBootstrapError> {
    let mode = parse_runtime_embedding_mode(&config.embedding.mode)?;
    let profile = parse_runtime_embedding_profile(&config.embedding.profile)?;

    match (mode, profile) {
        (RuntimeEmbeddingMode::Standalone, RuntimeEmbeddingProfile::StandaloneService)
        | (RuntimeEmbeddingMode::Embedded, RuntimeEmbeddingProfile::CdiCompatible)
        | (RuntimeEmbeddingMode::Embedded, RuntimeEmbeddingProfile::OsgiManaged) => {}
        _ => {
            return Err(PlatformBootstrapError::new(format!(
                "Embedding mode '{}' is incompatible with profile '{}'",
                config.embedding.mode, config.embedding.profile
            )));
        }
    }

    let adapters = config
        .enterprise
        .adapters
        .iter()
        .map(|adapter| parse_enterprise_adapter_family(adapter))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(RuntimeEmbeddingContract {
        mode,
        profile,
        adapters,
    })
}

fn resolve_enterprise_support_contracts(
    embedding_contract: &RuntimeEmbeddingContract,
) -> Result<Vec<EnterpriseAdapterSupportContract>, PlatformBootstrapError> {
    let mut contracts = Vec::new();
    for family in &embedding_contract.adapters {
        let contract = match family {
            EnterpriseAdapterFamily::Camel => EnterpriseAdapterSupportContract {
                family: *family,
                support_kind: EnterpriseSupportKind::ReplacementArchitecture,
                supported_profiles: vec![
                    RuntimeEmbeddingProfile::StandaloneService,
                    RuntimeEmbeddingProfile::CdiCompatible,
                    RuntimeEmbeddingProfile::OsgiManaged,
                ],
                external_source_anchor: "org.flowable.camel.*",
                support_statement: "Camel is closed through a bounded message/http bridge replacement architecture on top of the shared runtime embedding contract.",
            },
            EnterpriseAdapterFamily::Cxf => EnterpriseAdapterSupportContract {
                family: *family,
                support_kind: EnterpriseSupportKind::ReplacementArchitecture,
                supported_profiles: vec![
                    RuntimeEmbeddingProfile::StandaloneService,
                    RuntimeEmbeddingProfile::CdiCompatible,
                    RuntimeEmbeddingProfile::OsgiManaged,
                ],
                external_source_anchor: "org.flowable.cxf.*",
                support_statement: "CXF is closed through a bounded REST/HTTP exposure replacement architecture on top of the shared runtime embedding contract.",
            },
            EnterpriseAdapterFamily::Cdi => EnterpriseAdapterSupportContract {
                family: *family,
                support_kind: EnterpriseSupportKind::CompatibilityLayer,
                supported_profiles: vec![RuntimeEmbeddingProfile::CdiCompatible],
                external_source_anchor: "org.flowable.cdi.*",
                support_statement: "CDI is closed through a bounded embedding adapter that maps CDI-style embedding to the explicit Rust runtime embedding contract.",
            },
            EnterpriseAdapterFamily::Osgi => EnterpriseAdapterSupportContract {
                family: *family,
                support_kind: EnterpriseSupportKind::ReplacementArchitecture,
                supported_profiles: vec![RuntimeEmbeddingProfile::OsgiManaged],
                external_source_anchor: "org.flowable.osgi.*",
                support_statement: "OSGi is closed through a bounded module-registry replacement architecture on top of the explicit Rust runtime embedding contract.",
            },
        };

        if !contract
            .supported_profiles
            .contains(&embedding_contract.profile)
        {
            return Err(PlatformBootstrapError::new(format!(
                "Enterprise adapter '{:?}' is not supported for embedding profile '{:?}'",
                family, embedding_contract.profile
            )));
        }

        contracts.push(contract);
    }

    Ok(contracts)
}

fn resolve_directory_support_contract(
    config: &PlatformConfiguration,
) -> Result<DirectorySupportContract, PlatformBootstrapError> {
    let provider = parse_directory_provider_kind(&config.directory.provider)?;
    if matches!(provider, DirectoryProviderKind::LdapMirror)
        && config.directory.sync_on_bootstrap
        && config.directory.bundle_path.is_none()
    {
        return Err(PlatformBootstrapError::new(
            "Directory provider 'ldap-mirror' requires a bundle path when sync_on_bootstrap is enabled",
        ));
    }
    if matches!(provider, DirectoryProviderKind::LdapLive) && config.directory.sync_on_bootstrap {
        return Err(PlatformBootstrapError::new(
            "Directory provider 'ldap-live' does not support bootstrap synchronization; use 'ldap-mirror' for imports",
        ));
    }
    if matches!(provider, DirectoryProviderKind::LdapLive) && config.directory.bundle_path.is_none()
    {
        return Err(PlatformBootstrapError::new(
            "Directory provider 'ldap-live' requires a bundle path for bounded runtime reads",
        ));
    }

    let (
        external_source_anchor,
        support_statement,
        transport,
        auth_mode,
        deployment_mode,
        conflict_policy,
        filter_breadth,
        runtime_user_read_enabled,
        runtime_group_read_enabled,
        runtime_membership_read_enabled,
        runtime_user_write_enabled,
        runtime_group_write_enabled,
        runtime_membership_write_enabled,
        runtime_reconcile_enabled,
        runtime_bidirectional_sync_enabled,
    ) = match provider {
        DirectoryProviderKind::Internal => (
            "org.flowable.idm.engine.impl.persistence.entity.*",
            "Internal directory mode stays on the owned identity store and does not claim external LDAP synchronization.",
            "in-process-store".to_string(),
            "owned-store".to_string(),
            "in-process".to_string(),
            "owned-wins".to_string(),
            "owned-surface-only".to_string(),
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        ),
        DirectoryProviderKind::LdapMirror => (
            "org.flowable.ldap.*",
            "LDAP-backed directory closure is provided through a bounded ldap-mirror import contract that synchronizes a portable directory bundle into the owned Rust identity store during bootstrap.",
            "bundle-import".to_string(),
            "bootstrap-service-account".to_string(),
            "bootstrap-import".to_string(),
            "owned-after-import".to_string(),
            "bootstrap-snapshot-only".to_string(),
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        ),
        DirectoryProviderKind::LdapLive => (
            "org.flowable.ldap.*",
            "LDAP live breadth closure is provided through a repository-defined runtime contract that covers identity-surface query breadth, write-through mutations, and bidirectional reconcile between the owned store and the declared live directory source. The contract is expressed through explicit transport, auth, deployment, conflict, and filter semantics rather than implicit vendor discovery.",
            config.directory.transport.clone(),
            config.directory.auth_mode.clone(),
            config.directory.deployment_mode.clone(),
            config.directory.conflict_policy.clone(),
            config.directory.filter_breadth.clone(),
            true,
            true,
            true,
            true,
            true,
            true,
            true,
            true,
        ),
    };

    Ok(DirectorySupportContract {
        provider,
        sync_on_bootstrap: config.directory.sync_on_bootstrap,
        bundle_path: config.directory.bundle_path.clone(),
        transport,
        auth_mode,
        deployment_mode,
        conflict_policy,
        filter_breadth,
        external_source_anchor,
        support_statement,
        imported_user_count: 0,
        imported_group_count: 0,
        imported_membership_count: 0,
        runtime_user_read_enabled,
        runtime_group_read_enabled,
        runtime_membership_read_enabled,
        runtime_user_write_enabled,
        runtime_group_write_enabled,
        runtime_membership_write_enabled,
        runtime_reconcile_enabled,
        runtime_bidirectional_sync_enabled,
    })
}

fn resolve_operations_support_contract(
    config: &PlatformConfiguration,
) -> Result<OperationsSupportContract, PlatformBootstrapError> {
    let exposure = parse_operations_exposure_kind(&config.operations.exposure)?;
    let (
        external_source_anchor,
        support_statement,
        runtime_ledger_enabled,
        timer_ledger_enabled,
        topology_ledger_enabled,
        native_compatible_connector_enabled,
        mbean_registry_enabled,
        operations_bus_enabled,
        object_family_breadth,
    ) = match exposure {
        OperationsExposureKind::MetricsOnly => (
            "org.flowable.common.rest.api.*",
            "Operations exposure is limited to bounded health/ready/metrics surfaces and does not expose the JMX management contract.",
            false,
            false,
            false,
            false,
            false,
            false,
            OperationsObjectFamilyBreadth::MetricsSurfacesOnly,
        ),
        OperationsExposureKind::JmxBridge => (
            "org.flowable.common.management.jmx.*",
            "JMX closure is provided through a bounded management API bridge that exposes runtime, timer-coordination, topology, and directory support ledgers over the owned Rust HTTP management surface. It does not include an external transport or complete MBean stack.",
            true,
            true,
            true,
            false,
            false,
            false,
            OperationsObjectFamilyBreadth::LedgersOnly,
        ),
        OperationsExposureKind::JmxNativeCompatible => (
            "org.flowable.common.management.jmx.*",
            "JMX closure is provided through an owned native-compatible connector, MBean registry, and operations bus that preserve ObjectName, attribute, and operation semantics for the core Flowable runtime families plus platform support ledgers. It does not emulate an external RMI connector server or claim a full remote process transport stack.",
            true,
            true,
            true,
            true,
            true,
            true,
            OperationsObjectFamilyBreadth::CoreRuntimeAndPlatformLedgers,
        ),
    };

    Ok(OperationsSupportContract {
        exposure,
        management_api_enabled: config.operations.management_api_enabled,
        external_source_anchor,
        support_statement,
        runtime_ledger_enabled,
        timer_ledger_enabled,
        topology_ledger_enabled,
        native_compatible_connector_enabled,
        mbean_registry_enabled,
        operations_bus_enabled,
        object_family_breadth,
    })
}

fn resolve_topology_certification_contract(
    config: &PlatformConfiguration,
    embedding_contract: &RuntimeEmbeddingContract,
    enterprise_support_contracts: &[EnterpriseAdapterSupportContract],
    directory_support_contract: &DirectorySupportContract,
    operations_support_contract: &OperationsSupportContract,
) -> Result<TopologyCertificationContract, PlatformBootstrapError> {
    let profile = parse_certified_topology_profile(&config.topology.profile)?;
    let declared_ingress = normalize_topology_descriptor_value(&config.topology.ingress);
    let declared_packaging = normalize_topology_descriptor_value(&config.topology.packaging);
    let contains_family = |family: EnterpriseAdapterFamily| {
        enterprise_support_contracts
            .iter()
            .any(|contract| contract.family == family)
    };

    let contract = match profile {
        CertifiedTopologyProfile::RepositoryDefined => TopologyCertificationContract {
            profile,
            ingress: "repository-defined".to_string(),
            packaging: "repository-defined".to_string(),
            external_source_anchor: "repository-defined support contract",
            support_statement: "Repository-defined external replacement remains certified for the explicit owned support contract only.",
            startup_certified: true,
            auth_certified: true,
            cutover_certified: true,
            rollback_certified: true,
            recovery_certified: true,
            supported_historical_ingress: vec![
                "sqlite-direct".to_string(),
                "portable-bundle".to_string(),
                "raw-mysql-dump".to_string(),
                "raw-postgres-dump".to_string(),
            ],
        },
        CertifiedTopologyProfile::ReverseProxyTerminated => {
            if embedding_contract.mode != RuntimeEmbeddingMode::Standalone
                || embedding_contract.profile != RuntimeEmbeddingProfile::StandaloneService
            {
                return Err(PlatformBootstrapError::new(
                    "Topology profile 'reverse-proxy-terminated' requires standalone / standalone-service embedding",
                ));
            }
            if config.security.auth_mode != "basic" {
                return Err(PlatformBootstrapError::new(
                    "Topology profile 'reverse-proxy-terminated' requires basic auth mode",
                ));
            }
            if !operations_support_contract.management_api_enabled
                || !supports_jmx_topology_contract(operations_support_contract.exposure)
            {
                return Err(PlatformBootstrapError::new(
                    "Topology profile 'reverse-proxy-terminated' requires management_api_enabled=true and operations exposure 'jmx-bridge' or 'jmx-native-compatible'",
                ));
            }
            TopologyCertificationContract {
                profile,
                ingress: "reverse-proxy-terminated".to_string(),
                packaging: "standalone-service".to_string(),
                external_source_anchor: "org.flowable.spring.boot.* reverse-proxy deployments",
                support_statement: "Reverse-proxy-terminated standalone service is certified as a bounded contract-external topology with owned auth, management bridge, removed alias guardrails, and supported historical ingress.",
                startup_certified: true,
                auth_certified: true,
                cutover_certified: true,
                rollback_certified: true,
                recovery_certified: true,
                supported_historical_ingress: vec![
                    "sqlite-direct".to_string(),
                    "portable-bundle".to_string(),
                    "raw-mysql-dump".to_string(),
                    "raw-postgres-dump".to_string(),
                ],
            }
        }
        CertifiedTopologyProfile::CdiSidecar => {
            if embedding_contract.mode != RuntimeEmbeddingMode::Embedded
                || embedding_contract.profile != RuntimeEmbeddingProfile::CdiCompatible
            {
                return Err(PlatformBootstrapError::new(
                    "Topology profile 'cdi-sidecar' requires embedded / cdi-compatible embedding",
                ));
            }
            if !contains_family(EnterpriseAdapterFamily::Cdi) {
                return Err(PlatformBootstrapError::new(
                    "Topology profile 'cdi-sidecar' requires the 'cdi' enterprise adapter",
                ));
            }
            if !operations_support_contract.management_api_enabled {
                return Err(PlatformBootstrapError::new(
                    "Topology profile 'cdi-sidecar' requires management_api_enabled=true",
                ));
            }
            TopologyCertificationContract {
                profile,
                ingress: "sidecar-local".to_string(),
                packaging: "embedded-sidecar".to_string(),
                external_source_anchor: "org.flowable.cdi.* sidecar topologies",
                support_statement: "Embedded CDI sidecar deployment is certified as a bounded contract-external topology on top of the owned embedding, native management boundary, and directory/operations bridges.",
                startup_certified: true,
                auth_certified: true,
                cutover_certified: false,
                rollback_certified: false,
                recovery_certified: true,
                supported_historical_ingress: vec!["portable-bundle".to_string()],
            }
        }
        CertifiedTopologyProfile::OsgiOperationsNode => {
            if embedding_contract.mode != RuntimeEmbeddingMode::Embedded
                || embedding_contract.profile != RuntimeEmbeddingProfile::OsgiManaged
            {
                return Err(PlatformBootstrapError::new(
                    "Topology profile 'osgi-operations-node' requires embedded / osgi-managed embedding",
                ));
            }
            if !contains_family(EnterpriseAdapterFamily::Osgi) {
                return Err(PlatformBootstrapError::new(
                    "Topology profile 'osgi-operations-node' requires the 'osgi' enterprise adapter",
                ));
            }
            if !operations_support_contract.management_api_enabled
                || !supports_jmx_topology_contract(operations_support_contract.exposure)
            {
                return Err(PlatformBootstrapError::new(
                    "Topology profile 'osgi-operations-node' requires management_api_enabled=true and operations exposure 'jmx-bridge' or 'jmx-native-compatible'",
                ));
            }
            TopologyCertificationContract {
                profile,
                ingress: "operations-isolated".to_string(),
                packaging: "operations-node".to_string(),
                external_source_anchor: "org.flowable.osgi.* operations nodes",
                support_statement: "OSGi-managed operations node is certified as a bounded contract-external topology for startup, operations bridge, recovery, and removed management alias boundary closure.",
                startup_certified: true,
                auth_certified: true,
                cutover_certified: false,
                rollback_certified: false,
                recovery_certified: true,
                supported_historical_ingress: if directory_support_contract.provider
                    == DirectoryProviderKind::LdapMirror
                {
                    vec!["portable-bundle".to_string()]
                } else {
                    Vec::new()
                },
            }
        }
        CertifiedTopologyProfile::DeclaredExternal => {
            let ingress = parse_declared_topology_ingress(&declared_ingress)?;
            let packaging = parse_declared_topology_packaging(&declared_packaging)?;
            let supported_historical_ingress = vec![
                "source-manifest".to_string(),
                "sqlite-direct".to_string(),
                "sqlite-dump".to_string(),
                "portable-bundle".to_string(),
                "raw-mysql-dump".to_string(),
                "raw-postgres-dump".to_string(),
            ];

            match packaging {
                DeclaredTopologyPackaging::StandaloneService => {
                    if embedding_contract.mode != RuntimeEmbeddingMode::Standalone
                        || embedding_contract.profile != RuntimeEmbeddingProfile::StandaloneService
                    {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'standalone-service' requires standalone / standalone-service embedding",
                        ));
                    }
                    if config.security.auth_mode != "basic" {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'standalone-service' requires basic auth mode",
                        ));
                    }
                    if !operations_support_contract.management_api_enabled
                        || !supports_jmx_topology_contract(operations_support_contract.exposure)
                    {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'standalone-service' requires management_api_enabled=true and operations exposure 'jmx-bridge' or 'jmx-native-compatible'",
                        ));
                    }
                    if !matches!(
                        ingress,
                        DeclaredTopologyIngress::ReverseProxyTerminated
                            | DeclaredTopologyIngress::ServiceMeshTerminated
                    ) {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'standalone-service' requires ingress 'reverse-proxy-terminated' or 'service-mesh-terminated'",
                        ));
                    }

                    TopologyCertificationContract {
                        profile,
                        ingress: declared_topology_ingress_name(ingress).to_string(),
                        packaging: declared_topology_packaging_name(packaging).to_string(),
                        external_source_anchor: "org.flowable.spring.boot.* external ingress deployments",
                        support_statement: "Declared external standalone service topology is certified for reverse-proxy/service-mesh ingress, owned auth, removed management alias boundary, recovery, and source-manifest historical ingress.",
                        startup_certified: true,
                        auth_certified: true,
                        cutover_certified: true,
                        rollback_certified: true,
                        recovery_certified: true,
                        supported_historical_ingress,
                    }
                }
                DeclaredTopologyPackaging::EmbeddedSidecar => {
                    if embedding_contract.mode != RuntimeEmbeddingMode::Embedded
                        || embedding_contract.profile != RuntimeEmbeddingProfile::CdiCompatible
                    {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'embedded-sidecar' requires embedded / cdi-compatible embedding",
                        ));
                    }
                    if !contains_family(EnterpriseAdapterFamily::Cdi) {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'embedded-sidecar' requires the 'cdi' enterprise adapter",
                        ));
                    }
                    if ingress != DeclaredTopologyIngress::SidecarLocal {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'embedded-sidecar' requires ingress 'sidecar-local'",
                        ));
                    }
                    if !operations_support_contract.management_api_enabled {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'embedded-sidecar' requires management_api_enabled=true",
                        ));
                    }

                    TopologyCertificationContract {
                        profile,
                        ingress: declared_topology_ingress_name(ingress).to_string(),
                        packaging: declared_topology_packaging_name(packaging).to_string(),
                        external_source_anchor: "org.flowable.cdi.* sidecar families",
                        support_statement: "Declared external embedded sidecar topology is certified for owned embedding, bounded sidecar-local ingress, removed management alias boundary, and recovery.",
                        startup_certified: true,
                        auth_certified: true,
                        cutover_certified: false,
                        rollback_certified: false,
                        recovery_certified: true,
                        supported_historical_ingress: vec!["portable-bundle".to_string()],
                    }
                }
                DeclaredTopologyPackaging::OperationsNode => {
                    if embedding_contract.mode != RuntimeEmbeddingMode::Embedded
                        || embedding_contract.profile != RuntimeEmbeddingProfile::OsgiManaged
                    {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'operations-node' requires embedded / osgi-managed embedding",
                        ));
                    }
                    if !contains_family(EnterpriseAdapterFamily::Osgi) {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'operations-node' requires the 'osgi' enterprise adapter",
                        ));
                    }
                    if ingress != DeclaredTopologyIngress::OperationsIsolated {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'operations-node' requires ingress 'operations-isolated'",
                        ));
                    }
                    if !operations_support_contract.management_api_enabled
                        || !supports_jmx_topology_contract(operations_support_contract.exposure)
                    {
                        return Err(PlatformBootstrapError::new(
                            "Topology profile 'declared-external' with packaging 'operations-node' requires management_api_enabled=true and operations exposure 'jmx-bridge' or 'jmx-native-compatible'",
                        ));
                    }

                    TopologyCertificationContract {
                        profile,
                        ingress: declared_topology_ingress_name(ingress).to_string(),
                        packaging: declared_topology_packaging_name(packaging).to_string(),
                        external_source_anchor: "org.flowable.osgi.* external operations nodes",
                        support_statement: "Declared external operations-node topology is certified for isolated operations ingress, owned management bridges, recovery, and removed alias guardrails.",
                        startup_certified: true,
                        auth_certified: true,
                        cutover_certified: false,
                        rollback_certified: false,
                        recovery_certified: true,
                        supported_historical_ingress: if directory_support_contract.provider
                            == DirectoryProviderKind::LdapMirror
                        {
                            vec!["portable-bundle".to_string()]
                        } else {
                            Vec::new()
                        },
                    }
                }
            }
        }
    };

    Ok(contract)
}

fn parse_runtime_embedding_mode(
    value: &str,
) -> Result<RuntimeEmbeddingMode, PlatformBootstrapError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "standalone" => Ok(RuntimeEmbeddingMode::Standalone),
        "embedded" => Ok(RuntimeEmbeddingMode::Embedded),
        _ => Err(PlatformBootstrapError::new(format!(
            "Unsupported embedding mode '{}'",
            value
        ))),
    }
}

fn parse_runtime_embedding_profile(
    value: &str,
) -> Result<RuntimeEmbeddingProfile, PlatformBootstrapError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "standalone-service" => Ok(RuntimeEmbeddingProfile::StandaloneService),
        "cdi-compatible" => Ok(RuntimeEmbeddingProfile::CdiCompatible),
        "osgi-managed" => Ok(RuntimeEmbeddingProfile::OsgiManaged),
        _ => Err(PlatformBootstrapError::new(format!(
            "Unsupported embedding profile '{}'",
            value
        ))),
    }
}

fn parse_certified_topology_profile(
    value: &str,
) -> Result<CertifiedTopologyProfile, PlatformBootstrapError> {
    match value {
        "repository-defined" => Ok(CertifiedTopologyProfile::RepositoryDefined),
        "reverse-proxy-terminated" => Ok(CertifiedTopologyProfile::ReverseProxyTerminated),
        "cdi-sidecar" => Ok(CertifiedTopologyProfile::CdiSidecar),
        "osgi-operations-node" => Ok(CertifiedTopologyProfile::OsgiOperationsNode),
        "declared-external" => Ok(CertifiedTopologyProfile::DeclaredExternal),
        other => Err(PlatformBootstrapError::new(format!(
            "Unsupported topology profile '{}'",
            other
        ))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclaredTopologyIngress {
    ReverseProxyTerminated,
    ServiceMeshTerminated,
    SidecarLocal,
    OperationsIsolated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclaredTopologyPackaging {
    StandaloneService,
    EmbeddedSidecar,
    OperationsNode,
}

fn normalize_topology_descriptor_value(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn parse_declared_topology_ingress(
    value: &str,
) -> Result<DeclaredTopologyIngress, PlatformBootstrapError> {
    match value {
        "reverse-proxy-terminated" => Ok(DeclaredTopologyIngress::ReverseProxyTerminated),
        "service-mesh-terminated" => Ok(DeclaredTopologyIngress::ServiceMeshTerminated),
        "sidecar-local" => Ok(DeclaredTopologyIngress::SidecarLocal),
        "operations-isolated" => Ok(DeclaredTopologyIngress::OperationsIsolated),
        other => Err(PlatformBootstrapError::new(format!(
            "Unsupported declared topology ingress '{}'",
            other
        ))),
    }
}

fn parse_declared_topology_packaging(
    value: &str,
) -> Result<DeclaredTopologyPackaging, PlatformBootstrapError> {
    match value {
        "standalone-service" => Ok(DeclaredTopologyPackaging::StandaloneService),
        "embedded-sidecar" => Ok(DeclaredTopologyPackaging::EmbeddedSidecar),
        "operations-node" => Ok(DeclaredTopologyPackaging::OperationsNode),
        other => Err(PlatformBootstrapError::new(format!(
            "Unsupported declared topology packaging '{}'",
            other
        ))),
    }
}

fn declared_topology_ingress_name(value: DeclaredTopologyIngress) -> &'static str {
    match value {
        DeclaredTopologyIngress::ReverseProxyTerminated => "reverse-proxy-terminated",
        DeclaredTopologyIngress::ServiceMeshTerminated => "service-mesh-terminated",
        DeclaredTopologyIngress::SidecarLocal => "sidecar-local",
        DeclaredTopologyIngress::OperationsIsolated => "operations-isolated",
    }
}

fn declared_topology_packaging_name(value: DeclaredTopologyPackaging) -> &'static str {
    match value {
        DeclaredTopologyPackaging::StandaloneService => "standalone-service",
        DeclaredTopologyPackaging::EmbeddedSidecar => "embedded-sidecar",
        DeclaredTopologyPackaging::OperationsNode => "operations-node",
    }
}

fn parse_enterprise_adapter_family(
    value: &str,
) -> Result<EnterpriseAdapterFamily, PlatformBootstrapError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "camel" => Ok(EnterpriseAdapterFamily::Camel),
        "cxf" => Ok(EnterpriseAdapterFamily::Cxf),
        "cdi" => Ok(EnterpriseAdapterFamily::Cdi),
        "osgi" => Ok(EnterpriseAdapterFamily::Osgi),
        _ => Err(PlatformBootstrapError::new(format!(
            "Unsupported enterprise adapter '{}'",
            value
        ))),
    }
}

fn parse_directory_provider_kind(
    value: &str,
) -> Result<DirectoryProviderKind, PlatformBootstrapError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "internal" => Ok(DirectoryProviderKind::Internal),
        "ldap-mirror" => Ok(DirectoryProviderKind::LdapMirror),
        "ldap-live" => Ok(DirectoryProviderKind::LdapLive),
        _ => Err(PlatformBootstrapError::new(format!(
            "Unsupported directory provider '{}'",
            value
        ))),
    }
}

fn parse_operations_exposure_kind(
    value: &str,
) -> Result<OperationsExposureKind, PlatformBootstrapError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "metrics-only" => Ok(OperationsExposureKind::MetricsOnly),
        "jmx-bridge" => Ok(OperationsExposureKind::JmxBridge),
        "jmx-native-compatible" => Ok(OperationsExposureKind::JmxNativeCompatible),
        _ => Err(PlatformBootstrapError::new(format!(
            "Unsupported operations exposure '{}'",
            value
        ))),
    }
}

fn supports_jmx_topology_contract(exposure: OperationsExposureKind) -> bool {
    matches!(
        exposure,
        OperationsExposureKind::JmxBridge | OperationsExposureKind::JmxNativeCompatible
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DirectoryBundle {
    #[serde(default)]
    users: Vec<User>,
    #[serde(default)]
    groups: Vec<Group>,
    #[serde(default)]
    memberships: Vec<Membership>,
}

fn build_live_directory_provider(
    config: &DirectoryConfiguration,
) -> Result<Option<Arc<BoundedLdapLiveDirectoryProvider>>, PlatformBootstrapError> {
    if !matches!(
        parse_directory_provider_kind(&config.provider)?,
        DirectoryProviderKind::LdapLive
    ) {
        return Ok(None);
    }

    let bundle_path = config.bundle_path.as_ref().ok_or_else(|| {
        PlatformBootstrapError::new(
            "Directory provider 'ldap-live' requires a bundle path for bounded runtime directory operations",
        )
    })?;
    let provider = Arc::new(BoundedLdapLiveDirectoryProvider::new(PathBuf::from(
        bundle_path,
    )));
    provider.load_snapshot()?;
    Ok(Some(provider))
}

fn import_directory_bundle(
    process_engine: &ProcessEngine,
    config: &DirectoryConfiguration,
    directory_support_contract: &mut DirectorySupportContract,
) -> Result<(), PlatformBootstrapError> {
    if !directory_support_contract.sync_on_bootstrap {
        return Ok(());
    }
    if !matches!(
        directory_support_contract.provider,
        DirectoryProviderKind::LdapMirror
    ) {
        return Ok(());
    }

    let bundle_path = config.bundle_path.as_ref().ok_or_else(|| {
        PlatformBootstrapError::new(
            "Directory provider 'ldap-mirror' requires a bundle path when sync_on_bootstrap is enabled",
        )
    })?;
    let bundle = load_directory_bundle(Path::new(bundle_path))?;

    let identity_service = process_engine.get_identity_service();
    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let imported_user_count = bundle.users.len();
    let imported_group_count = bundle.groups.len();
    let imported_membership_count = bundle.memberships.len();

    for user in bundle.users {
        identity_service.save_user_in_session(user, &mut session);
    }
    for group in bundle.groups {
        identity_service.save_group_in_session(group, &mut session);
    }
    for membership in bundle.memberships {
        identity_service.create_membership_in_session(
            membership.user_id,
            membership.group_id,
            &mut session,
        );
    }
    session.flush_and_commit().unwrap();

    directory_support_contract.imported_user_count = imported_user_count;
    directory_support_contract.imported_group_count = imported_group_count;
    directory_support_contract.imported_membership_count = imported_membership_count;
    Ok(())
}

fn load_directory_bundle(path: &Path) -> Result<DirectoryBundle, PlatformBootstrapError> {
    let contents = std::fs::read_to_string(path).map_err(|error| {
        PlatformBootstrapError::new(format!(
            "Failed to read directory bundle '{}': {}",
            path.display(),
            error
        ))
    })?;
    toml::from_str(&contents).map_err(|error| {
        PlatformBootstrapError::new(format!(
            "Failed to parse directory bundle '{}': {}",
            path.display(),
            error
        ))
    })
}

fn persist_directory_bundle(
    path: &Path,
    bundle: &DirectoryBundle,
) -> Result<(), LiveDirectoryMutationError> {
    let contents = toml::to_string_pretty(bundle).map_err(|error| {
        LiveDirectoryMutationError::Storage(format!(
            "Failed to serialize directory bundle '{}': {}",
            path.display(),
            error
        ))
    })?;
    std::fs::write(path, contents).map_err(|error| {
        LiveDirectoryMutationError::Storage(format!(
            "Failed to write directory bundle '{}': {}",
            path.display(),
            error
        ))
    })
}

fn normalize_directory_bundle(bundle: &mut DirectoryBundle) {
    let mut unique_users = BTreeMap::new();
    for user in bundle.users.drain(..) {
        unique_users.insert(user.id.clone(), user);
    }
    bundle.users = unique_users.into_values().collect();

    let mut unique_groups = BTreeMap::new();
    for group in bundle.groups.drain(..) {
        unique_groups.insert(group.id.clone(), group);
    }
    bundle.groups = unique_groups.into_values().collect();

    let mut unique_memberships = BTreeMap::new();
    for membership in bundle.memberships.drain(..) {
        unique_memberships.insert(
            (membership.user_id.clone(), membership.group_id.clone()),
            membership,
        );
    }
    bundle.memberships = unique_memberships.into_values().collect();
}

fn normalize_properties_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 {
        let quoted = (trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\''));
        if quoted {
            return trimmed[1..trimmed.len() - 1].trim().to_string();
        }
    }

    trimmed.to_string()
}

fn normalize_database_path(value: &str) -> String {
    value
        .strip_prefix("jdbc:sqlite:")
        .or_else(|| value.strip_prefix("sqlite:"))
        .unwrap_or(value)
        .to_string()
}

fn merge_bind_address(
    current_bind_address: &str,
    address: Option<&str>,
    port: Option<&str>,
) -> Result<String, PlatformBootstrapError> {
    let (current_address, current_port) = split_bind_address(current_bind_address)?;
    let resolved_port = match port {
        Some(port) => port.parse::<u16>().map_err(|_| {
            PlatformBootstrapError::new(format!(
                "Property 'server.port' must be a valid u16, got '{port}'"
            ))
        })?,
        None => current_port.parse::<u16>().map_err(|_| {
            PlatformBootstrapError::new(format!(
                "Current bind address '{}' contains an invalid port",
                current_bind_address
            ))
        })?,
    };

    Ok(format!(
        "{}:{}",
        address.unwrap_or(current_address.as_str()),
        resolved_port
    ))
}

fn split_bind_address(bind_address: &str) -> Result<(String, String), PlatformBootstrapError> {
    if bind_address.starts_with('[')
        && let Some(index) = bind_address.rfind("]:")
    {
        return Ok((
            bind_address[..index + 1].to_string(),
            bind_address[index + 2..].to_string(),
        ));
    }

    bind_address
        .rsplit_once(':')
        .map(|(address, port)| (address.to_string(), port.to_string()))
        .ok_or_else(|| {
            PlatformBootstrapError::new(format!(
                "Bind address '{}' must be in host:port form",
                bind_address
            ))
        })
}

fn parse_properties_bool(key: &str, value: &str) -> Result<bool, PlatformBootstrapError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(PlatformBootstrapError::new(format!(
            "Property '{key}' must be a boolean, got '{value}'"
        ))),
    }
}

fn parse_properties_u64(key: &str, value: &str) -> Result<u64, PlatformBootstrapError> {
    value.trim().parse::<u64>().map_err(|error| {
        PlatformBootstrapError::new(format!(
            "Property '{key}' must be an unsigned integer, got '{value}': {error}"
        ))
    })
}

fn parse_http_runtime_mode(
    key: &str,
    value: &str,
) -> Result<HttpServiceRuntimeMode, PlatformBootstrapError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "deterministic" | "mock" | "stub" => Ok(HttpServiceRuntimeMode::Deterministic),
        "real" | "http" => Ok(HttpServiceRuntimeMode::Real),
        "async" | "pooled" => Ok(HttpServiceRuntimeMode::Async),
        _ => Err(PlatformBootstrapError::new(format!(
            "Property '{key}' must be one of deterministic, real, or async, got '{value}'"
        ))),
    }
}
